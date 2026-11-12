//! Task 12's parity gaps: PoB's Spectre Library (R18) and the top bar's level, bloodline and
//! loadouts (R21), against upstream PoB pressing the same controls. Upstream's side is
//! `fixtures/longtail_lib.lua`'s top-bar part: PoB's own buttons, dropdown picks and frames.

use super::{Session, fixture_xml, lock, pob_dir};
use auto_pob::engine::upstream::{golden_of, load_xml, upstream_lua};
use auto_pob::engine::views::ItemTooltipLine;
use auto_pob::engine::{Backend, BuildView, Command, PopupInput, PopupKind, PopupView, StatsView};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const LIB: &str = include_str!("../fixtures/longtail_lib.lua");

/// What `topState` reads: the top bar and what its acts change, on either side
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct Top {
    level: u32,
    auto: bool,
    secondary_id: u32,
    #[serde(default)]
    secondary: Vec<(u32, String)>,
    secondary_enabled: bool,
    #[serde(default)]
    alloc: Vec<u32>,
    #[serde(default)]
    loadouts: Vec<String>,
    loadout_sel: usize,
    spec: u32,
    item_set: u32,
    skill_set: u32,
    config_set: u32,
    specs: usize,
    item_sets: usize,
    skill_sets: usize,
    config_sets: usize,
    flags: BTreeMap<String, bool>,
    undo: BTreeMap<String, usize>,
    unsaved: bool,
    progress: String,
    #[serde(default)]
    spectres: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
}

impl Top {
    /// PoB fills some warning lines from `pairs` (Build.lua:1776-1795): compared sorted
    fn canonical(mut self) -> Top {
        self.warnings.sort();
        self
    }
}

struct Upstream {
    lua: mlua::Lua,
}

impl Upstream {
    fn new() -> Upstream {
        let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
        // the wrapper runs in dev mode, which writes a build to its file when it is left
        lua.load("launch.devMode = false").exec().unwrap();
        lua.load(LIB)
            .set_name("=longtail_lib")
            .exec()
            .expect("longtail lib");
        Upstream { lua }
    }

    /// Loads a build; the wrapper's load marks it unsaved where opening a file doesn't, so
    /// the flags start clear on both sides (`same` after an open needs our side cleared too)
    fn load(&self, xml: &str, name: &str) {
        load_xml(&self.lua, xml, name).unwrap();
        self.call::<()>("clearFlags", ());
        self.call::<()>("upFrame", ());
    }

    fn call<T: mlua::FromLuaMulti>(&self, function: &str, args: impl mlua::IntoLuaMulti) -> T {
        let f: mlua::Function = self.lua.globals().get(function).unwrap();
        f.call(args)
            .unwrap_or_else(|e| panic!("upstream {function}: {e}"))
    }

    fn get<T: serde::de::DeserializeOwned>(
        &self,
        function: &str,
        args: impl mlua::IntoLuaMulti,
    ) -> T {
        let value: mlua::Value = self.call(function, args);
        self.lua.from_value(value).unwrap()
    }

    fn top(&self) -> Top {
        self.get::<Top>("topState", ()).canonical()
    }

    fn popups(&self) -> usize {
        self.lua.load("return #main.popups").eval().unwrap()
    }

    fn golden(&self) -> auto_pob::engine::upstream::Golden {
        golden_of(&self.lua, Game::Poe1.pob_version()).expect("upstream stats")
    }
}

/// Our side: an engine session with the same reader loaded into its Lua state
struct Ours {
    s: Session,
}

impl Ours {
    fn new() -> Ours {
        let s = Session::start("parity-gaps");
        s.backend
            .lua
            .load(LIB)
            .set_name("=longtail_lib")
            .exec()
            .expect("longtail lib in our state");
        Ours { s }
    }

    /// Writes `xml` into the Builds folder as `name` and opens it
    fn open(&mut self, xml: &str, name: &str) {
        let path: PathBuf = self.s.builds_dir().join(format!("{name}.xml"));
        fs::write(&path, xml).unwrap();
        self.run(Command::OpenBuild { path }).expect("opening");
    }

    /// Applies a command as the loop does; every command leaves no popup behind
    fn run(&mut self, command: Command) -> anyhow::Result<()> {
        let what = format!("{command:?}");
        let out = self.s.run(command).map(|_| ());
        let popups: usize = self.eval("return #pob.main.main.popups");
        assert_eq!(popups, 0, "{what} left a popup open");
        out
    }

    fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.s
            .backend
            .lua
            .load(code)
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    fn top(&self) -> Top {
        let value: mlua::Value = self.eval("return topState(pob.build)");
        self.s
            .backend
            .lua
            .from_value::<Top>(value)
            .unwrap()
            .canonical()
    }

    fn clear_flags(&self) {
        self.eval::<()>("clearFlags(pob.build)");
    }

    fn build(&mut self) -> BuildView {
        self.s.backend.build_view().expect("build view")
    }

    fn stats(&mut self) -> StatsView {
        self.s.stats()
    }

    fn popup(&mut self) -> Option<PopupView> {
        self.s.backend.popup_view().expect("popup view")
    }
}

/// Both sides' state after the same act, and our top bar's view of it
fn same(ours: &mut Ours, up: &Upstream, what: &str) -> Top {
    same_state(ours, up, what, true)
}

