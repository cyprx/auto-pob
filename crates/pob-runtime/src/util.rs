use mlua::{Lua, Result as LuaResult, Table};
use std::path::Path;

/// Appends a search pattern to Lua's `package.path`.
pub fn append_lua_package_path(lua: &Lua, pattern: &str) -> LuaResult<()> {
    let package: Table = lua.globals().get("package")?;
    let mut package_path: String = package.get("path")?;

    if !package_path.is_empty() {
        package_path.push(';');
    }

    package_path.push_str(pattern);
    package.set("path", package_path)?;

    Ok(())
}

/// Adds a directory to Lua's `package.path`.
///
/// This adds the following patterns:
/// - `<path>/?.lua`
/// - `<path>/?/init.lua`
pub fn append_lua_package_dir(lua: &Lua, path: &Path) -> LuaResult<()> {
    let path = path.to_string_lossy();
    append_lua_package_path(lua, &format!("{path}/?.lua"))?;
    append_lua_package_path(lua, &format!("{path}/?/init.lua"))?;
    Ok(())
}

/// Rewrites PoB's `#@ <label>` script header into a Lua comment.
///
/// Some Path of Building scripts start with a line such as `#@ <optional_text>`.  Recent LuaJIT
/// versions interpret a leading `#` as a line number directive and reject this header so we
/// replace them with a comment.
///
/// commit that introduced changes to the lexer to enable parsing of line number directives:
/// <https://github.com/LuaJIT/LuaJIT/commit/c41d727dbffa5ec4f346dd9be0f4c64740f124aa>
pub fn fix_script_header(source: &mut [u8]) {
    // skip utf8-bom
    let start = if source.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    if source[start..].starts_with(b"#@") {
        source[start..start + 2].copy_from_slice(b"--");
    }
}
