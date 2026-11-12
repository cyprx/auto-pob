//! Starts Path of Building's Lua with our own bootstrap.

use super::buildpaths;
use super::commands::{
    CharacterPart, Command, CompareSource, ConfigValue, ControlTooltipQuery, DeleteKind,
    GemSearchQuery, GemTarget, GemToggle, GemTooltipQuery, ImportMode, ItemDb, ItemDbQuery,
    ItemTooltipQuery, PopupInput, PopupKind, SkillOption, SocketColour, Tab,
};
use super::net::{Dispatch, NetHandle};
use super::run::{Affected, Backend, BackgroundTask, DbSnapshot, Engine, Event};
use super::views::{
    AccountView, BreakdownView, BuildListView, BuildView, CalcsView, CellRef, CompareView,
    ConfigView, ControlTooltipView, DisplayItemView, GemRow, GemSearchView, GemTooltipView,
    GroupTooltipView, ItemDbView, ItemText, ItemTooltip, ItemTooltipView, ItemsView, ModLineList,
    NodePower, NodeTooltip, NotesView, PopupView, PowerReport, RequestsView, SettingsView,
    ShareView, SkillsView, SocketGroupText, StatsView, TooltipLine, TreeLayout, TreeState,
};
use crate::game::Game;
use anyhow::{Context as _, Result, anyhow};
use mlua::{FromLuaMulti, IntoLuaMulti, Lua, LuaSerdeExt};
use pob_runtime::net::{NetConfig, Waker};
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
    /// An Anoint sort step failed; the job stays stopped until the window is opened again or
    /// one of its controls is set.
    anoint_failed: bool,
    /// A command ended the open search; its last view (`ended: true`) is still to be sent.
    gem_ended: Option<GemSearchQuery>,
    /// The open item database search (R13); `None` when no panel is open
    item_db: Option<ItemDbQuery>,
    /// An ItemDb step failed; the job stays stopped until the next `SetItemDb`.
    item_db_failed: bool,
    /// The database closed or the build changed; `ItemDb(None)` is still to be sent (R21)
    item_db_ended: bool,
    /// Lua's heap after the last memory release, in bytes
    released_lua_bytes: usize,
    /// The subscripts of PoB's requests (R4, R9)
    net: NetHandle,
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
    /// Starts PoB from `pob_dir`, keeping its user data in `user_dir`; requests go out
    /// through curl with the app's timeouts (R7).
    pub fn start(game: Game, pob_dir: &Path, user_dir: &Path) -> Result<PobBackend> {
        PobBackend::start_with(game, pob_dir, user_dir, NetConfig::app())
    }

    /// `start` with the requests' timeouts and transport given (tests use the canned one).
    pub fn start_with(
        game: Game,
        pob_dir: &Path,
        user_dir: &Path,
        net: NetConfig,
    ) -> Result<PobBackend> {
        // mlua's error isn't Send + Sync, which anyhow requires; keep its message instead
        let lua_error = |e: mlua::Error| anyhow!("{e}");

        let lua = new_lua(pob_dir)?;
        super::native::register(&lua, pob_dir, user_dir).map_err(lua_error)?;
        // before the bootstrap: its first frame already initialises a build (R4)
        let net = super::net::register(&lua, pob_dir, net).map_err(lua_error)?;
        lua.load(include_str!("bootstrap.lua"))
            .set_name("@bootstrap.lua")
            .exec()
            .map_err(lua_error)
            .context("Starting Path of Building")?;
        for (name, source) in [
            ("@glue/builds.lua", include_str!("glue/builds.lua")),
            ("@glue/buildlist.lua", include_str!("glue/buildlist.lua")),
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
            ("@glue/itemdb.lua", include_str!("glue/itemdb.lua")),
            ("@glue/notes.lua", include_str!("glue/notes.lua")),
            ("@glue/net.lua", include_str!("glue/net.lua")),
            ("@glue/import.lua", include_str!("glue/import.lua")),
            ("@glue/settings.lua", include_str!("glue/settings.lua")),
            ("@glue/compare.lua", include_str!("glue/compare.lua")),
        ] {
            lua.load(source)
                .set_name(name)
                .exec()
                .map_err(lua_error)
                .with_context(|| format!("Loading {name}"))?;
        }
        // PoB has loaded Settings.xml: the shared values are what it holds now (R12)
        let glue: mlua::Table = lua.globals().get("glue").map_err(lua_error)?;
        let init: mlua::Function = glue.get("settings_init").map_err(lua_error)?;
        init.call::<()>(()).map_err(lua_error)?;
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
            anoint_failed: false,
            gem_ended: None,
            item_db: None,
            item_db_failed: false,
            item_db_ended: false,
            released_lua_bytes: 0,
            net,
        })
    }

    /// The subscripts and launch gate of this PoB (for the gate harness).
    pub fn net_handle(&self) -> &NetHandle {
        &self.net
    }

    /// The Builds folder as PoB holds it (`main.buildPath`).
    fn build_root(&self) -> Result<String> {
        let root: String = self.call("build_root", ())?;
        // a recursive RemoveDir may delete only below it (R16)
        if std::fs::canonicalize(&root).is_err() {
            anyhow::bail!("the Builds folder doesn't exist");
        }
        if !super::native::set_builds_root(&self.lua, Some(Path::new(&root))) {
            anyhow::bail!("that Builds folder isn't allowed");
        }
        Ok(root)
    }

    /// `path` as PoB spells an entry of the Builds folder, or why it is refused (R16).
    fn build_entry(&self, path: &Path, walks: bool) -> Result<String> {
        buildpaths::entry(&self.build_root()?, path, walks)
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

    /// Leaving or replacing the build (R14): ends the popup session, the item database
    /// (`Ended`) and the gem search, and turns the heat map off. Returns what to tell the UI.
    fn left_build(&mut self) -> Result<Affected> {
        let mut affected = Affected::default();
        self.reset_heatmap();
        // an Options window survives a build change (R19)
        if self.call::<_, bool>("popup_end", true)? {
            affected.popup = true;
            affected.settings = true;
        }
        if self.item_db.take().is_some() {
            self.item_db_failed = false;
            self.item_db_ended = true;
            self.call::<_, ()>("item_db_close", ())?;
            affected.item_db = true;
        }
        let live = self.gem_search.clone();
        self.end_gem_search()?;
        if live.is_some() {
            self.gem_ended = live;
            affected.gem_search = true;
        }
        Ok(affected)
    }

    /// Cancels the pending requests (R12, R13): every one, or only the build's.
    fn cancel_all_requests(&mut self, scope: CancelScope) -> Result<()> {
        let scope = match scope {
            CancelScope::All => None,
            CancelScope::Build => Some("build"),
        };
        self.call::<_, ()>("net_cancel_all", scope)
    }

    /// Cancels one request: its subscripts are aborted, then each goes through PoB's failure
    /// path with "Cancelled" (R12).
    fn cancel_request(&mut self, id: u64) -> Result<()> {
        let live: Vec<u64> = self.call("net_live_ids", id)?;
        for sub in &live {
            self.net.abort(*sub);
        }
        for sub in live {
            self.call::<_, ()>("net_fail", (sub, "Cancelled"))?;
        }
        Ok(())
    }

    /// What the network callbacks since the last call did to the build (R14), plus the
    /// requests view.
    fn network_affected(&mut self) -> Result<Affected> {
        #[derive(serde::Deserialize)]
        struct Effects {
            rebuilt: bool,
            edited: bool,
        }
        // a callback that made a new build cancels every other request first (R13), so no late
        // result lands on it; the cancelled entries go out with this step's requests view
        self.call::<_, ()>("net_cancel_replaced", ())?;
        let effects: Effects = self.view("net_take_effects", ())?;
        let mut affected = if effects.rebuilt {
            let mut affected = self.left_build()?;
            affected.merge(self.all_views()?);
            // the build was initialised inside a callback, not in a frame (the loop calculates)
            affected.recalc = true;
            affected
        } else if effects.edited {
            Affected {
                build: true,
                tree: true,
                items: true,
                skills: true,
                config: true,
                recalc: true,
                tree_layout: self.layout_changed()?,
                ..Affected::default()
            }
        } else {
            Affected::default()
        };
        affected.requests = true;
        // a request of the account import ends with PoB's status and stage changed
        affected.account = true;
        // a link imported as a comparison lands here (R5)
        affected.compare = true;
        affected.short_link = self.call("short_link_ready", ())?;
        if self.call::<_, bool>("popup_take_lost", ())? {
            affected.popup = true;
            affected.settings = true;
        }
        Ok(affected)
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
/// A build list command changed the list and nothing else.
fn list_changed() -> Affected {
    Affected {
        list: true,
        ..Affected::default()
    }
}

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

/// Which pending requests a command cancels before it acts (R13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelScope {
    /// Every request: the build is replaced
    All,
    /// Only `scope = "build"` ones: the build is left for the list, where a list-started
    /// import goes on
    Build,
}

pub(crate) fn cancels_requests(command: &Command) -> Option<CancelScope> {
    match command {
        // a comparison leaves the build as it is (R5)
        Command::ImportFromUrl {
            mode: ImportMode::Comparison,
            ..
        } => None,
        Command::OpenBuild { .. }
        | Command::NewBuild
        | Command::ImportShareCode { .. }
        | Command::ImportFromUrl { .. } => Some(CancelScope::All),
        Command::ListBuilds { .. } => Some(CancelScope::Build),
        _ => None,
    }
}

/// The import mode as the glue spells it.
fn mode_name(mode: ImportMode) -> &'static str {
    match mode {
        ImportMode::ThisBuild => "this",
        ImportMode::NewBuild => "new",
        ImportMode::Comparison => "compare",
    }
}

