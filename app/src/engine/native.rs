//! Rust-side functions that PoB's Lua calls as globals.

use flate2::{Compression, read::ZlibDecoder, read::ZlibEncoder};
use mlua::{Lua, LuaString, UserData, UserDataMethods, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

const REDACTED: &str = "<redacted>";

/// PoB's own lines, without the user's data (R21): a URL becomes `<redacted>`, and so does
/// what follows `/character/`, `/account/` or a `?` (PoEAPI logs "Failed to download
/// /character/<name>: ...").
pub(crate) fn redact(line: &str) -> String {
    line.split_inclusive(char::is_whitespace)
        .map(|piece| {
            let word = piece.trim_end_matches(char::is_whitespace);
            let space = &piece[word.len()..];
            let kept = if word.contains("://") {
                REDACTED.to_string()
            } else if let Some(at) = ["/character/", "/account/", "?"]
                .iter()
                .filter_map(|mark| word.find(mark).map(|i| i + mark.len()))
                .min()
            {
                // the sentence's own punctuation stays
                let tail = &word[at..];
                let punct = &tail[tail.trim_end_matches([':', ',', ';', '.', ')']).len()..];
                format!("{}{REDACTED}{punct}", &word[..at])
            } else {
                word.to_string()
            };
            kept + space
        })
        .collect()
}

/// The Builds folder a recursive `RemoveDir` may delete in (R16), held in the Lua state: as
/// PoB spells it, and canonical.
struct BuildsRoot(Option<(PathBuf, PathBuf)>);

/// Folders that no Builds root may be or contain: the PoB folder, the user folder and more
/// (the home folder), as given: each is resolved when a root is checked, since a first run's
/// user folder only exists once PoB has made it.
struct Protected(Vec<PathBuf>);

/// Adds folders (such as the home folder) that no Builds root may be or contain.
pub fn protect(lua: &Lua, folders: &[PathBuf]) {
    let mut all = lua
        .app_data_ref::<Protected>()
        .map(|p| p.0.clone())
        .unwrap_or_default();
    all.extend(
        folders
            .iter()
            .map(|f| std::path::absolute(f).unwrap_or_else(|_| f.clone())),
    );
    lua.set_app_data(Protected(all));
}

/// The protected folders now: canonical where they exist, as given where they don't
fn protected(lua: &Lua) -> Vec<PathBuf> {
    lua.app_data_ref::<Protected>()
        .map(|p| {
            p.0.iter()
                .map(|f| fs::canonicalize(f).unwrap_or_else(|_| f.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Sets the folder a recursive `RemoveDir` is confined to. A root that can't be resolved, is
/// `/`, or is or contains the PoB folder, the user folder or the home folder is refused: every
/// recursive delete then stays disarmed and this returns false.
pub fn set_builds_root(lua: &Lua, root: Option<&Path>) -> bool {
    let real = root.and_then(|r| fs::canonicalize(r).ok().map(|real| (r.to_path_buf(), real)));
    let home = std::env::var_os("HOME")
        .and_then(|h| fs::canonicalize(h).ok())
        .into_iter();
    let allowed = real.as_ref().is_some_and(|(_, real)| {
        real.parent().is_some()
            && !protected(lua)
                .into_iter()
                .chain(home.clone())
                .any(|f| f.starts_with(real))
    });
    if !allowed && root.is_some() {
        log::warn!("the Builds folder is not one recursive deletes may work in");
    }
    lua.set_app_data(BuildsRoot(real.filter(|_| allowed)));
    allowed
}

/// Arms the recursive `RemoveDir` for the Builds folder `root` as PoB holds it (R16), or says
/// why not. A root that can't be resolved or isn't allowed disarms it first: a refused root never
/// leaves an earlier one armed.
pub(crate) fn arm_builds_root(lua: &Lua, root: &str) -> anyhow::Result<()> {
    if fs::canonicalize(root).is_err() {
        set_builds_root(lua, None);
        anyhow::bail!("the Builds folder doesn't exist");
    }
    if !set_builds_root(lua, Some(Path::new(root))) {
        anyhow::bail!("that Builds folder isn't allowed");
    }
    Ok(())
}

/// Where `path` is below the root, and whether it is a link: every part above it must be a real
/// folder, the root itself is never it.
fn locate(root: Option<&(PathBuf, PathBuf)>, path: &Path) -> Option<(PathBuf, bool)> {
    let (spelled, real) = root?;
    let rel = path
        .strip_prefix(spelled)
        .or_else(|_| path.strip_prefix(real))
        .ok()
        .filter(|rel| path.is_absolute() && rel.file_name().is_some())?;
    let mut walk = real.clone();
    let count = rel.components().count();
    for (i, part) in rel.components().enumerate() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return None;
        }
        walk.push(part);
        let meta = fs::symlink_metadata(&walk).ok()?;
        if i + 1 == count {
            return Some((walk, meta.file_type().is_symlink()));
        }
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return None;
        }
    }
    None
}

/// `remove_dir_all` for a folder strictly below the root. A link as the last part is unlinked
/// and what it leads to stays; a link anywhere above it refuses. Links inside the tree are
/// unlinked, never followed (`remove_dir_all` does not traverse them).
fn recursive_remove(root: Option<&(PathBuf, PathBuf)>, path: &Path) -> std::io::Result<()> {
    match locate(root, path) {
        Some((walk, true)) if walk.is_dir() => fs::remove_file(walk),
        Some((walk, false)) if walk.is_dir() => fs::remove_dir_all(walk),
        _ => {
            log::warn!("refused a recursive delete outside the Builds folder");
            Err(std::io::Error::other("refused: outside the builds folder"))
        }
    }
}

/// Registers `Inflate`, `Deflate`, `NewFileSearch`, `ConPrintf` support and the path functions.
pub fn register(lua: &Lua, pob_dir: &Path, user_dir: &Path) -> mlua::Result<()> {
    let g = lua.globals();
    protect(lua, &[pob_dir.to_path_buf(), user_dir.to_path_buf()]);
    register_compression(lua)?;
    g.set("NewFileSearch", lua.create_function(new_file_search)?)?;
    g.set(
        "__log_info",
        lua.create_function(|_, msg: LuaString| {
            log::info!("{}", redact(&msg.to_string_lossy()));
            Ok(())
        })?,
    )?;

    // without a trailing slash, as SimpleGraphic and the classic runtime answer: PoB appends
    // "/Path of Building/" itself (Main.lua:94)
    let user = user_dir.to_string_lossy().trim_end_matches('/').to_owned();
    let pob = pob_dir.to_string_lossy().into_owned();
    g.set(
        "GetUserPath",
        lua.create_function(move |_, ()| Ok(user.clone()))?,
    )?;
    let p = pob.clone();
    g.set(
        "GetScriptPath",
        lua.create_function(move |_, ()| Ok(p.clone()))?,
    )?;
    g.set(
        "GetWorkDir",
        lua.create_function(move |_, ()| Ok(pob.clone()))?,
    )?;
    g.set(
        "MakeDir",
        // true when this call made the folder; nil and the reason otherwise, also for one that
        // is there already, so a move or a copy never merges into a folder
        lua.create_function(|_, path: String| {
            if Path::new(&path).exists() {
                return Ok((None, Some("File exists".to_string())));
            }
            Ok(match fs::create_dir_all(&path) {
                Ok(()) => (Some(true), None),
                Err(e) => (None, Some(e.to_string())),
            })
        })?,
    )?;
    g.set(
        "RemoveDir",
        // a folder, and with `recursive` everything in it (PoB asks first, BuildListControl.lua
        // :154), but only below the registered Builds folder (see `recursive_remove`)
        lua.create_function(|lua, (path, recursive): (String, Option<bool>)| {
            let removed = if recursive.unwrap_or(false) {
                let root = lua.app_data_ref::<BuildsRoot>().and_then(|r| r.0.clone());
                recursive_remove(root.as_ref(), Path::new(&path))
            } else {
                // a link to a folder that PoB lists is removed as a link, below the root only
                let root = lua.app_data_ref::<BuildsRoot>().and_then(|r| r.0.clone());
                match locate(root.as_ref(), Path::new(&path)) {
                    Some((walk, true)) if walk.is_dir() => fs::remove_file(walk),
                    _ => fs::remove_dir(path),
                }
            };
            Ok(match removed {
                Ok(()) => (Some(true), None),
                Err(e) => (None, Some(e.to_string())),
            })
        })?,
    )?;
    // Whole milliseconds since registration. A constant clock would stop PoB's
    // power coroutine from ever yielding (CalcsTab.lua:648).
    let start = Instant::now();
    g.set(
        "GetTime",
        lua.create_function(move |_, ()| Ok(start.elapsed().as_millis() as i64))?,
    )?;
    Ok(())
}

/// Installs the real `Inflate` and `Deflate` (share codes need them).
pub fn register_compression(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set("Inflate", lua.create_function(inflate)?)?;
    g.set("Deflate", lua.create_function(deflate)?)?;
    // the text a view shows for a PoB string (`pob.rs` reads strings that aren't UTF-8 with
    // their bad bytes as U+FFFD), so the glue can find an entry by the text the UI sent
    g.set(
        "AutoPobLossy",
        lua.create_function(|lua, text: LuaString| {
            match String::from_utf8_lossy(&text.as_bytes()) {
                std::borrow::Cow::Borrowed(_) => Ok(text),
                std::borrow::Cow::Owned(fixed) => lua.create_string(fixed),
            }
        })?,
    )?;
    Ok(())
}

fn inflate(lua: &Lua, data: LuaString) -> mlua::Result<Value> {
    let mut out = Vec::new();
    match ZlibDecoder::new(&data.as_bytes()[..]).read_to_end(&mut out) {
        Ok(_) => Ok(Value::String(lua.create_string(&out)?)),
        Err(_) => Ok(Value::Nil),
    }
}

fn deflate(lua: &Lua, data: LuaString) -> mlua::Result<Value> {
    let mut out = Vec::new();
    ZlibEncoder::new(&data.as_bytes()[..], Compression::best())
        .read_to_end(&mut out)
        .map_err(mlua::Error::external)?;
    Ok(Value::String(lua.create_string(&out)?))
}

/// Matches `*` (any run) and `?` (any one character) against the whole name.
fn glob_match(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) && pattern[p] != '*' {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

struct Entry {
    name: String,
    path: PathBuf,
}

struct SearchHandle {
    entries: Vec<Entry>,
    index: usize,
}

impl SearchHandle {
    fn current(&self) -> &Entry {
        &self.entries[self.index]
    }
}

impl UserData for SearchHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("NextFile", |_, this, ()| {
            if this.index + 1 < this.entries.len() {
                this.index += 1;
                Ok(true)
            } else {
                Ok(false)
            }
        });
        methods.add_method("GetFileName", |_, this, ()| Ok(this.current().name.clone()));
        methods.add_method("GetFileSize", |_, this, ()| {
            Ok(fs::metadata(&this.current().path).map_or(0, |m| m.len()))
        });
        methods.add_method("GetFileModifiedTime", |_, this, ()| {
            Ok(fs::metadata(&this.current().path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0.0, |d| d.as_secs_f64().floor()))
        });
    }
}

/// `NewFileSearch(pattern, dirsOnly)`: globs the last path component only.
fn new_file_search(
    _: &Lua,
    (pattern, dirs_only): (String, Option<bool>),
) -> mlua::Result<Option<SearchHandle>> {
    let dirs_only = dirs_only.unwrap_or(false);
    let (dir, file_pattern) = match pattern.rfind('/') {
        Some(0) => ("/", &pattern[1..]),
        Some(i) => (&pattern[..i], &pattern[i + 1..]),
        None => (".", pattern.as_str()),
    };
    let file_pattern: Vec<char> = file_pattern.chars().collect();
    let Ok(read) = fs::read_dir(dir) else {
        return Ok(None);
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // follows symlinks, like the classic app's `is_dir`
            let is_dir = e.path().is_dir();
            let chars: Vec<char> = name.chars().collect();
            (is_dir == dirs_only && glob_match(&file_pattern, &chars)).then(|| Entry {
                name,
                path: e.path(),
            })
        })
        .collect();
    if entries.is_empty() {
        return Ok(None);
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Some(SearchHandle { entries, index: 0 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_hides_names_queries_and_urls() {
        assert_eq!(
            redact("Failed to download /character/GateOne: Response code: 500"),
            "Failed to download /character/<redacted>: Response code: 500"
        );
        assert_eq!(
            redact("Download /character/xbox/GateTwo"),
            "Download /character/<redacted>"
        );
        assert_eq!(
            redact("see /account/view-profile/Gate-1234 and get-items?accountName=A%231234 x"),
            "see /account/<redacted> and get-items?<redacted> x"
        );
        assert_eq!(
            redact("Downloading page at: https://pobb.in/gate\nnext line"),
            "Downloading page at: <redacted>\nnext line"
        );
        assert_eq!(
            redact("Loading passive tree data for version '3.29'..."),
            "Loading passive tree data for version '3.29'..."
        );
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("auto-pob-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn populated() -> TempDir {
        let dir = TempDir::new(&format!("search-{:?}", std::thread::current().id()));
        for f in ["a.xml", "b.xml", "c.txt", "Ætheric – Build.xml"] {
            fs::write(dir.0.join(f), "x").unwrap();
        }
        fs::create_dir(dir.0.join("Sub Folder")).unwrap();
        dir
    }

    fn names(lua: &Lua, dir: &Path, pattern: &str, dirs: bool) -> Option<Vec<String>> {
        lua.globals()
            .set("dir", dir.to_string_lossy().into_owned())
            .unwrap();
        lua.load(format!(
            r#"
            local h = NewFileSearch(dir .. "/{pattern}", {dirs})
            if not h then return nil end
            local t = {{}}
            repeat t[#t + 1] = h:GetFileName() until not h:NextFile()
            table.sort(t)
            return t
            "#
        ))
        .eval::<Option<Vec<String>>>()
        .unwrap()
    }

    fn lua() -> Lua {
        let lua = Lua::new();
        register(&lua, Path::new("/pob"), Path::new("/user")).unwrap();
        lua
    }

    #[test]
    fn file_search_lists_matching_files() {
        let dir = populated();
        assert_eq!(
            names(&lua(), &dir.0, "*.xml", false).unwrap(),
            ["a.xml", "b.xml", "Ætheric – Build.xml"]
        );
    }

    #[test]
    fn file_search_lists_only_folders_when_asked() {
        let dir = populated();
        assert_eq!(names(&lua(), &dir.0, "*", true).unwrap(), ["Sub Folder"]);
    }

    #[test]
    fn file_search_returns_nil_when_nothing_matches() {
        let dir = populated();
        assert!(names(&lua(), &dir.0, "*.zip", false).is_none());
    }

    #[test]
    fn file_search_handles_unicode_names() {
        let dir = populated();
        assert_eq!(
            names(&lua(), &dir.0, "Æ*", false).unwrap(),
            ["Ætheric – Build.xml"]
        );
    }

    #[test]
    fn file_search_is_case_sensitive_and_supports_question_mark() {
        let dir = populated();
        assert!(names(&lua(), &dir.0, "A.xml", false).is_none());
        assert_eq!(names(&lua(), &dir.0, "?.txt", false).unwrap(), ["c.txt"]);
    }

    #[test]
    fn file_search_reports_modified_time_in_seconds() {
        let dir = populated();
        let lua = lua();
        lua.globals()
            .set("dir", dir.0.to_string_lossy().into_owned())
            .unwrap();
        let t: f64 = lua
            .load(r#"return NewFileSearch(dir .. "/a.xml"):GetFileModifiedTime()"#)
            .eval()
            .unwrap();
        assert!(t > 1.0e9);
    }

    #[test]
    fn paths_and_make_dir() {
        let lua = lua();
        assert_eq!(
            lua.load("return GetUserPath()").eval::<String>().unwrap(),
            "/user"
        );
        assert_eq!(
            lua.load("return GetScriptPath()").eval::<String>().unwrap(),
            "/pob"
        );
        assert_eq!(
            lua.load("return GetWorkDir()").eval::<String>().unwrap(),
            "/pob"
        );
        let tmp = TempDir::new("mkdir");
        let nested = tmp.0.join("a/b");
        lua.globals()
            .set("p", nested.to_string_lossy().into_owned())
            .unwrap();
        lua.load("MakeDir(p)").exec().unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn make_dir_says_why_it_did_not() {
        let lua = lua();
        let tmp = TempDir::new("mkdir-result");
        lua.globals()
            .set("p", tmp.0.join("new").to_string_lossy().into_owned())
            .unwrap();
        let first: (Option<bool>, Option<String>) = lua.load("return MakeDir(p)").eval().unwrap();
        assert_eq!(first, (Some(true), None));
        let again: (Option<bool>, Option<String>) = lua.load("return MakeDir(p)").eval().unwrap();
        assert_eq!(again, (None, Some("File exists".to_string())));
    }

    #[test]
    fn remove_dir_is_recursive_only_when_asked() {
        let lua = lua();
        let tmp = TempDir::new("rmdir");
        let full = tmp.0.join("full");
        fs::create_dir_all(full.join("inner")).unwrap();
        fs::write(full.join("inner/a.xml"), "a").unwrap();
        lua.globals()
            .set("p", full.to_string_lossy().into_owned())
            .unwrap();
        set_builds_root(&lua, Some(&tmp.0));
        let plain: (Option<bool>, Option<String>) = lua.load("return RemoveDir(p)").eval().unwrap();
        assert_eq!(plain.0, None);
        assert!(plain.1.is_some());
        assert!(full.is_dir(), "a folder with things in it stays");
        let all: (Option<bool>, Option<String>) =
            lua.load("return RemoveDir(p, true)").eval().unwrap();
        assert_eq!(all, (Some(true), None));
        assert!(!full.exists());
    }

    fn remove(lua: &Lua, path: &Path) -> (Option<bool>, Option<String>) {
        lua.globals()
            .set("p", path.to_string_lossy().into_owned())
            .unwrap();
        lua.load("return RemoveDir(p, true)").eval().unwrap()
    }

    #[test]
    fn recursive_remove_stays_inside_the_builds_root() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new("rmroot");
        let builds = tmp.0.join("Builds");
        let outside = tmp.0.join("outside");
        fs::create_dir_all(builds.join("tree/inner")).unwrap();
        fs::create_dir_all(outside.join("keep")).unwrap();
        fs::write(outside.join("keep/f.txt"), "keep").unwrap();
        fs::write(outside.join("s.xml"), "s").unwrap();
        let lua = lua();
        let refused = Some("refused: outside the builds folder".to_string());
        // nothing armed
        assert_eq!(remove(&lua, &builds.join("tree")), (None, refused.clone()));
        set_builds_root(&lua, Some(&builds));
        // a folder outside the root, the root, a relative path
        assert_eq!(remove(&lua, &outside), (None, refused.clone()));
        assert_eq!(remove(&lua, &builds), (None, refused.clone()));
        assert_eq!(remove(&lua, Path::new("outside")), (None, refused.clone()));
        assert_eq!(
            remove(&lua, &builds.join("../outside")),
            (None, refused.clone())
        );
        // a folder below a link; the link itself is only unlinked
        symlink(&outside, builds.join("escape")).unwrap();
        assert_eq!(
            remove(&lua, &builds.join("escape/keep")),
            (None, refused.clone())
        );
        assert!(outside.join("keep/f.txt").is_file());
        assert_eq!(remove(&lua, &builds.join("escape")), (Some(true), None));
        assert!(!builds.join("escape").exists() && outside.join("keep/f.txt").is_file());
        // a tree holding links out: they are unlinked, what they lead to stays
        symlink(&outside, builds.join("tree/inner/dir_link")).unwrap();
        symlink(outside.join("s.xml"), builds.join("tree/file_link")).unwrap();
        assert_eq!(remove(&lua, &builds.join("tree")), (Some(true), None));
        assert!(!builds.join("tree").exists());
        assert!(outside.join("keep/f.txt").is_file() && outside.join("s.xml").is_file());
        // the non-recursive form removes an empty folder and nothing else
        fs::create_dir_all(outside.join("empty")).unwrap();
        lua.globals()
            .set("p", outside.join("keep").to_string_lossy().into_owned())
            .unwrap();
        let plain: (Option<bool>, Option<String>) = lua.load("return RemoveDir(p)").eval().unwrap();
        assert_eq!(plain.0, None);
        assert!(outside.join("keep/f.txt").is_file());
    }

    #[test]
    fn a_root_that_holds_home_user_or_pob_stays_disarmed() {
        let tmp = TempDir::new("rmbad");
        let (home, user, pob) = (
            tmp.0.join("home"),
            tmp.0.join("home/user"),
            tmp.0.join("pob"),
        );
        let builds = user.join("Builds");
        for dir in [&pob, &builds] {
            fs::create_dir_all(dir.join("victim/inner")).unwrap();
        }
        let lua = lua();
        register(&lua, &pob, &user).unwrap();
        protect(&lua, std::slice::from_ref(&home));
        let refused = Some("refused: outside the builds folder".to_string());
        for bad in [Path::new("/"), &home, &user, &pob, &tmp.0] {
            assert!(!set_builds_root(&lua, Some(bad)), "{bad:?}");
            for victim in [
                pob.join("victim"),
                builds.join("victim"),
                user.join("Builds"),
            ] {
                assert_eq!(remove(&lua, &victim), (None, refused.clone()), "{bad:?}");
                assert!(victim.is_dir());
            }
        }
        assert!(set_builds_root(&lua, Some(&builds)));
        assert_eq!(remove(&lua, &builds.join("victim")), (Some(true), None));
        // a root that arms and then is replaced by a refused one disarms again
        fs::create_dir_all(builds.join("again")).unwrap();
        assert!(!set_builds_root(&lua, Some(&user)));
        assert_eq!(remove(&lua, &builds.join("again")), (None, refused));
    }

    #[test]
    fn a_user_folder_made_after_start_is_still_protected() {
        // a first run starts on a user folder nobody has made yet; PoB's MakeDir makes it
        let tmp = TempDir::new("rmfresh");
        let (user, pob) = (tmp.0.join("fresh/user"), tmp.0.join("pob"));
        fs::create_dir_all(&pob).unwrap();
        let lua = lua();
        register(&lua, &pob, &user).unwrap();
        let builds = user.join("Path of Building/Builds");
        fs::create_dir_all(builds.join("victim")).unwrap();
        for bad in [&user, &tmp.0.join("fresh")] {
            assert!(!set_builds_root(&lua, Some(bad)), "{bad:?}");
        }
        assert_eq!(
            remove(&lua, &builds.join("victim")),
            (None, Some("refused: outside the builds folder".to_string()))
        );
        assert!(set_builds_root(&lua, Some(&builds)));
    }

    #[test]
    fn a_root_that_cant_be_resolved_disarms_the_old_one() {
        let tmp = TempDir::new("rmgone");
        let (old, gone) = (tmp.0.join("Builds"), tmp.0.join("Gone"));
        fs::create_dir_all(old.join("victim/inner")).unwrap();
        let lua = lua();
        arm_builds_root(&lua, old.to_str().unwrap()).unwrap();
        let error = arm_builds_root(&lua, gone.to_str().unwrap()).unwrap_err();
        assert_eq!(error.to_string(), "the Builds folder doesn't exist");
        assert_eq!(
            remove(&lua, &old.join("victim")),
            (None, Some("refused: outside the builds folder".to_string()))
        );
        assert!(old.join("victim/inner").is_dir());
    }

    #[test]
    fn a_link_to_a_folder_is_unlinked_and_its_target_stays() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new("rmlink");
        let builds = tmp.0.join("Builds");
        fs::create_dir_all(builds.join("real")).unwrap();
        fs::write(builds.join("real/b.xml"), "b").unwrap();
        symlink(builds.join("real"), builds.join("lnk")).unwrap();
        symlink(builds.join("real"), builds.join("lnk2")).unwrap();
        let lua = lua();
        set_builds_root(&lua, Some(&builds));
        assert_eq!(remove(&lua, &builds.join("lnk")), (Some(true), None));
        assert!(!builds.join("lnk").exists() && builds.join("real/b.xml").is_file());
        // and the non-recursive form, for a link PoB listed as an empty folder
        lua.globals()
            .set("p", builds.join("lnk2").to_string_lossy().into_owned())
            .unwrap();
        let plain: (Option<bool>, Option<String>) = lua.load("return RemoveDir(p)").eval().unwrap();
        assert_eq!(plain, (Some(true), None));
        assert!(!builds.join("lnk2").exists() && builds.join("real/b.xml").is_file());
    }

    #[test]
    fn only_a_link_to_a_folder_is_unlinked() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new("rmfilelink");
        let builds = tmp.0.join("Builds");
        fs::create_dir_all(&builds).unwrap();
        fs::write(builds.join("f.xml"), "f").unwrap();
        symlink(builds.join("f.xml"), builds.join("to_file")).unwrap();
        symlink(builds.join("none"), builds.join("dangling")).unwrap();
        let lua = lua();
        set_builds_root(&lua, Some(&builds));
        for name in ["to_file", "dangling"] {
            assert_eq!(remove(&lua, &builds.join(name)).0, None, "{name}");
            lua.globals()
                .set("p", builds.join(name).to_string_lossy().into_owned())
                .unwrap();
            let plain: (Option<bool>, Option<String>) =
                lua.load("return RemoveDir(p)").eval().unwrap();
            assert_eq!(plain.0, None, "{name}");
            assert!(fs::symlink_metadata(builds.join(name)).is_ok());
        }
    }

    #[test]
    fn file_search_lists_dot_folders() {
        let dir = TempDir::new("dotdir");
        fs::create_dir_all(dir.0.join(".trash")).unwrap();
        assert_eq!(names(&lua(), &dir.0, "*", true).unwrap(), [".trash"]);
    }

    #[test]
    fn inflate_deflate_round_trip() {
        let s: String = lua()
            .load(r#"return Inflate(Deflate("hello hello hello"))"#)
            .eval()
            .unwrap();
        assert_eq!(s, "hello hello hello");
    }

    /// An Abyss timeless jewel is 74 MB inflated, read the first time a search or the tree needs
    /// it. Unoptimised, flate2's backend took about 2.1 s for it (a stall of the search's first
    /// step); the dev profile optimises it as the release build does, about 0.25 s. Checked in
    /// the workspace's Cargo.toml, not by timing, which a loaded machine makes flaky.
    #[test]
    fn inflate_is_optimised_in_every_build() {
        let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml");
        let doc: toml_edit::DocumentMut = fs::read_to_string(manifest).unwrap().parse().unwrap();
        // the backend flate2 builds with here (`rust_backend`), its checksums and flate2 itself
        for package in ["flate2", "miniz_oxide", "adler2", "simd-adler32"] {
            let level = doc["profile"]["dev"]["package"][package]["opt-level"].as_integer();
            assert_eq!(level, Some(3), "{package} isn't optimised in dev builds");
        }
    }
}
