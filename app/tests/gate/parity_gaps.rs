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

// ---- No silent writes, SP2 to SP7 (Task 16, R29, R34) ----

/// The pinned PoB folder by file, size and modification time, links not followed, without the
/// timeless LUT caches PoB itself may write there (R17)
fn pinned_files() -> BTreeMap<PathBuf, (u64, std::time::SystemTime)> {
    fn walk(dir: &std::path::Path, out: &mut BTreeMap<PathBuf, (u64, std::time::SystemTime)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(&path, out);
            } else if !(path.extension().is_some_and(|e| e == "bin")
                && path
                    .parent()
                    .is_some_and(|p| p.ends_with("TimelessJewelData")))
            {
                out.insert(path, (meta.len(), meta.modified().unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&pob_dir(), &mut out);
    out
}

/// The timeless LUT caches in the pinned PoB folder, by name: `pinned_files` leaves them out,
/// and none may appear there (the searches run in a folder of their own)
fn pinned_bins() -> Vec<String> {
    let dir = pob_dir().join("Data/TimelessJewelData");
    let mut bins: Vec<String> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "bin"))
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    bins.sort();
    bins
}

/// What the user folder holds: every folder (an empty one too) and every file with its bytes
/// and modification time, so a write of the same bytes shows as well
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Folder,
    File(Vec<u8>, std::time::SystemTime),
}

fn quiet_snapshot(dir: &std::path::Path) -> BTreeMap<PathBuf, Entry> {
    fn walk(dir: &std::path::Path, out: &mut BTreeMap<PathBuf, Entry>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                out.insert(path.clone(), Entry::Folder);
                walk(&path, out);
            } else {
                out.insert(
                    path.clone(),
                    Entry::File(fs::read(&path).unwrap(), meta.modified().unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, &mut out);
    out
}

/// The paths whose entry differs between two snapshots
fn changed_paths(
    a: &BTreeMap<PathBuf, Entry>,
    b: &BTreeMap<PathBuf, Entry>,
) -> std::collections::BTreeSet<PathBuf> {
    a.keys()
        .chain(b.keys())
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect()
}

/// The test's PoB folder: its own files (not the links into the pinned one) with their bytes,
/// and the LUT caches PoB wrote, by name
fn farm_files(farm: &std::path::Path) -> (BTreeMap<PathBuf, Vec<u8>>, Vec<String>) {
    fn walk(dir: &std::path::Path, files: &mut BTreeMap<PathBuf, Vec<u8>>, bins: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(&path, files, bins);
            } else if meta.is_file() {
                if path.extension().is_some_and(|e| e == "bin") {
                    bins.push(path.file_name().unwrap().to_string_lossy().into_owned());
                } else {
                    files.insert(path.clone(), fs::read(&path).unwrap());
                }
            }
        }
    }
    let (mut files, mut bins) = (BTreeMap::new(), Vec::new());
    walk(farm, &mut files, &mut bins);
    bins.sort();
    (files, bins)
}

/// A hand-written `Settings.xml`, laid out unlike PoB's writer, so any `main:SaveSettings`
/// shows in its bytes: two shared items and a shared set, and a last mode of this build list
fn quiet_seed() -> String {
    let ring = "Rarity: RARE\nQuiet Loop\nIron Ring\nImplicits: 0\n+20 to maximum Life";
    let body = "Rarity: UNIQUE\nKaom's Heart\nGlorious Plate\nImplicits: 0\n+500 to maximum Life";
    format!(
        "<PathOfBuilding>\n<Mode mode=\"LIST\"><Arg boolean=\"false\"/><Arg string=\"\"/></Mode>\n\
         <SharedItems><Item>{ring}</Item><Item>{body}</Item>\
         <ItemSet title=\"Quiet Set\"><Item slotName=\"Ring 1\">{ring}</Item></ItemSet></SharedItems>\n\
         <Misc buildSortMode=\"NAME\" showThousandsSeparators=\"false\"/>\n</PathOfBuilding>\n"
    )
}

/// One PoB session and what the user folder held after the last command allowed to write
struct Quiet {
    backend: auto_pob::engine::pob::PobBackend,
    user: PathBuf,
    pob: PathBuf,
    files: BTreeMap<PathBuf, Entry>,
}

impl Drop for Quiet {
    fn drop(&mut self) {
        // `remove_dir_all` unlinks the farm's links without following them
        let _ = fs::remove_dir_all(&self.pob);
        let _ = fs::remove_dir_all(&self.user);
    }
}

