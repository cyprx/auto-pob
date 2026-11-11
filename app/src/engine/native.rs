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
                format!("{}{REDACTED}", &word[..at])
            } else {
                word.to_string()
            };
            kept + space
        })
        .collect()
}

/// Registers `Inflate`, `Deflate`, `NewFileSearch`, `ConPrintf` support and the path functions.
pub fn register(lua: &Lua, pob_dir: &Path, user_dir: &Path) -> mlua::Result<()> {
    let g = lua.globals();
    register_compression(lua)?;
    g.set("NewFileSearch", lua.create_function(new_file_search)?)?;
    g.set(
        "__log_info",
        lua.create_function(|_, msg: LuaString| {
            log::info!("{}", redact(&msg.to_string_lossy()));
            Ok(())
        })?,
    )?;

    // PoB appends "Path of Building/" to the user path, so it needs a trailing slash
    let mut user = user_dir.to_string_lossy().into_owned();
    if !user.ends_with('/') {
        user.push('/');
    }
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
        lua.create_function(|_, path: String| {
            // PoB ignores the result; true means this call created the folder
            let existed = std::path::Path::new(&path).exists();
            let _ = fs::create_dir_all(&path);
            Ok(!existed && std::path::Path::new(&path).is_dir())
        })?,
    )?;
    g.set(
        "RemoveDir",
        // only an empty folder goes; anything else is left alone
        lua.create_function(|_, path: String| Ok(fs::remove_dir(path).is_ok()))?,
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
            "Failed to download /character/<redacted> Response code: 500"
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
            "/user/"
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
    fn inflate_deflate_round_trip() {
        let s: String = lua()
            .load(r#"return Inflate(Deflate("hello hello hello"))"#)
            .eval()
            .unwrap();
        assert_eq!(s, "hello hello hello");
    }
}
