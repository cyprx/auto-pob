//! The Compare tab (R4-R8): comparison builds, PoB's selectors, the Summary and the tree,
//! against upstream PoB pressing the same controls and drawing whole frames.

use super::net_harness::{NetSession, canned, canned_web, net_session};
use super::{assert_sidebar, fixture_xml, fixtures_dir, golden, lock, pob_dir};
use auto_pob::engine::upstream::{load_xml, upstream_lua};
use auto_pob::engine::{
    Backend, Command, CompareColour, CompareRow, CompareSource, CompareView, PopupInput,
};
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// R6's top-bar selectors: shown in every sub-view once a comparison is loaded
const TOP_BAR: [&str; 11] = [
    "compareSpecSelect",
    "compareSkillSetSelect",
    "compareItemSetSelect",
    "compareConfigSetSelect",
    "cmpSocketGroup",
    "cmpMainSkill",
    "cmpSkillPart",
    "cmpStageCount",
    "cmpMineCount",
    "cmpMinion",
    "cmpMinionSkill",
];

/// R6's controls of one sub-view, with the view PoB shows them in
const VIEW_CONTROLS: [(&str, &[&str]); 2] = [
    (
        "ITEMS",
        &[
            "primaryItemSetSelect",
            "compareItemSetSelect2",
            "itemsExpandedCheck",
        ],
    ),
    ("TREE", &["treeOverlayCheck"]),
];

/// Upstream PoB with `compare_lib.lua`, the build `name` loaded
struct Upstream {
    lua: mlua::Lua,
}

impl Upstream {
    fn new(name: &str) -> Upstream {
        let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
        // the wrapper runs in dev mode, which writes a build to its file when it is left
        lua.load("launch.devMode = false").exec().unwrap();
        lua.load(fs::read_to_string(fixtures_dir().join("compare_lib.lua")).unwrap())
            .set_name("=compare_lib")
            .exec()
            .expect("compare_lib.lua");
        let up = Upstream { lua };
        up.load(name);
        up
    }

    /// Loads the fixture as a saved build is opened: with nothing unsaved (the wrapper's load
    /// leaves `modFlag` set)
    fn load(&self, name: &str) {
        load_xml(
            &self.lua,
            &fs::read_to_string(fixture_xml(name)).unwrap(),
            name,
        )
        .unwrap();
        self.eval::<()>("build.modFlag = false runCallback('OnFrame')");
        assert!(!self.eval::<bool>("return build.unsaved and true or false"));
    }

    fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.lua
            .load(code)
            .set_name("=upstream compare")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    fn call<T: mlua::FromLuaMulti>(&self, function: &str, args: impl mlua::IntoLuaMulti) -> T {
        let f: mlua::Function = self.lua.globals().get(function).unwrap();
        f.call(args).unwrap_or_else(|e| panic!("{function}: {e}"))
    }

    fn json<T: serde::de::DeserializeOwned>(&self, code: &str) -> T {
        let value: mlua::Value = self.eval(code);
        self.lua.from_value(value).unwrap()
    }

    /// The Summary a whole frame draws, as rows like ours
    fn summary(&self) -> (Vec<CompareRow>, [String; 2]) {
        #[derive(serde::Deserialize)]
        struct Raw {
            headers: [String; 2],
            rows: Vec<Vec<String>>,
            positive: String,
            negative: String,
        }
        let raw: Raw = self.json("return summaryFrame()");
        let rows = raw
            .rows
            .into_iter()
            .map(|texts| {
                let diff = texts.get(3).cloned().unwrap_or_default();
                let better = if diff.starts_with(&raw.positive) {
                    Some(true)
                } else if diff.starts_with(&raw.negative) {
                    Some(false)
                } else {
                    None
                };
                CompareRow {
                    label: texts[0].clone(),
                    ours: texts[1].clone(),
                    theirs: texts[2].clone(),
                    diff,
                    better,
                }
            })
            .collect();
        (rows, raw.headers)
    }

