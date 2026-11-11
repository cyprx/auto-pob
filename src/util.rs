use ahash::AHasher;
use mlua::{Lua, Result as LuaResult, chunk::Chunk};
use std::{
    env, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

pub use pob_runtime::util::{append_lua_package_dir, fix_script_header};

pub fn get_executable_dir() -> anyhow::Result<PathBuf> {
    let exe_path = env::current_exe()?;
    let exe_parent_dir = exe_path.parent().unwrap().canonicalize()?;
    Ok(exe_parent_dir)
}

pub fn change_working_directory<P: AsRef<Path>>(path: P) -> anyhow::Result<()> {
    env::set_current_dir(path.as_ref()).map_err(|e| {
        anyhow::anyhow!(
            "Failed to change working directory {}: {}",
            path.as_ref().display(),
            e
        )
    })
}

pub fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut state = AHasher::default();
    t.hash(&mut state);
    state.finish()
}

/// Performs replacement only in lines that match a given pattern
pub fn replace_in_matching_lines(
    input: &str,
    match_pattern: &str,
    replace_pattern: &str,
    replacement_text: &str,
) -> String {
    let match_re = regex::Regex::new(match_pattern).expect("Invalid match regex");
    let replace_re = regex::Regex::new(replace_pattern).expect("Invalid replace regex");

    let mut output = String::new();
    for line in input.lines() {
        if match_re.is_match(line) {
            // if line matches the pattern, replace
            let replaced_line = replace_re.replace_all(line, replacement_text);
            output.push_str(&replaced_line);
        } else {
            // otherwise, keep original line
            output.push_str(line);
        }
        output.push('\n');
    }
    output
}

/// Loads a Lua script from `path` and applies `fix_script_header`.
///
/// The chunk name is set to `@<path>` to match `Lua::load`'s behavior for paths, so error messages
/// keep referring to the file.
pub fn load_lua_file<'lua>(lua: &'lua Lua, path: &Path) -> LuaResult<Chunk<'lua>> {
    let mut source = fs::read(path)?;
    fix_script_header(&mut source);
    Ok(lua.load(source).set_name(format!("@{}", path.display())))
}