/// `same`, with `unsaved` left out when our side ran no frame to refresh it: the modFlags it
/// derives from are still compared
fn same_state(ours: &mut Ours, up: &Upstream, what: &str, unsaved: bool) -> Top {
    let want = up.top();
    let mut got = ours.top();
    if !unsaved {
        got.unsaved = want.unsaved;
    }
    assert_eq!(got, want, "{what}: PoB state");
    assert_eq!(up.popups(), 0, "{what}: upstream left a popup open");
    let view = ours.build();
    assert_eq!(view.level, want.level, "{what}: level");
    assert_eq!(view.level_auto, want.auto, "{what}: level mode");
    assert_eq!(view.secondary, want.secondary, "{what}: bloodlines");
    assert_eq!(
        view.secondary_selected, want.secondary_id,
        "{what}: bloodline"
    );
    assert_eq!(view.loadouts, want.loadouts, "{what}: loadouts");
    let sel = want.loadout_sel;
    let shown =
        (!auto_pob::engine::views::is_loadout_marker(&want.loadouts[sel - 1])).then_some(sel - 1);
    assert_eq!(view.loadout_selected, shown, "{what}: loadout selected");
    assert_eq!(
        view.progress, want.progress,
        "{what}: point display tooltip"
    );
    let tooltip: Vec<String> = view
        .level_tooltip
        .as_ref()
        .expect("no level tooltip")
        .lines
        .iter()
        .map(|line| match line {
            ItemTooltipLine::Line { text, .. } => text.clone(),
            ItemTooltipLine::Separator { .. } => "--".into(),
        })
        .collect();
    assert_eq!(
        tooltip,
        up.get::<Vec<String>>("levelTooltip", ()),
        "{what}: level box tooltip"
    );
    want
}

/// Our rows against upstream's values, then against the lines PoB's own sidebar drew, both
/// ways
fn same_rows(
    what: &str,
    rows: &[auto_pob::engine::StatRow],
    values: &BTreeMap<String, f64>,
    sidebar: &[auto_pob::engine::upstream::SidebarLine],
) {
    assert!(!rows.is_empty(), "{what}: no rows");
    for row in rows {
        let value = values
            .get(&row.key)
            .unwrap_or_else(|| panic!("{what}: {} isn't upstream's", row.key));
        assert_eq!(row.value, *value, "{what}: {}", row.key);
    }
    let got: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.label.as_str(), r.text.as_str()))
        .collect();
    let want: Vec<(&str, &str)> = sidebar
        .iter()
        .map(|l| (l.label.as_str(), l.text.as_str()))
        .collect();
    assert_eq!(got, want, "{what}: sidebar lines");
}

/// Our sidebar's stats against upstream's calculation: the player's, and the minion's when
/// either side has one
fn same_stats(ours: &mut Ours, up: &Upstream, what: &str) -> StatsView {
    let view = ours.stats();
    let want = up.golden();
    same_rows(
        &format!("{what}: player"),
        &view.stats,
        &want.stats,
        &want.sidebar,
    );
    match (&want.minion_stats, &want.minion_sidebar) {
        (Some(values), Some(sidebar)) => {
            same_rows(&format!("{what}: minion"), &view.minion, values, sidebar)
        }
        _ => assert!(view.minion.is_empty(), "{what}: minion rows upstream lacks"),
    }
    let mut warnings = view.warnings.clone();
    warnings.sort();
    assert_eq!(warnings, up.top().warnings, "{what}: warning lines");
    view
}

fn rows(view: &PopupView, name: &str) -> Vec<(String, String)> {
    let control = view
        .controls
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no control {name}"));
    control
        .rows
        .iter()
        .map(|r| (r.key.clone(), r.label.clone()))
        .collect()
}

fn keys(rows: &[(String, String)]) -> Vec<String> {
    rows.iter().map(|r| r.0.clone()).collect()
}

fn press(ours: &mut Ours, name: &str) {
    ours.run(Command::PressPopupButton { name: name.into() })
        .unwrap_or_else(|e| panic!("pressing {name}: {e}"));
}

fn set(ours: &mut Ours, name: &str, value: PopupInput) {
    ours.run(Command::SetPopupControl {
        name: name.into(),
        value,
    })
    .unwrap_or_else(|e| panic!("setting {name}: {e}"));
}

