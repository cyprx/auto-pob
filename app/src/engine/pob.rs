//! Starts Path of Building's Lua with our own bootstrap.

use super::commands::{Command, ConfigValue, Tab};
use super::run::{Affected, Backend, Engine, Event};
use super::views::{BuildListView, BuildView, ConfigView, StatsView};
use crate::game::Game;
use anyhow::{Context as _, Result, anyhow};
use mlua::{FromLuaMulti, IntoLuaMulti, Lua, LuaSerdeExt};
use std::path::{Path, PathBuf};

pub struct PobBackend {
    pub lua: Lua,
    pub game: Game,
    pob_dir: PathBuf,
    /// The Builds sub-folder the list view shows; "" is the root.
    folder: String,
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

/// A Lua state with the standard library, `lua-utf8` and our prelude, running in `pob_dir`.
pub(crate) fn new_lua(pob_dir: &Path) -> Result<Lua> {
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
    Ok(lua)
}

impl PobBackend {
    /// Starts PoB from `pob_dir`, keeping its user data in `user_dir`.
    pub fn start(game: Game, pob_dir: &Path, user_dir: &Path) -> Result<PobBackend> {
        // mlua's error isn't Send + Sync, which anyhow requires; keep its message instead
        let lua_error = |e: mlua::Error| anyhow!("{e}");

        let lua = new_lua(pob_dir)?;
        super::native::register(&lua, pob_dir, user_dir).map_err(lua_error)?;
        lua.load(include_str!("bootstrap.lua"))
            .set_name("@bootstrap.lua")
            .exec()
            .map_err(lua_error)
            .context("Starting Path of Building")?;
        for (name, source) in [
            ("@glue/builds.lua", include_str!("glue/builds.lua")),
            ("@glue/stats.lua", include_str!("glue/stats.lua")),
            ("@glue/tooltip.lua", include_str!("glue/tooltip.lua")),
            ("@glue/config.lua", include_str!("glue/config.lua")),
            ("@glue/lab.lua", include_str!("glue/lab.lua")),
        ] {
            lua.load(source)
                .set_name(name)
                .exec()
                .map_err(lua_error)
                .with_context(|| format!("Loading {name}"))?;
        }
        Ok(PobBackend {
            lua,
            game,
            pob_dir: pob_dir.to_owned(),
            folder: String::new(),
        })
    }

    /// Calls `glue.<name>(args)`, keeping only the error message.
    fn call<A: IntoLuaMulti, R: FromLuaMulti>(&self, name: &str, args: A) -> Result<R> {
        let glue: mlua::Table = self.lua.globals().get("glue").map_err(|e| anyhow!("{e}"))?;
        let function: mlua::Function = glue.get(name).map_err(|e| anyhow!("{e}"))?;
        function.call(args).map_err(|e| match e {
            // errors raised with `error(message, 0)` arrive as the bare message
            mlua::Error::RuntimeError(message) => {
                anyhow!("{}", message.lines().next().unwrap_or_default())
            }
            other => anyhow!("{other}"),
        })
    }

