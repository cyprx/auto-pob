//! Task 13a's parity gaps: the Skills, Config, Calcs and Tree tabs (R22, R24, R31, NF-3, NF-4),
//! against upstream PoB pressing the same controls. Upstream's side is
//! `fixtures/tabs_lib.lua`: PoB's list drops (`ListControl:OnKeyUp`) and the controls' own
//! hover draws (`DrawTooltip`).

use super::{Session, fixture_xml, lock, pob_dir};
use auto_pob::engine::upstream::{golden_of, load_xml, upstream_lua};
use auto_pob::engine::{Backend, Command, HoverTarget, StatsView, TooltipLine};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const LIB: &str = include_str!("../fixtures/tabs_lib.lua");

/// What `tabState` reads, on either side
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct TabState {
    skill_sets: Vec<(u32, String)>,
    active_skill_set: u32,
    config_sets: Vec<(u32, String)>,
    active_config_set: u32,
    specs: Vec<SpecState>,
    active_spec: u32,
    #[serde(default)]
    items_spec_list: Vec<String>,
    items_spec_sel: u32,
    flags: BTreeMap<String, bool>,
    undo: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct SpecState {
    title: String,
    version: String,
    #[serde(default)]
    alloc: Vec<u32>,
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
            .set_name("=tabs_lib")
            .exec()
            .expect("tabs lib");
        Upstream { lua }
    }

    /// Loads a build and clears the flags the wrapper's load sets, as opening a file leaves
    /// them on our side
    fn load(&self, xml: &str, name: &str) {
        load_xml(&self.lua, xml, name).unwrap();
        self.lua.load("runCallback('OnFrame')").exec().unwrap();
        self.lua.load(CLEAR_FLAGS).exec().unwrap();
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

    fn state(&self) -> TabState {
        self.get("tabState", ())
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

/// The flags a fresh load leaves clear on both sides
const CLEAR_FLAGS: &str = r#"
    local b = (pob and pob.build) or build
    b.modFlag = false
    b.spec.modFlag = false
    b.treeTab.modFlag = false
    b.skillsTab.modFlag = false
    b.itemsTab.modFlag = false
    b.configTab.modFlag = false
    b.calcsTab.modFlag = false
"#;

/// Our side: an engine session with the same reader loaded into its Lua state
struct Ours {
    s: Session,
}

impl Ours {
    fn new() -> Ours {
        let s = Session::start("parity-gaps-tabs");
        s.backend
            .lua
            .load(LIB)
            .set_name("=tabs_lib")
            .exec()
            .expect("tabs lib in our state");
        Ours { s }
    }

    /// Writes `xml` into the Builds folder as `name`, opens it and clears the flags
    fn open(&mut self, xml: &str, name: &str) {
        let path: PathBuf = self.s.builds_dir().join(format!("{name}.xml"));
        fs::write(&path, xml).unwrap();
        self.run(Command::OpenBuild { path }).expect("opening");
        self.s.backend.lua.load(CLEAR_FLAGS).exec().unwrap();
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

    fn state(&self) -> TabState {
        let value: mlua::Value = self.eval("return tabState(pob.build)");
        self.s.backend.lua.from_value(value).unwrap()
    }

    fn stats(&mut self) -> StatsView {
        self.s.stats()
    }

    /// The engine's answer to a hover, as the loop sends it
    fn hover(&mut self, target: HoverTarget) -> Vec<TooltipLine> {
        let affected = self
            .s
            .backend
            .apply(Command::HoverTooltip(target.clone()), &|_| {})
            .expect("hover");
        assert_eq!(affected.hover.as_ref(), Some(&target), "the hover's flag");
        let view = self
            .s
            .backend
            .hover_tooltip_view(&target)
            .expect("hover view")
            .unwrap_or_else(|| panic!("no answer to {target:?}"));
        assert_eq!(view.target, target);
        let popups: usize = self.eval("return #pob.main.main.popups");
        assert_eq!(popups, 0, "the hover left a popup open");
        view.lines
    }
}

/// Both sides' sets, trees and flags after the same act
fn same(ours: &Ours, up: &Upstream, what: &str) -> TabState {
    let want = up.state();
    assert_eq!(ours.state(), want, "{what}: PoB state");
    assert_eq!(up.popups(), 0, "{what}: upstream left a popup open");
    want
}

/// Our sidebar's stats against upstream's calculation
fn same_stats(ours: &mut Ours, up: &Upstream, what: &str) {
    let view = ours.stats();
    let want = up.stats();
    assert!(!view.stats.is_empty(), "{what}: no stats");
    for row in &view.stats {
        let value = want
            .get(&row.key)
            .unwrap_or_else(|| panic!("{what}: {} isn't upstream's", row.key));
        assert_eq!(row.value, *value, "{what}: {}", row.key);
    }
}

fn texts(lines: &[TooltipLine]) -> Vec<&str> {
    lines
        .iter()
        .filter_map(|l| match l {
            TooltipLine::Line { text, .. } => Some(text.as_str()),
            TooltipLine::Separator { .. } => None,
        })
        .collect()
}

/// The attack fixture with three titled skill sets, its support gem disabled, and two more
/// trees and config sets for the managers' steps
fn attack_xml() -> String {
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let support = "nameSpec=\"Melee Physical Damage\" gemId=\"Metadata/Items/Gems/SupportGemMeleePhysicalDamage\" enabled=\"true\"";
    assert!(xml.contains(support));
    let xml = xml.replace(
        support,
        &support.replace("enabled=\"true\"", "enabled=\"false\""),
    );
    let first = "<SkillSet id=\"1\">";
    assert!(xml.contains(first));
    let xml = xml.replace(first, "<SkillSet id=\"1\" title=\"One\">");
    let end = "\t\t</SkillSet>\n";
    assert!(xml.contains(end));
    xml.replacen(
        end,
        &format!(
            "{end}\t\t<SkillSet id=\"2\" title=\"Two\">\n\t\t</SkillSet>\n\t\t<SkillSet id=\"3\" title=\"Three\">\n\t\t</SkillSet>\n"
        ),
        1,
    )
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_skills_match_pob() {
    let _guard = lock();
    let xml = attack_xml();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "tabs");
    up.load(&xml, "tabs");
    let start = same(&ours, &up, "opened");
    assert_eq!(
        start.skill_sets,
        [
            (1, "One".to_string()),
            (2, "Two".to_string()),
            (3, "Three".to_string())
        ]
    );

    // ---- Hover tooltips (R22): each box's lines equal upstream's own draw ----
    for (target, which, header) in [
        (
            HoverTarget::GemQuality { group: 0, slot: 0 },
            "quality",
            "Setting to 20 quality will give you:",
        ),
        (
            HoverTarget::GemEnabled { group: 0, slot: 1 },
            "enabled",
            "Enabling this gem will give you:",
        ),
        (
            HoverTarget::GemEnabled { group: 0, slot: 0 },
            "enabled",
            "Disabling this gem will give you:",
        ),
        (
            HoverTarget::GemCount { group: 0, slot: 0 },
            "count",
            "scales the DPS of associated skill",
        ),
    ] {
        let (group, slot) = match target {
            HoverTarget::GemQuality { group, slot }
            | HoverTarget::GemEnabled { group, slot }
            | HoverTarget::GemCount { group, slot } => (group, slot),
            _ => unreachable!(),
        };
        let lines = ours.hover(target.clone());
        let want: Vec<TooltipLine> = up.get("upGemHover", (group, slot, which));
        assert_eq!(lines, want, "{target:?}");
        assert!(
            texts(&lines).iter().any(|t| t.contains(header)),
            "{target:?} has no {header:?}: {lines:?}"
        );
    }
    // the quality hover also lists the gem's quality stats; the what-if lines compare stats
    let quality = ours.hover(HoverTarget::GemQuality { group: 0, slot: 0 });
    assert!(
        texts(&quality)
            .iter()
            .any(|t| t.ends_with("At +20% Quality:")),
        "{quality:?}"
    );
    assert!(quality.len() > 4, "no stat comparison: {quality:?}");
    // a hover changes nothing: the flags, undo and the calculation are upstream's after a click
    same(&ours, &up, "after the hovers");
    same_stats(&mut ours, &up, "after the hovers");
    // a gem that is gone has no answer
    assert!(
        ours.s
            .backend
            .hover_tooltip_view(&HoverTarget::GemQuality { group: 0, slot: 9 })
            .unwrap()
            .is_none()
    );

    // ---- Skill set reorder (R31) ----
    ours.run(Command::MoveSkillSet { from: 0, to: 2 })
        .expect("moving the set");
    up.call::<()>("upMoveSkillSet", (0, 2));
    let moved = same(&ours, &up, "MoveSkillSet{0, 2}");
    assert_eq!(
        moved.skill_sets.iter().map(|s| s.0).collect::<Vec<_>>(),
        [2, 3, 1]
    );
    // only the tab's modFlag: no undo state, no recalculation, the active set kept
    assert!(moved.flags["skills"]);
    assert_eq!(moved.undo, start.undo);
    assert_eq!(moved.active_skill_set, 1);
    assert!(!moved.flags["build_flag"]);
    ours.run(Command::MoveSkillSet { from: 2, to: 1 })
        .expect("moving it back up");
    up.call::<()>("upMoveSkillSet", (2, 1));
    let back = same(&ours, &up, "MoveSkillSet{2, 1}");
    assert_eq!(
        back.skill_sets.iter().map(|s| s.0).collect::<Vec<_>>(),
        [2, 1, 3]
    );
    let err = ours
        .run(Command::MoveSkillSet { from: 0, to: 3 })
        .unwrap_err();
    assert_eq!(err.to_string(), "no such skill set");
    same(&ours, &up, "a refused move");
    let unsaved: bool = ours.eval("return pob.build.unsaved and true or false");
    assert!(unsaved, "a reorder marks the build unsaved");
}

/// `attack_xml` with a second config set and a second, titled tree
fn sets_xml() -> String {
    let xml = attack_xml();
    let config = "<ConfigSet id=\"1\">";
    assert!(xml.contains(config));
    let xml = xml.replace(config, "<ConfigSet id=\"1\" title=\"First\">");
    let end = "\t\t</ConfigSet>\n";
    assert!(xml.contains(end));
    let xml = xml.replacen(
        end,
        &format!(
            "{end}\t\t<ConfigSet id=\"2\" title=\"Second\">\n\t\t\t<Input boolean=\"true\" name=\"buffOnslaught\"/>\n\t\t\t<CustomModifierBlock title=\"Default\" enabled=\"true\"></CustomModifierBlock>\n\t\t</ConfigSet>\n"
        ),
        1,
    );
    let start = xml.find("\t\t<Spec ").expect("a spec");
    let stop = xml.find("\t\t</Spec>\n").expect("the spec's end") + "\t\t</Spec>\n".len();
    let spec = &xml[start..stop];
    let main = spec.replacen("<Spec ", "<Spec title=\"Main\" ", 1);
    let bare = spec.replacen("<Spec ", "<Spec title=\"Bare\" ", 1);
    format!("{}{main}{bare}{}", &xml[..start], &xml[stop..])
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_config_match_pob() {
    let _guard = lock();
    let xml = sets_xml();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "tabs");
    up.load(&xml, "tabs");
    let start = same(&ours, &up, "opened");
    assert_eq!(start.config_sets.len(), 2);

    // ---- Config hover (R22): a check box, a dropdown and an edit box ----
    for (var, header) in [
        ("buffOnslaught", Some("Toggling this option will give you:")),
        (
            "usePowerCharges",
            Some("Toggling this option will give you:"),
        ),
        ("enemyIsBoss", None),
        ("enemyLevel", None),
    ] {
        let target = HoverTarget::ConfigOption { var: var.into() };
        let lines = ours.hover(target);
        let want: Vec<TooltipLine> = up.get("upConfigHover", var);
        assert_eq!(lines, want, "{var}");
        assert!(!lines.is_empty(), "{var}: no lines");
        if let Some(header) = header {
            assert!(
                texts(&lines).iter().any(|t| t.contains(header)),
                "{var} has no {header:?}: {lines:?}"
            );
        }
    }
    // a custom group's enable box compares the build without the group's mods
    let text = "+500 to Strength";
    ours.run(Command::SetCustomModText {
        group: 0,
        text: text.into(),
    })
    .expect("typing the mods");
    up.call::<()>("upCustomModText", (0, text));
    same_stats(&mut ours, &up, "custom mods typed");
    let lines = ours.hover(HoverTarget::CustomModEnabled { group: 0 });
    let want: Vec<TooltipLine> = up.get("upCustomModHover", 0);
    assert_eq!(lines, want, "custom group hover");
    assert!(
        texts(&lines)
            .iter()
            .any(|t| t.contains("Disabling this group will give you:")),
        "{lines:?}"
    );
    // hovers change nothing
    let before = same(&ours, &up, "after the hovers");
    same_stats(&mut ours, &up, "after the hovers");
    assert!(
        ours.s
            .backend
            .hover_tooltip_view(&HoverTarget::ConfigOption {
                var: "noSuchOption".into()
            })
            .unwrap()
            .is_none()
    );

    // ---- Config set Copy and reorder (NF-4) ----
    ours.run(Command::CopyConfigSet {
        id: 2,
        title: "Third".into(),
    })
    .expect("copying the set");
    up.call::<()>("upCopyConfigSet", (1, "Third"));
    let copied = same(&ours, &up, "CopyConfigSet");
    assert_eq!(copied.config_sets.len(), 3);
    assert_eq!(copied.config_sets[2].1, "Third");
    assert_eq!(copied.active_config_set, 1, "a copy isn't switched to");
    assert_eq!(copied.undo["config"], before.undo["config"] + 1);
    // the copy keeps the set's inputs
    let onslaught = format!(
        "return pob.build.configTab.configSets[{}].input.buffOnslaught",
        copied.config_sets[2].0
    );
    assert!(ours.eval::<bool>(&onslaught));
    let err = ours
        .run(Command::CopyConfigSet {
            id: 1,
            title: "  ".into(),
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "the config set needs a name");
    same(&ours, &up, "a refused copy");

    ours.run(Command::MoveConfigSet { from: 2, to: 0 })
        .expect("moving the set");
    up.call::<()>("upMoveConfigSet", (2, 0));
    let moved = same(&ours, &up, "MoveConfigSet{2, 0}");
    assert_eq!(moved.config_sets[0].1, "Third");
    assert!(moved.flags["config"]);
    assert_eq!(moved.undo, copied.undo, "a reorder adds no undo state");
    assert_eq!(moved.active_config_set, 1);
    same_stats(&mut ours, &up, "after the config sets");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_tree_match_pob() {
    let _guard = lock();
    let xml = sets_xml();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "tabs");
    up.load(&xml, "tabs");
    let start = same(&ours, &up, "opened");
    assert_eq!(
        start
            .specs
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["Main", "Bare"]
    );

    // ---- Tree Copy and reorder (NF-3) ----
    ours.run(Command::CopySpec {
        index: 0,
        title: "Copied".into(),
    })
    .expect("copying the tree");
    up.call::<()>("upCopySpec", (0, "Copied"));
    let copied = same(&ours, &up, "CopySpec");
    assert_eq!(copied.specs.len(), 3);
    assert_eq!(copied.specs[2].title, "Copied");
    assert_eq!(copied.specs[2].alloc, copied.specs[0].alloc);
    assert!(copied.specs[2].alloc.len() > 5, "the copy has the nodes");
    assert_eq!(copied.active_spec, 1, "a copy isn't selected");
    assert_eq!(copied.items_spec_list, ["Main", "Bare", "Copied"]);
    let err = ours
        .run(Command::CopySpec {
            index: 7,
            title: "x".into(),
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "unknown tree 7");

    // moving the active tree down keeps it active, at its new place
    ours.run(Command::MoveSpec { from: 0, to: 2 })
        .expect("moving the tree");
    up.call::<()>("upMoveSpec", (0, 2));
    let moved = same(&ours, &up, "MoveSpec{0, 2}");
    assert_eq!(
        moved
            .specs
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        ["Bare", "Copied", "Main"]
    );
    assert_eq!(moved.active_spec, 3);
    assert_eq!(moved.items_spec_list, ["Bare", "Copied", "Main"]);
    assert!(moved.flags["tree"]);
    // the tree state view follows the order
    let view = ours.s.backend.tree_state_view().expect("tree view");
    assert_eq!(view.active_spec, 2);
    assert_eq!(view.specs[2].title, "Main");
    same_stats(&mut ours, &up, "after the trees");
}
