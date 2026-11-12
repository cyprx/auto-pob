//! Task 12's parity gaps: PoB's Spectre Library (R18) and the top bar's level, bloodline and
//! loadouts (R21), against upstream PoB pressing the same controls. Upstream's side is
//! `fixtures/longtail_lib.lua`'s top-bar part: PoB's own buttons, dropdown picks and frames.

use super::{Session, fixture_xml, lock, pob_dir};
use auto_pob::engine::upstream::{golden_of, load_xml, upstream_lua};
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

    fn stats(&self) -> BTreeMap<String, f64> {
        golden_of(&self.lua, Game::Poe1.pob_version())
            .expect("upstream stats")
            .stats
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
    let want = up.top();
    assert_eq!(ours.top(), want, "{what}: PoB state");
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
    want
}

/// Our sidebar's stats against upstream's calculation, row by row
fn same_stats(ours: &mut Ours, up: &Upstream, what: &str) -> StatsView {
    let view = ours.stats();
    let want = up.stats();
    assert!(!view.stats.is_empty(), "{what}: no stats");
    for row in &view.stats {
        let value = want
            .get(&row.key)
            .unwrap_or_else(|| panic!("{what}: {} isn't upstream's", row.key));
        assert_eq!(row.value, *value, "{what}: {}", row.key);
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
    assert!(found.len() >= 3, "\"Archer\" finds too little: {found:?}");
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
    let (first, second, third) = (found[0].0.clone(), found[1].0.clone(), found[2].0.clone());
    for key in [&first, &second] {
        set(&mut ours, "source", PopupInput::Row(key.clone()));
        press(&mut ours, "source.add");
        up.call::<()>("libraryPress", ("source", key.as_str(), "add"));
    }
    set(&mut ours, "list", PopupInput::Row(first.clone()));
    press(&mut ours, "list.delete");
    up.call::<()>("libraryPress", ("list", first.as_str(), "delete"));
    let dest = keys(&rows(&ours.popup().unwrap(), "list"));
    assert_eq!(dest, up.get::<Vec<String>>("libraryDest", ()));
    assert_eq!(dest, std::slice::from_ref(&second));
    // nothing is committed before Save
    assert_eq!(ours.top().spectres, before);
    ours.clear_flags();
    up.call::<()>("clearFlags", ());
    press(&mut ours, "save");
    up.call::<()>("libraryClose", "save");
    assert!(ours.popup().is_none(), "Save left the window open");
    let saved = same(&mut ours, &up, "library saved");
    assert_eq!(saved.spectres, std::slice::from_ref(&second));
    assert!(saved.flags["build"], "Save marked nothing unsaved");
    let minions = ours.stats().main_skill.minion.expect("no minion dropdown");
    assert_eq!(minions.labels, up.get::<Vec<String>>("minionLabels", ()));
    assert_eq!(minions.labels.len(), 1);
    same_stats(&mut ours, &up, "library saved");
    // Cancel commits nothing
    open_and_search(&mut ours, &up);
    set(&mut ours, "source", PopupInput::Row(third.clone()));
    press(&mut ours, "source.add");
    up.call::<()>("libraryPress", ("source", third.as_str(), "add"));
    // the flags stay as Save left them: Cancel runs no frame on our side to refresh `unsaved`
    press(&mut ours, "cancel");
    up.call::<()>("libraryClose", "cancel");
    assert!(ours.popup().is_none(), "Cancel left the window open");
    let cancelled = same(&mut ours, &up, "library cancelled");
    assert_eq!(cancelled.spectres, saved.spectres);
    assert_eq!(cancelled.flags, saved.flags);

    // ---- Loadouts ----
    let base = same(&mut ours, &up, "before loadouts");
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
    assert_eq!(ours.top().specs, back.specs);
}
