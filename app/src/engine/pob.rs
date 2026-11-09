//! Starts Path of Building's Lua with our own bootstrap.

use crate::game::Game;
use anyhow::{Context as _, Result, anyhow};
use mlua::Lua;
use std::path::Path;

pub struct PobBackend {
    pub lua: Lua,
    pub game: Game,
}

/// Runs before the bootstrap.
const PRELUDE: &str = r##"
package.path = "./lua/?.lua;./lua/?/init.lua;" .. package.path

-- command line arguments; PoB reads an optional build import URL from here
arg = {}

-- PoB's files start with a "#@" line that LuaJIT rejects; strip it when loading
local function load_stripped(name)
	local file, err = io.open(name, "rb")
	if not file then return nil, err end
	local source = file:read("*a"):gsub("^#@[^\n]*", "")
	file:close()
	return loadstring(source, "@" .. name)
end
loadfile = load_stripped
dofile = function(name) return assert(load_stripped(name))() end

-- there is no console to answer prompts
io.read = function() return "" end
"##;

impl PobBackend {
    /// Starts PoB from `pob_dir`, keeping its user data in `user_dir`.
    pub fn start(game: Game, pob_dir: &Path, user_dir: &Path) -> Result<PobBackend> {
        // mlua's error isn't Send + Sync, which anyhow requires; keep its message instead
        let lua_error = |e: mlua::Error| anyhow!("{e}");

        // PoB loads its files relative to the working directory
        std::env::set_current_dir(pob_dir)
            .with_context(|| format!("PoB not found in {}", pob_dir.display()))?;

        // `unsafe_new` gives PoB the full standard library, like the main runtime does
        let lua = unsafe { Lua::unsafe_new() };
        let utf8 = luautf8::create_module(&lua).map_err(lua_error)?;
        lua.register_module("lua-utf8", utf8).map_err(lua_error)?;
        lua.load(PRELUDE)
            .set_name("=prelude")
            .exec()
            .map_err(lua_error)?;
        super::native::register(&lua, pob_dir, user_dir).map_err(lua_error)?;
        lua.load(include_str!("bootstrap.lua"))
            .set_name("@bootstrap.lua")
            .exec()
            .map_err(lua_error)
            .context("Starting Path of Building")?;
        Ok(PobBackend { lua, game })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned_dir() -> std::path::PathBuf {
        std::env::var_os("AUTO_POB_POB_DIR")
            .expect("set AUTO_POB_POB_DIR to the pinned PoB folder")
            .into()
    }

    /// Changes the working directory, so run with `--ignored --test-threads=1`.
    #[test]
    #[ignore = "needs the pinned PoB install"]
    fn starts_pinned_pob() {
        let user = std::env::temp_dir().join(format!("auto-pob-user-{}", std::process::id()));
        let backend = PobBackend::start(Game::Poe1, &pinned_dir(), &user).unwrap();
        let ok: bool = backend.lua.load("return pob.main ~= nil").eval().unwrap();
        assert!(ok);
        backend
            .lua
            .load(r#"ConPrintf("hello %d", 1)"#)
            .exec()
            .unwrap();
        let _ = std::fs::remove_dir_all(&user);
    }
}
