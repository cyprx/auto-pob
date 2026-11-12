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
    class_id: u32,
    ascend_id: u32,
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
    b.unsaved = false
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

    /// Every file under our user folder, with its length and modification time
    fn files(&self) -> BTreeMap<PathBuf, (u64, std::time::SystemTime)> {
        fn walk(dir: &std::path::Path, out: &mut BTreeMap<PathBuf, (u64, std::time::SystemTime)>) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    walk(&path, out);
                } else {
                    out.insert(path, (meta.len(), meta.modified().unwrap()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.s.user, &mut out);
        out
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

/// Clears the flags and `unsaved` on both sides, so the next act's own flags show
fn clear_flags(ours: &Ours, up: &Upstream) {
    ours.s.backend.lua.load(CLEAR_FLAGS).exec().unwrap();
    up.lua.load(CLEAR_FLAGS).exec().unwrap();
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
    // a second group, for a hover on a group other than the one shown
    let skill_end = "\t\t\t</Skill>\n";
    assert!(xml.contains(skill_end));
    let xml = xml.replacen(
        skill_end,
        &format!(
            "{skill_end}\t\t\t<Skill mainActiveSkill=\"1\" includeInFullDPS=\"nil\" enabled=\"true\" label=\"Second\">\n\t\t\t\t<Gem quality=\"0\" level=\"20\" count=\"1\" skillId=\"Clarity\" enableGlobal1=\"true\" enableGlobal2=\"true\" nameSpec=\"Clarity\" gemId=\"Metadata/Items/Gems/SkillGemClarity\" enabled=\"true\" variantId=\"Clarity\"/>\n\t\t\t</Skill>\n"
        ),
        1,
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
    let written = ours.files();
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

    // a group other than the shown one: our hover shows it only for the call and puts the
    // shown group back; upstream's is a click on its row first
    let shown = "local t = pob.build.skillsTab; for i, g in ipairs(t.socketGroupList) do if g == t.displayGroup then return i end end return 0";
    let before: u32 = ours.eval(shown);
    let lines = ours.hover(HoverTarget::GemQuality { group: 1, slot: 0 });
    let want: Vec<TooltipLine> = up.get("upGemHover", (1, 0, "quality"));
    assert_eq!(lines, want, "the second group's quality");
    assert!(
        texts(&lines).iter().any(|t| t.contains("Clarity")),
        "{lines:?}"
    );
    assert_eq!(
        ours.eval::<u32>(shown),
        before,
        "the shown group was put back"
    );
    same(&ours, &up, "after a hover on the second group");
    same_stats(&mut ours, &up, "after a hover on the second group");

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
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
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
    let written = ours.files();
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
    clear_flags(&ours, &up);
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
    assert!(copied.flags["unsaved"], "CopyConfigSet: unsaved");
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

    clear_flags(&ours, &up);
    ours.run(Command::MoveConfigSet { from: 2, to: 0 })
        .expect("moving the set");
    up.call::<()>("upMoveConfigSet", (2, 0));
    let moved = same(&ours, &up, "MoveConfigSet{2, 0}");
    assert_eq!(moved.config_sets[0].1, "Third");
    assert!(moved.flags["config"]);
    assert!(moved.flags["unsaved"], "MoveConfigSet: unsaved");
    assert_eq!(moved.undo, copied.undo, "a reorder adds no undo state");
    assert_eq!(moved.active_config_set, 1);
    same_stats(&mut ours, &up, "after the config sets");
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
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
    let written = ours.files();
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
    assert!(copied.flags["unsaved"], "CopySpec: unsaved");
    assert_eq!(copied.items_spec_list, ["Main", "Bare", "Copied"]);
    let err = ours
        .run(Command::CopySpec {
            index: 7,
            title: "x".into(),
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "unknown tree 7");

    // moving the active tree down keeps it active, at its new place
    clear_flags(&ours, &up);
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
    assert!(moved.flags["unsaved"], "MoveSpec: unsaved");
    // the tree state view follows the order
    let view = ours.s.backend.tree_state_view().expect("tree view");
    assert_eq!(view.active_spec, 2);
    assert_eq!(view.specs[2].title, "Main");
    same_stats(&mut ours, &up, "after the trees");
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
}

/// What `upCalcsWidths` reads of one section
#[derive(Debug, serde::Deserialize)]
struct UpSection {
    id: String,
    widths: Vec<f32>,
    pop_out: bool,
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_calcs_match_pob() {
    let _guard = lock();
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "attack");
    up.load(&xml, "attack");
    let written = ours.files();
    let view = ours.s.backend.calcs_view().expect("calcs view");
    let want: Vec<UpSection> = up.get("upCalcsWidths", ());
    assert_eq!(view.sections.len(), want.len(), "sections");
    let mut compared = 0;
    let mut seen = std::collections::BTreeSet::new();
    for (section, up_section) in view.sections.iter().zip(&want) {
        assert_eq!(section.id, up_section.id);
        assert_eq!(
            section.col_widths.len(),
            up_section.widths.len(),
            "{}: sub-sections",
            section.id
        );
        for (si, (&ours_w, &up_w)) in section
            .col_widths
            .iter()
            .zip(&up_section.widths)
            .enumerate()
        {
            // PoB gives a width only to the cells it lays out
            if up_w > 0.0 {
                assert_eq!(ours_w, up_w, "{} sub {si}: column width", section.id);
                compared += 1;
                seen.insert(ours_w as u32);
            }
        }
        assert_eq!(
            section.enabled && section.pop_out,
            up_section.pop_out,
            "{}: the \"^\" button",
            section.id
        );
    }
    assert!(compared > 20, "only {compared} widths compared");
    // both PoB's fixed widths and the section-wide fallback are in the attack build
    assert!(seen.contains(&95) && seen.contains(&114), "{seen:?}");
    assert!(seen.contains(&94), "no fallback width: {seen:?}");
    // the selector section has controls and no pop-out
    let select = view
        .sections
        .iter()
        .find(|s| s.id == "SkillSelect")
        .unwrap();
    assert!(!select.pop_out);
    assert_eq!(up.popups(), 0);
    let popups: usize = ours.eval("return #pob.main.main.popups");
    assert_eq!(popups, 0);
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_power_match_pob() {
    let _guard = lock();
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "attack");
    up.load(&xml, "attack");
    let written = ours.files();

    // ---- Custom heat map depth (NF-6): a depth PoB's list doesn't have ----
    ours.run(Command::SetHeatmap {
        stat: Some("Life".into()),
        max_depth: Some(7),
    })
    .expect("heat map");
    let mut steps = 0;
    while let Some(task) = ours.s.backend.next_background() {
        ours.s
            .backend
            .background_step(task, &|_| {})
            .expect("power step");
        steps += 1;
        assert!(steps < 100_000, "the power calculation never ends");
    }
    let depth: Option<u32> = up.call("upPowerCustom", ("Life", "7"));
    assert_eq!(depth, Some(7));
    let ours_depth: Option<u32> = ours.eval("return pob.build.calcsTab.nodePowerMaxDepth");
    assert_eq!(ours_depth, depth);
    assert_eq!(ours.s.backend.tree_state_view().unwrap().max_depth, Some(7));

    // ---- The report's filter and Show Masteries (NF-5) ----
    let listed = |ours: &mut Ours| -> (Vec<String>, auto_pob::engine::PowerReport) {
        let report = ours.s.backend.power_report_view().expect("report");
        let mut rows: Vec<String> = report
            .listed
            .iter()
            .map(|&i| format!("{} {}", report.rows[i].id, report.rows[i].name))
            .collect();
        rows.sort();
        (rows, report)
    };
    let (rows, report) = listed(&mut ours);
    assert_eq!((report.filter, report.masteries), (0, true));
    let want: Vec<String> = up.get("upReportListed", ());
    assert_eq!(rows, want, "Show Unallocated");
    assert!(
        rows.len() < report.rows.len(),
        "PoB's list drops rows of no power: {} of {}",
        rows.len(),
        report.rows.len()
    );
    assert!(report.rows.iter().any(|r| r.kind == "Mastery"));
    for (filter, masteries, what) in [
        (2, true, "Show Allocated"),
        (1, true, "Show Unallocated & Clusters"),
        (1, false, "no masteries"),
    ] {
        ours.run(Command::SetPowerReportFilter { filter, masteries })
            .expect("filter");
        up.call::<()>("upReportFilter", filter);
        up.call::<()>("upReportMasteries", masteries);
        let (rows, report) = listed(&mut ours);
        assert_eq!((report.filter, report.masteries), (filter, masteries));
        let want: Vec<String> = up.get("upReportListed", ());
        assert_eq!(rows, want, "{what}");
        assert!(!rows.is_empty(), "{what}: nothing listed");
        if !masteries {
            assert!(
                report
                    .listed
                    .iter()
                    .all(|&i| report.rows[i].kind != "Mastery"),
                "{what}"
            );
        }
    }
    let err = ours
        .run(Command::SetPowerReportFilter {
            filter: 3,
            masteries: true,
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "no such filter");
    let popups: usize = ours.eval("return #pob.main.main.popups");
    assert_eq!(popups, 0);
    assert_eq!(up.popups(), 0);
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
}

#[derive(Debug, PartialEq, serde::Deserialize)]
struct UpChoice {
    label: String,
    value: String,
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_version_import_notes_match_pob() {
    let _guard = lock();
    let xml = sets_xml();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "tabs");
    up.load(&xml, "tabs");
    let written = ours.files();

    // ---- Version dropdown (NF-7): PoB's list, and a conversion to an older version ----
    let state = ours.s.backend.tree_state_view().unwrap();
    let want: Vec<UpChoice> = up.get("upVersions", ());
    assert!(want.len() > 3, "{want:?}");
    let ours_versions: Vec<(String, String)> = state
        .versions
        .iter()
        .map(|c| (c.label.clone(), c.value.clone()))
        .collect();
    let up_versions: Vec<(String, String)> = want.into_iter().map(|c| (c.label, c.value)).collect();
    assert_eq!(ours_versions, up_versions);
    let older = up_versions
        .iter()
        .rev()
        .find(|(_, v)| v.starts_with("3_2") && !v.contains("ruthless") && *v != state.version)
        .expect("an older version")
        .1
        .clone();
    ours.run(Command::ConvertSpec {
        version: older.clone(),
        keep_copy: true,
    })
    .expect("converting");
    up.call::<()>("upConvertVersion", (older.as_str(), "Copy + Convert"));
    let converted = same(&ours, &up, "Version pick, Copy + Convert");
    assert_eq!(converted.specs.len(), 3);
    assert_eq!(converted.specs[1].version, older);
    same_stats(&mut ours, &up, "converted");
    // PoB's "Convert to <latest>" under the older tree, and its "Tree Converted" message
    let events = std::cell::RefCell::new(Vec::new());
    let affected = ours
        .s
        .backend
        .apply(Command::ConvertSpecLatest, &|e| events.borrow_mut().push(e))
        .expect("converting to the latest");
    if affected.recalc {
        ours.s.backend.recalculate().unwrap();
    }
    let want: BTreeMap<String, String> = up.get("upConvertLatest", ());
    let messages: Vec<(String, String)> = events
        .into_inner()
        .into_iter()
        .filter_map(|e| match e {
            auto_pob::engine::Event::Message { title, text } => Some((title, text)),
            _ => None,
        })
        .collect();
    assert_eq!(messages, [(want["title"].clone(), want["text"].clone())]);
    assert_eq!(want["title"], "Tree Converted");
    assert!(want["text"].contains("has been converted to"), "{want:?}");
    let latest = same(&ours, &up, "Convert to the latest");
    assert_eq!(latest.specs.len(), 4);
    assert_eq!(latest.specs[2].version, state.version);
    let popups: usize = ours.eval("return #pob.main.main.popups");
    assert_eq!(popups, 0);
    // the newest tree isn't offered the button
    let err = ours.run(Command::ConvertSpecLatest).unwrap_err();
    assert_eq!(err.to_string(), "PoB doesn't offer that here");

    // ---- Import: the name box's label and Start (controller ruling), PoB's notes ----
    for text in [
        "",
        "Name",
        "Name#123",
        "Name#1234",
        "Name-1234",
        "#1234",
        " #1234",
        "Name#1234 ",
    ] {
        let want: (bool, bool) = up.get("upAccountName", text);
        assert_eq!(
            auto_pob::engine::views::account_name_state(text),
            want,
            "{text:?}"
        );
    }
    let account = ours.s.backend.account_view().expect("account view");
    let label: String = up
        .lua
        .load("return StripEscapes(build.importTab.controls.siteAccountNameMissingDiscriminator.label)")
        .eval()
        .unwrap();
    assert_eq!(account.public.missing_discriminator, label);
    assert!(label.starts_with("Missing discriminator"));
    let note: String = up
        .lua
        .load("return StripEscapes(build.importTab.controls.siteCharBanditNote.label)")
        .eval()
        .unwrap();
    assert_eq!(account.public.bandit_note, note);
    assert!(note.contains("bandit"), "{note}");

    // ---- The account history and its X (NF-8) ----
    for name in ["Zed#0001", "alice#1234", "Bob#9999"] {
        ours.eval::<()>(&format!("saveAccountHistory(pob.build, {name:?})"));
        up.lua
            .load(format!("saveAccountHistory(build, {name:?})"))
            .exec()
            .unwrap();
    }
    let history: Vec<String> = up.get("accountHistory", ());
    assert_eq!(history, ["alice#1234", "Bob#9999", "Zed#0001"]);
    let view = ours.s.backend.account_view().unwrap();
    assert_eq!(view.public.history, history);
    // PoB's dropdown selects the last account saved; X works on it before any pick
    let selected: Option<String> = up.get("accountSelected", ());
    assert_eq!(selected.as_deref(), Some("Bob#9999"));
    assert_eq!(view.public.history_selected, selected);
    ours.run(Command::RemoveAccountHistory {
        account: "Bob#9999".into(),
    })
    .expect("removing");
    up.call::<()>("upRemoveAccount", "Bob#9999");
    let history: Vec<String> = up.get("accountHistory", ());
    assert_eq!(history, ["alice#1234", "Zed#0001"]);
    let view = ours.s.backend.account_view().unwrap();
    assert_eq!(view.public.history, history);
    // the pick put the name in PoB's box, on both sides
    assert_eq!(view.public.account, "Bob#9999");
    let err = ours
        .run(Command::RemoveAccountHistory {
            account: "Bob#9999".into(),
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "no such account in the history");

    // ---- The notes' zoom: our sizes are PoB's ZoomText steps ----
    let keys = "+++-0--------0";
    let want: Vec<u32> = up.get("upNotesZoom", keys);
    let mut height = auto_pob::ui::notes::NOTES_LINE_HEIGHT;
    let ours_heights: Vec<u32> = keys
        .chars()
        .map(|k| {
            height = auto_pob::ui::notes::zoom_line_height(
                height,
                k,
                auto_pob::ui::notes::NOTES_LINE_HEIGHT,
            );
            height
        })
        .collect();
    assert_eq!(ours_heights, want);
    assert!(want.contains(&10), "the floor is reached: {want:?}");
    let popups: usize = ours.eval("return #pob.main.main.popups");
    assert_eq!(popups, 0);
    assert_eq!(up.popups(), 0);
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tab_gaps_class_change_match_pob() {
    let _guard = lock();
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let up = Upstream::new();
    let mut ours = Ours::new();
    ours.open(&xml, "attack");
    up.load(&xml, "attack");
    let written = ours.files();
    let start = same(&ours, &up, "opened");
    assert_eq!(start.specs[0].class_id, 1, "a Marauder");

    // a Necromancer node: the Marauder tree doesn't reach the Witch, so PoB asks
    let necro: u32 = up.call("ascendancyNode", "Necromancer");
    let ours_necro: u32 = ours.eval("return ascendancyNode('Necromancer', pob.build)");
    assert_eq!(ours_necro, necro);
    let class = {
        let state = ours.s.backend.tree_state_view().unwrap();
        state
            .classes
            .into_iter()
            .find(|c| c.name == "Witch")
            .unwrap()
    };
    assert!(class.needs_confirm, "our view doesn't ask for the Witch");
    let title: Option<String> = up.call("upClickNode", (necro, "Connect Path"));
    assert_eq!(title.as_deref(), Some("Class Change"));
    ours.run(Command::AllocNodeChangingClass {
        id: necro,
        connect: true,
    })
    .expect("Connect Path");
    let connected = same(&ours, &up, "Class Change, Connect Path");
    assert_eq!(connected.specs[0].class_id, 3, "a Witch");
    assert!(connected.specs[0].ascend_id != 0);
    assert!(connected.specs[0].alloc.contains(&necro));
    assert!(
        connected.specs[0].alloc.len() > start.specs[0].alloc.len(),
        "the path to the Witch was added"
    );
    same_stats(&mut ours, &up, "connected");

    // a Deadeye node from the Witch tree: Continue resets the tree
    let deadeye: u32 = up.call("ascendancyNode", "Deadeye");
    let title: Option<String> = up.call("upClickNode", (deadeye, "Continue"));
    assert_eq!(title.as_deref(), Some("Class Change"));
    ours.run(Command::AllocNodeChangingClass {
        id: deadeye,
        connect: false,
    })
    .expect("Continue");
    let reset = same(&ours, &up, "Class Change, Continue");
    assert!(reset.specs[0].alloc.contains(&deadeye));
    assert!(reset.specs[0].alloc.len() < 5, "{:?}", reset.specs[0].alloc);
    same_stats(&mut ours, &up, "reset");

    // with nothing left to lose PoB doesn't ask: the plain click switches
    let juggernaut: u32 = up.call("ascendancyNode", "Juggernaut");
    let err = ours
        .run(Command::AllocNodeChangingClass {
            id: juggernaut,
            connect: false,
        })
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "PoB doesn't ask here: the class changes with the click"
    );
    let title: Option<String> = up.call("upClickNode", (juggernaut, "Continue"));
    assert_eq!(title, None);
    ours.run(Command::AllocNode { id: juggernaut })
        .expect("the plain click");
    same(&ours, &up, "a click that needs no question");
    // nothing the steps did wrote a file
    assert_eq!(ours.files(), written, "files written");
}
