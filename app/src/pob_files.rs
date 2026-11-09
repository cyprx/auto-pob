//! Download and pin the Path of Building files this app runs against.

use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use anyhow::Context;
use flate2::read::GzDecoder;

use crate::game::Game;

const COMPLETE_MARKER: &str = ".complete";
const ENV_POB_DIR: &str = "AUTO_POB_POB_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadProgress {
    Downloading { done: u64, total: Option<u64> },
    Extracting,
}

/// Returns the PoB folder: `AUTO_POB_POB_DIR` if set, else the pinned install (downloaded on
/// first use).
pub fn ensure_pob(
    game: Game,
    data_dir: &Path,
    progress: &mut dyn FnMut(DownloadProgress),
) -> anyhow::Result<PathBuf> {
    // PoB changes the working directory, so a relative override must be made absolute first
    let override_dir = std::env::var_os(ENV_POB_DIR)
        .map(|dir| std::path::absolute(&dir).unwrap_or_else(|_| PathBuf::from(dir)));
    ensure_pob_with(
        game,
        data_dir,
        override_dir,
        &mut http_fetch(game),
        progress,
    )
}

type Fetch<'a> = &'a mut dyn FnMut() -> anyhow::Result<Box<dyn Read>>;

fn ensure_pob_with(
    game: Game,
    data_dir: &Path,
    override_dir: Option<PathBuf>,
    fetch: Fetch<'_>,
    progress: &mut dyn FnMut(DownloadProgress),
) -> anyhow::Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir);
    }
    let dir = game.install_dir(data_dir);
    if dir.join(COMPLETE_MARKER).exists() {
        return Ok(dir);
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("remove partial {}", dir.display()))?;
    }
    fs::create_dir_all(&dir)?;

    let reader = fetch()?;
    let mut done = 0u64;
    let counting = CountingReader {
        inner: reader,
        on_read: &mut |n| {
            done += n as u64;
            progress(DownloadProgress::Downloading { done, total: None });
        },
    };
    extract(counting, &dir).context("extract PoB tarball")?;
    progress(DownloadProgress::Extracting);
    fs::write(dir.join(COMPLETE_MARKER), game.pob_version())?;
    Ok(dir)
}

struct CountingReader<'a> {
    inner: Box<dyn Read>,
    on_read: &'a mut dyn FnMut(usize),
}

impl Read for CountingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            (self.on_read)(n);
        }
        Ok(n)
    }
}

fn http_fetch(game: Game) -> impl FnMut() -> anyhow::Result<Box<dyn Read>> {
    move || {
        let url = format!(
            "https://github.com/{}/archive/refs/tags/v{}.tar.gz",
            game.repo(),
            game.pob_version()
        );
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        let response = client.get(&url).send()?.error_for_status()?;
        Ok(Box::new(response) as Box<dyn Read>)
    }
}

/// Downloads the pinned PoB tarball for `game` and extracts it into `dest` (no marker, no cache).
pub fn download_into(game: Game, dest: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest)?;
    extract(http_fetch(game)()?, dest).context("extract PoB tarball")
}

/// Where a tarball entry (root folder already stripped) lands, or `None` to skip it.
fn map_entry(relative: &Path, dest: &Path) -> Option<PathBuf> {
    // Reject anything but plain components (no `..`, no absolute paths).
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    if let Ok(tail) = relative.strip_prefix("src") {
        return (!tail.as_os_str().is_empty()).then(|| dest.join(tail));
    }
    if let Ok(tail) = relative.strip_prefix("runtime/lua") {
        return (!tail.as_os_str().is_empty()).then(|| dest.join("lua").join(tail));
    }
    (relative == Path::new("manifest.xml") || relative == Path::new("LICENSE.md"))
        .then(|| dest.join(relative))
}

