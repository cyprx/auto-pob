//! The Compare tab (R4-R8): comparison builds, PoB's selectors, the Summary and the tree,
//! against upstream PoB pressing the same controls and drawing whole frames.

use super::net_harness::{NetSession, canned, canned_web, net_session};
use super::{assert_sidebar, fixture_xml, fixtures_dir, golden, lock, pob_dir};
use auto_pob::engine::upstream::{load_xml, upstream_lua};
use auto_pob::engine::{
    Backend, CardLine, Command, CompareColour, CompareConfigRow, CompareItemRow, CompareRow,
    CompareSkillRow, CompareSource, CompareView, ConfigValue, PopupControl, PopupControlKind,
    PopupInput,
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
const VIEW_CONTROLS: [(&str, &[&str]); 3] = [
    (
        "ITEMS",
        &[
            "primaryItemSetSelect",
            "compareItemSetSelect2",
            "itemsExpandedCheck",
        ],
    ),
    (
        "TREE",
        &[
            "treeOverlayCheck",
            "leftVersionSelect",
            "rightVersionSelect",
        ],
    ),
    ("CONFIG", &["configToggleBtn"]),
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
    let empty = scratch.0.join("empty.xml");
    fs::write(&empty, "").unwrap();
    assert_eq!(
        err(
            &mut s,
            Command::LoadCompare {
                source: CompareSource::File(empty),
            }
        ),
        "Build file is empty or unreadable."
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

    // ---- The count boxes: typed as PoB's edit boxes take text ----
    let set = choice(&view(&mut s), "compareSkillSetSelect", 0);
    run(&mut s, set).unwrap();
    up.call::<()>("pressChoice", ("compareSkillSetSelect", 1));
    for (group, boxed, typed) in [(1, "cmpStageCount", "5"), (2, "cmpMineCount", "3")] {
        let pick = choice(&view(&mut s), "cmpSocketGroup", group);
        run(&mut s, pick).unwrap();
        up.call::<()>("pressChoice", ("cmpSocketGroup", group + 1));
        let before = same_summary(&mut s, &up, boxed);
        let control = view(&mut s)
            .controls
            .into_iter()
            .find(|c| c.name == boxed)
            .unwrap();
        assert!(control.shown && control.enabled, "{boxed} isn't offered");
        // a `%D` box, so drawn with PoB's + and - (EditControl.lua:64-75)
        assert_eq!(
            control.numeric,
            up.eval::<bool>(&format!(
                "return build.compareTab.controls.{boxed}.isNumeric == true"
            )),
            "{boxed}"
        );
        assert!(control.digits && control.numeric, "{boxed} is not a %D box");
        // PoB's filter drops what isn't a digit
        run(
            &mut s,
            Command::SetCompareControl {
                name: boxed.into(),
                value: PopupInput::Text(format!("x{typed}")),
            },
        )
        .unwrap();
        up.call::<()>("pressText", (boxed, typed));
        let after = same_summary(&mut s, &up, &format!("{boxed} typed"));
        assert_ne!(after, before, "{boxed} changed nothing");
        let control = view(&mut s)
            .controls
            .into_iter()
            .find(|c| c.name == boxed)
            .unwrap();
        assert_eq!(control.text, typed);
        assert_eq!(
            control.text,
            up.eval::<String>(&format!("return build.compareTab.controls.{boxed}.buf"))
        );
    }
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
    // on the Compare tab's tree before anything is loaded: nothing to colour against yet
    let affected = run(&mut s, Command::SetCompareOverlay { on: true }).unwrap();
    assert!(affected.tree && !affected.build && !affected.recalc);
    assert!(s.backend.tree_state_view().unwrap().compare.is_empty());
    let dot = fixture_xml("dot");
    load_file(&mut s, &dot);
    up.call::<()>(
        "loadCompare",
        (fs::read_to_string(&dot).unwrap(), "dot".to_string()),
    );
    let theirs = up.tree_colours();
    assert_eq!(s.backend.tree_state_view().unwrap().compare, theirs);
    // off the Compare tab's tree: the Tree tab's own state (not comparing)
    run(&mut s, Command::SetCompareOverlay { on: false }).unwrap();
    let tree = s.backend.tree_state_view().unwrap();
    assert!(tree.compare.is_empty() && tree.compare_spec.is_none());

    // ---- The overlay: PoB's colours against the comparison's spec ----
    let affected = run(&mut s, Command::SetCompareOverlay { on: true }).unwrap();
    assert!(affected.tree && !affected.build && !affected.recalc);
    let tree = s.backend.tree_state_view().unwrap();
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
    // the copied tree against the comparison's: the same nodes, so only sockets can differ
    // (PoB copies no jewels)
    let copied = s.backend.tree_state_view().unwrap().compare;
    assert_eq!(copied, up.tree_colours());
    assert!(
        copied.iter().all(|c| c.rgb == [0.0, 0.0, 1.0]),
        "{copied:?}"
    );
    // the original tree again: the colours from before the copy
    run(&mut s, Command::SelectSpec { index: 0 }).unwrap();
    up.eval::<()>("build.treeTab:SetActiveSpec(1) runCallback('OnFrame')");
    let original = s.backend.tree_state_view().unwrap().compare;
    assert_eq!(original, up.tree_colours());
    assert_eq!(original, theirs);
    run(&mut s, Command::SelectSpec { index: 1 }).unwrap();
    up.eval::<()>("build.treeTab:SetActiveSpec(2) runCallback('OnFrame')");

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
    assert!(web.unrouted().is_empty());
    let secs = started.elapsed().as_secs_f64();
    assert!(secs < 20.0, "took {secs:.1}s");
}

// ---- Items, skills and config (Task 8) ----

#[derive(Debug, PartialEq, serde::Deserialize)]
struct ItemState {
    raws: Vec<String>,
    #[serde(default)]
    slot: Option<String>,
    set: u32,
}

impl Upstream {
    /// The Items view's rows: slots and items from the frame's state, the rest from what the
    /// frame drew for each row (TradeHelpers.lua:461-530): its label, this build's name, PoB's
    /// marker and the comparison's name, then Buy, Copy and Equip (or "Missing slot") when
    /// the comparison has an item there
    fn items(&self) -> Vec<CompareItemRow> {
        #[derive(serde::Deserialize)]
        struct State {
            slot: String,
            label: String,
            #[serde(default)]
            ours: Option<u32>,
            differ: bool,
        }
        #[derive(serde::Deserialize)]
        struct Drawn {
            height: f64,
            text: String,
        }
        #[derive(serde::Deserialize)]
        struct Raw {
            rows: Vec<State>,
            drawn: Vec<Drawn>,
            warning: String,
        }
        let raw: Raw = self.json("return itemsFrame()");
        // a name as drawn: its rarity colour, the name or "(empty)", maybe the warning
        let name = |text: &str| -> (Option<String>, String, bool) {
            let (text, warned) = match text.strip_suffix(&raw.warning) {
                Some(rest) => (rest, true),
                None => (text, false),
            };
            let colour = if text.starts_with("^x") {
                8
            } else if text.starts_with('^') {
                2
            } else {
                0
            };
            let (colour, bare) = text.split_at(colour);
            (
                (bare != "(empty)").then(|| bare.to_string()),
                colour.to_string(),
                warned,
            )
        };
        let mut at = 0;
        raw.rows
            .into_iter()
            .map(|row| {
                let label = format!("^7{}:", row.label);
                at += raw.drawn[at..]
                    .iter()
                    .position(|d| d.text == label && d.height == 16.0)
                    .unwrap_or_else(|| panic!("{label} isn't drawn in order"));
                let texts: Vec<&str> = raw.drawn[at + 1..(at + 7).min(raw.drawn.len())]
                    .iter()
                    .map(|d| d.text.as_str())
                    .collect();
                assert_eq!(raw.drawn[at + 2].height, 14.0, "{label}: no marker");
                let (ours_name, ours_colour, ours_unallocated) = name(texts[0]);
                let (theirs, theirs_colour, theirs_unallocated) = name(texts[2]);
                let can_copy = texts.get(3) == Some(&"^7Buy") && texts.get(4) == Some(&"^7Copy");
                let can_equip = can_copy && texts.get(5) == Some(&"^7Equip");
                if can_copy && !can_equip {
                    assert_eq!(texts.get(5), Some(&"^xBBBBBBMissing slot"), "{label}");
                }
                at += 1;
                CompareItemRow {
                    slot: row.slot,
                    label: row.label,
                    ours: row.ours,
                    theirs,
                    differ: row.differ,
                    can_copy,
                    can_equip,
                    diff_label: texts[1].to_string(),
                    ours_unallocated,
                    theirs_unallocated,
                    ours_name,
                    ours_colour,
                    theirs_colour,
                    ours_card: vec![],
                    theirs_card: vec![],
                }
            })
            .collect()
    }

    /// The Skills view's pairs, parsed from what DrawSkills drew in a whole frame: a header
    /// (height 18) at the pair's `y` per side, the gem lines under it; the left column is this
    /// build's
    fn skills(&self) -> Vec<CompareSkillRow> {
        #[derive(serde::Deserialize)]
        struct Drawn {
            x: f64,
            y: f64,
            height: f64,
            text: String,
        }
        #[derive(serde::Deserialize)]
        struct Raw {
            strings: Vec<Drawn>,
            added: String,
            missing: String,
        }
        let raw: Raw = self.json("return skillsFrame()");
        // the names are drawn `colWidth` apart in the tab's coordinates; in the list's own the
        // comparison's column starts at `colWidth` (CompareTab.lua:4370-4371, 4446-4454)
        let column = raw.strings[1].x - raw.strings[0].x;
        let body = &raw.strings[2..];
        struct Pair {
            y: f64,
            ours_header: Option<String>,
            theirs_header: Option<String>,
            ours: Vec<String>,
            theirs: Vec<String>,
        }
        let mut pairs: Vec<Pair> = Vec::new();
        for d in body {
            let ours = d.x < column;
            if d.height == 18.0 {
                if pairs.last().is_none_or(|p| p.y != d.y) {
                    pairs.push(Pair {
                        y: d.y,
                        ours_header: None,
                        theirs_header: None,
                        ours: vec![],
                        theirs: vec![],
                    });
                }
                let pair = pairs.last_mut().unwrap();
                if ours {
                    pair.ours_header = Some(d.text.clone());
                } else {
                    pair.theirs_header = Some(d.text.clone());
                }
            } else {
                let pair = pairs.last_mut().expect("a gem line before any header");
                if ours {
                    pair.ours.push(d.text.clone());
                } else {
                    pair.theirs.push(d.text.clone());
                }
            }
        }
        pairs
            .into_iter()
            .map(|p| {
                let marked = p
                    .ours
                    .iter()
                    .chain(&p.theirs)
                    .any(|l| l.starts_with(&raw.added) || l.starts_with(&raw.missing));
                CompareSkillRow {
                    differ: marked || p.ours_header.is_none() || p.theirs_header.is_none(),
                    label: p.ours_header.or(p.theirs_header.clone()).unwrap(),
                    theirs_label: p.theirs_header,
                    ours: p.ours,
                    theirs: p.theirs,
                }
            })
            .collect()
    }

    fn config(&self) -> Vec<CompareConfigRow> {
        self.json("return configFrame()")
    }

    fn item_state(&self, slot: &str) -> ItemState {
        let f: mlua::Function = self.lua.globals().get("itemState").unwrap();
        self.lua.from_value(f.call(slot).unwrap()).unwrap()
    }

    fn config_input(&self) -> BTreeMap<String, String> {
        self.json("return configInput()")
    }

    fn config_boxes(&self) -> Vec<(String, String, String)> {
        self.json("return configBoxes()")
    }
}

fn our_item_state(s: &NetSession, slot: &str) -> ItemState {
    let f: mlua::Function = s
        .backend
        .lua
        .load(
            r#"return function(slot)
                local t = pob.build.itemsTab
                local raws = {}
                for i, id in ipairs(t.itemOrderList) do raws[i] = t.items[id]:BuildRaw() end
                local sel = t.items[t.slots[slot].selItemId]
                return { raws = raws, slot = sel and sel:BuildRaw() or nil, set = t.activeItemSetId }
            end"#,
        )
        .eval()
        .unwrap();
    s.backend.lua.from_value(f.call(slot).unwrap()).unwrap()
}

fn our_config_input(s: &NetSession) -> BTreeMap<String, String> {
    s.eval::<mlua::Table>(
        "local out = {} for k, v in pairs(pob.build.configTab.input) do out[k] = tostring(v) end return out",
    )
    .pairs::<String, String>()
    .map(|p| p.unwrap())
    .collect()
}

/// Both builds' trees (`specState` in compare_lib.lua)
#[derive(Debug, PartialEq, serde::Deserialize)]
struct SpecState {
    specs: Vec<(String, String)>,
    active: u32,
    ours: Vec<u32>,
    theirs_version: String,
    theirs: Vec<u32>,
}

impl Upstream {
    fn spec_state(&self) -> SpecState {
        self.json("return specState()")
    }
}

fn our_spec_state(s: &NetSession) -> SpecState {
    let table: mlua::Value = s.eval(
        r#"local build = pob.build
        local entry = build.compareTab:GetActiveCompare()
        local function nodes(spec)
            local out = {}
            for id in pairs(spec.allocNodes) do out[#out + 1] = id end
            table.sort(out)
            return out
        end
        local specs = {}
        for i, spec in ipairs(build.treeTab.specList) do
            specs[i] = { spec.title or "Default", spec.treeVersion }
        end
        return { specs = specs, active = build.treeTab.activeSpec, ours = nodes(build.spec),
            theirs_version = entry.spec.treeVersion, theirs = nodes(entry.spec) }"#,
    );
    s.backend.lua.from_value(table).unwrap()
}

/// Rows of all three views agree with upstream's, and the items are drawn in that order
fn same_views(s: &mut NetSession, up: &Upstream, what: &str) -> CompareView {
    let ours = view(s);
    let items = up.items();
    assert!(!items.is_empty(), "{what}: no item rows");
    assert_eq!(ours.items, items, "{what}: items");
    let skills = up.skills();
    assert!(!skills.is_empty(), "{what}: no skill rows");
    assert_eq!(ours.skills, skills, "{what}: skills");
    let rows: Vec<CompareConfigRow> = ours
        .config
        .iter()
        .map(|r| CompareConfigRow {
            ours_control: None,
            theirs_control: None,
            ..r.clone()
        })
        .collect();
    assert_eq!(rows, up.config(), "{what}: config");
    assert_eq!(
        config_boxes(&ours),
        up.config_boxes(),
        "{what}: config boxes"
    );
    ours
}

/// What each Config row's two boxes hold, as upstream's `configBoxes` reads PoB's controls
fn config_boxes(view: &CompareView) -> Vec<(String, String, String)> {
    let value = |c: &Option<PopupControl>| {
        let c = c.as_ref().expect("a box");
        match c.kind {
            PopupControlKind::Check => c.checked.to_string(),
            PopupControlKind::Edit => c.text.clone(),
            PopupControlKind::Dropdown => c.selected.to_string(),
            other => panic!("a {other:?} box"),
        }
    };
    view.config
        .iter()
        .map(|r| {
            (
                r.var.clone(),
                value(&r.ours_control),
                value(&r.theirs_control),
            )
        })
        .collect()
}

/// Opens a build file in the session's Builds folder, as the list does
fn open_xml(s: &mut NetSession, name: &str, xml: &str) {
    let dest = s.user.join(format!("Path of Building/Builds/{name}.xml"));
    fs::write(&dest, xml).unwrap();
    run(s, Command::OpenBuild { path: dest }).expect("open");
}

/// Their item's tooltip in `slot`: ours, and upstream's `AddItemTooltip` on the entry's tab
fn same_their_tooltip(s: &mut NetSession, up: &Upstream, slot: &str) {
    let ours = s
        .backend
        .item_tooltip_view(&auto_pob::engine::ItemTooltipQuery {
            id: 0,
            slot: Some(slot.into()),
            theirs: true,
            shift: false,
            shared: None,
        })
        .unwrap()
        .unwrap_or_else(|| panic!("their item in {slot}"));
    let theirs: auto_pob::engine::ItemTooltip = {
        let f: mlua::Function = up.lua.globals().get("theirTooltip").unwrap();
        up.lua.from_value(f.call(slot).unwrap()).unwrap()
    };
    assert!(!ours.tooltip.lines.is_empty(), "{slot}");
    assert_eq!(ours.tooltip, theirs, "{slot}");
}

/// Copies their item on both sides; the items, the slot and the views agree, the build is
/// unsaved and recalculated
fn copy_both(s: &mut NetSession, up: &Upstream, slot: &str, and_use: bool) -> ItemState {
    let start = our_item_state(s, slot);
    assert_eq!(start, up.item_state(slot), "{slot}: before");
    let affected = run(
        s,
        Command::CopyCompareItem {
            slot: slot.into(),
            and_use,
        },
    )
    .unwrap();
    assert!(affected.items && affected.build && affected.recalc && affected.compare);
    up.call::<()>("copyItem", (slot, and_use));
    let copied = our_item_state(s, slot);
    assert_ne!(copied, start, "{slot}: nothing copied");
    assert_eq!(copied, up.item_state(slot), "{slot}: after");
    assert_eq!(
        copied.slot != start.slot,
        and_use,
        "{slot}: equipped or not"
    );
    assert!(
        s.backend.build_view().unwrap().unsaved,
        "{slot}: not unsaved"
    );
    assert!(up.eval::<bool>("return build.unsaved and true or false"));
    same_summary(s, up, &format!("{slot} copied"));
    same_views(s, up, &format!("{slot} copied"));
    start
}

/// Items undo on both sides, back to `start`
fn undo_items(s: &mut NetSession, up: &Upstream, slot: &str, start: &ItemState) {
    run(s, Command::Undo(auto_pob::engine::Tab::Items)).unwrap();
    up.call::<()>("itemsUndo", ());
    assert_eq!(&our_item_state(s, slot), start, "{slot}: undo");
    same_views(s, up, &format!("{slot} undone"));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn compare_views_and_copies_match_pob() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    let mut s = net_session("compare-views", canned(&web), None);
    let scratch = Scratch::new("views");
    let gear_xml = super::items_harness::setup_xml("gear");
    let gear = scratch.0.join("gear.xml");
    fs::write(&gear, &gear_xml).unwrap();
    let ring3_xml: String = Upstream::new("attack").eval("return makeRing3Variant()");
    let ring3 = scratch.0.join("ring3.xml");
    fs::write(&ring3, &ring3_xml).unwrap();
    s.open_fixture("attack");
    let up = Upstream::new("attack");
    let refused = s.backend.net_handle().refused();
    let load_xml_up = |name: &str, xml: &str| {
        let f: mlua::Function = up.lua.globals().get("loadBuildFromXML").unwrap();
        f.call::<()>((xml.to_string(), name.to_string())).unwrap();
        up.eval::<()>("build.modFlag = false runCallback('OnFrame')");
    };

    // ---- (attack, gear): the items differ, the skills match ----
    let before = files(&s.user);
    load_file(&mut s, &gear);
    up.call::<()>("loadCompare", (gear_xml.clone(), "gear".to_string()));
    let ours = same_views(&mut s, &up, "attack, gear");
    assert!(
        ours.items
            .iter()
            .any(|r| r.differ && r.can_copy && r.can_equip)
    );
    assert!(ours.items.iter().any(|r| !r.differ && !r.can_copy));
    assert!(
        ours.items
            .iter()
            .any(|r| r.diff_label.ends_with("(extra)") || r.diff_label.ends_with("(missing)"))
    );
    assert!(
        ours.items
            .iter()
            .any(|r| r.diff_label.ends_with("(both empty)"))
    );
    assert!(
        ours.items
            .iter()
            .any(|r| r.slot.contains("Abyssal Socket") && r.can_copy)
    );
    let jewel = ours
        .items
        .iter()
        .find(|r| r.slot.starts_with("Jewel ") && r.can_copy)
        .expect("a jewel row to copy")
        .slot
        .clone();
    assert!(
        ours.skills
            .iter()
            .any(|r| !r.differ && r.theirs_label.is_some())
    );
    // PoB's skills scrollbar scrolled past the columns' gap: each side keeps its lines. The
    // bar only scrolls when the columns are wider than the screen, so the text is made wide
    // for this check (headless widths are 1).
    s.backend
        .lua
        .load("glue_width = DrawStringWidth DrawStringWidth = function() return 1500 end")
        .exec()
        .unwrap();
    let unscrolled = view(&mut s).skills;
    s.backend
        .lua
        .load("pob.build.compareTab.controls.skillsHScrollBar.offset = 900")
        .exec()
        .unwrap();
    let scrolled = view(&mut s).skills;
    let offset: i64 = s
        .backend
        .lua
        .load("return pob.build.compareTab.controls.skillsHScrollBar.offset")
        .eval()
        .unwrap();
    s.backend
        .lua
        .load(
            "DrawStringWidth = glue_width glue_width = nil \
             pob.build.compareTab.controls.skillsHScrollBar.offset = 0",
        )
        .exec()
        .unwrap();
    assert_eq!(offset, 900, "PoB's bar didn't keep the scroll");
    assert_eq!(scrolled, unscrolled, "a scrolled bar moved lines across");
    assert_eq!(view(&mut s).skills, ours.skills);

    // ---- Their items' tooltips: a base slot and a jewel socket ----
    same_their_tooltip(&mut s, &up, "Body Armour");
    same_their_tooltip(&mut s, &up, &jewel);
    // no item there, or no such slot: nothing
    for slot in ["Ring 2", "Nowhere"] {
        let none = s
            .backend
            .item_tooltip_view(&auto_pob::engine::ItemTooltipQuery {
                id: 0,
                slot: Some(slot.into()),
                theirs: true,
                shift: false,
                shared: None,
            })
            .unwrap();
        assert!(none.is_none(), "{slot}");
    }

    // ---- Hovering Equip: PoB's stat comparison of equipping their item here ----
    let equips: Vec<String> = view(&mut s)
        .items
        .iter()
        .filter(|r| r.can_equip)
        .map(|r| r.slot.clone())
        .collect();
    assert!(equips.len() >= 2, "{equips:?}");
    for (k, slot) in equips.iter().enumerate().take(2) {
        let target = auto_pob::engine::HoverTarget::CompareEquip { slot: slot.clone() };
        let ours = s
            .backend
            .hover_tooltip_view(&target)
            .unwrap()
            .unwrap_or_else(|| panic!("no answer for {slot}"));
        let theirs: Vec<auto_pob::engine::views::TooltipLine> = {
            let f: mlua::Function = up.lua.globals().get("equipHover").unwrap();
            up.lua.from_value(f.call(k + 1).unwrap()).unwrap()
        };
        assert_eq!(ours.lines, theirs, "{slot}");
        assert!(
            format!("{:?}", ours.lines).contains("Equipping this item in"),
            "{slot}: {:?}",
            ours.lines
        );
    }
    let none = s
        .backend
        .hover_tooltip_view(&auto_pob::engine::HoverTarget::CompareEquip {
            slot: "Nowhere".into(),
        })
        .unwrap();
    assert!(none.is_none());

    // ---- Copy and equip their body armour, then their jewel; Items undo takes each back ----
    let start = copy_both(&mut s, &up, "Body Armour", true);
    let row = view(&mut s)
        .items
        .into_iter()
        .find(|r| r.slot == "Body Armour")
        .unwrap();
    assert!(
        row.ours.is_some() && row.diff_label.ends_with("(match)"),
        "{row:?}"
    );
    assert_eq!(
        err(
            &mut s,
            Command::CopyCompareItem {
                slot: "Ring 2".into(),
                and_use: false,
            }
        ),
        "PoB doesn't offer that here"
    );
    assert_eq!(
        err(
            &mut s,
            Command::CopyCompareItem {
                slot: "Nowhere".into(),
                and_use: false,
            }
        ),
        "no such slot"
    );
    undo_items(&mut s, &up, "Body Armour", &start);
    let start = copy_both(&mut s, &up, &jewel, true);
    undo_items(&mut s, &up, &jewel, &start);

    // ---- This build with no socket groups: every pair is the comparison's alone ----
    run(&mut s, Command::DeleteAllSocketGroups).unwrap();
    up.call::<()>("deleteAllGroups", ());
    let ours = same_views(&mut s, &up, "no groups here");
    assert!(
        ours.skills.iter().all(|r| r.ours.is_empty()
            && !r.theirs.is_empty()
            && r.differ
            && r.theirs_label.as_ref() == Some(&r.label)),
        "{:?}",
        ours.skills
    );
    assert_eq!(files(&s.user), before, "a file was written");

    // ---- (dot, attack): other skills, other config; copy the config, Config undo ----
    s.open_fixture("dot");
    up.load("dot");
    // two options this build sets: a check and a list
    for (var, value) in [
        ("usePowerCharges", ConfigValue::Bool(true)),
        ("enemyIsBoss", ConfigValue::Choice("Uber".into())),
    ] {
        run(
            &mut s,
            Command::SetConfig {
                var: var.into(),
                value,
            },
        )
        .unwrap();
    }
    up.eval::<()>(
        r#"local controls = build.configTab.varControls
        controls.usePowerCharges.state = true
        controls.usePowerCharges.changeFunc(true)
        controls.enemyIsBoss:SetSel(4)
        runCallback("OnFrame")"#,
    );
    let before = files(&s.user);
    let attack = fixture_xml("attack");
    load_file(&mut s, &attack);
    up.call::<()>(
        "loadCompare",
        (fs::read_to_string(&attack).unwrap(), "attack".to_string()),
    );
    let ours = same_views(&mut s, &up, "dot, attack");
    assert!(ours.skills.iter().any(|r| r.differ));
    assert!(
        ours.config.iter().any(|r| !r.common),
        "the builds' config is the same"
    );
    // PoB lists the options relevant to either build after the differing ones
    assert!(ours.config.iter().any(|r| r.common), "{:?}", ours.config);
    let differing = |rows: &[CompareConfigRow]| rows.iter().filter(|r| !r.common).count();
    // ---- "Show All Configurations": the options set alike, relevant or not ----
    let toggle = |s: &mut NetSession, on: bool| {
        let affected = run(
            s,
            Command::SetCompareControl {
                name: "configToggleBtn".into(),
                value: PopupInput::Check(on),
            },
        )
        .unwrap();
        assert!(affected.compare && !affected.build, "{affected:?}");
    };
    let checked = |view: &CompareView| {
        view.controls
            .iter()
            .find(|c| c.name == "configToggleBtn")
            .unwrap()
            .checked
    };
    let hidden = view(&mut s);
    assert!(!checked(&hidden));
    toggle(&mut s, true);
    up.call::<()>("pressConfigToggle", ());
    let all = same_views(&mut s, &up, "all configurations");
    assert!(checked(&all));
    assert!(all.config.len() > hidden.config.len(), "{:?}", all.config);
    assert_eq!(differing(&all.config), differing(&hidden.config));
    assert_eq!(selectors_of(&all, &["configToggleBtn"]), {
        let names = up.lua.to_value(&["configToggleBtn"]).unwrap();
        let f: mlua::Function = up.lua.globals().get("selectorsIn").unwrap();
        up.lua
            .from_value::<Vec<Selector>>(f.call(("CONFIG", names)).unwrap())
            .unwrap()
    });
    // asking for the state it has presses nothing
    toggle(&mut s, true);
    assert_eq!(view(&mut s).config, all.config);
    toggle(&mut s, false);
    up.call::<()>("pressConfigToggle", ());
    assert_eq!(
        same_views(&mut s, &up, "ineligible hidden").config,
        hidden.config
    );

    // ---- The Tree view's version dropdowns: PoB's "Convert to Version" popup ----
    let start = view(&mut s);
    let left = start
        .controls
        .iter()
        .find(|c| c.name == "leftVersionSelect")
        .unwrap()
        .clone();
    assert!(
        left.shown && left.enabled && left.choices.len() > 1,
        "{left:?}"
    );
    let older = left.selected.checked_sub(1).unwrap_or(1);
    let convert =
        |theirs: bool, index: usize, label: &str, keep_copy: bool| Command::ConvertCompareTree {
            theirs,
            index,
            label: label.into(),
            keep_copy,
        };
    // the spec's own version opens no popup; a stale list is refused
    assert_eq!(
        err(
            &mut s,
            convert(false, left.selected, &left.choices[left.selected], false)
        ),
        "PoB doesn't offer that here"
    );
    assert_eq!(
        err(&mut s, convert(false, older, "Nowhere", false)),
        "the list changed; pick again"
    );
    assert_eq!(
        our_spec_state(&s),
        up.spec_state(),
        "before the conversions"
    );
    // this build's tree, keeping a copy
    let affected = run(&mut s, convert(false, older, &left.choices[older], true)).unwrap();
    assert!(
        affected.tree && affected.compare && affected.build,
        "{affected:?}"
    );
    up.call::<()>("pressVersion", ("leftVersionSelect", older + 1, true));
    let mine = our_spec_state(&s);
    assert_eq!(mine, up.spec_state(), "ours converted");
    assert_eq!(mine.specs.len(), 2, "no copy kept: {:?}", mine.specs);
    same_summary(&mut s, &up, "ours converted");
    // the dropdown follows the active spec's version, as PoB's draw sets it
    let other = mine
        .specs
        .iter()
        .position(|(_, version)| *version != mine.specs[mine.active as usize - 1].1)
        .expect("a spec of the old version");
    run(&mut s, Command::SelectSpec { index: other }).unwrap();
    up.eval::<()>(&format!(
        "runCallback('OnFrame') build.treeTab.controls.specSelect:SetSel({}) runCallback('OnFrame')",
        other + 1
    ));
    assert_eq!(our_spec_state(&s), up.spec_state(), "the old spec again");
    let versions = |up: &Upstream| {
        up.view_selectors()
            .into_iter()
            .filter(|c| c.name.ends_with("VersionSelect"))
            .collect::<Vec<_>>()
    };
    let back = view(&mut s);
    let ours_versions = selectors_of(&back, &["leftVersionSelect", "rightVersionSelect"]);
    assert_eq!(
        ours_versions,
        versions(&up),
        "versions after the spec switch"
    );
    assert_eq!(
        ours_versions[0].selected, left.selected,
        "{ours_versions:?}"
    );
    run(
        &mut s,
        Command::SelectSpec {
            index: mine.active as usize - 1,
        },
    )
    .unwrap();
    up.eval::<()>(&format!(
        "runCallback('OnFrame') build.treeTab.controls.specSelect:SetSel({}) runCallback('OnFrame')",
        mine.active
    ));
    assert_eq!(our_spec_state(&s), mine, "the converted spec again");
    // the comparison's tree, replaced
    let right = view(&mut s)
        .controls
        .iter()
        .find(|c| c.name == "rightVersionSelect")
        .unwrap()
        .clone();
    assert!(right.shown && right.enabled, "{right:?}");
    let their_older = right.selected.checked_sub(1).unwrap_or(1);
    let affected = run(
        &mut s,
        convert(true, their_older, &right.choices[their_older], false),
    )
    .unwrap();
    assert!(affected.compare && !affected.build, "{affected:?}");
    up.call::<()>(
        "pressVersion",
        ("rightVersionSelect", their_older + 1, false),
    );
    let converted_state = our_spec_state(&s);
    assert_eq!(converted_state, up.spec_state(), "theirs converted");
    assert_ne!(converted_state.theirs_version, mine.theirs_version);
    same_summary(&mut s, &up, "theirs converted");
    let converted = view(&mut s);
    assert_eq!(
        selectors_of(&converted, &["leftVersionSelect", "rightVersionSelect"]),
        up.view_selectors()
            .into_iter()
            .filter(|c| c.name.ends_with("VersionSelect"))
            .collect::<Vec<_>>()
    );
    // ---- the Config view's boxes: PoB's controls for both builds' values ----
    // their check box: the comparison's option changes and it is recalculated
    let boxes = same_views(&mut s, &up, "before the config boxes");
    let theirs = boxes
        .config
        .iter()
        .find_map(|r| {
            let c = r.theirs_control.as_ref()?;
            (c.kind == PopupControlKind::Check).then(|| (c.name.clone(), c.checked))
        })
        .expect("a check box row");
    let start = our_config_input(&s);
    run(
        &mut s,
        Command::SetCompareControl {
            name: theirs.0.clone(),
            value: PopupInput::Check(!theirs.1),
        },
    )
    .unwrap();
    up.call::<()>("pressConfigBox", (theirs.0.as_str(), "check", ()));
    assert_eq!(our_config_input(&s), start, "their box changed our option");
    same_summary(&mut s, &up, "their config box");
    same_views(&mut s, &up, "their config box");
    // our number box: this build's option, recalculated
    let current = view(&mut s);
    let ours_box = current
        .config
        .iter()
        .find_map(|r| {
            let c = r.ours_control.as_ref()?;
            (c.kind == PopupControlKind::Edit && c.digits).then(|| (c.name.clone(), c.text.clone()))
        })
        .expect("a number box row");
    let (ours_box, was) = ours_box;
    let affected = run(
        &mut s,
        Command::SetCompareControl {
            name: ours_box.clone(),
            value: PopupInput::Text("3".into()),
        },
    )
    .unwrap();
    assert!(affected.config && affected.recalc && affected.compare);
    up.call::<()>("pressConfigBox", (ours_box.as_str(), "text", "3"));
    assert_ne!(our_config_input(&s), start);
    assert_eq!(our_config_input(&s), up.config_input());
    same_summary(&mut s, &up, "our config box");
    same_views(&mut s, &up, "our config box");
    // PoB's box callbacks set buildFlag only: the unsaved mark is upstream's
    let up_unsaved: bool = up.eval("return build.unsaved and true or false");
    assert_eq!(s.backend.build_view().unwrap().unsaved, up_unsaved);
    // their dropdown: a pick, then back
    let pick = view(&mut s)
        .config
        .iter()
        .find_map(|r| {
            let c = r.theirs_control.as_ref()?;
            (c.kind == PopupControlKind::Dropdown && c.choices.len() > 1)
                .then(|| (c.name.clone(), c.selected, c.choices.clone()))
        })
        .expect("a dropdown row");
    let (drop, was_at, choices) = pick;
    for index in [(was_at + 1) % choices.len(), was_at] {
        run(
            &mut s,
            Command::SetCompareControl {
                name: drop.clone(),
                value: PopupInput::Choice {
                    index,
                    label: choices[index].clone(),
                },
            },
        )
        .unwrap();
        up.call::<()>("pressConfigBox", (drop.as_str(), "choice", index + 1));
        same_summary(&mut s, &up, "their config dropdown");
        same_views(&mut s, &up, "their config dropdown");
    }
    // both back as they were: PoB's boxes add no undo state, so the Config undo below goes
    // back past them
    run(
        &mut s,
        Command::SetCompareControl {
            name: ours_box.clone(),
            value: PopupInput::Text(was.clone()),
        },
    )
    .unwrap();
    up.call::<()>("pressConfigBox", (ours_box.as_str(), "text", was.as_str()));
    run(
        &mut s,
        Command::SetCompareControl {
            name: theirs.0.clone(),
            value: PopupInput::Check(theirs.1),
        },
    )
    .unwrap();
    up.call::<()>("pressConfigBox", (theirs.0.as_str(), "check", ()));
    assert_eq!(our_config_input(&s), start);
    // a check box clicked back leaves PoB's input false, not unset ("No", not "(not set)")
    same_views(&mut s, &up, "the config boxes back");

    // the copy below starts from here
    let ours = same_views(&mut s, &up, "after the conversions");
    let start = our_config_input(&s);
    assert_eq!(start, up.config_input());
    let affected = run(&mut s, Command::CopyCompareConfig).unwrap();
    assert!(affected.config && affected.build && affected.recalc && affected.compare);
    up.call::<()>("pressCopyConfig", ());
    let copied = our_config_input(&s);
    assert_ne!(copied, start);
    assert_eq!(copied, up.config_input());
    assert!(s.backend.build_view().unwrap().unsaved);
    same_summary(&mut s, &up, "config copied");
    // PoB copies the options the comparison sets; one it leaves unset stays as it was here
    let after = same_views(&mut s, &up, "after the config copy");
    assert!(
        differing(&after.config) < differing(&ours.config),
        "{:?}",
        after.config
    );
    assert!(
        after
            .config
            .iter()
            .all(|r| r.var != "enemyIsBoss" || r.common)
    );
    run(&mut s, Command::Undo(auto_pob::engine::Tab::Config)).unwrap();
    assert_eq!(our_config_input(&s), start);
    assert_eq!(
        view(&mut s).config,
        ours.config,
        "the config rows after undo"
    );

    assert_eq!(files(&s.user), before, "a file was written");

    // ---- (gear, ring3): PoB's "(match)" on other texts, "Missing slot", an unallocated
    // socket on their side, and this build's item set ----
    open_xml(&mut s, "gear", &gear_xml);
    load_xml_up("gear", &gear_xml);
    let before = files(&s.user);
    load_file(&mut s, &ring3);
    up.call::<()>("loadCompare", (ring3_xml.clone(), "ring3".to_string()));
    let first = same_views(&mut s, &up, "gear, ring3");
    let ring1 = first.items.iter().find(|r| r.slot == "Ring 1").unwrap();
    assert!(
        ring1.differ && ring1.diff_label.ends_with("(match)"),
        "{ring1:?}"
    );
    let ring3_row = first
        .items
        .iter()
        .find(|r| r.slot == "Ring 3")
        .expect("a Ring 3 row");
    assert!(ring3_row.can_copy && !ring3_row.can_equip, "{ring3_row:?}");
    let unallocated = first
        .items
        .iter()
        .find(|r| r.theirs_unallocated)
        .expect("their jewel in an unallocated socket")
        .slot
        .clone();
    // ---- Expanded mode: PoB's cards, each mod marked against the other side's (the rings
    // carry the same mods with other values) ----
    let expand = |s: &mut NetSession, on: bool| {
        run(
            s,
            Command::SetCompareControl {
                name: "itemsExpandedCheck".into(),
                value: PopupInput::Check(on),
            },
        )
        .unwrap();
        up.call::<()>("setItemsExpanded", on);
    };
    expand(&mut s, true);
    let rows = view(&mut s).items;
    let cards: Vec<Vec<CardLine>> = up.json("return itemsExpandedFrame()");
    let ours_cards: Vec<Vec<CardLine>> = rows
        .iter()
        .flat_map(|r| [r.ours_card.clone(), r.theirs_card.clone()])
        .collect();
    assert_eq!(ours_cards, cards, "expanded cards");
    assert!(
        rows.iter()
            .all(|r| !r.ours_card.is_empty() && !r.theirs_card.is_empty())
    );
    let marked = |mark: &str| {
        ours_cards.iter().flatten().any(|l| {
            l.text.starts_with(&format!("^x33FF77{mark}"))
                || l.text.starts_with(&format!("^xDD0022{mark}"))
        })
    };
    assert!(marked("+ "), "no mod only on one side: {ours_cards:?}");
    assert!(
        marked("> ") || marked("< "),
        "no mod with another value: {ours_cards:?}"
    );
    // the compact rows again, with no cards
    expand(&mut s, false);
    assert!(
        same_views(&mut s, &up, "gear, ring3 compact")
            .items
            .iter()
            .all(|r| r.ours_card.is_empty() && r.theirs_card.is_empty())
    );
    assert_eq!(
        err(
            &mut s,
            Command::CopyCompareItem {
                slot: "Ring 3".into(),
                and_use: true,
            }
        ),
        "PoB doesn't offer that here"
    );
    // Copy without Equip is offered
    let start = copy_both(&mut s, &up, "Ring 3", false);
    undo_items(&mut s, &up, "Ring 3", &start);
    same_their_tooltip(&mut s, &up, &unallocated);

    let first = view(&mut s);
    let pick = choice(&first, "primaryItemSetSelect", 1);
    let affected = run(&mut s, pick).unwrap();
    assert!(affected.items && affected.build && affected.recalc && affected.compare);
    up.call::<()>("pressItemsChoice", ("primaryItemSetSelect", 2));
    let second = same_views(&mut s, &up, "gear set 2");
    assert_ne!(second.items, first.items, "the item set changed nothing");
    let state = our_item_state(&s, "Ring 2");
    assert_eq!(state, up.item_state("Ring 2"));
    assert_ne!(state.set, 1);
    assert_eq!(files(&s.user), before, "a file was written");

    // ---- (ring3, gear): this build's jewel in an unallocated socket ----
    open_xml(&mut s, "ring3", &ring3_xml);
    load_xml_up("ring3", &ring3_xml);
    load_file(&mut s, &gear);
    up.call::<()>("loadCompare", (gear_xml.clone(), "gear".to_string()));
    let ours = same_views(&mut s, &up, "ring3, gear");
    assert!(ours.items.iter().any(|r| r.ours_unallocated));

    // ---- Review Focus 5: a build change leaves nothing of the comparison ----
    s.open_fixture("attack");
    assert_eq!(s.backend.compare_view().unwrap(), None);
    assert_eq!(
        err(
            &mut s,
            Command::CopyCompareItem {
                slot: "Body Armour".into(),
                and_use: true,
            }
        ),
        "no comparison build loaded"
    );
    assert_eq!(
        err(&mut s, Command::CopyCompareConfig),
        "no comparison build loaded"
    );
    let tree = s.backend.tree_state_view().unwrap();
    assert!(
        tree.specs
            .iter()
            .all(|spec| !spec.title.contains("(Compared)")),
        "{:?}",
        tree.specs
    );

    assert_eq!(
        s.backend.net_handle().refused(),
        refused,
        "a launch was tried"
    );
    assert!(web.requests().is_empty());
    assert!(web.unrouted().is_empty());
    let secs = started.elapsed().as_secs_f64();
    assert!(secs < 40.0, "took {secs:.1}s");
}