/// Opens the library on our side and upstream's, and searches both for "Archer" in names and
/// skills (mode Both). Returns our filtered rows, in PoB's order.
fn open_and_search(ours: &mut Ours, up: &Upstream) -> Vec<(String, String)> {
    ours.run(Command::OpenPopup(PopupKind::SpectreLibrary))
        .expect("opening the library");
    up.call::<()>("openLibrary", ());
    let view = ours.popup().expect("no library window");
    assert_eq!(view.title, "Spectre Library");
    assert_eq!(view.kind, PopupKind::SpectreLibrary);
    // the whole source list is PoB's sort (by name, then id): the same order on both sides
    let all = rows(&view, "source");
    assert!(all.len() > 50, "the source list is short: {}", all.len());
    assert_eq!(keys(&all), up.get::<Vec<String>>("librarySource", ()));
    set(
        ours,
        "source.searchModeDropDown",
        PopupInput::Choice {
            index: 2,
            label: "Both".into(),
        },
    );
    set(ours, "source.searchText", PopupInput::Text("Archer".into()));
    up.call::<()>("librarySearch", ("Archer", 3));
    let view = ours.popup().unwrap();
    let found = rows(&view, "source");
    // the filtered list walks the source with `pairs`: compared in PoB's sort, by name and id
    let canonical = |mut list: Vec<(String, String)>| {
        list.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
        list
    };
    let ours_keys = keys(&canonical(found.clone()));
    // upstream's ids sorted the same way, by the names the full list matched
    let label_of: BTreeMap<String, String> = all.iter().cloned().collect();
    let mut up_keys: Vec<String> = up.get("librarySource", ());
    up_keys.sort_by(|a, b| (&label_of[a], a).cmp(&(&label_of[b], b)));
    assert_eq!(ours_keys, up_keys, "the search's rows");
    assert!(found.len() >= 4, "\"Archer\" finds too little: {found:?}");
    found
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn spectre_and_top_bar_match_pob() {
    let _guard = lock();
    let minion = fs::read_to_string(fixture_xml("minion")).unwrap();
    // the minion fixture's tree, a version back: PoB lists that tree's bloodlines
    assert!(minion.contains("treeVersion=\"3_29\""));
    let older = minion.replace("treeVersion=\"3_29\"", "treeVersion=\"3_27\"");
    // the fixture's Raise Zombie made Raise Spectre
    let zombie = "skillMinionCalcs=\"RaisedZombie\" skillMinionSkillCalcs=\"1\" ";
    assert!(minion.contains(zombie));
    let spectre = minion
        .replace(zombie, "")
        .replace(" skillMinion=\"RaisedZombie\"", "")
        .replace("SkillGemRaiseZombie", "SkillGemRaiseSpectre")
        .replace("skillId=\"RaiseZombie\"", "skillId=\"RaiseSpectre\"")
        .replace("variantId=\"RaiseZombie\"", "variantId=\"RaiseSpectre\"")
        .replace("nameSpec=\"Raise Zombie\"", "nameSpec=\"Raise Spectre\"");

    let up = Upstream::new();
    let mut ours = Ours::new();

    // ---- Bloodline, on the 3_27 tree ----
    ours.open(&older, "older");
    up.load(&older, "older");
    let start = same(&mut ours, &up, "3_27 opened");
    assert_eq!(start.spec, 1);
    assert!(start.secondary_enabled, "3_27 offers no bloodline");
    assert!(start.secondary.len() > 1);
    assert_eq!(start.secondary[0], (0, "None".to_string()));
    // Raise Zombie: neither "Manage Spectres..." shows
    let (side, calcs): (bool, bool) = up.call("spectreButtons", ());
    assert!(!side && !calcs);
    assert!(!ours.stats().main_skill.spectre_library);
    let selector = ours.s.backend.calcs_view().unwrap().selector;
    assert!(!selector.spectre_library);
    let (id, _) = start
        .secondary
        .iter()
        .find(|(_, label)| label == "Aul Bloodline")
        .cloned()
        .expect("no Aul Bloodline on 3_27");
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SelectSecondaryAscendancy { id }).unwrap();
    up.call::<()>("pickBloodline", id);
    let picked = same(&mut ours, &up, "bloodline picked");
    assert_eq!(picked.secondary_id, id);
    assert_ne!(picked.alloc, start.alloc, "the bloodline allocated nothing");
    assert!(picked.undo["spec"] > start.undo["spec"], "no undo state");
    let err = ours
        .run(Command::SelectSecondaryAscendancy { id: 9999 })
        .unwrap_err();
    assert_eq!(err.to_string(), "PoB doesn't offer that bloodline here");

    // ---- Level ----
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SetCharacterLevel { level: 75 }).unwrap();
    up.call::<()>("typeLevel", "75");
    let typed = same(&mut ours, &up, "level typed");
    assert_eq!(typed.level, 75);
    assert!(!typed.auto);
    assert!(
        typed.flags["build"],
        "typing the level marked nothing unsaved"
    );
    let stats = same_stats(&mut ours, &up, "level 75");
    assert!(!stats.warnings.is_empty(), "the fixture has no warnings");
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SetLevelMode { auto: true }).unwrap();
    up.call::<()>("pressLevelMode", ());
    let auto = same(&mut ours, &up, "Auto pressed");
    assert!(auto.auto);
    assert_ne!(auto.level, 75, "Auto left the typed level");
    same_stats(&mut ours, &up, "Auto");
    // typed while in Auto: PoB's callback switches to Manual (Build.lua:223)
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SetCharacterLevel { level: 60 }).unwrap();
    up.call::<()>("typeLevel", "60");
    let manual = same(&mut ours, &up, "typed in Auto");
    assert!(!manual.auto);
    assert_eq!(manual.level, 60);
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SetLevelMode { auto: true }).unwrap();
    up.call::<()>("pressLevelMode", ());
    assert!(same(&mut ours, &up, "Auto pressed again").auto);
    // already Auto: nothing is pressed on either side
    ours.clear_flags();
    ours.run(Command::SetLevelMode { auto: true }).unwrap();
    up.call::<()>("clearFlags", ());
    up.call::<()>("upFrame", ());
    same(&mut ours, &up, "Auto again");

    // ---- Spectre Library, on Raise Spectre ----
    ours.open(&spectre, "spectre");
    up.load(&spectre, "spectre");
    same(&mut ours, &up, "spectre build opened");
    let (side, calcs): (bool, bool) = up.call("spectreButtons", ());
    assert!(side && calcs, "upstream shows no Manage Spectres");
    let stats = ours.stats();
    assert!(stats.main_skill.spectre_library);
    assert!(
        ours.s
            .backend
            .calcs_view()
            .unwrap()
            .selector
            .spectre_library
    );
    let before = up.top().spectres;
    let found = open_and_search(&mut ours, &up);
    let [first, second, third, fourth] = [0, 1, 2, 3].map(|i| found[i].0.clone());
    for key in [&first, &second, &third] {
        set(&mut ours, "source", PopupInput::Row(key.clone()));
        press(&mut ours, "source.add");
        up.call::<()>("libraryPress", ("source", key.as_str(), "add"));
    }
    set(&mut ours, "list", PopupInput::Row(first.clone()));
    press(&mut ours, "list.delete");
    up.call::<()>("libraryPress", ("list", first.as_str(), "delete"));
    // PoB's "Spectres in Build" is mutable: the last row dragged to the top
    let view = ours.popup().unwrap();
    assert!(
        view.controls.iter().any(|c| c.name == "list" && c.reorder),
        "the build's list isn't reorderable"
    );
    assert!(
        !view
            .controls
            .iter()
            .any(|c| c.name == "source" && c.reorder),
        "the source list is reorderable"
    );
    set(
        &mut ours,
        "list",
        PopupInput::RowMove {
            key: third.clone(),
            to: 0,
        },
    );
    up.call::<()>("libraryMove", (third.as_str(), 1));
    let dest = keys(&rows(&ours.popup().unwrap(), "list"));
    assert_eq!(dest, up.get::<Vec<String>>("libraryDest", ()));
    assert_eq!(dest, [third.clone(), second.clone()]);
    // nothing is committed before Save
    assert_eq!(ours.top().spectres, before);
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    press(&mut ours, "save");
    up.call::<()>("libraryClose", "save");
    assert!(ours.popup().is_none(), "Save left the window open");
    let saved = same(&mut ours, &up, "library saved");
    assert_eq!(saved.spectres, [third.clone(), second.clone()]);
    assert!(saved.flags["build"], "Save marked nothing unsaved");
    let minions = ours.stats().main_skill.minion.expect("no minion dropdown");
    assert_eq!(minions.labels, up.get::<Vec<String>>("minionLabels", ()));
    assert_eq!(minions.labels.len(), 2);
    let stats = same_stats(&mut ours, &up, "library saved");
    assert!(!stats.minion.is_empty(), "the spectre has no stats");
    // Cancel commits nothing and marks nothing unsaved
    open_and_search(&mut ours, &up);
    set(&mut ours, "source", PopupInput::Row(fourth.clone()));
    press(&mut ours, "source.add");
    up.call::<()>("libraryPress", ("source", fourth.as_str(), "add"));
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    press(&mut ours, "cancel");
    up.call::<()>("libraryClose", "cancel");
    assert!(ours.popup().is_none(), "Cancel left the window open");
    // Cancel runs no frame on our side to refresh `unsaved`; the flags it derives from count
    let cancelled = same_state(&mut ours, &up, "library cancelled", false);
    assert_eq!(cancelled.spectres, saved.spectres);
    assert!(
        cancelled.flags.values().all(|f| !f),
        "Cancel marked something unsaved"
    );

    // ---- Loadouts ----
    // still no frame on our side since the Cancel
    let base = same_state(&mut ours, &up, "before loadouts", false);
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::NewLoadout { name: "L2".into() }).unwrap();
    up.call::<()>("newLoadout", "L2");
    let made = same(&mut ours, &up, "New Loadout");
    assert_eq!(made.specs, base.specs + 1);
    assert_eq!(made.item_sets, base.item_sets + 1);
    assert!(made.loadouts.contains(&"L2".to_string()));
    assert!(made.flags["build"]);
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SelectLoadout { label: "L2".into() })
        .unwrap();
    up.call::<()>("pickLoadout", "L2");
    let chosen = same(&mut ours, &up, "L2 picked");
    assert_eq!(
        (
            chosen.spec,
            chosen.item_set,
            chosen.skill_set,
            chosen.config_set
        ),
        (2, 2, 2, 2)
    );
    assert!(ours.build().loadout_selected.is_some());
    ours.run(Command::SyncLoadouts).unwrap();
    up.call::<()>("pickLoadout", "^7^7Sync");
    let synced = same(&mut ours, &up, "Sync");
    assert_eq!(synced.loadout_sel, 1);
    assert_eq!(ours.build().loadout_selected, None);
    // back to Default
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    ours.run(Command::SelectLoadout {
        label: "Default".into(),
    })
    .unwrap();
    up.call::<()>("pickLoadout", "Default");
    let back = same(&mut ours, &up, "Default picked");
    assert_eq!(back.spec, 1);
    // PoB's headers and actions aren't loadouts; a blank name makes nothing
    let err = ours
        .run(Command::SelectLoadout {
            label: "^7^7Sync".into(),
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "PoB doesn't offer that here");
    let err = ours
        .run(Command::SelectLoadout {
            label: "nope".into(),
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "no such loadout");
    let err = ours
        .run(Command::NewLoadout { name: "  ".into() })
        .unwrap_err();
    assert_eq!(err.to_string(), "the loadout needs a name");
    // a name PoB's Save won't take is its Cancel: the dropdown is left on its header, as PoB
    // leaves it after New Loadout, and nothing else changes
    up.call::<()>("newLoadoutCancelled", ());
    let after = same_state(&mut ours, &up, "after the refusals", false);
    assert_eq!(after.loadout_sel, 1);
    assert_eq!(
        Top {
            loadout_sel: back.loadout_sel,
            unsaved: back.unsaved,
            ..after
        },
        back
    );

    // ---- The library is build-owned (R18, R19): it outlives commands on its build only ----
    ours.run(Command::OpenPopup(PopupKind::SpectreLibrary))
        .expect("opening the library");
    ours.run(Command::SetCharacterLevel { level: 80 }).unwrap();
    assert!(
        ours.popup().is_some(),
        "a command on the same build ended the library"
    );
    ours.open(&spectre, "spectre again");
    assert!(ours.popup().is_none(), "the library outlived its build");
}

// ---- Task 14: the Help window ----

/// Upstream's About window, section by section: `OpenAboutPopup(i)` scrolls its list to
/// section `i` (Main.lua:1434-1436), so each section starts at the row the scroll offset
/// names. Asking for one past the last falls back to the first (Main.lua:1407-1416). Each
/// section: its title row and the cells of the rows under it, up to the next section.
const UPSTREAM_HELP: &str = r#"
local main = launch.main
local function open(i)
    main:OpenAboutPopup(i)
    local p = main.popups[1]
    local list, offset = p.controls.changelog.list, p.controls.changelog.controls.scrollBar.offset
    local labels = { p.controls.version.label, p.controls.forum.label, p.controls.github.label }
    main:ClosePopup()
    -- textSize (Main.lua:1337): a row is 16 high in the offsets
    return list, offset / 16, labels
end
local list, first, labels = open(1)
local starts = { first }
while true do
    local _, start = open(#starts + 1)
    if start == first then break end
    starts[#starts + 1] = start
end
local sections = {}
for k, start in ipairs(starts) do
    local stop = starts[k + 1] or #list
    local rows = {}
    for r = start + 2, stop do
        local row = list[r]
        if row[1] then
            local cells = { row[1] }
            if row[2] then cells[2] = row[2] end
            rows[#rows + 1] = cells
        end
    end
    sections[k] = { title = list[start + 1][1], rows = rows }
end
-- with no section asked for, the window opens on the version history, each version where
-- its `sectionHeights` entry says (Main.lua:1342-1351, 1431)
main:OpenAboutPopup()
local p = main.popups[1]
local changes, heights = p.controls.changelog.list, p.controls.changelog.sectionHeights
main:ClosePopup()
local versions = {}
for k, height in ipairs(heights) do
    local start = height / 16
    local stop = heights[k + 1] and heights[k + 1] / 16 or #changes
    local rows = {}
    for r = start + 2, stop do
        if changes[r][1] then rows[#rows + 1] = { changes[r][1] } end
    end
    versions[k] = { title = changes[start + 1][1], rows = rows }
end
return { sections = sections, header = labels, versions = versions, popups = #main.popups }
"#;

/// A changelog in PoB's format (two versions, a sub-title and blank lines), written next to
/// help.txt for both sides
const CHANGELOG: &str = "VERSION[2.67.2][2026/08/07]

--- Fixed Calculations ---
* Fix one thing (someone)
* Fix another (someone else)

VERSION[2.67.1][2026/08/01]

--- Other changes ---
* Change a thing
";

#[derive(Debug, serde::Deserialize)]
struct UpHelp {
    sections: Vec<UpSection>,
    header: Vec<String>,
    versions: Vec<UpSection>,
    popups: usize,
}

#[derive(Debug, serde::Deserialize)]
struct UpSection {
    title: String,
    rows: Vec<Vec<String>>,
}

/// The help.txt fixture is PoB 2.67.2's own: PoB's updater takes a file whose sha1, with
/// CRLF line ends, is the manifest's (UpdateCheck.lua:191)
fn check_help_fixture(lua: &mlua::Lua, text: &str) {
    let manifest = fs::read_to_string(pob_dir().join("manifest.xml")).unwrap();
    let line = manifest
        .lines()
        .find(|l| l.contains("name=\"help.txt\""))
        .expect("help.txt isn't in the manifest");
    let want = line
        .split("sha1=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    lua.globals().set("HELP_TEXT", text).unwrap();
    let got: String = lua
        .load(r#"return require("sha1")((HELP_TEXT:gsub("\n", "\r\n")))"#)
        .eval()
        .unwrap();
    assert_eq!(got, want, "the help.txt fixture isn't PoB's");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn help_matches_pob() {
    use super::longtail_harness::LongtailSession;
    use auto_pob::engine::{Event, Snapshot, send_snapshots};
    let _guard = lock();
    let mut s = LongtailSession::start("help", &|_| {
        "<PathOfBuilding>\n\t<Misc showThousandsSeparators=\"false\"/>\n</PathOfBuilding>\n".into()
    });

    // The farm's entries link to the pinned folder, which may or may not hold the help files:
    // their links are removed (unlinking never follows a link), so nothing below can reach
    // the pinned folder, and the files go into the farm itself
    for name in ["help.txt", "changelog.txt"] {
        let link = s.pob.join(name);
        if fs::symlink_metadata(&link).is_ok() {
            fs::remove_file(&link).unwrap();
        }
    }
    // without them the window lists what's missing, reads nothing and leaves no popup
    let missing = s
        .run(Command::Help)
        .and_then(|_| s.backend.help_view())
        .unwrap();
    assert_eq!(missing.missing, ["help.txt", "changelog.txt"]);
    assert!(missing.sections.is_empty() && missing.header.is_empty());
    assert_eq!(s.open_popups(), 0);

    let text = fs::read_to_string(super::fixtures_dir().join("help.txt")).unwrap();
    fs::write(s.pob.join("help.txt"), &text).unwrap();
    // help.txt alone: the sections, and changelog.txt still listed as missing
    let partial = s.backend.help_view().unwrap();
    assert_eq!(partial.missing, ["changelog.txt"]);
    assert!(!partial.sections.is_empty() && partial.changelog.is_empty());
    fs::write(s.pob.join("changelog.txt"), CHANGELOG).unwrap();
    let up = upstream_lua(&s.pob).expect("starting upstream PoB");
    check_help_fixture(&up, &text);
    // as our engine runs it (bootstrap.lua:181-194): help.txt from PoB's folder, no DEV[] rows
    up.load("launch.devMode = false").exec().unwrap();
    let value: mlua::Value = up.load(UPSTREAM_HELP).eval().expect("upstream help");
    let theirs: UpHelp = up.from_value(value).unwrap();
    assert_eq!(theirs.popups, 0);

    // the query is list-safe: it is answered whatever is open, through the loop's own path
    let affected = s.run(Command::Help).unwrap();
    assert!(affected.help);
    let events = std::cell::RefCell::new(Vec::new());
    send_snapshots(&mut s.backend, 1, affected, &|e| {
        events.borrow_mut().push(e)
    });
    let events = events.into_inner();
    let ours = match events.as_slice() {
        [
            Event::Snapshot {
                snapshot: Snapshot::Help(view),
                ..
            },
        ] => view.clone(),
        other => panic!("expected the help snapshot alone, got {other:?}"),
    };
    assert_eq!(s.open_popups(), 0, "the About window was left open");

    assert_eq!(ours.header, theirs.header);
    assert!(ours.missing.is_empty());
    let versions: Vec<(String, Vec<String>)> = ours
        .changelog
        .iter()
        .map(|v| (format!("^7{}", v.title), v.lines.clone()))
        .collect();
    let up_versions: Vec<(String, Vec<String>)> = theirs
        .versions
        .iter()
        .map(|v| {
            (
                v.title.clone(),
                v.rows.iter().map(|r| r[0].clone()).collect(),
            )
        })
        .collect();
    assert_eq!(versions, up_versions);
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].0, "^7Version 2.67.2 (2026/08/07)");
    assert!(
        versions[0]
            .1
            .contains(&"^7--- Fixed Calculations ---".to_string())
    );
    assert!(ours.header[0].contains("v2.67.2"), "{:?}", ours.header);
    assert_eq!(ours.sections.len(), theirs.sections.len());
    for (k, (o, t)) in ours.sections.iter().zip(&theirs.sections).enumerate() {
        assert_eq!(t.title, format!("^7{} ({})", o.title, k + 1), "section {k}");
        let rows: Vec<String> = t.rows.iter().map(|cells| cells.join("\t")).collect();
        assert_eq!(o.lines, rows, "section {}", o.title);
    }
    let titles: Vec<&str> = ours.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles[0], "Help File Introduction");
    // "Help >>" opens section 7 (Build.lua:312)
    assert_eq!(titles[6], "Loadouts");
    assert!(titles.contains(&"Skills Tab") && titles.contains(&"Items Tab"));
    // non-default rows: two columns, the Contents PoB numbers, and no developer rows
    let hotkeys = &ours.sections[titles.iter().position(|t| *t == "Hotkeys").unwrap()];
    assert!(
        hotkeys
            .lines
            .contains(&"^7Ctrl + 1\t^7Jump to tree tab".to_string()),
        "{:?}",
        &hotkeys.lines[..6]
    );
    assert!(ours.sections[1].lines.contains(&"^74. Hotkeys".to_string()));
    assert!(
        !ours
            .sections
            .iter()
            .flat_map(|s| &s.lines)
            .any(|l| l.contains("Developer Mode")),
        "a DEV[] row was shown"
    );
}

// ---- Task 14: the Tree tab's P and Ctrl+D ----

/// Upstream presses a key as PoB gets it: the key event, then a frame on the Tree tab
/// (Main.lua:458-460, Build.lua:1150-1186, TreeTab.lua:359-384, PassiveTreeView.lua:198-218).
/// Ctrl is held through `IsKeyDown`, as `longtail_lib.lua` does for the build list.
fn up_key(up: &Upstream, key: &str, ctrl: bool) {
    up.lua.globals().set("KEY", key).unwrap();
    up.lua.globals().set("CTRL", ctrl).unwrap();
    up.lua
        .load(
            r#"
            assert(build.viewMode == "TREE", "not on the Tree tab")
            local real = IsKeyDown
            IsKeyDown = function(k) return CTRL and k == "CTRL" end
            runCallback("OnKeyDown", KEY, false)
            runCallback("OnFrame")
            runCallback("OnKeyUp", KEY)
            runCallback("OnFrame")
            IsKeyDown = real
            "#,
        )
        .exec()
        .expect("upstream key");
}

/// The tree view's flags and the controls TreeTab:Draw syncs to them, and whether the build
/// was marked changed
const TREE_FLAGS: &str = r#"
local tab = B.treeTab
return {
    heat = tab.viewer.showHeatMap and true or false,
    check = tab.controls.treeHeatMap.state and true or false,
    stat_shown = tab.controls.treeHeatMapStatSelect.shown and true or false,
    diff = tab.viewer.showStatDifferences and true or false,
    changed = (B.modFlag or B.buildFlag) and true or false,
}
"#;

#[derive(Debug, PartialEq, serde::Deserialize)]
struct TreeFlags {
    heat: bool,
    check: bool,
    stat_shown: bool,
    diff: bool,
    changed: bool,
}

fn up_flags(up: &Upstream) -> TreeFlags {
    up.lua.load("B = build").exec().unwrap();
    let value: mlua::Value = up.lua.load(TREE_FLAGS).eval().unwrap();
    up.lua.from_value(value).unwrap()
}

fn our_flags(ours: &Ours) -> TreeFlags {
    ours.eval::<()>("B = pob.build");
    let value: mlua::Value = ours.eval(TREE_FLAGS);
    ours.s.backend.lua.from_value(value).unwrap()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_keys_match_pob() {
    let _guard = lock();
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "attack");
    up.load(&xml, "attack");
    ours.clear_flags();
    ours.eval::<()>("pob.build.buildFlag = false");
    up.lua.load("build.buildFlag = false").exec().unwrap();
    let start = our_flags(&ours);
    assert_eq!(start, up_flags(&up));
    // PoB's defaults: no heat map, stat differences on
    assert!(!start.heat && start.diff);

    // P shows the heat map with no stat chosen: damage against defence; nothing changes
    ours.run(Command::ToggleHeatmap).unwrap();
    up_key(&up, "p", false);
    let shown = our_flags(&ours);
    assert_eq!(shown, up_flags(&up), "after P");
    assert!(shown.heat && shown.check && shown.stat_shown && !shown.changed);
    let state = ours.s.backend.tree_state_view().unwrap();
    assert!(state.heatmap_shown);
    assert_eq!(state.heatmap, None);
    // the power calculation starts, as PoB's Draw starts it (PassiveTreeView.lua:773-775)
    assert_eq!(
        ours.s.backend.next_background(),
        Some(auto_pob::engine::BackgroundTask::NodePower)
    );
    // and P again hides it
    ours.run(Command::ToggleHeatmap).unwrap();
    up_key(&up, "p", false);
    let hidden = our_flags(&ours);
    assert_eq!(hidden, up_flags(&up), "after P twice");
    assert!(!hidden.heat);
    assert!(!ours.s.backend.tree_state_view().unwrap().heatmap_shown);

    // Ctrl+D: the node tooltips stop listing what allocating changes
    let node: u32 = ours.eval(
        "for id, node in pairs(pob.build.spec.nodes) do \
         if node.type == 'Notable' and not node.alloc and node.path then return id end end",
    );
    let with = ours.s.backend.node_tooltip_view(node).unwrap().unwrap();
    ours.run(Command::SetTreeStatDifferences { on: false })
        .unwrap();
    up_key(&up, "d", true);
    let off = our_flags(&ours);
    assert_eq!(off, up_flags(&up), "after Ctrl+D");
    assert!(!off.diff && !off.changed);
    assert!(
        !ours
            .s
            .backend
            .tree_state_view()
            .unwrap()
            .show_stat_differences
    );
    let without = ours.s.backend.node_tooltip_view(node).unwrap().unwrap();
    assert!(
        without.lines.len() < with.lines.len(),
        "the tooltip still lists the differences"
    );
    ours.run(Command::SetTreeStatDifferences { on: true })
        .unwrap();
    up_key(&up, "d", true);
    assert_eq!(our_flags(&ours), up_flags(&up), "after Ctrl+D twice");
    assert!(
        ours.s
            .backend
            .tree_state_view()
            .unwrap()
            .show_stat_differences
    );
}

// ---- Task 14 fix: the Shift-traced path ----

/// Picks, by lowest ids, an unallocated node B next to an allocated one and an unallocated
/// node C next to B whose own path doesn't go through B: the trace B, C is then not C's path.
const TRACE_PICK: &str = r#"
local spec = B.spec
local ids = {}
for id in pairs(spec.nodes) do ids[#ids + 1] = id end
table.sort(ids)
for _, bId in ipairs(ids) do
    local b = spec.nodes[bId]
    if not b.alloc and b.path and #b.path == 1 and b.type == "Normal" then
        local linked = {}
        for _, n in ipairs(b.linked) do linked[#linked + 1] = n end
        table.sort(linked, function(x, y) return x.id < y.id end)
        for _, c in ipairs(linked) do
            if not c.alloc and c.path and c.type ~= "Mastery" and c.path[2] ~= b
                and not c.ascendancyName then
                return { b.id, c.id }
            end
        end
    end
end
"#;

/// Upstream's click on the trace's end, as PassiveTreeView.lua:500-506 runs it
const UP_TRACE_ALLOC: &str = r#"
local spec = build.spec
local path = {}
for i, id in ipairs(TRACE) do path[i] = spec.nodes[id] end
local node = path[#path]
spec:AllocNode(node, path)
spec:AddUndoState()
build.buildFlag = true
runCallback("OnFrame")
local alloc = {}
for id in pairs(spec.allocNodes) do alloc[#alloc + 1] = id end
table.sort(alloc)
return alloc
"#;

#[test]
#[ignore = "needs the pinned PoB install"]
fn trace_path_matches_pob() {
    let _guard = lock();
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "attack");
    up.load(&xml, "attack");
    ours.eval::<()>("B = pob.build");
    up.lua.load("B = build").exec().unwrap();
    let pick: Vec<u32> = ours.eval(TRACE_PICK);
    let up_pick: Vec<u32> = up.lua.load(TRACE_PICK).eval().unwrap();
    assert_eq!(pick, up_pick);
    let (b, c) = (pick[0], pick[1]);
    let own: Vec<u32> = ours.eval(&format!(
        "local p = {{}} for i, n in ipairs(pob.build.spec.nodes[{c}].path) do p[i] = n.id end return p"
    ));
    assert!(!own.contains(&b), "C's own path goes through B: {own:?}");

    // the tooltip over the trace's end counts the trace (PassiveTreeView.lua:1715-1716)
    ours.run(Command::PreviewTrace {
        id: c,
        path: vec![b, c],
    })
    .unwrap();
    let tip = ours.s.backend.node_tooltip_view(c).unwrap().unwrap();
    assert_eq!(tip.trace, [b, c]);
    let text = format!("{:?}", tip.lines);
    assert!(text.contains("2 nodes in trace path"), "{text}");
    ours.run(Command::PreviewNode { id: c }).unwrap();
    let plain = ours.s.backend.node_tooltip_view(c).unwrap().unwrap();
    assert!(plain.trace.is_empty());
    assert!(!format!("{:?}", plain.lines).contains("trace path"));

    // the click allocates the trace, not C's own path
    ours.clear_flags();
    ours.run(Command::AllocTracePath {
        id: c,
        path: vec![b, c],
    })
    .unwrap();
    up.lua.globals().set("TRACE", vec![b, c]).unwrap();
    let up_alloc: Vec<u32> = up.lua.load(UP_TRACE_ALLOC).eval().unwrap();
    let mut alloc = ours.s.backend.tree_state_view().unwrap().allocated;
    alloc.sort();
    assert_eq!(alloc, up_alloc);
    assert!(alloc.contains(&b) && alloc.contains(&c));
    assert!(own.iter().filter(|n| **n != c).all(|n| !alloc.contains(n)));
    assert!(ours.build().unsaved, "the allocation isn't marked unsaved");
    let stats = same_stats(&mut ours, &up, "after the trace");
    assert!(!stats.stats.is_empty());
    // a trace may pass through an allocated node (PassiveSpec.lua:898): it is allocated
    // already and the path goes on from it
    let through: Vec<u32> = ours.eval(
        "local spec = pob.build.spec local ids = {} for id in pairs(spec.nodes) do ids[#ids + 1] = id end \
         table.sort(ids) for _, id in ipairs(ids) do local a = spec.nodes[id] \
         if a.alloc and a.type == 'Normal' then for _, d in ipairs(a.linked) do \
         if not d.alloc and d.path and d.type == 'Normal' and not d.ascendancyName then \
         return { a.id, d.id } end end end end",
    );
    let (a, d) = (through[0], through[1]);
    ours.run(Command::AllocTracePath {
        id: d,
        path: vec![a, d],
    })
    .unwrap();
    up.lua.globals().set("TRACE", vec![a, d]).unwrap();
    let up_alloc: Vec<u32> = up.lua.load(UP_TRACE_ALLOC).eval().unwrap();
    let mut alloc = ours.s.backend.tree_state_view().unwrap().allocated;
    alloc.sort();
    assert_eq!(alloc, up_alloc, "through an allocated node");
    assert!(alloc.contains(&d));
    // a target that isn't connected to the tree is refused, and nothing changes
    let far: u32 = ours.eval(
        "local spec = pob.build.spec local ids = {} for id in pairs(spec.nodes) do ids[#ids + 1] = id end \
         table.sort(ids) for _, id in ipairs(ids) do local n = spec.nodes[id] \
         if not n.alloc and not n.path and n.type == 'Normal' then return id end end",
    );
    let err = ours
        .run(Command::AllocTracePath {
            id: far,
            path: vec![far],
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "that passive isn't connected to your tree");
    let mut after = ours.s.backend.tree_state_view().unwrap().allocated;
    after.sort();
    assert_eq!(after, alloc);
}
