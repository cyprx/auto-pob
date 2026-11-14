//! The tree canvas's drawing against PassiveTreeView:Draw, recorded in upstream PoB
//! (`fixtures/tree_draw_lib.lua`): every image our scene draws with nothing hovered, by file,
//! sprite, place, colour and draw layer, and every connector by its nodes.

use super::{Session, check_recipe_with, fixtures_dir, lock, pob_dir, upstream_with_tree_lib};
use auto_pob::engine::{Backend, Command, TreeLayout, TreeState};
use auto_pob::ui::tree::scene::{self, DrawSets, Image, Look, Tag};
use eframe::egui::{Pos2, Rect};
use mlua::LuaSerdeExt;
use std::collections::{BTreeMap, HashMap};
use std::fs;

/// What upstream drew, as `recordTreeDraw` lists it
#[derive(Debug, Clone, serde::Deserialize)]
struct Drawn {
    file: String,
    #[serde(default)]
    rect: Option<[f32; 4]>,
    x: f32,
    y: f32,
    colour: Vec<f32>,
    layer: i32,
    #[serde(default)]
    connector: Option<[u32; 2]>,
    #[serde(default)]
    flipped: Option<bool>,
}

/// What has to be equal: the file, the sprite (`None`: the whole file), the colour in 1/1000s,
/// the layer, a connector's nodes, and whether it is a lower half
type Key = (
    String,
    Option<[i32; 4]>,
    [i32; 4],
    i32,
    Option<[u32; 2]>,
    bool,
);

/// One drawn image, comparable across the two, and where it is
#[derive(Debug, Clone)]
struct Rec {
    key: Key,
    at: [f32; 2],
}

fn colour_key(c: &[f32]) -> [i32; 4] {
    let get = |i: usize| (c.get(i).copied().unwrap_or(1.0) * 1000.0).round() as i32;
    [get(0), get(1), get(2), get(3)]
}

fn rect_key(r: [f32; 4]) -> [i32; 4] {
    r.map(|v| v.round() as i32)
}

/// Upstream's recording, in comparable form. A background's texture coordinates follow its
/// view, so only its file counts.
fn theirs(list: &[Drawn]) -> Vec<Rec> {
    list.iter()
        // the hovered node's tooltip (layers 99 and 100) is egui's
        .filter(|d| d.layer < 99)
        .map(|d| {
            let flipped = d.flipped.unwrap_or(false);
            let rect = match d.rect {
                Some(_) if flipped || d.file.contains("Background") => None,
                // the 1x1 headless sheet gives pixels: left, top, right, bottom
                Some([l, t, r, b]) => Some(rect_key([l, t, r - l, b - t])),
                None => None,
            };
            Rec {
                key: (
                    d.file.clone(),
                    rect,
                    colour_key(&d.colour),
                    d.layer,
                    d.connector,
                    flipped,
                ),
                at: [d.x, d.y],
            }
        })
        .collect()
}

/// Our scene with nothing hovered, as upstream's recording view sees it (PoB's zoom 0.25)
fn ours(layout: &TreeLayout, state: &TreeState) -> Vec<Rec> {
    ours_hovering(layout, state, None)
}

/// Our scene with `hover` hovered, its path and dependents as the canvas finds them
fn ours_hovering(layout: &TreeLayout, state: &TreeState, hover: Option<u32>) -> Vec<Rec> {
    let sets = DrawSets::new(state, scene::ascendancy_of(state));
    let index: HashMap<u32, usize> = layout
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let lists = |l: &[auto_pob::engine::NodePath]| -> HashMap<u32, Vec<u32>> {
        l.iter().map(|p| (p.node, p.path.clone())).collect()
    };
    let hovering = scene::hover_sets(
        hover,
        None,
        &sets,
        &lists(&state.paths),
        &lists(&state.depends),
    );
    let look = Look {
        hover,
        path: hovering.path.as_ref(),
        deps: &hovering.deps,
        trace_end: None,
        heat: None,
        power_theme: Default::default(),
        view: Rect::from_min_max(Pos2::new(-1e6, -1e6), Pos2::new(1e6, 1e6)),
        pob_zoom: 0.25,
        time_ms: None,
    };
    let root = pob_dir();
    scene::build(layout, state, &sets, &index, &look)
        .iter()
        .map(|image: &Image| {
            let sheet = layout.sheets[image.sprite.sheet]
                .as_ref()
                .expect("a drawn sheet is loaded");
            let file = sheet
                .path
                .strip_prefix(&root)
                .unwrap_or(&sheet.path)
                .to_string_lossy()
                .replace('\\', "/");
            let whole = image.sprite.rect == [0.0, 0.0, sheet.size[0] as f32, sheet.size[1] as f32];
            let flipped = image.tag == Tag::Group && image.uv[0].y == 1.0;
            let rect = (!whole && image.tag != Tag::Background && !flipped)
                .then(|| rect_key(image.sprite.rect));
            let connector = match image.tag {
                Tag::Connector(ids) => Some(ids),
                _ => None,
            };
            let at = match image.tag {
                // a quad's centre, as recorded
                Tag::Connector(_) | Tag::JewelRing(_) => {
                    let c = image
                        .quad
                        .iter()
                        .fold(eframe::egui::Vec2::ZERO, |a, b| a + *b)
                        / 4.0;
                    [c.x, c.y]
                }
                _ => [image.anchor.x, image.anchor.y],
            };
            Rec {
                key: (
                    file,
                    rect,
                    colour_key(&image.tint),
                    i32::from(image.layer),
                    connector,
                    flipped,
                ),
                at,
            }
        })
        .collect()
}