fn extract(tarball: impl Read, dest: &Path) -> anyhow::Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let mut parts = path.components();
        parts.next(); // root folder
        let Some(target) = map_entry(parts.as_path(), dest) else {
            continue;
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&target)?;
        io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::{fs, io::Read, path::PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("auto-pob-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn tarball() -> Vec<u8> {
        let mut b = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (path, data) in [
            ("PathOfBuilding-2.67.2/src/Launch.lua", "launch"),
            ("PathOfBuilding-2.67.2/src/Classes/Item.lua", "item"),
            ("PathOfBuilding-2.67.2/runtime/lua/xml.lua", "xml"),
            ("PathOfBuilding-2.67.2/manifest.xml", "<m/>"),
            ("PathOfBuilding-2.67.2/docs/x.md", "doc"),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data.as_bytes()).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn noop(_: DownloadProgress) {}

    #[test]
    fn extract_maps_src_and_runtime_lua() {
        let dest = scratch("extract");
        extract(&tarball()[..], &dest).unwrap();
        assert!(dest.join("Launch.lua").exists());
        assert!(dest.join("Classes/Item.lua").exists());
        assert!(dest.join("lua/xml.lua").exists());
        assert!(dest.join("manifest.xml").exists());
        assert!(!dest.join("docs").exists());
        assert!(!dest.join("src").exists());
    }

    /// A reader that yields `data`, then fails.
    struct FailingAfter(std::io::Cursor<Vec<u8>>);

    impl Read for FailingAfter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.read(buf)? {
                0 => Err(io::Error::other("connection lost")),
                n => Ok(n),
            }
        }
    }

    #[test]
    fn failed_or_truncated_fetch_leaves_no_marker() {
        let bytes = tarball();
        type Attempt = Box<dyn Fn() -> anyhow::Result<Box<dyn Read>>>;
        let attempts: [Attempt; 3] = [
            Box::new(|| anyhow::bail!("offline")),
            // cut off mid-stream by an error
            Box::new({
                let cut = bytes[..bytes.len() / 2].to_vec();
                move || {
                    Ok(Box::new(FailingAfter(std::io::Cursor::new(cut.clone()))) as Box<dyn Read>)
                }
            }),
            // cut off cleanly (a short body)
            Box::new({
                let cut = bytes[..bytes.len() / 2].to_vec();
                move || Ok(Box::new(std::io::Cursor::new(cut.clone())) as Box<dyn Read>)
            }),
        ];
        for (i, fetch) in attempts.iter().enumerate() {
            let data = scratch(&format!("truncated{i}"));
            let result = ensure_pob_with(Game::Poe1, &data, None, &mut || fetch(), &mut noop);
            assert!(result.is_err(), "attempt {i} succeeded");
            let dir = Game::Poe1.install_dir(&data);
            assert!(!dir.join(".complete").exists(), "attempt {i} left a marker");
        }
    }

    #[test]
    fn entries_escaping_the_destination_are_not_written() {
        let outer = scratch("escape");
        let dest = outer.join("dest");
        fs::create_dir_all(&dest).unwrap();
        let mut b = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (path, data) in [
            ("Root/src/../../evil.txt", "evil"),
            ("Root/src/ok.lua", "ok"),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            // set_path refuses `..`, so write the raw name bytes
            h.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            h.set_cksum();
            b.append(&h, data.as_bytes()).unwrap();
        }
        let bytes = b.into_inner().unwrap().finish().unwrap();
        extract(&bytes[..], &dest).unwrap();
        assert!(dest.join("ok.lua").exists());
        assert!(!outer.join("evil.txt").exists());
        assert!(!dest.join("evil.txt").exists());
        // and the mapping itself rejects `..` and absolute paths
        assert_eq!(map_entry(Path::new("src/../../x"), &dest), None);
        assert_eq!(map_entry(Path::new("/etc/passwd"), &dest), None);
        assert_eq!(map_entry(Path::new("src/a/../b"), &dest), None);
    }

    #[test]
    fn incomplete_install_is_redone() {
        let data = scratch("incomplete");
        let dir = Game::Poe1.install_dir(&data);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("stray.txt"), "x").unwrap();
        let bytes = tarball();
        let got = ensure_pob_with(
            Game::Poe1,
            &data,
            None,
            &mut || Ok(Box::new(std::io::Cursor::new(bytes.clone())) as Box<dyn Read>),
            &mut noop,
        )
        .unwrap();
        assert_eq!(got, dir);
        assert!(!dir.join("stray.txt").exists());
        assert!(dir.join("Launch.lua").exists());
        assert!(dir.join(".complete").exists());
    }

    #[test]
    fn complete_install_is_reused() {
        let data = scratch("complete");
        let dir = Game::Poe1.install_dir(&data);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".complete"), "").unwrap();
        let got = ensure_pob_with(
            Game::Poe1,
            &data,
            None,
            &mut || panic!("must not fetch"),
            &mut noop,
        )
        .unwrap();
        assert_eq!(got, dir);
    }

    #[test]
    fn env_override_wins() {
        let data = scratch("env");
        let over = PathBuf::from("/somewhere/pob");
        let got = ensure_pob_with(
            Game::Poe1,
            &data,
            Some(over.clone()),
            &mut || panic!("must not fetch"),
            &mut noop,
        )
        .unwrap();
        assert_eq!(got, over);
        assert!(!Game::Poe1.install_dir(&data).exists());
    }

    #[test]
    #[ignore = "needs network"]
    fn downloads_pinned_pob() {
        let data = scratch("net");
        let dir = ensure_pob_with(
            Game::Poe1,
            &data,
            None,
            &mut http_fetch(Game::Poe1),
            &mut noop,
        )
        .unwrap();
        assert!(dir.join("Launch.lua").exists());
        assert!(dir.join("HeadlessWrapper.lua").exists());
    }

    #[test]
    #[ignore = "needs network; populates the real data dir"]
    fn install_pinned_pob_into_data_dir() {
        let base = directories::BaseDirs::new().unwrap();
        let started = std::time::Instant::now();
        let mut bytes = 0u64;
        let dir = ensure_pob(Game::Poe1, base.data_dir(), &mut |p| {
            if let DownloadProgress::Downloading { done, .. } = p {
                bytes = done;
            }
        })
        .unwrap();
        eprintln!("downloaded {bytes} bytes in {:?}", started.elapsed());
        assert!(dir.join(".complete").exists());
    }
}