impl Quiet {
    fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.backend
            .lua
            .load(code)
            .set_name("=quiet test")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    fn settings(&self) -> PathBuf {
        self.user.join("Path of Building/Settings.xml")
    }

    fn builds(&self) -> PathBuf {
        self.user.join("Path of Building/Builds")
    }

    /// Applies a command as the loop does; no popup may stay in `main.popups`
    fn apply(&mut self, command: Command) -> anyhow::Result<auto_pob::engine::Affected> {
        let what = format!("{command:?}");
        let out = self.backend.apply(command, &|_| {});
        if out.as_ref().is_ok_and(|a| a.recalc) {
            self.backend.recalculate().unwrap();
        }
        let popups: usize = self.eval("return #pob.main.main.popups");
        assert_eq!(popups, 0, "{what} left a popup open");
        out
    }

    /// The user folder against the last allowed write: no file or folder added, removed or
    /// changed, and no file written again with the same bytes
    fn check(&self, what: &str) {
        let changed = changed_paths(&quiet_snapshot(&self.user), &self.files);
        assert!(changed.is_empty(), "{what} wrote {changed:?}");
    }

    /// A command that must write nothing; it may change the build in memory
    fn quiet(&mut self, command: Command) -> auto_pob::engine::Affected {
        let what = format!("{command:?}");
        let affected = self
            .apply(command)
            .unwrap_or_else(|e| panic!("{what}: {e:#}"));
        self.check(&what);
        affected
    }

    /// A command the user invoked to write a build file (SP2's saves, R16's list commands): it
    /// changes exactly `paths` (relative to the Builds folder), and `Settings.xml` stays as it
    /// was (R34)
    fn replay(&mut self, command: Command, paths: &[&str]) {
        let what = format!("{command:?}");
        self.apply(command)
            .unwrap_or_else(|e| panic!("{what}: {e:#}"));
        let now = quiet_snapshot(&self.user);
        let want: std::collections::BTreeSet<PathBuf> =
            paths.iter().map(|p| self.builds().join(p)).collect();
        assert_eq!(changed_paths(&now, &self.files), want, "{what}");
        self.files = now;
    }

    /// One of R29's writers: `Settings.xml` is written, nothing else
    fn writer(&mut self, command: Command) -> auto_pob::engine::Affected {
        let what = format!("{command:?}");
        let affected = self
            .apply(command)
            .unwrap_or_else(|e| panic!("{what}: {e:#}"));
        let now = quiet_snapshot(&self.user);
        let settings = self.settings();
        assert_eq!(
            changed_paths(&now, &self.files),
            [settings].into(),
            "{what}: Settings.xml, and only it"
        );
        self.files = now;
        affected
    }

    /// Every view the UI asks for, each checked for writes
    fn views(&mut self, what: &str) {
        let b = &mut self.backend;
        if b.build_loaded().unwrap() {
            b.build_view().unwrap();
            b.stats_view().unwrap();
            b.config_view().unwrap();
            b.tree_state_view().unwrap();
            b.calcs_view().unwrap();
            b.skills_view().unwrap();
            b.items_view().unwrap();
            b.notes_view().unwrap();
            b.display_item_view().unwrap();
            b.compare_view().unwrap();
            b.tree_link_view().unwrap();
            b.share_view().unwrap();
            b.item_db_view().unwrap();
            b.account_view().unwrap();
        }
        b.build_list_view().unwrap();
        b.popup_view().unwrap();
        b.settings_view().unwrap();
        b.help_view().unwrap();
        b.requests_view().unwrap();
        b.browse_view("").unwrap();
        self.check(&format!("the views at {what}"));
    }

    fn popup(&mut self) -> PopupView {
        self.backend
            .popup_view()
            .unwrap()
            .expect("no popup is open")
    }

    /// Picks a popup dropdown entry by its label
    fn choose(&mut self, name: &str, label: &str) {
        let view = self.popup();
        let control = view.controls.iter().find(|c| c.name == name).unwrap();
        let index = control
            .choices
            .iter()
            .position(|c| c == label)
            .unwrap_or_else(|| panic!("{name} has no {label}: {:?}", control.choices));
        self.quiet(Command::SetPopupControl {
            name: name.into(),
            value: PopupInput::Choice {
                index,
                label: label.into(),
            },
        });
    }