/// Matches the two lists: same key, and (but for connectors, which their nodes name) within a
/// few tree units, as upstream's 1x1 images put a half background's anchor 1.3 units off
fn assert_same(what: &str, ours: Vec<Rec>, theirs: Vec<Rec>) {
    let mut pool: BTreeMap<String, Vec<Rec>> = BTreeMap::new();
    for r in ours {
        pool.entry(format!("{:?}", r.key)).or_default().push(r);
    }
    let mut missing = Vec::new();
    for t in theirs {
        let list = pool.entry(format!("{:?}", t.key)).or_default();
        let near = |r: &Rec| {
            t.key.4.is_some()
                || ((r.at[0] - t.at[0]).abs() < 3.0 && (r.at[1] - t.at[1]).abs() < 3.0)
        };
        match list.iter().position(near) {
            Some(i) => {
                list.swap_remove(i);
            }
            None => missing.push(t),
        }
    }
    let extra: Vec<Rec> = pool.into_values().flatten().collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: {} drawn by PoB only, {} by us only\nPoB only: {:#?}\nours only: {:#?}",
        missing.len(),
        extra.len(),
        &missing[..missing.len().min(15)],
        &extra[..extra.len().min(15)],
    );
}

fn load_recorder(lua: &mlua::Lua) {
    lua.load(fs::read_to_string(fixtures_dir().join("tree_draw_lib.lua")).unwrap())
        .set_name("=tree_draw_lib")
        .exec()
        .expect("tree_draw_lib.lua");
}

/// Upstream's Draw of its build, compared with the spec the Tree tab compares with
fn record(lua: &mlua::Lua) -> Vec<Rec> {
    record_hovering(lua, None)
}

/// `record` with the cursor on `hover`, which PoB must find there
fn record_hovering(lua: &mlua::Lua, hover: Option<u32>) -> Vec<Rec> {
    load_recorder(lua);
    lua.globals().set("gate_hover", hover).unwrap();
    let (value, found): (mlua::Value, Option<u32>) = lua
        .load(
            r#"
            local tab = build.treeTab
            local viewer = tab.viewer
            viewer.compareSpec = tab.isComparing and tab.specList[tab.activeCompareSpec] or nil
            return recordTreeDraw(viewer, build, gate_hover)
            "#,
        )
        .eval()
        .expect("recording Draw");
    assert_eq!(found, hover, "the node PoB hovers");
    let list: Vec<Drawn> = lua.from_value(value).expect("the recorded list");
    assert!(list.len() > 1000, "{} images", list.len());
    theirs(&list)
}

/// Upstream's drawing after a tree recipe, and ours at its last checkpoint
fn recipe_matches(name: &str, setup: &str) {
    let source = fs::read_to_string(fixtures_dir().join(format!("tree/{name}.lua"))).unwrap();
    let base = source
        .lines()
        .find_map(|l| l.strip_prefix("base = \""))
        .and_then(|l| l.strip_suffix('"'))
        .unwrap_or("attack")
        .to_owned();
    let lua = upstream_with_tree_lib(&base);
    lua.load(source).exec().unwrap();
    lua.load("recipe()").exec().unwrap();
    lua.load(setup).exec().unwrap();
    let theirs = record(&lua);
    let mut last = None;
    check_recipe_with(name, &mut |s, check| {
        if check.is_some() {
            last = Some(snapshot(s));
        }
    });
    let (layout, state) = last.expect("a checkpoint");
    assert_same(name, ours(&layout, &state), theirs);
}

fn snapshot(s: &mut Session) -> (TreeLayout, TreeState) {
    let layout = s.backend.tree_layout_view().unwrap();
    let state = s.backend.tree_state_view().unwrap();
    (layout, state)
}