    fn view<T: serde::de::DeserializeOwned>(
        &self,
        name: &str,
        args: impl IntoLuaMulti,
    ) -> Result<T> {
        let value: mlua::Value = self.call(name, args)?;
        self.lua.from_value(value).map_err(|e| anyhow!("{e}"))
    }
}

/// A config change touches the build and the config tab, and needs a recalculation (after
/// which the loop also refreshes the stats).
fn config_changed() -> Affected {
    Affected {
        build: true,
        config: true,
        recalc: true,
        ..Affected::default()
    }
}

impl Backend for PobBackend {
    fn apply(&mut self, command: Command, send: &dyn Fn(Event)) -> Result<Affected> {
        match command {
            Command::ListBuilds { folder } => {
                if folder.contains("..") || folder.starts_with('/') {
                    anyhow::bail!("that isn't a valid folder");
                }
                self.folder = if folder.is_empty() || folder.ends_with('/') {
                    folder
                } else {
                    format!("{folder}/")
                };
                Ok(Affected {
                    list: true,
                    ..Affected::default()
                })
            }
            Command::NewBuild => {
                self.call::<_, ()>("new_build", ())?;
                Ok(Affected::ALL_VIEWS)
            }
            Command::OpenBuild { path } => {
                self.call::<_, ()>("open_build", path.to_string_lossy().into_owned())?;
                Ok(Affected::ALL_VIEWS)
            }
            Command::ImportShareCode { code } => {
                self.call::<_, ()>("import_code", code)?;
                Ok(Affected::ALL_VIEWS)
            }
            Command::Save => {
                self.call::<_, ()>("save", ())?;
                Ok(Affected {
                    build: true,
                    list: true,
                    ..Affected::default()
                })
            }
            Command::SaveAs {
                folder,
                name,
                overwrite,
            } => {
                self.call::<_, ()>("save_as", (folder, name, overwrite))?;
                Ok(Affected {
                    build: true,
                    list: true,
                    ..Affected::default()
                })
            }
            // run_loop handles Shutdown before applying commands; nothing to do here
            Command::Shutdown => Ok(Affected::default()),
            Command::Lab(lab) => super::lab::apply(&self.lua, &self.pob_dir, lab, send),
            Command::SetConfig { var, value } => {
                let value = match value {
                    ConfigValue::Bool(b) => mlua::Value::Boolean(b),
                    ConfigValue::Number(n) => mlua::Value::Number(n),
                    ConfigValue::Text(t) | ConfigValue::Choice(t) => {
                        mlua::Value::String(self.lua.create_string(t).map_err(|e| anyhow!("{e}"))?)
                    }
                    ConfigValue::Clear => mlua::Value::Nil,
                };
                self.call::<_, ()>("set_config", (var, value))?;
                Ok(config_changed())
            }
            Command::SelectConfigSet { id } => {
                self.call::<_, ()>("select_config_set", id)?;
                Ok(config_changed())
            }
            Command::NewConfigSet { title } => {
                self.call::<_, ()>("new_config_set", title)?;
                Ok(config_changed())
            }
            Command::RenameConfigSet { id, title } => {
                self.call::<_, ()>("rename_config_set", (id, title))?;
                Ok(config_changed())
            }
            Command::DeleteConfigSet { id } => {
                self.call::<_, ()>("delete_config_set", id)?;
                Ok(config_changed())
            }
            Command::SetCustomModText { group, text } => {
                self.call::<_, ()>("set_custom_mod_text", (group + 1, text))?;
                Ok(config_changed())
            }
            Command::SetCustomModTitle { group, title } => {
                self.call::<_, ()>("set_custom_mod_title", (group + 1, title))?;
                Ok(config_changed())
            }
            Command::SetCustomModEnabled { group, enabled } => {
                self.call::<_, ()>("set_custom_mod_enabled", (group + 1, enabled))?;
                Ok(config_changed())
            }
            Command::AddCustomModGroup => {
                self.call::<_, ()>("add_custom_mod_group", ())?;
                Ok(config_changed())
            }
            Command::RemoveCustomModGroup { group } => {
                self.call::<_, ()>("remove_custom_mod_group", group + 1)?;
                Ok(config_changed())
            }
            Command::Undo(Tab::Config) => {
                self.call::<_, ()>("undo", "config")?;
                Ok(config_changed())
            }
            Command::Redo(Tab::Config) => {
                self.call::<_, ()>("redo", "config")?;
                Ok(config_changed())
            }
        }
    }

    fn recalculate(&mut self) -> Result<()> {
        self.call::<_, ()>("recalculate", ())
    }

    fn build_view(&mut self) -> Result<BuildView> {
        self.view("build_view", ())
    }

    fn build_list_view(&mut self) -> Result<BuildListView> {
        self.view("list_builds", self.folder.clone())
    }

    fn stats_view(&mut self) -> Result<StatsView> {
        self.view("stats_view", ())
    }

    fn config_view(&mut self) -> Result<ConfigView> {
        self.view("config_view", ())
    }
}

impl Engine {
    /// Starts the engine thread on the real PoB: finds or downloads the pinned install, then
    /// starts it with its user data in `user_dir`.
    pub fn spawn(
        game: Game,
        data_dir: PathBuf,
        user_dir: PathBuf,
        wake_ui: impl Fn() + Send + 'static,
    ) -> Engine {
        Engine::spawn_with(
            move |send| {
                let pob_dir = crate::pob_files::ensure_pob(game, &data_dir, &mut |progress| {
                    send(Event::Downloading(progress))
                })?;
                Ok(Box::new(PobBackend::start(game, &pob_dir, &user_dir)?) as Box<dyn Backend>)
            },
            wake_ui,
        )
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