/// The one Compare control that edits this build: the Items view's own item set (R6)
const PRIMARY_ITEM_SET: &str = "primaryItemSetSelect";

/// A Compare command changed the comparisons; the tree goes too, for the overlay (R7).
fn compare_changed() -> Affected {
    Affected {
        compare: true,
        tree: true,
        ..Affected::default()
    }
}

/// Commands that leave or replace the build (R14).
fn leaves_build(command: &Command) -> bool {
    matches!(
        command,
        Command::ListBuilds { .. }
            | Command::NewBuild
            | Command::OpenBuild { .. }
            | Command::ImportShareCode { .. }
    )
}

/// Commands that close the item database panel: its own Close, or leaving the build (R21).
pub(crate) fn ends_item_db(command: &Command) -> bool {
    matches!(
        command,
        Command::CloseItemDb
            | Command::ListBuilds { .. }
            | Command::NewBuild
            | Command::OpenBuild { .. }
            | Command::ImportShareCode { .. }
    )
}

/// The name `glue/itemdb.lua` knows a database by (also its serde name)
fn db_name(db: ItemDb) -> &'static str {
    match db {
        ItemDb::Uniques => "Uniques",
        ItemDb::Rares => "Rares",
    }
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
    if let Command::SetCompareControl { name, .. } = command {
        return name == PRIMARY_ITEM_SET;
    }
    !matches!(
        command,
        Command::OpenPopup(_)
            | Command::SetPopupControl { .. }
            | Command::PressPopupButton { .. }
            | Command::ClosePopup
            // the app's own values; they change neither the item nor the build (R12)
            | Command::SetOverrides { .. }
            | Command::ControlTooltip(_)
            | Command::SetItemDb(_)
            | Command::CloseItemDb
            | Command::ItemTooltip(_)
            | Command::CopyItem { .. }
            | Command::Breakdown(_)
            | Command::GemTooltip(_)
            | Command::GroupTooltip { .. }
            | Command::CopySocketGroup { .. }
            | Command::PreviewNode { .. }
            | Command::ExportTreeLink
            // a shrink only reads the tree and asks the network (R20)
            | Command::ShrinkTreeLink
            // sharing reads the build and asks the network; the import box is PoB's own (R16, R17)
            | Command::SetImportCode { .. }
            | Command::ShareBuild { .. }
            // a link replaces the build only when it arrives, and a code ends the session
            // itself (`left_build`)
            | Command::ImportFromUrl { .. }
            | Command::ExportShareCode
            | Command::ListPublicCharacters { .. }
            | Command::ImportPublicCharacter { .. }
            | Command::ClosePublicCharacters
            | Command::SignIn
            | Command::SignOut
            | Command::ListCharacters { .. }
            | Command::ImportCharacter { .. }
            // the tree search's debounce can land after a node-replacement open: it changes
            // neither the item nor the build
            | Command::SetTreeSearch { .. }
            // the comparisons are apart from this build; only a copy into it, or a pick of its
            // own item set, edits it (R5-R8)
            | Command::LoadCompare { .. }
            | Command::SelectCompare { .. }
            | Command::RemoveCompare { .. }
            | Command::SetCompareOverlay { .. }
            // a cancel runs PoB's failure path, which leaves the build alone (R12)
            | Command::CancelRequest { .. }
            | Command::Wake
            | Command::Shutdown
    )
}

