//! Starts Path of Building's Lua with our own bootstrap.

use super::commands::{
    Command, ConfigValue, ControlTooltipQuery, GemSearchQuery, GemTarget, GemToggle,
    GemTooltipQuery, ItemTooltipQuery, PopupInput, PopupKind, SkillOption, SocketColour, Tab,
};
use super::run::{Affected, Backend, BackgroundTask, Engine, Event};
use super::views::{
    BreakdownView, BuildListView, BuildView, CalcsView, CellRef, ConfigView, ControlTooltipView,
    DisplayItemView, GemRow, GemSearchView, GemTooltipView, GroupTooltipView, ItemText,
    ItemTooltip, ItemTooltipView, ItemsView, ModLineList, NodePower, NodeTooltip, NotesView,
    PopupView, PowerReport, SkillsView, SocketGroupText, StatsView, TooltipLine, TreeLayout,
    TreeState,
};
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
    /// The key of the last tree layout sent, so an unchanged layout isn't sent again.
    last_layout_key: Option<String>,
    /// Whether the heat map is on, as the last `SetHeatmap` left it.
    heatmap_on: bool,
    /// A power step failed. Until the next `SetHeatmap` there is no background work, or the
    /// loop would run the failing step again and again.
    power_failed: bool,
    /// The open gem search (R9); `None` when the picker is closed. Whether its DPS sort has
    /// work is asked of PoB's control (`gem_dps_pending`), so it can't drift.
    gem_search: Option<GemSearchQuery>,
    /// A DPS step failed; the job stays stopped until the next `SearchGems`.
    gem_failed: bool,
    /// A command ended the open search; its last view (`ended: true`) is still to be sent.
    gem_ended: Option<GemSearchQuery>,
    /// Lua's heap after the last memory release, in bytes
    released_lua_bytes: usize,
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
            ("@glue/tree.lua", include_str!("glue/tree.lua")),
            ("@glue/power.lua", include_str!("glue/power.lua")),
            ("@glue/calcs.lua", include_str!("glue/calcs.lua")),
            ("@glue/controls.lua", include_str!("glue/controls.lua")),
            ("@glue/skills.lua", include_str!("glue/skills.lua")),
            ("@glue/gems.lua", include_str!("glue/gems.lua")),
            ("@glue/items.lua", include_str!("glue/items.lua")),
            ("@glue/crafting.lua", include_str!("glue/crafting.lua")),
            ("@glue/popups.lua", include_str!("glue/popups.lua")),
            ("@glue/notes.lua", include_str!("glue/notes.lua")),
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
            last_layout_key: None,
            heatmap_on: false,
            power_failed: false,
            gem_search: None,
            gem_failed: false,
            gem_ended: None,
            released_lua_bytes: 0,
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

    /// Whether the layout the UI holds no longer matches what the tree draws.
    fn layout_changed(&self) -> Result<bool> {
        let key = super::tree::layout_key(&self.lua).map_err(|e| anyhow!("{e}"))?;
        Ok(self.last_layout_key.as_deref() != Some(key.as_str()))
    }

    /// Every view, plus the tree layout when it differs from the one last sent. For commands
    /// that replace the whole build.
    fn all_views(&self) -> Result<Affected> {
        Ok(Affected {
            tree_layout: self.layout_changed()?,
            ..Affected::ALL_VIEWS
        })
    }

    /// A change to the trees: the layout goes out only if it differs from the last one sent.
    /// `rebuild` is false for changes that leave the build as it is.
    fn spec_edited(&self, rebuild: bool) -> Result<Affected> {
        Ok(Affected {
            build: rebuild,
            recalc: rebuild,
            tree: true,
            tree_layout: self.layout_changed()?,
            ..Affected::default()
        })
    }

    /// A tree edit changes the build and the tree state, and needs a recalculation. It can
    /// change node text too (a mastery's effect), so the layout is checked.
    fn tree_edited(&self) -> Result<Affected> {
        Ok(Affected {
            build: true,
            tree: true,
            recalc: true,
            tree_layout: self.layout_changed()?,
            ..Affected::default()
        })
    }

    /// A change to the items or the slots: a jewel change can rebuild cluster subgraphs, so the
    /// tree layout is checked too (R17).
    fn items_edited(&self) -> Result<Affected> {
        Ok(Affected {
            build: true,
            items: true,
            recalc: true,
            tree_layout: self.layout_changed()?,
            ..Affected::default()
        })
    }

    /// A build that is opened or replaced starts with the heat map off.
    fn reset_heatmap(&mut self) {
        self.heatmap_on = false;
        self.power_failed = false;
    }

    /// Closes the gem search, as the structural commands do (R13).
    fn end_gem_search(&mut self) -> Result<()> {
        self.gem_failed = false;
        if self.gem_search.take().is_some() {
            self.call::<_, ()>("gem_search_close", ())?;
        }
        Ok(())
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

/// A change to the socket groups (or their undo): the build is edited and everything is
/// recalculated.
fn skills_changed() -> Affected {
    Affected {
        build: true,
        skills: true,
        recalc: true,
        ..Affected::default()
    }
}

/// A change that sets no `buildFlag` (sort, move): the glue ran a frame, which refreshes
/// `unsaved` (R17).
fn items_listed() -> Affected {
    Affected {
        build: true,
        items: true,
        ..Affected::default()
    }
}

/// A change to the display item only: PoB counts none of it as an edit of the build (R10).
fn display_changed() -> Affected {
    Affected {
        display_item: true,
        ..Affected::default()
    }
}

/// A change on the Calcs tab marks the build unsaved; `recalc` is for the ones that set PoB's
/// `buildFlag`.
fn calcs_changed(recalc: bool) -> Affected {
    Affected {
        build: true,
        calcs: true,
        recalc,
        ..Affected::default()
    }
}

/// The gem box a target names, as `gems.lua` takes it: a 1-based slot or "imbued".
fn lua_target(lua: &Lua, target: GemTarget) -> Result<mlua::Value> {
    Ok(match target {
        GemTarget::Slot(slot) => mlua::Value::Integer(slot as i64 + 1),
        GemTarget::Imbued => {
            mlua::Value::String(lua.create_string("imbued").map_err(|e| anyhow!("{e}"))?)
        }
    })
}

/// Commands that end the gem search (R13): they add, drop or reorder the groups or gems, or
/// leave the build.
fn ends_gem_search(command: &Command) -> bool {
    matches!(
        command,
        Command::AddSocketGroup
            | Command::DeleteSocketGroup { .. }
            | Command::DeleteAllSocketGroups
            | Command::MoveSocketGroup { .. }
            | Command::PasteSocketGroup { .. }
            | Command::DeleteGem { .. }
            | Command::SelectSkillSet { .. }
            | Command::NewSkillSet { .. }
            | Command::CopySkillSet { .. }
            | Command::DeleteSkillSet { .. }
            | Command::Undo(Tab::Skills)
            | Command::Redo(Tab::Skills)
            | Command::ListBuilds { .. }
            | Command::NewBuild
            | Command::OpenBuild { .. }
            | Command::ImportShareCode { .. }
            | Command::CloseGemSearch
    )
}

/// Whether a command ends the popup session (R4). The session's own commands and the queries
/// leave it open; anything else may change the item or the build under it.
pub(crate) fn ends_popup_session(command: &Command) -> bool {
    !matches!(
        command,
        Command::OpenPopup(_)
            | Command::SetPopupControl { .. }
            | Command::PressPopupButton { .. }
            | Command::ClosePopup
            | Command::ControlTooltip(_)
            | Command::ItemTooltip(_)
            | Command::CopyItem { .. }
            | Command::Breakdown(_)
            | Command::GemTooltip(_)
            | Command::GroupTooltip { .. }
            | Command::CopySocketGroup { .. }
            | Command::PreviewNode { .. }
            | Command::ExportTreeLink
            | Command::Shutdown
    )
}

/// What `glue.popup_press` reports.
#[derive(serde::Deserialize)]
struct Pressed {
    ended: bool,
    build_flag: bool,
}

/// A popup session command changed the popup, and the display item behind it (a press can
/// save into it).
fn popup_changed() -> Affected {
    Affected {
        popup: true,
        ..Affected::default()
    }
}

impl PobBackend {
    fn apply_command(&mut self, command: Command, _send: &dyn Fn(Event)) -> Result<Affected> {
        match command {
            Command::SearchGems(query) => {
                let target = lua_target(&self.lua, query.target)?;
                self.call::<_, ()>(
                    "gem_search_open",
                    (
                        query.group + 1,
                        target,
                        query.text.clone(),
                        query.filter.pob_tag(),
                    ),
                )?;
                // a failed sort stopped with PoB's cache check satisfied, so the same search
                // wouldn't start it again: Retry asks for the restart
                if std::mem::take(&mut self.gem_failed) {
                    self.call::<_, ()>("gem_dps_restart", ())?;
                }
                self.gem_search = Some(query);
                self.gem_ended = None;
                Ok(Affected {
                    gem_search: true,
                    ..Affected::default()
                })
            }
            // ended above
            Command::CloseGemSearch => Ok(Affected::default()),
            Command::ItemTooltip(query) => Ok(Affected {
                item_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::CopyItem { id } => Ok(Affected {
                item_text: Some(id),
                ..Affected::default()
            }),
            Command::GemTooltip(query) => Ok(Affected {
                gem_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::ListBuilds { folder } => {
                // leaving a build stops the power work, as `SetHeatmap { stat: None }` does
                if self.heatmap_on {
                    self.call::<_, ()>("set_heatmap", (None::<String>, None::<u32>))?;
                    self.heatmap_on = false;
                    self.power_failed = false;
                }
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
                self.reset_heatmap();
                self.call::<_, ()>("new_build", ())?;
                self.all_views()
            }
            Command::OpenBuild { path } => {
                self.reset_heatmap();
                self.call::<_, ()>("open_build", path.to_string_lossy().into_owned())?;
                self.all_views()
            }
            Command::ImportShareCode { code } => {
                self.reset_heatmap();
                self.call::<_, ()>("import_code", code)?;
                self.all_views()
            }
            Command::Save => {
                self.call::<_, ()>("save", ())?;
                // PoB's save rewrites the notes' `^_` escapes
                Ok(Affected {
                    build: true,
                    list: true,
                    notes: true,
                    ..Affected::default()
                })
            }
            Command::SaveAs {
                folder,
                name,
                overwrite,
            } => {
                self.call::<_, ()>("save_as", (folder, name, overwrite))?;
                // PoB's save rewrites the notes' `^_` escapes
                Ok(Affected {
                    build: true,
                    list: true,
                    notes: true,
                    ..Affected::default()
                })
            }
            // run_loop handles Shutdown before applying commands; nothing to do here
            Command::Shutdown => Ok(Affected::default()),
            Command::PreviewNode { id } => Ok(Affected {
                preview: Some(id),
                ..Affected::default()
            }),
            Command::Breakdown(cell) => Ok(Affected {
                breakdown: Some(cell),
                ..Affected::default()
            }),
            Command::ExportTreeLink => Ok(Affected {
                link: true,
                ..Affected::default()
            }),
            Command::SetTreeSearch { text } => {
                self.call::<_, ()>("set_tree_search", text)?;
                Ok(Affected {
                    build: true,
                    tree: true,
                    ..Affected::default()
                })
            }
            Command::SetHeatmap { stat, max_depth } => {
                let off = stat.is_none();
                // an unknown stat is an error that changes nothing, so the flags stay too
                self.call::<_, ()>("set_heatmap", (stat, max_depth))?;
                self.power_failed = false;
                self.heatmap_on = !off;
                Ok(Affected {
                    tree: true,
                    power: true,
                    recalc: true,
                    ..Affected::default()
                })
            }
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
            Command::AllocNode { id } => {
                self.call::<_, ()>("alloc_node", id)?;
                self.tree_edited()
            }
            Command::DeallocNode { id } => {
                self.call::<_, ()>("dealloc_node", id)?;
                self.tree_edited()
            }
            Command::SelectClass { id, connect } => {
                self.call::<_, ()>("select_class", (id, connect))?;
                self.tree_edited()
            }
            Command::SelectAscendancy { id } => {
                self.call::<_, ()>("select_ascendancy", id)?;
                self.tree_edited()
            }
            Command::ResetTree => {
                self.call::<_, ()>("reset_tree", ())?;
                self.tree_edited()
            }
            Command::SetMastery { node, effect } => {
                self.call::<_, ()>("set_mastery", (node, effect))?;
                self.tree_edited()
            }
            Command::SelectSpec { index } => {
                self.call::<_, ()>("select_spec", index + 1)?;
                self.spec_edited(true)
            }
            Command::NewSpec { title } => {
                self.call::<_, ()>("new_spec", title)?;
                self.spec_edited(true)
            }
            Command::RenameSpec { index, title } => {
                self.call::<_, ()>("rename_spec", (index + 1, title))?;
                self.spec_edited(true)
            }
            Command::DeleteSpec { index } => {
                self.call::<_, ()>("delete_spec", index + 1)?;
                self.spec_edited(true)
            }
            Command::SetCompareSpec { index } => {
                self.call::<_, ()>("set_compare_spec", index.map(|i| i + 1))?;
                self.spec_edited(false)
            }
            Command::ConvertSpec { version, keep_copy } => {
                self.call::<_, ()>("convert_spec", (version, keep_copy))?;
                self.spec_edited(true)
            }
            Command::ConvertAllSpecs { version } => {
                self.call::<_, ()>("convert_all_specs", version)?;
                self.spec_edited(true)
            }
            Command::ImportTreeLink { title, url } => {
                self.call::<_, ()>("import_tree_link", (title, url))?;
                self.spec_edited(true)
            }
            Command::Undo(Tab::Tree) => {
                self.call::<_, ()>("tree_undo", ())?;
                self.tree_edited()
            }
            Command::Redo(Tab::Tree) => {
                self.call::<_, ()>("tree_redo", ())?;
                self.tree_edited()
            }
            Command::SetCalcsSelect { control, index } => {
                self.call::<_, ()>("calcs_select", (control.pob_name(), index + 1))?;
                Ok(calcs_changed(true))
            }
            Command::SetCalcsCount { control, text } => {
                self.call::<_, ()>("calcs_set_count", (control.pob_name(), text))?;
                Ok(calcs_changed(true))
            }
            Command::SetMainSocketGroup { group } => {
                self.call::<_, ()>("skills_set_main", group + 1)?;
                Ok(skills_changed())
            }
            Command::SetMainSkillSelect { control, index } => {
                self.call::<_, ()>("main_skill_select", (control.pob_name(), index + 1))?;
                Ok(skills_changed())
            }
            Command::SetMainSkillCount { control, text } => {
                self.call::<_, ()>("main_skill_set_count", (control.pob_name(), text))?;
                Ok(skills_changed())
            }
            Command::SetShowMinion { show } => {
                self.call::<_, ()>("calcs_show_minion", show)?;
                Ok(calcs_changed(false))
            }
            Command::SetCalcsSearch { text } => {
                self.call::<_, ()>("calcs_search", text)?;
                Ok(Affected {
                    calcs: true,
                    ..Affected::default()
                })
            }
            Command::SetCalcsCollapsed {
                section,
                sub,
                collapsed,
            } => {
                self.call::<_, ()>("calcs_collapse", (section, sub + 1, collapsed))?;
                Ok(calcs_changed(false))
            }
            Command::SetNotes { text } => {
                self.call::<_, ()>("set_notes", text)?;
                // no recalculation: the notes feed nothing
                Ok(Affected {
                    build: true,
                    notes: true,
                    ..Affected::default()
                })
            }
            Command::Undo(Tab::Calcs) => {
                self.call::<_, ()>("calcs_undo", ())?;
                Ok(calcs_changed(true))
            }
            Command::Redo(Tab::Calcs) => {
                self.call::<_, ()>("calcs_redo", ())?;
                Ok(calcs_changed(true))
            }
            Command::AddSocketGroup => {
                self.call::<_, ()>("skills_add_group", ())?;
                Ok(skills_changed())
            }
            Command::DeleteSocketGroup { group } => {
                self.call::<_, ()>("skills_delete_group", group + 1)?;
                Ok(skills_changed())
            }
            Command::DeleteAllSocketGroups => {
                self.call::<_, ()>("skills_delete_all", ())?;
                Ok(skills_changed())
            }
            Command::MoveSocketGroup { from, to } => {
                self.call::<_, ()>("skills_move_group", (from + 1, to + 1))?;
                Ok(skills_changed())
            }
            Command::PasteSocketGroup { text } => {
                self.call::<_, ()>("skills_paste", text)?;
                Ok(skills_changed())
            }
            Command::SetGroupLabel { group, text } => {
                self.call::<_, ()>("skills_set_label", (group + 1, text))?;
                Ok(skills_changed())
            }
            Command::SetGroupSlot { group, slot } => {
                self.call::<_, ()>("skills_set_slot", (group + 1, slot))?;
                Ok(skills_changed())
            }
            Command::SetGroupEnabled { group, on } => {
                self.call::<_, ()>("skills_set_enabled", (group + 1, on))?;
                Ok(skills_changed())
            }
            Command::SetGroupFullDps { group, on } => {
                self.call::<_, ()>("skills_set_full_dps", (group + 1, on))?;
                Ok(skills_changed())
            }
            Command::SetGroupCount { group, text } => {
                self.call::<_, ()>("skills_set_group_count", (group + 1, text))?;
                Ok(skills_changed())
            }
            Command::SetImbuedSupport { group, gem_id } => {
                self.call::<_, ()>("skills_set_imbued", (group + 1, gem_id))?;
                Ok(skills_changed())
            }
            Command::OptimiseSockets { group } => {
                self.call::<_, ()>("skills_optimise", group + 1)?;
                Ok(skills_changed())
            }
            Command::GroupTooltip { group } => Ok(Affected {
                group_tooltip: Some(group),
                ..Affected::default()
            }),
            Command::CopySocketGroup { group } => Ok(Affected {
                group_text: Some(group),
                ..Affected::default()
            }),
            Command::SetGem {
                group,
                slot,
                gem_id,
            } => {
                self.call::<_, ()>("skills_set_gem", (group + 1, slot + 1, gem_id))?;
                Ok(skills_changed())
            }
            Command::SetGemLevel { group, slot, text } => {
                self.call::<_, ()>("skills_set_gem_level", (group + 1, slot + 1, text))?;
                Ok(skills_changed())
            }
            Command::SetGemQuality { group, slot, text } => {
                self.call::<_, ()>("skills_set_gem_quality", (group + 1, slot + 1, text))?;
                Ok(skills_changed())
            }
            Command::SetGemCount { group, slot, text } => {
                self.call::<_, ()>("skills_set_gem_count", (group + 1, slot + 1, text))?;
                Ok(skills_changed())
            }
            Command::SetGemEnabled { group, slot, on } => {
                self.call::<_, ()>("skills_set_gem_enabled", (group + 1, slot + 1, on))?;
                Ok(skills_changed())
            }
            Command::SetGemToggle {
                group,
                slot,
                which,
                on,
            } => {
                let which = if which == GemToggle::First { 1 } else { 2 };
                self.call::<_, ()>("skills_set_gem_toggle", (group + 1, slot + 1, which, on))?;
                Ok(skills_changed())
            }
            Command::DeleteGem { group, slot } => {
                self.call::<_, ()>("skills_delete_gem", (group + 1, slot + 1))?;
                Ok(skills_changed())
            }
            Command::SelectSkillSet { id } => {
                self.call::<_, ()>("skills_select_set", id)?;
                Ok(skills_changed())
            }
            Command::NewSkillSet { title } => {
                self.call::<_, ()>("skills_new_set", title)?;
                Ok(skills_changed())
            }
            Command::CopySkillSet { id, title } => {
                self.call::<_, ()>("skills_copy_set", (id, title))?;
                Ok(skills_changed())
            }
            Command::DeleteSkillSet { id } => {
                self.call::<_, ()>("skills_delete_set", id)?;
                Ok(skills_changed())
            }
            Command::RenameSkillSet { id, title } => {
                self.call::<_, ()>("skills_rename_set", (id, title))?;
                // a title feeds no calculation, but the build is edited
                Ok(Affected {
                    build: true,
                    skills: true,
                    ..Affected::default()
                })
            }
            Command::SetSkillOption(option) => {
                match option {
                    SkillOption::SortByDps(on) => {
                        self.call::<_, ()>("skills_option_check", ("sortGemsByDPS", on))?
                    }
                    SkillOption::ShowLegacy(on) => {
                        self.call::<_, ()>("skills_option_check", ("showLegacyGems", on))?
                    }
                    SkillOption::SortField(i) => self.call::<_, ()>(
                        "skills_option_drop",
                        ("sortGemsByDPSFieldControl", i + 1),
                    )?,
                    SkillOption::DefaultLevel(i) => {
                        self.call::<_, ()>("skills_option_drop", ("defaultLevel", i + 1))?
                    }
                    SkillOption::SupportTypes(i) => {
                        self.call::<_, ()>("skills_option_drop", ("showSupportGemTypes", i + 1))?
                    }
                    SkillOption::DefaultQuality(text) => {
                        self.call::<_, ()>("skills_option_quality", text)?
                    }
                }
                // PoB's sort cache checks the options, so the DPS sort may start again
                // not unsaved, no recalculation (R18)
                Ok(Affected {
                    skills: true,
                    gem_search: self.gem_search.is_some(),
                    ..Affected::default()
                })
            }
            Command::Undo(Tab::Skills) => {
                self.call::<_, ()>("skills_undo", ())?;
                Ok(skills_changed())
            }
            Command::Redo(Tab::Skills) => {
                self.call::<_, ()>("skills_redo", ())?;
                Ok(skills_changed())
            }
            Command::SortItems => {
                self.call::<_, ()>("items_sort", ())?;
                Ok(items_listed())
            }
            Command::DeleteItem { id, confirmed } => {
                self.call::<_, ()>("items_delete", (id, confirmed))?;
                self.items_edited()
            }
            Command::DeleteUnusedItems => {
                self.call::<_, ()>("items_delete_unused", ())?;
                self.items_edited()
            }
            Command::DeleteAllItems => {
                self.call::<_, ()>("items_delete_all", ())?;
                self.items_edited()
            }
            Command::MoveItem { from, to } => {
                let (Some(lua_from), Some(lua_to)) = (from.checked_add(1), to.checked_add(1))
                else {
                    return Err(anyhow!("no such item"));
                };
                self.call::<_, ()>("items_move", (lua_from, lua_to))?;
                Ok(items_listed())
            }
            Command::SetItemListFilter { index } => {
                self.call::<_, ()>("items_set_filter", index + 1)?;
                Ok(Affected {
                    items: true,
                    ..Affected::default()
                })
            }
            Command::EquipItem { slot, id } => {
                self.call::<_, ()>("items_equip", (slot, id))?;
                self.items_edited()
            }
            Command::SetFlaskActive { slot, on } => {
                self.call::<_, ()>("items_set_flask", (slot, on))?;
                self.items_edited()
            }
            Command::SelectWeaponSet { second } => {
                self.call::<_, ()>("items_select_weapon_set", second)?;
                self.items_edited()
            }
            Command::SelectItemSet { id } => {
                self.call::<_, ()>("items_select_set", id)?;
                self.items_edited()
            }
            Command::NewItemSet { title } => {
                self.call::<_, ()>("items_new_set", title)?;
                self.items_edited()
            }
            Command::CopyItemSet { id, title } => {
                self.call::<_, ()>("items_copy_set", (id, title))?;
                self.items_edited()
            }
            Command::DeleteItemSet { id } => {
                self.call::<_, ()>("items_delete_set", id)?;
                self.items_edited()
            }
            Command::RenameItemSet { id, title } => {
                self.call::<_, ()>("items_rename_set", (id, title))?;
                Ok(items_listed())
            }
            Command::MoveItemSet { from, to } => {
                let (Some(lua_from), Some(lua_to)) = (from.checked_add(1), to.checked_add(1))
                else {
                    return Err(anyhow!("no such item set"));
                };
                self.call::<_, ()>("items_move_set", (lua_from, lua_to))?;
                // PoB's drop onto the same place changes nothing
                if from == to {
                    return Ok(Affected::default());
                }
                Ok(items_listed())
            }
            Command::PasteItem { text } => {
                self.call::<_, ()>("display_paste", text)?;
                Ok(display_changed())
            }
            Command::EditItem { id } => {
                self.call::<_, ()>("display_edit", id)?;
                Ok(display_changed())
            }
            Command::SetDisplayItemText { rarity, text } => {
                self.call::<_, ()>("display_set_text", (rarity, text))?;
                Ok(display_changed())
            }
            Command::CancelDisplayItem { if_id } => {
                self.call::<_, ()>("display_cancel", if_id)?;
                Ok(display_changed())
            }
            Command::SaveDisplayItem => {
                self.call::<_, ()>("display_save", ())?;
                Ok(Affected {
                    display_item: true,
                    ..self.items_edited()?
                })
            }
            Command::SetShowStatDifferences { on } => {
                let changed: bool = self.call("display_show_differences", on)?;
                Ok(Affected {
                    items: true,
                    display_item: true,
                    recalc: changed,
                    ..Affected::default()
                })
            }
            Command::SetItemVariant { control, index } => {
                self.call::<_, ()>("display_variant", (control, index + 1))?;
                Ok(display_changed())
            }
            Command::SetItemVersion { index } => {
                self.call::<_, ()>("display_version", index + 1)?;
                Ok(display_changed())
            }
            Command::SetItemQuality { text } => {
                self.call::<_, ()>("display_quality", text)?;
                Ok(display_changed())
            }
            Command::SetItemInfluence { which, index } => {
                self.call::<_, ()>("display_influence", (which, index + 1))?;
                Ok(display_changed())
            }
            Command::SetItemCatalyst { index } => {
                self.call::<_, ()>("display_catalyst", index + 1)?;
                Ok(display_changed())
            }
            Command::SetItemCatalystQuality { text } => {
                self.call::<_, ()>("display_catalyst_quality", text)?;
                Ok(display_changed())
            }
            Command::SetItemSocket { index, colour } => {
                let colour = match colour {
                    SocketColour::R => "R",
                    SocketColour::G => "G",
                    SocketColour::B => "B",
                    SocketColour::W => "W",
                };
                self.call::<_, ()>("display_socket", (index + 1, colour))?;
                Ok(display_changed())
            }
            Command::SetItemSocketLink { index, linked } => {
                self.call::<_, ()>("display_link", (index + 1, linked))?;
                Ok(display_changed())
            }
            Command::AddItemSocket => {
                self.call::<_, ()>("display_add_socket", ())?;
                Ok(display_changed())
            }
            Command::SetRangeLine { line, value } => {
                self.call::<_, ()>("display_range", (line + 1, value))?;
                Ok(display_changed())
            }
            Command::SetRangeLineMutated { line, on } => {
                self.call::<_, ()>("display_mutated", (line + 1, on))?;
                Ok(display_changed())
            }
            Command::ToggleModLine { line } => {
                let list = match line.list {
                    ModLineList::Enchant => "enchant",
                    ModLineList::Scourge => "scourge",
                    ModLineList::Implicit => "implicit",
                    ModLineList::Explicit => "explicit",
                    ModLineList::Crucible => "crucible",
                };
                self.call::<_, ()>("display_toggle_mod", (list, line.index + 1))?;
                Ok(Affected {
                    display_item: true,
                    recalc: true,
                    ..Affected::default()
                })
            }
            Command::OpenPopup(kind) => {
                let (name, arg) = match kind {
                    PopupKind::Craft => ("craft", 0),
                    PopupKind::EditText => ("edit_text", 0),
                    PopupKind::Enchant { second } => ("enchant", i64::from(second)),
                    PopupKind::Anoint { slot } => ("anoint", slot as i64 + 1),
                    PopupKind::Corrupt => ("corrupt", 0),
                    PopupKind::AddModifier => ("add_modifier", 0),
                    PopupKind::AddImplicit => ("add_implicit", 0),
                    PopupKind::AddCrucible => ("add_crucible", 0),
                    PopupKind::NodeReplacement { .. } => ("node_replacement", 0),
                };
                // Enchant's flag is a boolean for the glue
                if name == "enchant" {
                    self.call::<_, ()>("popup_open", (name, arg != 0))?;
                } else {
                    self.call::<_, ()>("popup_open", (name, arg))?;
                }
                Ok(popup_changed())
            }
            Command::SetPopupControl { name, value } => {
                match value {
                    PopupInput::Choice { index, label } => {
                        self.call::<_, ()>("popup_set", (name, "choice", index, label))?
                    }
                    PopupInput::Number(value) => {
                        self.call::<_, ()>("popup_set", (name, "number", value, ()))?
                    }
                    PopupInput::Check(on) => {
                        self.call::<_, ()>("popup_set", (name, "check", on, ()))?
                    }
                    PopupInput::Text(text) => {
                        self.call::<_, ()>("popup_set", (name, "text", text, ()))?
                    }
                    PopupInput::Row(key) => {
                        self.call::<_, ()>("popup_set", (name, "row", key, ()))?
                    }
                }
                Ok(popup_changed())
            }
            Command::PressPopupButton { name } => {
                let pressed: Pressed = self.view("popup_press", name)?;
                // a tattoo sets PoB's `buildFlag`: the tree changed (R22)
                let mut affected = if pressed.build_flag {
                    self.tree_edited()?
                } else {
                    Affected::default()
                };
                affected.popup = true;
                affected.display_item = true;
                // a popup PoB closed ends the session; the view says so by being empty
                let _ = pressed.ended;
                Ok(affected)
            }
            Command::ClosePopup => {
                self.call::<_, ()>("popup_close", ())?;
                Ok(popup_changed())
            }
            Command::ControlTooltip(query) => Ok(Affected {
                control_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::Undo(Tab::Items) => {
                self.call::<_, ()>("items_undo", ())?;
                self.items_edited()
            }
            Command::Redo(Tab::Items) => {
                self.call::<_, ()>("items_redo", ())?;
                self.items_edited()
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
}

/// How much Lua's heap must have grown since the last release for another to be worth a full
/// collection. A node power or gem DPS sort leaves hundreds of megabytes of garbage; a tooltip
/// or a single edit leaves little.
const RELEASE_GROWTH_BYTES: usize = 32 << 20;

impl Backend for PobBackend {
    fn apply(&mut self, command: Command, send: &dyn Fn(Event)) -> Result<Affected> {
        // a structural command ends the search only when it went through (R13); one that
        // failed changed nothing, and a changed group table ends the search on its own
        let ends = ends_gem_search(&command);
        let closes = matches!(command, Command::CloseGemSearch);
        // any other command may change what the popup is about, so it ends the session first
        // and the window is told (R4)
        let popup_ended = ends_popup_session(&command) && self.call::<_, bool>("popup_end", ())?;
        let mut affected = self.apply_command(command, send)?;
        affected.popup |= popup_ended;
        if ends {
            // the picker is told it ended, unless it asked to close
            let live = self.gem_search.clone();
            self.end_gem_search()?;
            if !closes && live.is_some() {
                self.gem_ended = live;
                affected.gem_search = true;
            }
        }
        Ok(affected)
    }

    fn recalculate(&mut self) -> Result<()> {
        // a new output revision makes PoB's sort cache start the DPS sort again
        self.call::<_, ()>("recalculate", ())
    }

    fn build_view(&mut self) -> Result<BuildView> {
        self.view("build_view", ())
    }

    fn build_list_view(&mut self) -> Result<BuildListView> {
        self.view("list_builds", self.folder.clone())
    }

    fn build_loaded(&mut self) -> Result<bool> {
        self.call("build_loaded", ())
    }

    fn stats_view(&mut self) -> Result<StatsView> {
        self.view("stats_view", ())
    }

    fn config_view(&mut self) -> Result<ConfigView> {
        self.view("config_view", ())
    }

    fn tree_layout_view(&mut self) -> Result<TreeLayout> {
        let layout = super::tree::layout(&self.lua, &self.pob_dir).map_err(|e| anyhow!("{e}"))?;
        self.last_layout_key = Some(layout.key.clone());
        Ok(layout)
    }

    fn tree_state_view(&mut self) -> Result<TreeState> {
        self.view("tree_state", ())
    }

    fn calcs_view(&mut self) -> Result<CalcsView> {
        self.view("calcs_view", ())
    }

    fn skills_view(&mut self) -> Result<SkillsView> {
        self.view("skills_view", ())
    }

    fn items_view(&mut self) -> Result<ItemsView> {
        self.view("items_view", ())
    }

    fn display_item_view(&mut self) -> Result<Option<DisplayItemView>> {
        self.view("display_item_view", ())
    }

    fn socket_group_text(&mut self, group: usize) -> Result<SocketGroupText> {
        let text: String = self.call("skills_copy", group + 1)?;
        Ok(SocketGroupText { group, text })
    }

    fn group_tooltip_view(&mut self, group: usize) -> Result<GroupTooltipView> {
        self.view("skills_group_tooltip", group + 1)
    }

    fn notes_view(&mut self) -> Result<NotesView> {
        self.view("notes_view", ())
    }

    fn next_background(&self) -> Option<BackgroundTask> {
        if self.gem_search.is_some()
            && !self.gem_failed
            && self.call::<_, bool>("gem_dps_pending", ()).unwrap_or(false)
        {
            return Some(BackgroundTask::GemDps);
        }
        let power = self.heatmap_on
            && !self.power_failed
            && self.call::<_, bool>("power_pending", ()).unwrap_or(false);
        power.then_some(BackgroundTask::NodePower)
    }

    fn background_step(&mut self, task: BackgroundTask, send: &dyn Fn(Event)) -> Result<Affected> {
        #[derive(serde::Deserialize)]
        struct Step {
            percent: Option<u32>,
            finished: bool,
        }
        match task {
            BackgroundTask::NodePower => {
                let step: Step = match self.view("power_step", ()) {
                    Ok(step) => step,
                    Err(e) => {
                        self.power_failed = true;
                        return Err(e);
                    }
                };
                if let Some(percent) = step.percent {
                    send(Event::Progress {
                        task: BackgroundTask::NodePower,
                        percent,
                    });
                }
                Ok(Affected {
                    power: true,
                    report: step.finished,
                    ..Affected::default()
                })
            }
            BackgroundTask::GemDps => {
                let step: Step = match self.view("gem_dps_step", ()) {
                    Ok(step) => step,
                    Err(e) => {
                        self.gem_failed = true;
                        return Err(e);
                    }
                };
                if let Some(percent) = step.percent {
                    send(Event::Progress {
                        task: BackgroundTask::GemDps,
                        percent,
                    });
                }
                // an ended search has no more work; its view says so
                Ok(Affected {
                    gem_search: true,
                    ..Affected::default()
                })
            }
        }
    }

    fn gem_search_view(&mut self) -> Result<Option<GemSearchView>> {
        #[derive(serde::Deserialize)]
        struct Found {
            #[serde(default)]
            rows: Vec<GemRow>,
            #[serde(default)]
            base_dps: f64,
            #[serde(default)]
            sorting: bool,
            #[serde(default)]
            percent: Option<u32>,
            #[serde(default)]
            ended: bool,
        }
        if let Some(query) = self.gem_ended.take() {
            return Ok(Some(GemSearchView {
                query,
                rows: vec![],
                base_dps: 0.0,
                sorting: false,
                percent: None,
                ended: true,
            }));
        }
        let Some(query) = self.gem_search.clone() else {
            return Ok(None);
        };
        let found: Found = self.view("gem_search_view", ())?;
        if found.ended {
            self.end_gem_search()?;
        }
        Ok(Some(GemSearchView {
            query,
            rows: found.rows,
            base_dps: found.base_dps,
            sorting: found.sorting && !found.ended && !self.gem_failed,
            percent: found.percent,
            ended: found.ended,
        }))
    }

    fn gem_tooltip_view(&mut self, query: &GemTooltipQuery) -> Result<GemTooltipView> {
        #[derive(serde::Deserialize)]
        struct Found {
            #[serde(default)]
            lines: Vec<TooltipLine>,
        }
        let target = lua_target(&self.lua, query.target)?;
        let found: Found = self.view(
            "gem_tooltip",
            (query.group + 1, target, query.gem_id.clone()),
        )?;
        Ok(GemTooltipView {
            query: query.clone(),
            lines: found.lines,
        })
    }

    fn item_tooltip_view(&mut self, query: &ItemTooltipQuery) -> Result<Option<ItemTooltipView>> {
        let found: Option<ItemTooltip> =
            self.view("item_tooltip", (query.id, query.slot.clone()))?;
        Ok(found.map(|tooltip| ItemTooltipView {
            query: query.clone(),
            tooltip,
        }))
    }

    fn popup_view(&mut self) -> Result<Option<PopupView>> {
        self.view("popup_view", ())
    }

    fn control_tooltip_view(
        &mut self,
        query: &ControlTooltipQuery,
    ) -> Result<Option<ControlTooltipView>> {
        let found: Option<ItemTooltip> = match query {
            ControlTooltipQuery::Popup { control, index } => {
                self.view("popup_tooltip", (control.clone(), index.map(|i| i as i64)))?
            }
            // the affix editor and the item databases answer in later tasks
            ControlTooltipQuery::Affix { .. } | ControlTooltipQuery::DbItem { .. } => None,
        };
        Ok(found.map(|tooltip| ControlTooltipView {
            query: query.clone(),
            tooltip,
        }))
    }

    fn item_text(&mut self, id: u32) -> Result<ItemText> {
        let text: String = self.call("item_copy", id)?;
        Ok(ItemText { id, text })
    }

    fn node_power_view(&mut self) -> Result<NodePower> {
        self.view("node_power", ())
    }

    fn power_report_view(&mut self) -> Result<PowerReport> {
        self.view("power_report", ())
    }

    fn node_tooltip_view(&mut self, id: u32) -> Result<Option<NodeTooltip>> {
        self.view("node_tooltip", id)
    }

    fn breakdown_view(&mut self, cell: &CellRef) -> Result<BreakdownView> {
        self.view(
            "calcs_breakdown",
            (cell.section.clone(), cell.sub, cell.row, cell.cell),
        )
    }

    fn release_memory(&mut self) {
        let before = self.lua.used_memory();
        if before < self.released_lua_bytes + RELEASE_GROWTH_BYTES {
            return;
        }
        let start = std::time::Instant::now();
        // LuaJIT runs its collector only as it allocates, so the garbage a job left stays
        // until something else allocates; and what Lua frees stays in malloc's arenas.
        if let Err(e) = self.lua.gc_collect() {
            log::warn!("Lua collection failed: {e}");
        }
        super::memory::release_free_memory();
        self.released_lua_bytes = self.lua.used_memory();
        log::debug!(
            "released memory in {:?}: Lua heap {} MB -> {} MB, RSS {} MB",
            start.elapsed(),
            before >> 20,
            self.released_lua_bytes >> 20,
            super::memory::resident_bytes().map_or(0, |b| b >> 20),
        );
    }

    fn tree_link_view(&mut self) -> Result<String> {
        self.call("tree_link", ())
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

    #[test]
    fn queries_do_not_end_the_popup_session() {
        use crate::engine::{CellRef, GemTarget, ItemDb};
        let keeps = [
            Command::ItemTooltip(ItemTooltipQuery { id: 1, slot: None }),
            Command::ControlTooltip(ControlTooltipQuery::Popup {
                control: "save".into(),
                index: None,
            }),
            Command::ControlTooltip(ControlTooltipQuery::DbItem {
                db: ItemDb::Uniques,
                name: "x".into(),
            }),
            Command::CopyItem { id: 1 },
            Command::Breakdown(CellRef {
                section: "s".into(),
                sub: 0,
                row: 0,
                cell: 0,
            }),
            Command::GemTooltip(GemTooltipQuery {
                group: 0,
                target: GemTarget::Slot(0),
                gem_id: None,
            }),
            Command::OpenPopup(PopupKind::Craft),
            Command::SetPopupControl {
                name: "rarity".into(),
                value: PopupInput::Check(true),
            },
            Command::PressPopupButton {
                name: "save".into(),
            },
            Command::ClosePopup,
        ];
        for command in keeps {
            assert!(
                !ends_popup_session(&command),
                "{command:?} ends the session"
            );
        }
        // anything that edits the item, the build or the tab does
        let ends = [
            Command::SortItems,
            Command::PasteItem {
                text: String::new(),
            },
            Command::SetMainSkillCount {
                control: crate::engine::CalcsCount::Stages,
                text: "1".into(),
            },
        ];
        for command in ends {
            assert!(
                ends_popup_session(&command),
                "{command:?} keeps the session"
            );
        }
    }

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
