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
    // the Help window's text and version history (Main.lua:1336-1376)
    ["manifest.xml", "LICENSE.md", "help.txt", "changelog.txt"]
        .iter()
        .any(|name| relative == Path::new(name))
        .then(|| dest.join(relative))
}

fn extract(tarball: impl Read, dest: &Path) -> anyhow::Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(tarball));
    // the help files are checked against the manifest, which may come after them
    let mut held: Vec<(&'static str, Vec<u8>)> = Vec::new();
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
        if let Some(&name) = HELP_FILES.iter().find(|n| parts.as_path() == Path::new(n)) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            held.push((name, bytes));
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&target)?;
        io::copy(&mut entry, &mut file)?;
    }
    // a help file that doesn't match the manifest is left out: the Help window offers to
    // fetch it
    let manifest = fs::read_to_string(dest.join("manifest.xml")).unwrap_or_default();
    for (name, bytes) in held {
        if verify(&manifest, name, &bytes).is_ok() {
            write_replacing(&dest.join(name), &bytes)?;
        } else {
            log::warn!("{name} in the PoB download doesn't match its manifest; left out");
        }
    }
    Ok(())
}

/// PoB's files the Help window reads (Main.lua:1336-1376)
pub const HELP_FILES: [&str; 2] = ["help.txt", "changelog.txt"];

/// The PoB folder `ensure_pob` runs from, without downloading anything.
pub fn pob_dir(game: Game, data_dir: &Path) -> PathBuf {
    std::env::var_os(ENV_POB_DIR)
        .map(|dir| std::path::absolute(&dir).unwrap_or_else(|_| PathBuf::from(dir)))
        .unwrap_or_else(|| game.install_dir(data_dir))
}

/// The user asked for PoB's missing help files: fetches each from the pinned release.
pub fn download_help_files(game: Game, dir: &Path) -> anyhow::Result<Vec<String>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    top_up_help_files(dir, &mut |name| {
        let url = format!(
            "https://raw.githubusercontent.com/{}/v{}/{name}",
            game.repo(),
            game.pob_version()
        );
        let response = client.get(&url).send()?.error_for_status()?;
        Ok(response.bytes()?.to_vec())
    })
}