/// A part of a character import as the glue takes it: the button and its options.
fn part_args(part: &CharacterPart) -> (&'static str, bool, bool, bool) {
    match *part {
        CharacterPart::Tree { delete_jewels } => ("tree", delete_jewels, false, false),
        CharacterPart::ItemsAndSkills {
            clear_items,
            clear_skills,
            ignore_weapon_swap,
        } => ("items", clear_items, clear_skills, ignore_weapon_swap),
    }
}

/// What `glue.popup_press` reports.
#[derive(serde::Deserialize)]
struct Pressed {
    build_flag: bool,
}

/// A popup session command changed the popup only (a press also flags the display item).
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
            Command::SetItemDb(query) => {
                let table = self.lua.create_table().map_err(|e| anyhow!("{e}"))?;
                for (key, value) in [
                    ("slot", query.slot),
                    ("kind", query.kind),
                    ("league", query.league),
                    ("requirement", query.requirement),
                    ("obtainable", query.obtainable),
                    ("sort", query.sort),
                    ("search_mode", query.search_mode),
                ] {
                    table.set(key, value as i64).map_err(|e| anyhow!("{e}"))?;
                }
                table
                    .set("search", query.search.clone())
                    .map_err(|e| anyhow!("{e}"))?;
                // a refused query leaves a failed job failed
                let restart = self.item_db_failed;
                self.call::<_, ()>("item_db_open", (db_name(query.db), table, restart))?;
                self.item_db_failed = false;
                self.item_db = Some(query);
                self.item_db_ended = false;
                Ok(Affected {
                    item_db: true,
                    ..Affected::default()
                })
            }
            // ended above
            Command::CloseItemDb => Ok(Affected::default()),
            Command::ViewDbItem { db, name } => {
                self.call::<_, ()>("item_db_view_item", (db_name(db), name))?;
                Ok(display_changed())
            }
            Command::AddDbItem { db, name } => {
                self.call::<_, ()>("item_db_add", (db_name(db), name))?;
                self.items_edited()
            }
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
                self.folder = buildpaths::folder(&self.build_root()?, &folder, false, false)?;
                Ok(Affected {
                    list: true,
                    ..Affected::default()
                })
            }
            Command::RenameBuild { path, name } => {
                let path = self.build_entry(&path, false)?;
                buildpaths::name_free(&path, &name, true)?;
                self.call::<_, ()>("rename_build", (path, name))?;
                Ok(list_changed())
            }
            Command::CopyBuild { path, name } => {
                let path = self.build_entry(&path, true)?;
                buildpaths::name_free(&path, &name, false)?;
                self.call::<_, ()>("copy_build", (path, name))?;
                Ok(list_changed())
            }
            Command::DeleteBuild { path, kind } => {
                let path = self.build_entry(&path, false)?;
                let kind_name = match kind {
                    DeleteKind::Build => "build",
                    DeleteKind::EmptyFolder => "empty",
                    DeleteKind::Folder => "folder",
                };
                // the contents are looked at again: what the user was asked about must be
                // what is deleted
                // (a link to a folder is a folder, as PoB lists it)
                let looked = std::fs::metadata(&path).ok();
                let is_dir = looked.as_ref().is_some_and(|m| m.is_dir());
                let has_contents = is_dir
                    && std::fs::read_dir(&path).is_ok_and(|mut entries| entries.next().is_some());
                let as_asked = match kind {
                    DeleteKind::Build => looked.is_some() && !is_dir,
                    DeleteKind::EmptyFolder => is_dir && !has_contents,
                    DeleteKind::Folder => is_dir && has_contents,
                };
                if !as_asked {
                    anyhow::bail!("the folder changed; look again");
                }
                self.call::<_, ()>("delete_build", (path.clone(), kind_name))?;
                // PoB ignores what its delete answers: look at the file system
                if std::fs::symlink_metadata(&path).is_ok() {
                    anyhow::bail!("couldn't delete that build");
                }
                Ok(list_changed())
            }
            Command::MoveBuild { path, folder } => {
                let root = self.build_root()?;
                let path = self.build_entry(&path, true)?;
                let folder = buildpaths::folder(&root, &folder, true, true)?;
                buildpaths::move_target_free(&path, &root, &folder)?;
                self.call::<_, ()>("move_build", (path, folder))?;
                Ok(list_changed())
            }
            Command::CopyBuildTo { path, folder } => {
                let root = self.build_root()?;
                let path = self.build_entry(&path, true)?;
                let folder = buildpaths::folder(&root, &folder, true, true)?;
                buildpaths::move_target_free(&path, &root, &folder)?;
                self.call::<_, ()>("copy_build_to", (path, folder))?;
                Ok(list_changed())
            }
            Command::NewFolder { name } => {
                let folder = buildpaths::folder(&self.build_root()?, &self.folder, true, true)?;
                self.call::<_, ()>("new_folder", (folder, name))?;
                Ok(list_changed())
            }
            Command::NewFolderIn { folder, name } => {
                let folder = buildpaths::folder(&self.build_root()?, &folder, true, true)?;
                self.call::<_, ()>("new_folder", (folder, name))?;
                Ok(list_changed())
            }
            Command::SetBuildSort { sort } => {
                self.call::<_, ()>("set_build_sort", sort.id())?;
                // the sort is a setting too (R12)
                Ok(Affected {
                    settings: true,
                    ..list_changed()
                })
            }
            Command::SetBuildFilter { text } => {
                self.call::<_, ()>("set_build_filter", text)?;
                Ok(list_changed())
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
            Command::SetImportCode { text } => {
                self.call::<_, ()>("set_import_code", text)?;
                Ok(Affected {
                    share: true,
                    ..Affected::default()
                })
            }
            Command::ImportFromUrl { url, mode } => {
                let comparison = mode == ImportMode::Comparison;
                let mode = mode_name(mode);
                // a link is a request, finished by a later Network step; a code imports at once
                let request: Option<u64> = self.call("import_from_url", (url, mode))?;
                let mut affected = match request {
                    Some(_) => self.network_affected()?,
                    // a code added a comparison at once; the build is as it was (R5)
                    None if comparison => compare_changed(),
                    // a code replaced the build at once; a link does so only when its request
                    // ends (`network_affected`), and a failed fetch keeps the build as it was
                    None => {
                        let mut affected = self.left_build()?;
                        affected.merge(self.all_views()?);
                        affected
                    }
                };
                affected.share = true;
                Ok(affected)
            }
            Command::ShareBuild { site } => {
                self.call::<_, u64>("share_build", site)?;
                let mut affected = self.network_affected()?;
                affected.share = true;
                Ok(affected)
            }
            // the box is emptied, as PoB's Copy button does, so a link showing is gone
            Command::ExportShareCode => Ok(Affected {
                share: true,
                share_code: true,
                ..Affected::default()
            }),
            Command::ListPublicCharacters { account, realm } => {
                self.call::<_, u64>("list_public_characters", (account, realm.pob_id()))?;
                self.network_affected()
            }
            Command::ImportPublicCharacter { league, name, part } => {
                let (kind, a, b, c) = part_args(&part);
                self.call::<_, u64>("import_public_character", (league, name, kind, a, b, c))?;
                self.network_affected()
            }
            Command::ClosePublicCharacters => {
                self.call::<_, ()>("close_public_characters", ())?;
                Ok(Affected {
                    account: true,
                    ..Affected::default()
                })
            }
            Command::SignIn => {
                self.call::<_, u64>("sign_in", ())?;
                self.network_affected()
            }
            Command::SignOut => {
                self.call::<_, ()>("sign_out", ())?;
                Ok(Affected {
                    account: true,
                    ..Affected::default()
                })
            }
            // nothing may be fetched (the list is there, or PoB's limiter answered at once)
            Command::ListCharacters { realm } => {
                self.call::<_, Option<u64>>("list_characters", realm.pob_id())?;
                self.network_affected()
            }
            Command::ImportCharacter {
                realm,
                league,
                name,
                part,
            } => {
                let (kind, a, b, c) = part_args(&part);
                self.call::<_, Option<u64>>(
                    "import_character",
                    (realm.pob_id(), league, name, kind, a, b, c),
                )?;
                self.network_affected()
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
            // run_loop handles Shutdown before applying commands, and never applies Wake
            Command::Shutdown | Command::Wake => Ok(Affected::default()),
            Command::CancelRequest { id } => {
                self.cancel_request(id)?;
                self.network_affected()
            }
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
            Command::ShrinkTreeLink => {
                self.call::<_, u64>("shrink_tree_link", ())?;
                self.network_affected()
            }
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
            Command::RemoveAllTattoos => {
                self.call::<_, ()>("remove_all_tattoos", ())?;
                self.tree_edited()
            }
            Command::LoadCompare { source } => {
                match source {
                    CompareSource::File(path) => self
                        .call::<_, ()>("compare_load_file", path.to_string_lossy().into_owned())?,
                    CompareSource::Code(code) => self.call::<_, ()>("compare_load_code", code)?,
                }
                Ok(compare_changed())
            }
            Command::SelectCompare { index } => {
                self.call::<_, ()>("compare_select", index + 1)?;
                Ok(compare_changed())
            }
            Command::RemoveCompare { index } => {
                self.call::<_, ()>("compare_remove", index + 1)?;
                Ok(compare_changed())
            }
            Command::SetCompareControl { name, value } => {
                let primary = name == PRIMARY_ITEM_SET;
                match value {
                    PopupInput::Choice { index, label } => {
                        self.call::<_, ()>("compare_set", (name, "choice", index, label))?
                    }
                    PopupInput::Number(value) => {
                        self.call::<_, ()>("compare_set", (name, "number", value, ()))?
                    }
                    PopupInput::Check(on) => {
                        self.call::<_, ()>("compare_set", (name, "check", on, ()))?
                    }
                    PopupInput::Text(text) => {
                        self.call::<_, ()>("compare_set", (name, "text", text, ()))?
                    }
                    PopupInput::Row(key) => {
                        self.call::<_, ()>("compare_set", (name, "row", key, ()))?
                    }
                }
                let mut affected = compare_changed();
                // PoB's own item set selector, with an Items undo state (CompareTab.lua:781)
                if primary {
                    affected.merge(self.items_edited()?);
                }
                Ok(affected)
            }
            Command::CopyCompareTree { and_use } => {
                self.call::<_, ()>("compare_copy_tree", and_use)?;
                let mut affected = self.spec_edited(true)?;
                affected.compare = true;
                Ok(affected)
            }
            Command::SetCompareOverlay { on } => {
                self.call::<_, ()>("compare_overlay", on)?;
                Ok(Affected {
                    tree: true,
                    ..Affected::default()
                })
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
                // a PoEURL link is a request; the tree is added when its step runs (R20)
                let request: Option<u64> = self.call("import_tree_link", (title, url))?;
                match request {
                    Some(_) => self.network_affected(),
                    None => self.spec_edited(true),
                }
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
            Command::SetAffix { slot, index, label } => {
                self.call::<_, ()>("affix_set", (slot, index, label))?;
                Ok(display_changed())
            }
            Command::SetAffixRange { slot, value } => {
                self.call::<_, ()>("affix_range", (slot, value))?;
                Ok(display_changed())
            }
            Command::SetCraftingSort { index } => {
                self.call::<_, ()>("crafting_sort", index + 1)?;
                Ok(display_changed())
            }
            Command::SetClusterSkill { index } => {
                self.call::<_, ()>("cluster_skill", index + 1)?;
                Ok(display_changed())
            }
            Command::SetClusterNodeCount { count } => {
                self.call::<_, ()>("cluster_count", count)?;
                Ok(display_changed())
            }
            Command::RemoveCustomMod { index } => {
                self.call::<_, ()>("remove_custom", index + 1)?;
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
                self.anoint_failed = false;
                let (name, arg) = match kind {
                    PopupKind::Craft => ("craft", 0),
                    // the item text editor only ever takes over a Craft session (R2)
                    PopupKind::EditText => anyhow::bail!(
                        "the item text editor opens from Craft's Create, not on its own"
                    ),
                    PopupKind::Enchant { second } => ("enchant", i64::from(second)),
                    PopupKind::Anoint { slot } => ("anoint", slot as i64 + 1),
                    PopupKind::Corrupt => ("corrupt", 0),
                    PopupKind::AddModifier => ("add_modifier", 0),
                    PopupKind::AddImplicit => ("add_implicit", 0),
                    PopupKind::AddCrucible => ("add_crucible", 0),
                    PopupKind::NodeReplacement { node } => ("node_replacement", i64::from(node)),
                    PopupKind::Options => ("options", 0),
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
                // a failed sort's builder is gone and a same-value pick runs no PoB code, so
                // the control that restarts it is the glue's (Retry)
                let restart = std::mem::take(&mut self.anoint_failed);
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
                if restart {
                    self.call::<_, ()>("anoint_sort_restart", ())?;
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
                // Options' Save changes the settings and may hand back overrides (R12)
                affected.settings = true;
                // a popup PoB closed ends the session; the view says so by being empty
                Ok(affected)
            }
            Command::ClosePopup => {
                self.call::<_, ()>("popup_close", ())?;
                // Escape on Options is its Cancel, which puts the settings back
                Ok(Affected {
                    settings: true,
                    ..popup_changed()
                })
            }
            Command::SetOverrides { values } => {
                let table = self.lua.create_table().map_err(|e| anyhow!("{e}"))?;
                for (key, value) in values {
                    table.set(key, value).map_err(|e| anyhow!("{e}"))?;
                }
                self.call::<_, ()>("set_overrides", table)?;
                Ok(Affected {
                    settings: true,
                    ..Affected::default()
                })
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
        let ends_popup = ends_popup_session(&command);
        let ends_db = ends_item_db(&command);
        let leaves = leaves_build(&command);
        // a late result must never land on another build (R13); the cancelled entries go out
        // with the command's views
        // text PoB's box rejects changes nothing, so it cancels nothing either (R13)
        if let Command::ImportFromUrl { url, mode } = &command {
            let mode = mode_name(*mode);
            self.call::<_, ()>("import_check", (url.clone(), mode))?;
        }
        let cancelled = match cancels_requests(&command) {
            Some(scope) => {
                self.cancel_all_requests(scope)?;
                Some(self.network_affected()?)
            }
            None => None,
        };
        let mut affected = self.apply_command(command, send)?;
        if let Some(cancelled) = cancelled {
            affected.merge(cancelled);
        }
        if leaves {
            affected.merge(self.left_build()?);
            return Ok(affected);
        }
        // any other command may change what the popup is about, so it ends the session once
        // it went through, and the window is told (R4); a failed one changed nothing
        if ends_popup && self.call::<_, bool>("popup_end", false)? {
            affected.popup = true;
            // an Options window that ends is cancelled, which puts the settings back
            affected.settings = true;
        }
        // a save PoB made by itself cancelled an open Options window first
        if self.call::<_, bool>("popup_take_lost", ())? {
            affected.popup = true;
            affected.settings = true;
        }
        // the panel is told once that its database closed (R21)
        if ends_db && self.item_db.take().is_some() {
            self.item_db_failed = false;
            self.item_db_ended = true;
            self.call::<_, ()>("item_db_close", ())?;
            affected.item_db = true;
        }
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
        let mut view: BuildListView = self.view("list_builds", self.folder.clone())?;
        // a missing Builds folder was already refused by the listing
        let root = self.build_root()?;
        view.tree = buildpaths::folder_tree(&root);
        view.root = PathBuf::from(root);
        Ok(view)
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
        // only while a subscript has something to deliver: a pending request alone never
        // spins the loop (R9)
        if self.net.has_ready() {
            return Some(BackgroundTask::Network);
        }
        if self.gem_search.is_some()
            && !self.gem_failed
            && self.call::<_, bool>("gem_dps_pending", ()).unwrap_or(false)
        {
            return Some(BackgroundTask::GemDps);
        }
        if !self.anoint_failed
            && self
                .call::<_, bool>("anoint_sort_pending", ())
                .unwrap_or(false)
        {
            return Some(BackgroundTask::AnointSort);
        }
        if let Some(query) = &self.item_db
            && !self.item_db_failed
            && self
                .call::<_, bool>("item_db_pending", db_name(query.db))
                .unwrap_or(false)
        {
            return Some(BackgroundTask::ItemDb);
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
            #[serde(default)]
            rejected: Option<String>,
        }
        match task {
            BackgroundTask::Network => {
                // a callback's failure finishes its request; the step itself goes on (R25)
                self.net.service(&self.lua, Dispatch::Glue)?;
                self.network_affected()
            }
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
            BackgroundTask::AnointSort => {
                let step: Step = match self.view("anoint_sort_step", ()) {
                    Ok(step) => step,
                    Err(e) => {
                        self.anoint_failed = true;
                        return Err(e);
                    }
                };
                if let Some(percent) = step.percent {
                    send(Event::Progress {
                        task: BackgroundTask::AnointSort,
                        percent,
                    });
                }
                // the window shows the rows once the builder ends
                Ok(Affected {
                    popup: step.finished,
                    ..Affected::default()
                })
            }
            BackgroundTask::ItemDb => {
                let Some(query) = &self.item_db else {
                    return Ok(Affected::default());
                };
                let step: Step = match self.view("item_db_step", db_name(query.db)) {
                    Ok(step) => step,
                    Err(e) => {
                        self.item_db_failed = true;
                        return Err(e);
                    }
                };
                if let Some(percent) = step.percent {
                    send(Event::Progress {
                        task: BackgroundTask::ItemDb,
                        percent,
                    });
                }
                // a query that waited out the load turned out bad: the panel closes, so the
                // UI's query and PoB's controls agree, and the reason is the job's failure
                if let Some(message) = step.rejected {
                    self.item_db = None;
                    self.item_db_ended = true;
                    let _ = self.call::<_, ()>("item_db_close", ());
                    send(Event::BackgroundFailed {
                        task: BackgroundTask::ItemDb,
                        message,
                    });
                    return Ok(Affected {
                        item_db: true,
                        ..Affected::default()
                    });
                }
                // the panel shows the rows once the list builder ends
                Ok(Affected {
                    item_db: step.finished,
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

    fn item_db_view(&mut self) -> Result<DbSnapshot> {
        if std::mem::take(&mut self.item_db_ended) {
            return Ok(DbSnapshot::Ended);
        }
        let Some(query) = &self.item_db else {
            return Ok(DbSnapshot::Quiet);
        };
        let db = query.db;
        let mut view: ItemDbView = self.view("item_db_view", db_name(db))?;
        view.sorting &= !self.item_db_failed;
        Ok(DbSnapshot::View(view))
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
            ControlTooltipQuery::Affix { slot, index } => {
                self.view("affix_tooltip", (*slot, index.map(|i| i as i64)))?
            }
            ControlTooltipQuery::DbItem { db, name } => {
                self.view("item_db_tooltip", (db_name(*db), name.clone()))?
            }
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

    fn short_tree_link_view(&mut self) -> Result<Option<String>> {
        self.call("take_short_link", ())
    }

    fn requests_view(&mut self) -> Result<RequestsView> {
        self.view("requests_view", ())
    }

    fn settings_view(&mut self) -> Result<SettingsView> {
        self.view("settings_view", ())
    }

    fn share_view(&mut self) -> Result<ShareView> {
        self.view("share_view", ())
    }

    fn share_code(&mut self) -> Result<String> {
        self.call("share_code", ())
    }

    fn account_view(&mut self) -> Result<AccountView> {
        self.view("account_view", ())
    }

    fn compare_view(&mut self) -> Result<Option<CompareView>> {
        self.view("compare_view", ())
    }

    fn set_waker(&mut self, wake: Waker) {
        self.net.set_waker(wake);
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
            Command::OpenPopup(PopupKind::Options),
            Command::SetOverrides {
                values: Default::default(),
            },
            Command::OpenPopup(PopupKind::NodeReplacement { node: 1 }),
            Command::SetPopupControl {
                name: "rarity".into(),
                value: PopupInput::Check(true),
            },
            Command::PressPopupButton {
                name: "save".into(),
            },
            Command::ClosePopup,
            Command::SetTreeSearch {
                text: "life".into(),
            },
            // the DB search's debounce can land after an OpenPopup (R4)
            Command::SetItemDb(ItemDbQuery {
                db: ItemDb::Rares,
                slot: 0,
                kind: 0,
                league: 0,
                requirement: 0,
                obtainable: 0,
                sort: 0,
                search: String::new(),
                search_mode: 0,
            }),
            Command::CloseItemDb,
            Command::CancelRequest { id: 1 },
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
            Command::RemoveAllTattoos,
            Command::PasteItem {
                text: String::new(),
            },
            Command::ViewDbItem {
                db: ItemDb::Uniques,
                name: "x".into(),
            },
            Command::AddDbItem {
                db: ItemDb::Uniques,
                name: "x".into(),
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

    #[test]
    fn build_change_cancels_pending_requests() {
        use crate::engine::fake::Fake;
        // R13: replacing the build cancels every request, before the command acts; going to
        // the list cancels only the build's own
        let replacing = [
            Command::OpenBuild {
                path: "a.xml".into(),
            },
            Command::NewBuild,
            Command::ImportShareCode { code: "x".into() },
        ];
        for command in replacing {
            let (mut fake, log) = Fake::new();
            fake.pending = vec![(1, "build"), (2, "list")];
            let shown = format!("apply {command:?}");
            let _ = fake.apply(command, &|_| {});
            assert_eq!(
                log.lock().unwrap().requests,
                ["cancel 1", "cancel 2", &shown]
            );
            assert!(fake.pending.is_empty());
        }
        let (mut fake, log) = Fake::new();
        fake.pending = vec![(1, "build"), (2, "list")];
        let _ = fake.apply(
            Command::ListBuilds {
                folder: String::new(),
            },
            &|_| {},
        );
        assert_eq!(
            log.lock().unwrap().requests,
            ["cancel 1", "apply ListBuilds { folder: \"\" }"]
        );
        assert_eq!(fake.pending, [(2, "list")]);
        // anything else cancels nothing
        let (mut fake, log) = Fake::new();
        fake.pending = vec![(1, "build")];
        let _ = fake.apply(Command::Save, &|_| {});
        assert_eq!(log.lock().unwrap().requests, ["apply Save"]);
    }

    #[test]
    fn import_from_url_cancels_pending_first() {
        use crate::engine::ImportMode;
        use crate::engine::fake::Fake;
        // R13: either mode replaces the build, so every request is cancelled before it acts
        for mode in [ImportMode::ThisBuild, ImportMode::NewBuild] {
            let (mut fake, log) = Fake::new();
            fake.pending = vec![(1, "build"), (2, "list")];
            let command = Command::ImportFromUrl {
                url: "https://pobb.in/x".into(),
                mode,
            };
            let shown = format!("apply {command:?}");
            let _ = fake.apply(command, &|_| {});
            assert_eq!(
                log.lock().unwrap().requests,
                ["cancel 1", "cancel 2", &shown]
            );
            assert!(fake.pending.is_empty());
        }
        // it leaves the build only once something replaced it (a code at once, a link when its
        // request ends), not when the command is applied
        assert!(!leaves_build(&Command::ImportFromUrl {
            url: String::new(),
            mode: ImportMode::NewBuild
        }));
        // a share only reads the build
        assert_eq!(
            cancels_requests(&Command::ShareBuild {
                site: "POBBin".into()
            }),
            None
        );
        // a comparison leaves the build as it is: nothing is cancelled (R5)
        let (mut fake, log) = Fake::new();
        fake.pending = vec![(1, "build"), (2, "list")];
        let command = Command::ImportFromUrl {
            url: "https://pobb.in/x".into(),
            mode: ImportMode::Comparison,
        };
        let shown = format!("apply {command:?}");
        let _ = fake.apply(command, &|_| {});
        assert_eq!(log.lock().unwrap().requests, [shown]);
        assert_eq!(fake.pending.len(), 2);
    }

    #[test]
    fn compare_commands_keep_the_build_and_its_session() {
        use crate::engine::CompareSource;
        let keeps = [
            Command::LoadCompare {
                source: CompareSource::File("a.xml".into()),
            },
            Command::LoadCompare {
                source: CompareSource::Code("x".into()),
            },
            Command::SelectCompare { index: 1 },
            Command::RemoveCompare { index: 0 },
            Command::SetCompareOverlay { on: true },
            Command::SetCompareControl {
                name: "cmpMainSkill".into(),
                value: PopupInput::Choice {
                    index: 1,
                    label: "Double Strike".into(),
                },
            },
            Command::ImportFromUrl {
                url: "https://pobb.in/x".into(),
                mode: ImportMode::Comparison,
            },
        ];
        for command in keeps {
            assert_eq!(cancels_requests(&command), None, "{command:?}");
            assert!(!leaves_build(&command), "{command:?}");
            assert!(!ends_popup_session(&command), "{command:?}");
            assert!(!ends_item_db(&command), "{command:?}");
            assert!(!ends_gem_search(&command), "{command:?}");
        }
        // a copy into this build, or a pick of its own item set, edits it
        for command in [
            Command::CopyCompareTree { and_use: true },
            Command::SetCompareControl {
                name: PRIMARY_ITEM_SET.into(),
                value: PopupInput::Choice {
                    index: 0,
                    label: "Default".into(),
                },
            },
        ] {
            assert!(ends_popup_session(&command), "{command:?}");
            assert!(!leaves_build(&command), "{command:?}");
            assert_eq!(cancels_requests(&command), None, "{command:?}");
        }
    }

    #[test]
    fn account_import_commands_neither_cancel_nor_leave_the_build() {
        use crate::engine::{CharacterPart, Realm};
        let commands = [
            Command::ListPublicCharacters {
                account: "A#1234".into(),
                realm: Realm::Pc,
            },
            Command::ImportPublicCharacter {
                league: None,
                name: "A".into(),
                part: CharacterPart::Tree {
                    delete_jewels: true,
                },
            },
            Command::ClosePublicCharacters,
            Command::SignIn,
            Command::SignOut,
            Command::ListCharacters { realm: Realm::Xbox },
            Command::ImportCharacter {
                realm: Realm::Pc,
                league: Some("Standard".into()),
                name: "A".into(),
                part: CharacterPart::ItemsAndSkills {
                    clear_items: true,
                    clear_skills: true,
                    ignore_weapon_swap: false,
                },
            },
        ];
        for command in commands {
            assert_eq!(cancels_requests(&command), None, "{command:?}");
            assert!(!leaves_build(&command), "{command:?}");
            assert!(!ends_popup_session(&command), "{command:?}");
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