    fn entries(&self) -> (Vec<String>, usize) {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(default)]
            labels: Vec<String>,
            active: usize,
        }
        let raw: Raw = self.json("return entries()");
        (raw.labels, raw.active)
    }

    fn selectors(&self) -> Vec<Selector> {
        let names = self.lua.to_value(&TOP_BAR).unwrap();
        let f: mlua::Function = self.lua.globals().get("selectors").unwrap();
        self.lua.from_value(f.call(names).unwrap()).unwrap()
    }

    /// The Items and Tree views' controls, after a frame on each
    fn view_selectors(&self) -> Vec<Selector> {
        let mut out = Vec::new();
        for (mode, names) in VIEW_CONTROLS {
            let names = self.lua.to_value(&names).unwrap();
            let f: mlua::Function = self.lua.globals().get("selectorsIn").unwrap();
            let got: Vec<Selector> = self.lua.from_value(f.call((mode, names)).unwrap()).unwrap();
            out.extend(got);
        }
        out
    }

    fn tree_colours(&self) -> Vec<CompareColour> {
        #[derive(serde::Deserialize)]
        struct Raw {
            node: u32,
            rgb: [f32; 3],
        }
        let raw: Vec<Raw> = self.json("return treeFrame()");
        raw.into_iter()
            .map(|r| CompareColour {
                node: r.node,
                rgb: r.rgb,
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, serde::Deserialize)]
struct Selector {
    name: String,
    shown: bool,
    enabled: bool,
    #[serde(default)]
    choices: Vec<String>,
    selected: usize,
}

fn our_selectors(view: &CompareView) -> Vec<Selector> {
    selectors_of(view, &TOP_BAR)
}

fn selectors_of(view: &CompareView, names: &[&str]) -> Vec<Selector> {
    names
        .iter()
        .map(|name| {
            let c = view
                .controls
                .iter()
                .find(|c| c.name == *name)
                .unwrap_or_else(|| panic!("no control {name}"));
            Selector {
                name: c.name.clone(),
                shown: c.shown,
                enabled: c.enabled,
                choices: c.choices.clone(),
                selected: c.selected,
            }
        })
        .collect()
}

/// A temp folder for files the test hands PoB; removed on drop
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("auto-pob-cmp-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Every file under `root` with its bytes
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

fn view(s: &mut NetSession) -> CompareView {
    s.backend
        .compare_view()
        .expect("compare view")
        .expect("a comparison is loaded")
}

/// Applies a command as the loop does (a recalculation when asked), and checks no popup stayed
fn run(s: &mut NetSession, command: Command) -> anyhow::Result<auto_pob::engine::Affected> {
    let shown = format!("{command:?}");
    let result = s.backend.apply(command, &|_| {});
    if let Ok(affected) = &result
        && affected.recalc
    {
        s.backend.recalculate().unwrap();
    }
    let popups: usize = s.eval("return #pob.main.main.popups");
    assert_eq!(popups, 0, "{shown} left a popup");
    result
}

fn err(s: &mut NetSession, command: Command) -> String {
    format!("{:#}", run(s, command).unwrap_err())
}

fn load_file(s: &mut NetSession, path: &Path) {
    let affected = run(
        s,
        Command::LoadCompare {
            source: CompareSource::File(path.into()),
        },
    )
    .expect("load comparison");
    assert!(affected.compare && !affected.build && !affected.recalc);
}

fn choice(view: &CompareView, name: &str, index: usize) -> Command {
    let control = view.controls.iter().find(|c| c.name == name).unwrap();
    Command::SetCompareControl {
        name: name.into(),
        value: PopupInput::Choice {
            index,
            label: control.choices[index].clone(),
        },
    }
}

/// Rows and headers ours and upstream's agree on, at `what`
fn same_summary(s: &mut NetSession, up: &Upstream, what: &str) -> Vec<CompareRow> {
    let ours = view(s);
    let (rows, headers) = up.summary();
    assert!(!rows.is_empty(), "{what}: upstream drew no rows");
    assert_eq!(ours.summary.len(), rows.len(), "{what}: row count");
    for (i, (a, b)) in ours.summary.iter().zip(&rows).enumerate() {
        assert_eq!(a, b, "{what}: row {i}");
    }
    assert_eq!(ours.headers, headers, "{what}: headers");
    assert_eq!(our_selectors(&ours), up.selectors(), "{what}: selectors");
    let (labels, active) = up.entries();
    assert_eq!(
        (ours.entries.clone(), ours.active),
        (labels, active),
        "{what}"
    );
    ours.summary
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn compare_summary_matches_pob() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    let mut s = net_session("compare-summary", canned(&web), None);
    let scratch = Scratch::new("summary");
    s.open_fixture("attack");
    let up = Upstream::new("attack");
    let refused = s.backend.net_handle().refused();

    // ---- Nothing loaded: no view, and the commands say so ----
    assert_eq!(s.backend.compare_view().unwrap(), None);
    assert_eq!(
        err(&mut s, Command::SelectCompare { index: 0 }),
        "no comparison build loaded"
    );
    assert_eq!(
        err(
            &mut s,
            Command::SetCompareControl {
                name: "cmpMainSkill".into(),
                value: PopupInput::Check(true),
            }
        ),
        "no comparison build loaded"
    );
    let before = files(&s.user);

    // ---- (attack, dot) from a file, named after it ----
    let dot = fixture_xml("dot");
    load_file(&mut s, &dot);
    up.call::<()>(
        "loadCompare",
        (fs::read_to_string(&dot).unwrap(), "dot".to_string()),
    );
    let rows = same_summary(&mut s, &up, "attack, dot");
    assert_eq!(view(&mut s).entries, ["dot"]);
    // the sub-views' own controls, as PoB shows them there
    let in_views: Vec<&str> = VIEW_CONTROLS
        .iter()
        .flat_map(|(_, names)| *names)
        .copied()
        .collect();
    let theirs = up.view_selectors();
    assert_eq!(selectors_of(&view(&mut s), &in_views), theirs);
    assert!(theirs.iter().all(|c| c.shown && c.enabled));
    assert_eq!(theirs[0].choices, ["Default"]);
    // every kind of diff shows up
    assert!(rows.iter().any(|r| r.better == Some(true)));
    assert!(rows.iter().any(|r| r.better == Some(false)));
    assert!(rows.iter().any(|r| r.better.is_none() && r.diff.is_empty()));
    assert!(rows.iter().all(|r| r.diff.is_empty() == r.better.is_none()));
    // this build is untouched
    let build = s.backend.build_view().unwrap();
    assert!(!build.unsaved);
    assert_sidebar(
        "attack",
        "with a comparison",
        &s.backend.stats_view().unwrap().stats,
        &golden("attack").sidebar,
    );

    // ---- The same build from its share code ----
    let code: String = up.call("shareCode", fs::read_to_string(&dot).unwrap());
    run(
        &mut s,
        Command::LoadCompare {
            source: CompareSource::Code(code.clone()),
        },
    )
    .expect("load code");
    up.call::<()>("loadCode", code);
    let from_code = same_summary(&mut s, &up, "from a code");
    assert_eq!(from_code, rows, "a code gives what the file gave");
    assert_eq!(view(&mut s).entries, ["dot", "Imported build"]);
    assert_eq!(view(&mut s).active, 1);

    // ---- A bad code is PoB's failure, and changes nothing ----
    assert_eq!(
        err(
            &mut s,
            Command::LoadCompare {
                source: CompareSource::Code("not a build".into()),
            }
        ),
        "Failed to import build for comparison."
    );
    assert!(!up.eval::<bool>("return build.compareTab:ImportFromCode('not a build')"));
    let missing = scratch.0.join("missing.xml");
    assert_eq!(
        err(
            &mut s,
            Command::LoadCompare {
                source: CompareSource::File(missing),
            }
        ),
        "Couldn't open 'missing.xml'."
    );
    assert_eq!(view(&mut s).entries.len(), 2);

    // ---- Selecting and removing follow PoB's activeCompareIndex ----
    assert_eq!(
        err(&mut s, Command::SelectCompare { index: 2 }),
        "no such comparison"
    );
    let minion = fixture_xml("minion");
    load_file(&mut s, &minion);
    up.call::<()>(
        "loadCompare",
        (fs::read_to_string(&minion).unwrap(), "minion".to_string()),
    );
    same_summary(&mut s, &up, "a third");
    assert_eq!(view(&mut s).active, 2);
    run(&mut s, Command::SelectCompare { index: 0 }).unwrap();
    up.call::<()>("pressSelect", 1);
    same_summary(&mut s, &up, "selected the first");
    assert_eq!(view(&mut s).active, 0);
    // removing an entry that isn't the active one picks it first, as PoB's Remove only
    // removes the active one: the next one becomes active, not the one that was
    run(&mut s, Command::SelectCompare { index: 2 }).unwrap();
    up.call::<()>("pressSelect", 3);
    run(&mut s, Command::RemoveCompare { index: 0 }).unwrap();
    up.call::<()>("pressRemove", 1);
    same_summary(&mut s, &up, "removed the first");
    assert_eq!(view(&mut s).entries, ["Imported build", "minion"]);
    assert_eq!(view(&mut s).active, 0);

    // ---- Controls PoB doesn't show or doesn't have ----
    assert_eq!(
        err(
            &mut s,
            Command::SetCompareControl {
                name: "cmpSkillPart".into(),
                value: PopupInput::Choice {
                    index: 0,
                    label: String::new(),
                },
            }
        ),
        "PoB doesn't offer that here"
    );
    assert_eq!(
        err(
            &mut s,
            Command::SetCompareControl {
                name: "removeBtn".into(),
                value: PopupInput::Check(true),
            }
        ),
        "no such control"
    );
    assert_eq!(files(&s.user), before, "a file was written");

    // ---- (minion, variant): the minion header, and the selectors change the rows ----
    let variant = scratch.0.join("variant.xml");
    let variant_xml: String = {
        let maker = Upstream::new("attack");
        maker.eval("return makeVariant()")
    };
    fs::write(&variant, &variant_xml).unwrap();
    s.open_fixture("minion");
    // a build change drops the comparisons
    assert_eq!(s.backend.compare_view().unwrap(), None);
    let before = files(&s.user);
    up.load("minion");
    load_file(&mut s, &variant);
    up.call::<()>("loadCompare", (variant_xml, "variant".to_string()));
    let plain = same_summary(&mut s, &up, "minion, variant");
    assert_eq!(view(&mut s).headers[1], "^xFF9922variant");
    // the minion's stats are diffed: the first row is the minion's damage
    assert!(
        plain[0].ours != plain[0].theirs,
        "the rows don't differ: {plain:?}"
    );

    let set = choice(&view(&mut s), "compareSkillSetSelect", 1);
    run(&mut s, set).unwrap();
    up.call::<()>("pressChoice", ("compareSkillSetSelect", 2));
    let second_set = same_summary(&mut s, &up, "skill set 2");
    assert_ne!(second_set, plain, "the skill set changed nothing");
    let ours = view(&mut s);
    let main_skill = ours
        .controls
        .iter()
        .find(|c| c.name == "cmpMainSkill")
        .unwrap();
    assert!(main_skill.shown && main_skill.enabled && main_skill.choices.len() == 2);
    assert_eq!(main_skill.selected, 0);
    let pick = choice(&ours, "cmpMainSkill", 1);
    run(&mut s, pick).unwrap();
    up.call::<()>("pressChoice", ("cmpMainSkill", 2));
    let second_skill = same_summary(&mut s, &up, "main skill 2");
    assert_ne!(second_skill, second_set, "the main skill changed nothing");
    assert_eq!(
        our_selectors(&view(&mut s))
            .iter()
            .find(|c| c.name == "cmpMainSkill")
            .unwrap()
            .selected,
        1
    );
    // a pick against a list that changed is refused
    let stale = Command::SetCompareControl {
        name: "cmpMainSkill".into(),
        value: PopupInput::Choice {
            index: 0,
            label: "something else".into(),
        },
    };
    assert_eq!(err(&mut s, stale), "the list changed; pick again");
    assert_eq!(files(&s.user), before, "a file was written");

    // ---- No request, no launch ----
    assert_eq!(
        s.backend.net_handle().refused(),
        refused,
        "a launch was tried"
    );
    assert!(s.backend.net_handle().running_ids().is_empty());
    assert!(web.requests().is_empty());
    assert!(web.unrouted().is_empty());
    let secs = started.elapsed().as_secs_f64();
    assert!(secs < 25.0, "took {secs:.1}s");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn compare_tree_colours_match_pob() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    let mut s = net_session("compare-tree", canned(&web), None);
    s.open_fixture("attack");
    let up = Upstream::new("attack");
    let dot = fixture_xml("dot");
    load_file(&mut s, &dot);
    up.call::<()>(
        "loadCompare",
        (fs::read_to_string(&dot).unwrap(), "dot".to_string()),
    );
    // not on the Compare tab's tree: the Tree tab's own state (not comparing)
    let tree = s.backend.tree_state_view().unwrap();
    assert!(tree.compare.is_empty() && tree.compare_spec.is_none());

    // ---- The overlay: PoB's colours against the comparison's spec ----
    let affected = run(&mut s, Command::SetCompareOverlay { on: true }).unwrap();
    assert!(affected.tree && !affected.build && !affected.recalc);
    let tree = s.backend.tree_state_view().unwrap();
    let theirs = up.tree_colours();
    assert!(
        theirs.iter().any(|c| c.rgb == [0.0, 1.0, 0.0])
            && theirs.iter().any(|c| c.rgb == [1.0, 0.0, 0.0]),
        "the overlay has no added or removed nodes"
    );
    assert_eq!(tree.compare, theirs);
    assert_eq!(tree.compare_spec, None);
    // PoB clears the overlay's spec once drawn
    assert!(s.eval::<bool>("return pob.build.treeTab.viewer.compareSpec == nil"));
    assert!(!s.backend.build_view().unwrap().unsaved);

    // ---- Side by side: PoB's unchecked overlay colours nothing ----
    run(
        &mut s,
        Command::SetCompareControl {
            name: "treeOverlayCheck".into(),
            value: PopupInput::Check(false),
        },
    )
    .unwrap();
    up.call::<()>("pressCheck", ("treeOverlayCheck", false));
    assert_eq!(up.tree_colours(), []);
    assert_eq!(s.backend.tree_state_view().unwrap().compare, []);
    run(
        &mut s,
        Command::SetCompareControl {
            name: "treeOverlayCheck".into(),
            value: PopupInput::Check(true),
        },
    )
    .unwrap();
    up.call::<()>("pressCheck", ("treeOverlayCheck", true));
    assert_eq!(s.backend.tree_state_view().unwrap().compare, theirs);

    // ---- Copy and use: PoB's spec list, titles and nodes ----
    let affected = run(&mut s, Command::CopyCompareTree { and_use: true }).unwrap();
    assert!(affected.tree && affected.build && affected.compare);
    up.call::<()>("pressCopy", true);
    let tree = s.backend.tree_state_view().unwrap();
    #[derive(serde::Deserialize)]
    struct Specs {
        titles: Vec<String>,
        active: usize,
        #[serde(default)]
        allocated: Vec<u32>,
        unsaved: bool,
    }
    let specs: Specs = up.json(
        r#"local titles, allocated = {}, {}
        for i, spec in ipairs(build.treeTab.specList) do titles[i] = spec.title or "Default" end
        for id in pairs(build.spec.allocNodes) do allocated[#allocated + 1] = id end
        table.sort(allocated)
        return { titles = titles, active = build.treeTab.activeSpec - 1, allocated = allocated,
          unsaved = build.unsaved and true or false }"#,
    );
    let titles: Vec<String> = tree.specs.iter().map(|spec| spec.title.clone()).collect();
    assert_eq!(titles, ["Default", "Default (Compared)"]);
    assert_eq!(titles, specs.titles);
    assert_eq!(tree.active_spec, 1);
    assert_eq!(tree.active_spec, specs.active);
    assert!(!tree.allocated.is_empty());
    assert_eq!(tree.allocated, specs.allocated);
    // PoB's copy only asks for a recalculation: it leaves the build saved (Deviation)
    assert_eq!(s.backend.build_view().unwrap().unsaved, specs.unsaved);
    // the copied tree against the comparison's
    assert_eq!(
        s.backend.tree_state_view().unwrap().compare,
        up.tree_colours()
    );

    // ---- The Tree tab's own compare once the overlay is off ----
    run(&mut s, Command::SetCompareSpec { index: Some(0) }).unwrap();
    let tree = s.backend.tree_state_view().unwrap();
    assert_eq!(tree.compare_spec, None, "the overlay goes first");
    run(&mut s, Command::SetCompareOverlay { on: false }).unwrap();
    let tree = s.backend.tree_state_view().unwrap();
    assert_eq!(tree.compare_spec, Some(0));
    assert!(!tree.compare.is_empty());
    assert_ne!(tree.compare, theirs);

    assert!(web.requests().is_empty());
    let secs = started.elapsed().as_secs_f64();
    assert!(secs < 20.0, "took {secs:.1}s");
}