/// The whole tree as PoB draws it for a build: the background, the class art, every group's
/// background and ascendancy emblem, every connector in its state and every node's art
#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_drawing_matches_pob() {
    let _guard = lock();
    let lua = upstream_with_tree_lib("attack");
    let theirs = record(&lua);
    let mut s = super::attack_session("tree-draw");
    let (layout, state) = snapshot(&mut s);
    // PoB draws fewer connectors than the tree has links: none to a class start or a mastery
    assert!(layout.connectors.len() < layout.edges.len());
    assert_same("attack", ours(&layout, &state), theirs);

    // a search: PoB's rings over its matches
    s.run(Command::SetTreeSearch {
        text: "life".into(),
    })
    .unwrap();
    lua.load("build.treeTab.viewer.searchStr = 'life'")
        .exec()
        .unwrap();
    let theirs = record(&lua);
    let (layout, state) = snapshot(&mut s);
    assert!(state.search_matches.len() > 10);
    assert_same("search", ours(&layout, &state), theirs);
}

/// Hovering a node: its path's frames and connectors, its dependents' red, a hovered mastery's
/// art, a hovered socket's radius colours and rings (PassiveTreeView.lua:323-376, 832-1000,
/// 1209-1230)
#[test]
#[ignore = "needs the pinned PoB install"]
fn hovered_tree_drawing_matches_pob() {
    let _guard = lock();
    let lua = upstream_with_tree_lib("attack");
    let mut s = super::attack_session("tree-draw-hover");
    let (layout, state) = snapshot(&mut s);
    let kind = |id: u32| {
        layout
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.kind.clone())
            .unwrap_or_default()
    };
    // the farthest reachable notable, the allocated node with the most dependents, a reachable
    // mastery, a socket
    let far = state
        .paths
        .iter()
        .filter(|p| kind(p.node) == "Notable")
        .max_by_key(|p| (p.path.len(), p.node))
        .expect("a reachable notable")
        .node;
    let root = state
        .depends
        .iter()
        .filter(|p| !matches!(kind(p.node).as_str(), "ClassStart" | "AscendClassStart"))
        .max_by_key(|p| (p.path.len(), p.node))
        .expect("an allocated node")
        .node;
    let mastery = state
        .paths
        .iter()
        .filter(|p| {
            layout
                .nodes
                .iter()
                .any(|n| n.id == p.node && n.kind == "Mastery" && n.mastery_effects)
        })
        .min_by_key(|p| (p.path.len(), p.node))
        .expect("a reachable mastery")
        .node;
    let socket = layout
        .nodes
        .iter()
        .filter(|n| n.radius_socket)
        .map(|n| n.id)
        .min()
        .expect("a socket");
    for hover in [far, root, mastery, socket] {
        let theirs = record_hovering(&lua, Some(hover));
        assert_same(
            &format!("hovering {hover}"),
            ours_hovering(&layout, &state, Some(hover)),
            theirs,
        );
    }
}

/// A cluster jewel only the compared tree has: its group art green at 0.6, its connectors green
/// in the Active art, its nodes green; and the compared tree's own colours on ours
/// (PassiveTreeView.lua:651-660, 747-770, 1102-1141)
#[test]
#[ignore = "needs the pinned PoB install"]
fn compared_tree_drawing_matches_pob() {
    let _guard = lock();
    recipe_matches("compare_only", "");
}

/// A jewel's radius art and its socket's overlay (PassiveTreeView.lua:844-848, 1156-1248)
#[test]
#[ignore = "needs the pinned PoB install"]
fn jewel_drawing_matches_pob() {
    let _guard = lock();
    let recipe = super::upstream_tree_recipe("jewel");
    let xml = recipe.xml.expect("the jewel recipe saves its build");
    let lua = auto_pob::engine::upstream::upstream_lua(&pob_dir()).unwrap();
    auto_pob::engine::upstream::load_xml(&lua, &xml, "jewel").unwrap();
    let theirs = record(&lua);
    let mut s = Session::start("tree-draw-jewel");
    let path = s.user.join("jewel.xml");
    fs::create_dir_all(&s.user).unwrap();
    fs::write(&path, &xml).unwrap();
    s.open_fixture("jewel", &path);
    let (layout, state) = snapshot(&mut s);
    assert!(state.socket_rings.iter().any(|r| !r.rings.is_empty()));
    assert_same("jewel", ours(&layout, &state), theirs);
}

/// A chosen bloodline: its emblem unfaded, its connectors faded (PassiveTreeView.lua:572, 711)
#[test]
#[ignore = "needs the pinned PoB install"]
fn bloodline_drawing_matches_pob() {
    let _guard = lock();
    recipe_matches("bloodline", "");
}