/// Fetches the help files `dir` lacks, checks each against `dir`'s manifest.xml and only
/// then writes them, each through a temp file and a rename. A file that doesn't match writes
/// nothing at all. Returns the names written.
pub fn top_up_help_files(
    dir: &Path,
    fetch: &mut dyn FnMut(&str) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<Vec<String>> {
    let manifest =
        fs::read_to_string(dir.join("manifest.xml")).context("PoB's manifest.xml can't be read")?;
    let mut fetched = Vec::new();
    for name in HELP_FILES {
        if dir.join(name).exists() {
            continue;
        }
        let bytes = fetch(name).with_context(|| format!("couldn't download {name}"))?;
        verify(&manifest, name, &bytes)?;
        fetched.push((name, bytes));
    }
    for (name, bytes) in &fetched {
        write_replacing(&dir.join(name), bytes)?;
    }
    Ok(fetched.into_iter().map(|(n, _)| n.to_owned()).collect())
}

/// `bytes` are the manifest's `name`: its sha1 is the manifest's, of the bytes or of them
/// with CRLF line ends, as PoB's updater takes them (UpdateCheck.lua:191).
fn verify(manifest: &str, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let tag = format!("name=\"{name}\"");
    let want = manifest
        .lines()
        .find(|l| l.contains(&tag))
        .and_then(|l| l.split("sha1=\"").nth(1))
        .and_then(|rest| rest.split('"').next())
        .with_context(|| format!("{name} isn't in PoB's manifest"))?;
    let mut crlf = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    for &b in bytes {
        if b == b'\n' {
            crlf.push(b'\r');
        }
        crlf.push(b);
    }
    if sha1_hex(bytes) == want || sha1_hex(&crlf) == want {
        Ok(())
    } else {
        anyhow::bail!("{name} doesn't match PoB's manifest")
    }
}

/// Writes `path` through a temp file next to it and a rename, so it is whole or absent.
fn write_replacing(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("download");
    fs::write(&tmp, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// SHA-1 (FIPS 180-4) as lowercase hex: only to compare PoB's files with its manifest.
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
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
            ("PathOfBuilding-2.67.2/help.txt", "---[Help]"),
            ("PathOfBuilding-2.67.2/changelog.txt", "VERSION[1][2]"),
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
    fn sha1_matches_the_standard_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(sha1_hex(long), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    }

    /// A PoB folder whose manifest names help.txt as `help` (CRLF form) and changelog.txt
    /// as `log` (as is)
    fn help_dir(name: &str, help: &str, log: &str) -> PathBuf {
        let dir = scratch(name);
        let crlf = help.replace('\n', "\r\n");
        fs::write(
            dir.join("manifest.xml"),
            format!(
                "<PoBVersion>\n\t<File name=\"help.txt\" sha1=\"{}\" />\n\t<File name=\"changelog.txt\" sha1=\"{}\" />\n</PoBVersion>\n",
                sha1_hex(crlf.as_bytes()),
                sha1_hex(log.as_bytes())
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn missing_help_files_are_fetched_checked_and_written() {
        let dir = help_dir("topup", "---[Help]\nline\n", "VERSION[1][2]\n");
        let mut asked = Vec::new();
        let written = top_up_help_files(&dir, &mut |name| {
            asked.push(name.to_owned());
            Ok(match name {
                "help.txt" => b"---[Help]\nline\n".to_vec(),
                _ => b"VERSION[1][2]\n".to_vec(),
            })
        })
        .unwrap();
        assert_eq!(written, ["help.txt", "changelog.txt"]);
        assert_eq!(asked, ["help.txt", "changelog.txt"]);
        assert_eq!(
            fs::read_to_string(dir.join("help.txt")).unwrap(),
            "---[Help]\nline\n"
        );
        assert!(!dir.join("help.download").exists());
        // present files aren't asked for again
        let written = top_up_help_files(&dir, &mut |_| panic!("asked again")).unwrap();
        assert!(written.is_empty());
    }

    #[test]
    fn a_file_that_doesnt_match_the_manifest_writes_nothing() {
        let dir = help_dir("topup-bad", "---[Help]\n", "VERSION[1][2]\n");
        let err = top_up_help_files(&dir, &mut |name| {
            Ok(match name {
                "help.txt" => b"---[Help]\n".to_vec(),
                _ => b"tampered\n".to_vec(),
            })
        })
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "changelog.txt doesn't match PoB's manifest"
        );
        // the good one isn't written either
        assert!(!dir.join("help.txt").exists());
        assert!(!dir.join("changelog.txt").exists());
    }

    #[test]
    fn extraction_leaves_out_help_files_that_dont_match() {
        let dest = scratch("extract-help");
        let help = "---[Help]\n";
        let manifest = format!(
            "<m>\n<File name=\"help.txt\" sha1=\"{}\" />\n<File name=\"changelog.txt\" sha1=\"0000\" />\n</m>",
            sha1_hex(help.as_bytes())
        );
        let mut b = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        // the help files come before the manifest
        for (path, data) in [
            ("Root/help.txt", help),
            ("Root/changelog.txt", "VERSION[1][2]"),
            ("Root/manifest.xml", manifest.as_str()),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data.as_bytes()).unwrap();
        }
        let bytes = b.into_inner().unwrap().finish().unwrap();
        extract(&bytes[..], &dest).unwrap();
        assert_eq!(fs::read_to_string(dest.join("help.txt")).unwrap(), help);
        assert!(!dest.join("changelog.txt").exists());
    }

    #[test]
    fn extract_maps_src_and_runtime_lua() {
        let dest = scratch("extract");
        extract(&tarball()[..], &dest).unwrap();
        assert!(dest.join("Launch.lua").exists());
        assert!(dest.join("Classes/Item.lua").exists());
        assert!(dest.join("lua/xml.lua").exists());
        assert!(dest.join("manifest.xml").exists());
        // the Help window's files are checked against the manifest: this one names neither
        assert!(!dest.join("help.txt").exists());
        assert!(!dest.join("changelog.txt").exists());
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