    fn press(&mut self, name: &str) -> auto_pob::engine::Affected {
        self.quiet(Command::PressPopupButton { name: name.into() })
    }

    /// Runs the background job `task` until it is no longer next
    fn drain(&mut self, task: auto_pob::engine::BackgroundTask) {
        while self.backend.next_background() == Some(task) {
            self.backend
                .background_step(task, &|_| {})
                .unwrap_or_else(|e| panic!("{task:?} step: {e:#}"));
        }
        self.check(&format!("{task:?}"));
    }

    fn open(&mut self, name: &str) {
        let path = self.builds().join(format!("{name}.xml"));
        self.quiet(Command::OpenBuild { path });
    }
}

/// Every SP7 read, view and query, Compare's loads and copies, trade, timeless, Help and
/// hovers write nothing; one command of each kind from SP2-SP6 leaves `Settings.xml`
/// byte-identical; only R29's writers write it (R29, R34). The pinned folder, apart from the
/// LUT caches, and `auto-pob.toml` are left as they were.
#[test]
#[ignore = "needs the pinned PoB install"]
fn no_silent_writes_longtail() {
    use super::net_harness::{canned, canned_web, net_user};
    use auto_pob::engine::pob::PobBackend;
    use auto_pob::engine::{
        BackgroundTask, BuildSort, BuySource, CalcsControl, CellRef, CompareSource, ConfigValue,
        ControlTooltipQuery, DeleteKind, GemFilter, GemSearchQuery, GemTarget, GemTooltipQuery,
        HoverTarget, ItemDb, ItemDbQuery, ItemTooltipQuery, SharedItemSource, SkillOption, Tab,
    };
    let _guard = lock();
    let started = std::time::Instant::now();
    let pinned = pinned_files();
    let pinned_bins_before = pinned_bins();
    // the engine has no path to auto-pob.toml: this guards against one appearing
    let config = auto_pob::settings::config_path();
    let config_before = fs::read(&config).ok();

    let pob = super::timeless::farm("quiet");
    let (farm_before, bins_before) = farm_files(&pob);
    assert!(bins_before.is_empty());
    let user = net_user("quiet", Some(&quiet_seed()));
    let builds = user.join("Path of Building/Builds");
    for name in ["attack", "dot"] {
        fs::copy(fixture_xml(name), builds.join(format!("{name}.xml"))).unwrap();
    }
    // the minion fixture's Raise Zombie made Raise Spectre, for the Spectre Library
    let spectre = fs::read_to_string(fixture_xml("minion"))
        .unwrap()
        .replace(
            "skillMinionCalcs=\"RaisedZombie\" skillMinionSkillCalcs=\"1\" ",
            "",
        )
        .replace(" skillMinion=\"RaisedZombie\"", "")
        .replace("SkillGemRaiseZombie", "SkillGemRaiseSpectre")
        .replace("skillId=\"RaiseZombie\"", "skillId=\"RaiseSpectre\"")
        .replace("variantId=\"RaiseZombie\"", "variantId=\"RaiseSpectre\"")
        .replace("nameSpec=\"Raise Zombie\"", "nameSpec=\"Raise Spectre\"");
    fs::write(builds.join("spectre.xml"), spectre).unwrap();
    // attack with a better maul, for Compare's item copy
    let attack = fs::read_to_string(fixture_xml("attack")).unwrap();
    assert!(attack.contains("Driftwood Maul\nQuality: 0\n"));
    fs::write(
        builds.join("gear.xml"),
        attack.replace(
            "Driftwood Maul\nQuality: 0\n",
            "Driftwood Maul\nQuality: 20\n",
        ),
    )
    .unwrap();
    let seeded = quiet_snapshot(&user);
    let backend = PobBackend::start_with(Game::Poe1, &pob, &user, canned(&canned_web()))
        .expect("starting PoB");
    let mut s = Quiet {
        backend,
        user,
        pob,
        files: seeded,
    };
    s.check("starting PoB");
    let refused = s.backend.net_handle().refused();
    s.views("start");

    // ---- SP7: the app's values, Help, the build list's reads ----
    s.quiet(Command::SetTradeDefaults {
        realm: "PC".into(),
        league: "Standard".into(),
    });
    s.quiet(Command::SetOverrides {
        values: BTreeMap::from([("showThousandsSeparators".to_string(), "true".to_string())]),
    });
    s.quiet(Command::Help);
    s.quiet(Command::ListBuilds { folder: "".into() });
    s.quiet(Command::SetBuildFilter { text: "dot".into() });
    s.quiet(Command::SetBuildFilter { text: "".into() });
    s.quiet(Command::BrowseBuilds { folder: "".into() });
    s.views("the build list");
    s.open("attack");
    s.views("attack opened");

    // ---- SP7: hovers, Shift tooltips, the top bar ----
    for target in [
        HoverTarget::GemQuality { group: 0, slot: 0 },
        HoverTarget::GemEnabled { group: 0, slot: 0 },
        HoverTarget::GemCount { group: 0, slot: 0 },
        HoverTarget::ConfigOption {
            var: "usePowerCharges".into(),
        },
    ] {
        s.quiet(Command::HoverTooltip(target.clone()));
        s.backend.hover_tooltip_view(&target).unwrap();
    }
    let items = s.backend.items_view().unwrap();
    let equipped: Vec<(String, u32)> = items
        .items
        .iter()
        .filter_map(|i| i.equipped.as_ref().map(|e| (e.slot.clone(), i.id)))
        .collect();
    let (slot, id) = equipped.first().cloned().expect("attack equips nothing");
    for shift in [false, true] {
        let query = ItemTooltipQuery {
            id,
            slot: Some(slot.clone()),
            theirs: false,
            shift,
            shared: None,
        };
        s.quiet(Command::ItemTooltip(query.clone()));
        s.backend.item_tooltip_view(&query).unwrap();
        let shared = ItemTooltipQuery {
            shared: Some(0),
            ..query
        };
        s.backend.item_tooltip_view(&shared).unwrap();
    }
    s.quiet(Command::CopySharedItem { index: 0 });
    s.check("tooltips");
    s.quiet(Command::SetCharacterLevel { level: 90 });
    s.quiet(Command::SetLevelMode { auto: true });
    s.quiet(Command::NewLoadout {
        name: "Quiet".into(),
    });
    s.quiet(Command::SyncLoadouts);
    s.views("hovers and the top bar");

    // ---- SP7: Compare, its loads, views, copies and Buy similar ----
    let code = s.backend.share_code().unwrap();
    s.quiet(Command::LoadCompare {
        source: CompareSource::File(s.builds().join("dot.xml")),
    });
    s.views("dot compared");
    s.quiet(Command::LoadCompare {
        source: CompareSource::Code(code),
    });
    s.quiet(Command::LoadCompare {
        source: CompareSource::File(s.builds().join("gear.xml")),
    });
    s.quiet(Command::SelectCompare { index: 0 });
    let view = s.backend.compare_view().unwrap().expect("a comparison");
    s.check("the compare view");
    if let Some(control) = view.controls.iter().find(|c| c.choices.len() > 1) {
        s.quiet(Command::SetCompareControl {
            name: control.name.clone(),
            value: PopupInput::Choice {
                index: 1,
                label: control.choices[1].clone(),
            },
        });
    }
    s.quiet(Command::SelectCompare { index: 2 });
    let view = s.backend.compare_view().unwrap().expect("a comparison");
    s.quiet(Command::SetCompareOverlay { on: true });
    s.backend.tree_state_view().unwrap();
    s.quiet(Command::SetCompareOverlay { on: false });
    let row = view
        .items
        .iter()
        .find(|r| r.can_copy && r.theirs.is_some())
        .expect("no item of gear's to copy")
        .slot
        .clone();
    s.backend
        .item_tooltip_view(&ItemTooltipQuery {
            id: 0,
            slot: Some(row.clone()),
            theirs: true,
            shift: true,
            shared: None,
        })
        .unwrap();
    s.quiet(Command::OpenPopup(PopupKind::BuySimilar {
        source: BuySource::CompareSlot(row.clone()),
    }));
    s.popup();
    let url = s.press("search").external_url.expect("no trade link");
    assert!(
        url.url
            .starts_with("https://www.pathofexile.com/trade/search/")
    );
    s.press("close");
    s.quiet(Command::CopyCompareItem {
        slot: row,
        and_use: true,
    });
    s.quiet(Command::CopyCompareTree { and_use: false });
    s.quiet(Command::CopyCompareConfig);
    s.views("the copies");
    s.quiet(Command::RemoveCompare { index: 1 });
    s.quiet(Command::SelectCompare { index: 0 });

    // ---- SP7: Buy similar on our own items ----
    s.quiet(Command::OpenPopup(PopupKind::BuySimilar {
        source: BuySource::Slot(slot.clone()),
    }));
    s.press("search").external_url.expect("no trade link");
    s.quiet(Command::ClosePopup);
    s.quiet(Command::EditItem { id });
    s.quiet(Command::OpenPopup(PopupKind::BuySimilar {
        source: BuySource::DisplayItem,
    }));
    s.press("search").external_url.expect("no trade link");
    s.quiet(Command::ClosePopup);
    s.quiet(Command::CancelDisplayItem { if_id: None });

    // ---- SP7: the timeless search, its trade link and a cancelled second search ----
    s.quiet(Command::OpenPopup(PopupKind::TimelessJewel));
    let socket = s
        .popup()
        .controls
        .iter()
        .find(|c| c.name == "socketSelect")
        .unwrap()
        .choices[1]
        .clone();
    s.choose("jewelSelect", "Lethal Pride");
    s.choose("conquerorSelect", "Kaom (Strength of Blood)");
    s.choose("socketSelect", &socket);
    s.quiet(Command::SetPopupControl {
        name: "searchList".into(),
        value: PopupInput::Text("karui_notable_add_armour, 2, 0, 0".into()),
    });
    s.press("searchButton");
    s.drain(BackgroundTask::Timeless);
    s.views("the timeless results");
    s.quiet(Command::ControlTooltip(ControlTooltipQuery::Popup {
        control: "searchResults".into(),
        index: Some(0),
    }));
    s.quiet(Command::SetPopupControl {
        name: "searchResults".into(),
        value: PopupInput::Row("1".into()),
    });
    s.press("searchTradeButton")
        .external_url
        .expect("no timeless trade link");
    s.press("searchButton");
    s.quiet(Command::CancelTimelessSearch);
    s.quiet(Command::ClosePopup);

    // ---- SP7: Options and the Mod Browser, closed unsaved ----
    s.quiet(Command::OpenPopup(PopupKind::Options));
    s.quiet(Command::SetPopupControl {
        name: "showThousandsSeparators".into(),
        value: PopupInput::Check(false),
    });
    s.views("Options open");
    s.quiet(Command::ClosePopup);
    s.quiet(Command::AddCustomModGroup);
    let group = s.backend.config_view().unwrap().custom_mods.len() - 1;
    s.quiet(Command::OpenPopup(PopupKind::ModBrowser { group }));
    s.popup();
    s.quiet(Command::ClosePopup);

    // ---- SP7 tab parity: shared items read, databases, set and tree managers ----
    s.quiet(Command::ViewSharedItem { index: 0 });
    s.quiet(Command::CancelDisplayItem { if_id: None });
    s.quiet(Command::DropSharedItem {
        index: 1,
        slot: Some("Body Armour".into()),
        at: None,
    });
    s.quiet(Command::DropSharedItem {
        index: 0,
        slot: None,
        at: Some(0),
    });
    s.quiet(Command::ImportSharedItemSet { index: 0 });
    let db_query = ItemDbQuery {
        db: ItemDb::Uniques,
        slot: 0,
        kind: 0,
        league: 0,
        requirement: 0,
        obtainable: 0,
        sort: 0,
        search: "Kaom's Binding".into(),
        search_mode: 0,
    };
    s.quiet(Command::SetItemDb(db_query));
    s.drain(BackgroundTask::ItemDb);
    let rows = match s.backend.item_db_view().unwrap() {
        auto_pob::engine::DbSnapshot::View(view) => view.rows,
        _ => panic!("no database view"),
    };
    let name = rows.first().expect("no unique found").name.clone();
    assert!(name.starts_with("Kaom's Binding"), "{name}");
    s.quiet(Command::ControlTooltip(ControlTooltipQuery::DbItem {
        db: ItemDb::Uniques,
        name: name.clone(),
        shift: true,
    }));
    s.quiet(Command::ViewDbItem {
        db: ItemDb::Uniques,
        name: name.clone(),
    });
    s.quiet(Command::AddDbItem {
        db: ItemDb::Uniques,
        name: name.clone(),
        second_slot: false,
    });
    s.quiet(Command::DropDbItemOnList {
        db: ItemDb::Uniques,
        name: name.clone(),
        at: None,
    });
    s.quiet(Command::DropDbItem {
        db: ItemDb::Uniques,
        name,
        slot: "Belt".into(),
    });
    s.quiet(Command::CloseItemDb);
    s.quiet(Command::QuickEquipItem {
        id,
        second_slot: false,
    });
    s.quiet(Command::NewSkillSet {
        title: "Quiet".into(),
    });
    s.quiet(Command::MoveSkillSet { from: 0, to: 1 });
    let config_set = s.backend.config_view().unwrap().active_set;
    s.quiet(Command::CopyConfigSet {
        id: config_set,
        title: "Quiet".into(),
    });
    s.quiet(Command::MoveConfigSet { from: 0, to: 1 });
    s.quiet(Command::CopySpec {
        index: 0,
        title: "Quiet".into(),
    });
    s.quiet(Command::MoveSpec { from: 0, to: 1 });
    s.quiet(Command::ToggleHeatmap);
    s.quiet(Command::SetTreeStatDifferences { on: true });
    s.quiet(Command::SetPowerReportFilter {
        filter: 1,
        masteries: true,
    });
    s.views("tab parity");

    // ---- SP7: the Spectre Library before its Save ----
    s.open("spectre");
    s.quiet(Command::OpenPopup(PopupKind::SpectreLibrary));
    s.quiet(Command::SetPopupControl {
        name: "source.searchText".into(),
        value: PopupInput::Text("Archer".into()),
    });
    s.views("the Spectre Library");
    s.quiet(Command::ClosePopup);

    // ---- SP2-SP6, one command of each kind: none writes Settings.xml ----
    s.open("attack");
    let node: u32 = s.eval(
        "local best for id, node in pairs(pob.build.spec.nodes) do \
         if not node.alloc and node.pathDist == 1 and node.type == 'Normal' and (not best or id < best) then best = id end \
         end return best",
    );
    let attack_code = s.backend.share_code().unwrap();
    for command in [
        Command::SetConfig {
            var: "usePowerCharges".into(),
            value: ConfigValue::Bool(true),
        },
        Command::NewConfigSet {
            title: "Quiet".into(),
        },
        Command::SetCustomModText {
            group: 0,
            text: "+10 to maximum Life".into(),
        },
        Command::Undo(Tab::Config),
        Command::Redo(Tab::Config),
        Command::PreviewNode { id: node },
        Command::AllocNode { id: node },
        Command::DeallocNode { id: node },
        Command::SetTreeSearch {
            text: "life".into(),
        },
        Command::ExportTreeLink,
        Command::NewSpec {
            title: "Quiet".into(),
        },
        Command::SelectSpec { index: 0 },
        Command::SetCompareSpec { index: Some(1) },
        Command::SetCompareSpec { index: None },
        Command::Undo(Tab::Tree),
        Command::SetCalcsSearch {
            text: "life".into(),
        },
        Command::SetCalcsSelect {
            control: CalcsControl::Mode,
            index: 1,
        },
        Command::SetNotes {
            text: "quiet".into(),
        },
        Command::SetGemLevel {
            group: 0,
            slot: 0,
            text: "19".into(),
        },
        Command::SetGemQuality {
            group: 0,
            slot: 0,
            text: "20".into(),
        },
        Command::GroupTooltip { group: 0 },
        Command::CopySocketGroup { group: 0 },
        Command::AddSocketGroup,
        Command::SetSkillOption(SkillOption::SortByDps(false)),
        Command::SearchGems(GemSearchQuery {
            group: 0,
            target: GemTarget::Slot(0),
            text: "fire".into(),
            filter: GemFilter::All,
        }),
        Command::GemTooltip(GemTooltipQuery {
            group: 0,
            target: GemTarget::Slot(0),
            gem_id: None,
        }),
        Command::CloseGemSearch,
        Command::Undo(Tab::Skills),
        Command::EditItem { id },
        Command::SetItemQuality { text: "10".into() },
        Command::SaveDisplayItem,
        Command::OpenPopup(PopupKind::Craft),
        Command::ClosePopup,
        Command::SetShowStatDifferences { on: false },
        Command::NewItemSet {
            title: "Quiet".into(),
        },
        Command::CopyItem { id },
        Command::SortItems,
        Command::Undo(Tab::Items),
        Command::ExportShareCode,
        Command::SetImportCode {
            text: attack_code.clone(),
        },
    ] {
        s.quiet(command);
    }
    // the set managers, the rest of each tab's edits, and an import
    s.quiet(Command::NewSkillSet {
        title: "Quiet two".into(),
    });
    let skill_sets = s.backend.skills_view().unwrap().sets;
    let (first, last) = (skill_sets[0].id, skill_sets.last().unwrap().id);
    let config_sets = s.backend.config_view().unwrap().sets;
    let item_sets = s.backend.items_view().unwrap().sets;
    let specs = s.backend.tree_state_view().unwrap().specs.len();
    let groups = s.backend.skills_view().unwrap().groups.len();
    let text = s.backend.socket_group_text(0).unwrap().text;
    for command in [
        Command::RenameSkillSet {
            id: last,
            title: "Quiet three".into(),
        },
        Command::CopySkillSet {
            id: last,
            title: "Quiet copy".into(),
        },
        Command::SelectSkillSet { id: last },
        Command::SelectSkillSet { id: first },
        Command::DeleteSkillSet { id: last },
        Command::RenameConfigSet {
            id: config_sets.last().unwrap().id,
            title: "Quiet two".into(),
        },
        Command::SelectConfigSet {
            id: config_sets[0].id,
        },
        Command::DeleteConfigSet {
            id: config_sets.last().unwrap().id,
        },
        Command::SetCustomModTitle {
            group: 0,
            title: "Quiet".into(),
        },
        Command::SetCustomModEnabled {
            group: 0,
            enabled: false,
        },
        Command::RemoveCustomModGroup { group: 0 },
        Command::RenameSpec {
            index: specs - 1,
            title: "Quiet two".into(),
        },
        Command::DeleteSpec { index: specs - 1 },
        Command::RenameItemSet {
            id: item_sets.last().unwrap().id,
            title: "Quiet two".into(),
        },
        Command::CopyItemSet {
            id: item_sets.last().unwrap().id,
            title: "Quiet copy".into(),
        },
        Command::MoveItemSet { from: 0, to: 1 },
        Command::SelectItemSet {
            id: item_sets.last().unwrap().id,
        },
        Command::SelectItemSet {
            id: item_sets[0].id,
        },
        Command::DeleteItemSet {
            id: item_sets.last().unwrap().id,
        },
        Command::SetCalcsCollapsed {
            section: "HitDamage".into(),
            sub: 0,
            collapsed: true,
        },
        Command::SetMainSocketGroup { group: 0 },
        Command::SetGroupLabel {
            group: 0,
            text: "Quiet".into(),
        },
        Command::SetGroupEnabled { group: 0, on: true },
        Command::SetGroupFullDps { group: 0, on: true },
        Command::SetGemEnabled {
            group: 0,
            slot: 0,
            on: true,
        },
        Command::PasteSocketGroup { text },
        Command::MoveSocketGroup {
            from: groups,
            to: 0,
        },
        Command::DeleteGem { group: 0, slot: 0 },
        Command::DeleteSocketGroup { group: 0 },
        Command::SetItemListFilter { index: 0 },
        Command::EquipItem {
            slot: slot.clone(),
            id: None,
        },
        Command::EquipItem {
            slot: slot.clone(),
            id: Some(id),
        },
        Command::SelectWeaponSet { second: true },
        Command::SelectWeaponSet { second: false },
        Command::DeleteUnusedItems,
        Command::ResetTree,
        Command::Undo(Tab::Tree),
    ] {
        s.quiet(command);
    }
    s.quiet(Command::PasteItem {
        text: "Rarity: RARE\nQuiet Visor\nRoyal Burgonet\nImplicits: 0\n+80 to maximum Life".into(),
    });
    s.quiet(Command::SetDisplayItemText {
        rarity: 2,
        text: "Quiet Visor\nRoyal Burgonet\nImplicits: 0\n+90 to maximum Life".into(),
    });
    s.quiet(Command::SaveDisplayItem);
    let copied = s
        .backend
        .items_view()
        .unwrap()
        .items
        .iter()
        .map(|i| i.id)
        .max()
        .unwrap();
    assert_ne!(copied, id, "the pasted item wasn't added");
    s.quiet(Command::MoveItem { from: 0, to: 1 });
    s.quiet(Command::DeleteItem {
        id: copied,
        confirmed: true,
    });
    let calcs = s.backend.calcs_view().unwrap();
    let cell = calcs
        .sections
        .iter()
        .find_map(|section| {
            section.subs.iter().enumerate().find_map(|(sub, part)| {
                part.rows.iter().find_map(|row| {
                    row.cells.iter().find(|c| c.breakdown).map(|c| CellRef {
                        section: section.id.clone(),
                        sub,
                        row: row.index,
                        cell: c.index,
                    })
                })
            })
        })
        .expect("no breakdown cell");
    s.quiet(Command::Breakdown(cell.clone()));
    s.backend.breakdown_view(&cell).unwrap();
    s.views("SP2-SP6 commands");
    // the user's own writes: build files change, Settings.xml doesn't
    let open = s.backend.build_view().unwrap().path.expect("a saved build");
    let open = open
        .strip_prefix(s.builds())
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    s.replay(Command::Save, &[&open]);
    s.replay(
        Command::SaveAs {
            folder: "".into(),
            name: "quiet-copy".into(),
            overwrite: false,
        },
        &["quiet-copy.xml"],
    );
    s.quiet(Command::NewBuild);
    s.quiet(Command::ListBuilds { folder: "".into() });
    s.replay(
        Command::NewFolder {
            name: "quiet".into(),
        },
        &["quiet"],
    );
    s.replay(
        Command::CopyBuild {
            path: s.builds().join("dot.xml"),
            name: "dot2".into(),
        },
        &["dot2.xml"],
    );
    s.replay(
        Command::RenameBuild {
            path: s.builds().join("dot2.xml"),
            name: "dot3".into(),
        },
        &["dot2.xml", "dot3.xml"],
    );
    s.replay(
        Command::MoveBuild {
            path: s.builds().join("dot3.xml"),
            folder: "quiet/".into(),
        },
        &["dot3.xml", "quiet/dot3.xml"],
    );
    s.replay(
        Command::DeleteBuild {
            path: s.builds().join("quiet/dot3.xml"),
            kind: DeleteKind::Build,
        },
        &["quiet/dot3.xml"],
    );
    s.check("the build list's own writes");
    // a share code imports as a new build in memory, with no file until it is saved
    s.quiet(Command::ImportShareCode { code: attack_code });
    s.views("an imported build");

    // ---- R29's writers: Settings.xml only ----
    s.writer(Command::SetBuildSort {
        sort: BuildSort::Level,
    });
    s.open("attack");
    s.quiet(Command::OpenPopup(PopupKind::Options));
    s.quiet(Command::SetPopupControl {
        name: "defaultGemQuality".into(),
        value: PopupInput::Text("7".into()),
    });
    s.writer(Command::PressPopupButton {
        name: "save".into(),
    });
    let set = s.backend.items_view().unwrap().active_set;
    for command in [
        Command::AddSharedItem {
            source: SharedItemSource::Item { id },
            index: 2,
        },
        Command::MoveSharedItem { from: 2, to: 0 },
        Command::DeleteSharedItem { index: 0 },
        Command::AddSharedItemSet { id: set },
        Command::RenameSharedItemSet {
            index: 1,
            title: "Quiet Two".into(),
        },
        Command::MoveSharedItemSet { from: 1, to: 0 },
        Command::DeleteSharedItemSet { index: 0 },
    ] {
        s.writer(command);
    }

    // ---- PoB's exit save (engine M-5): Settings.xml only ----
    s.backend.shutdown();
    assert_eq!(
        changed_paths(&quiet_snapshot(&s.user), &s.files),
        [s.settings()].into(),
        "the exit save"
    );

    // ---- nothing reached the network, the pinned folder or auto-pob.toml ----
    // Buy similar's league fetch, refused at each of its three opens (R10); nothing else
    assert_eq!(
        s.backend.net_handle().refused() - refused,
        3,
        "a request other than Buy similar's league fetch was attempted"
    );
    let requests = s.backend.requests_view().unwrap();
    assert!(requests.pending.is_empty() && requests.finished.is_empty());
    let (farm_after, bins) = farm_files(&s.pob);
    assert!(
        farm_after == farm_before,
        "PoB's own files in the farm changed"
    );
    assert!(!bins.is_empty(), "the search read no LUT");
    assert!(pinned_files() == pinned, "the pinned PoB folder changed");
    assert_eq!(
        pinned_bins(),
        pinned_bins_before,
        "a LUT cache was written into the pinned PoB folder"
    );
    assert_eq!(
        fs::read(&config).ok(),
        config_before,
        "auto-pob.toml changed"
    );
    eprintln!("no_silent_writes_longtail: {:?}", started.elapsed());
}
