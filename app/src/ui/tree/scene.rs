//! What the tree canvas draws, as PassiveTreeView:Draw draws it (PassiveTreeView.lua:536-1248):
//! the images in PoB's order and draw layers, in tree units, before the camera puts them on
//! screen. Pure, so tests can hold it against what PoB draws.

use super::overlays::{self, PowerTheme};
use crate::engine::{Connector, GroupArt, NodePower, Sprite, TreeLayout, TreeNode, TreeState};
use eframe::egui::{Rect, Vec2};
use std::collections::{HashMap, HashSet};

/// PoB draws an asset 1.33 tree units per pixel each side of its anchor (`DrawAsset`)
pub const ART_SCALE: f32 = 1.33;
/// The Split Personality path's colour (`colorCodes.SPLITPERSONALITY`, Global.lua:61)
const SPLIT_PERSONALITY: [f32; 3] = [1.0, 214.0 / 255.0, 42.0 / 255.0];
/// The group art of a cluster jewel only the compared spec has (PassiveTreeView.lua:654)
const COMPARE_ONLY_GROUP: [f32; 4] = [0.0, 1.0, 0.0, 0.6];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
/// PoB's draw layers (`SetDrawLayer(nil, n)`)
pub const LAYER_BACK: u8 = 0;
pub const LAYER_MASTERY: u8 = 15;
pub const LAYER_CONNECTOR: u8 = 20;
pub const LAYER_NODE: u8 = 25;
pub const LAYER_SEARCH: u8 = 30;

/// What an image is, for tests
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Background,
    ClassArt,
    Group,
    Connector([u32; 2]),
    Node(u32),
    CompareOnly(u32),
    SearchRing(u32),
    HoverRing(u32),
    JewelRing(u32),
}

/// One image to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub layer: u8,
    /// Within a layer, what must go above what: a node's frame over its icon, the rings over
    /// the nodes. Images of one pass may be drawn in any order, so the canvas can draw each
    /// file's at once.
    pub pass: u8,
    pub sprite: Sprite,
    /// The corners in tree units, in PoB's order (`DrawImageQuad`; an upright image goes
    /// top-left, top-right, bottom-right, bottom-left)
    pub quad: [Vec2; 4],
    /// Each corner's texture coordinates, as fractions of the sprite's rectangle; past 1 the
    /// texture repeats (a line connector)
    pub uv: [Vec2; 4],
    /// RGBA, not premultiplied
    pub tint: [f32; 4],
    /// Where PoB puts it: `DrawAsset`'s point, a ring's centre, a quad's first corner
    pub anchor: Vec2,
    pub tag: Tag,
}

/// What the drawing depends on besides the layout and the state: the hovered node and its path
/// or dependents, the heat map, the view
pub struct Look<'a> {
    /// PoB's `hoverNode`, after the trace refused any
    pub hover: Option<u32>,
    /// `hoverPath`: `None` when PoB has none
    pub path: Option<&'a HashSet<u32>>,
    /// `hoverDep`
    pub deps: &'a HashSet<u32>,
    /// The last node of the Shift trace
    pub trace_end: Option<u32>,
    /// The heat map is on (`showHeatMap`), with the power PoB colours by, if any yet
    pub heat: Option<Option<(&'a NodePower, &'a HashMap<u32, usize>)>>,
    pub power_theme: PowerTheme,
    /// The view, in tree units; what is wholly outside is left out
    pub view: Rect,
    /// PoB's `zoom` (1.2 to the zoom level): search rings and flavour text depend on it
    pub pob_zoom: f32,
    /// PoB's `GetTime()` for turning jewel rings, `None` with animations off
    pub time_ms: Option<f64>,
}

/// Lookups over [`TreeState`] the drawing needs, built once per state
#[derive(Debug, Default)]
pub struct DrawSets {
    pub allocated: HashSet<u32>,
    pub granted: HashSet<u32>,
    pub comparing: bool,
    pub compare_allocated: HashSet<u32>,
    pub compare_missing: HashSet<u32>,
    pub compare_colour: HashMap<u32, [f32; 3]>,
    pub split: HashSet<u32>,
    pub socket_overlay: HashMap<u32, String>,
    pub variable: HashSet<u32>,
    pub search: HashSet<u32>,
    pub leap: HashSet<u32>,
    /// `curAscendClassBaseName`'s id
    pub ascendancy: Option<String>,
    pub bloodline: Option<String>,
    pub class_id: u32,
}

impl DrawSets {
    pub fn new(state: &TreeState, ascendancy: Option<String>) -> Self {
        let set = |ids: &[u32]| ids.iter().copied().collect::<HashSet<u32>>();
        Self {
            allocated: set(&state.allocated),
            granted: set(&state.draw.granted),
            comparing: state.draw.comparing,
            compare_allocated: set(&state.draw.compare_allocated),
            compare_missing: set(&state.draw.compare_missing),
            compare_colour: state.compare.iter().map(|c| (c.node, c.rgb)).collect(),
            split: set(&state.draw.split_path),
            socket_overlay: state
                .draw
                .socket_overlays
                .iter()
                .map(|s| (s.node, s.overlay.clone()))
                .collect(),
            variable: set(&state.draw.variable_sockets),
            search: set(&state.search_matches),
            leap: set(&state.draw.leap),
            ascendancy,
            bloodline: state.bloodline.clone(),
            class_id: state.class_id,
        }
    }
}

/// The id the build's ascendancy's nodes carry (`curAscendClassBaseName`), none without one
pub fn ascendancy_of(state: &TreeState) -> Option<String> {
    state
        .classes
        .iter()
        .find(|c| c.id == state.class_id)
        .and_then(|c| c.ascendancies.iter().find(|a| a.id == state.ascendancy_id))
        .filter(|a| a.id != 0)
        .map(|a| a.ascend_id.clone())
}

/// PoB's `hoverPath` and `hoverDep` (PassiveTreeView.lua:323-376)
#[derive(Debug, Default, PartialEq)]
pub struct HoverSets {
    pub path: Option<HashSet<u32>>,
    pub deps: HashSet<u32>,
}

/// The path and dependents PoB draws for the hovered node: with Shift held, the trace and no
/// dependents; else for a node with a path, its path (none when an Intuitive Leap-like jewel
/// reaches it) and its dependents. An allocated node's path is empty.
pub fn hover_sets(
    hovered: Option<u32>,
    trace: Option<&[u32]>,
    sets: &DrawSets,
    paths: &HashMap<u32, Vec<u32>>,
    depends: &HashMap<u32, Vec<u32>>,
) -> HoverSets {
    if let Some(trace) = trace {
        return HoverSets {
            path: Some(trace.iter().copied().collect()),
            deps: HashSet::new(),
        };
    }
    let Some(id) = hovered else {
        return HoverSets::default();
    };
    let deps = || depends.get(&id).into_iter().flatten().copied().collect();
    if sets.allocated.contains(&id) {
        return HoverSets {
            path: Some(HashSet::new()),
            deps: deps(),
        };
    }
    match paths.get(&id) {
        Some(path) if !sets.leap.contains(&id) => HoverSets {
            path: Some(path.iter().copied().collect()),
            deps: deps(),
        },
        Some(_) => HoverSets {
            path: Some(HashSet::new()),
            deps: deps(),
        },
        None => HoverSets::default(),
    }
}

/// An upright image of `sprite` around `at` (`DrawAsset`); `flip` turns it upside down
fn asset(sprite: Sprite, centre: Vec2, flip: bool) -> ([Vec2; 4], [Vec2; 4]) {
    let half = Vec2::new(sprite.rect[2], sprite.rect[3]) * ART_SCALE;
    let (lo, hi) = (centre - half, centre + half);
    let quad = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)];
    let (top, bottom) = if flip { (1.0, 0.0) } else { (0.0, 1.0) };
    let uv = [
        Vec2::new(0.0, top),
        Vec2::new(1.0, top),
        Vec2::new(1.0, bottom),
        Vec2::new(0.0, bottom),
    ];
    (quad, uv)
}

/// The scene being built
struct Scene<'a> {
    images: Vec<Image>,
    layer: u8,
    pass: u8,
    view: Rect,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl Scene<'_> {
    fn visible(&self, centre: Vec2, reach: f32) -> bool {
        self.view.expand(reach).contains(centre.to_pos2())
    }

    /// `DrawAsset(sprite, x, y, scale)`, centred on `at`
    fn draw_asset(&mut self, sprite: Option<Sprite>, at: Vec2, tint: [f32; 4], tag: Tag) {
        let Some(sprite) = sprite else {
            return;
        };
        let reach = sprite.rect[2].max(sprite.rect[3]) * ART_SCALE;
        if !self.visible(at, reach) {
            return;
        }
        let (quad, uv) = asset(sprite, at, false);
        self.images.push(Image {
            layer: self.layer,
            pass: self.pass,
            sprite,
            quad,
            uv,
            tint,
            anchor: at,
            tag,
        });
    }

    /// `DrawAsset(sprite, x, y, scale, true)`: the art above `at`, and mirrored below it
    fn draw_half(&mut self, sprite: Sprite, at: Vec2, tint: [f32; 4], tag: Tag) {
        let h = sprite.rect[3] * ART_SCALE;
        if !self.visible(at, sprite.rect[2].max(2.0 * sprite.rect[3]) * ART_SCALE) {
            return;
        }
        for (centre, flip) in [
            (at - Vec2::new(0.0, h), false),
            (at + Vec2::new(0.0, h), true),
        ] {
            let (quad, uv) = asset(sprite, centre, flip);
            self.images.push(Image {
                layer: self.layer,
                pass: self.pass,
                sprite,
                quad,
                uv,
                tint,
                anchor: at,
                tag,
            });
        }
    }

    /// An image `size` tree units across around `at`, turned `angle` radians
    /// (`DrawImage`, `DrawImageRotated`)
    fn draw_square(
        &mut self,
        sprite: Option<Sprite>,
        at: Vec2,
        size: f32,
        angle: f32,
        tint: [f32; 4],
        tag: Tag,
    ) {
        let Some(sprite) = sprite else {
            return;
        };
        // a search ring off the view may be drawn at its edge
        let snappable = matches!(tag, Tag::SearchRing(_));
        // PoB draws a jewel's inner ring even at radius 0, as nothing
        if size < 0.0 || !(snappable || self.visible(at, size)) {
            return;
        }
        let h = size / 2.0;
        let (sin, cos) = angle.sin_cos();
        let corner = |x: f32, y: f32| at + Vec2::new(x * cos - y * sin, x * sin + y * cos);
        self.images.push(Image {
            layer: self.layer,
            pass: self.pass,
            sprite,
            quad: [corner(-h, -h), corner(h, -h), corner(h, h), corner(-h, h)],
            uv: [
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            tint,
            anchor: at,
            tag,
        });
    }
}

fn rgba([r, g, b]: [f32; 3]) -> [f32; 4] {
    [r, g, b, 1.0]
}

/// A connector's state in PoB (`getState`, PassiveTreeView.lua:661-673)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineState {
    Normal,
    Intermediate,
    Active,
}

impl LineState {
    fn name(self) -> &'static str {
        match self {
            LineState::Normal => "Normal",
            LineState::Intermediate => "Intermediate",
            LineState::Active => "Active",
        }
    }
}

/// The state and colour PoB draws a connector in (renderConnector, PassiveTreeView.lua:674-712)
pub fn connector_look(
    connector: &Connector,
    sets: &DrawSets,
    look: &Look,
) -> (LineState, [f32; 4]) {
    let [a, b] = connector.ids;
    let alloc = |id: &u32| sets.allocated.contains(id);
    let state = if alloc(&a) && alloc(&b) {
        LineState::Active
    } else if let Some(path) = look.path {
        let on = |id: &u32| alloc(id) || look.hover == Some(*id) || path.contains(id);
        if on(&a) && on(&b) {
            LineState::Intermediate
        } else {
            LineState::Normal
        }
    } else {
        LineState::Normal
    };
    let mut colour = if sets.split.contains(&a) && sets.split.contains(&b) {
        rgba(SPLIT_PERSONALITY)
    } else {
        WHITE
    };
    let mut state = state;
    // the compared spec's state, where it has both nodes: only its allocation counts, as
    // hoverPath holds this spec's nodes
    if sets.comparing && !sets.compare_missing.contains(&a) && !sets.compare_missing.contains(&b) {
        let base_active =
            sets.compare_allocated.contains(&a) && sets.compare_allocated.contains(&b);
        if base_active && state != LineState::Active {
            state = LineState::Active;
            colour = GREEN;
        } else if !base_active && state == LineState::Active {
            colour = RED;
        }
    }
    if look.deps.contains(&a) && look.deps.contains(&b) {
        colour = RED;
    } else if connector
        .ascendancy
        .as_deref()
        .is_some_and(|asc| Some(asc) != sets.ascendancy.as_deref())
    {
        colour = [0.75, 0.75, 0.75, 1.0];
    }
    (state, colour)
}

/// The art of a connector in a state: the abyss art where both ends are conquered and it
/// exists (PassiveTreeView.lua:715-719)
fn connector_art(layout: &TreeLayout, connector: &Connector, state: LineState) -> Option<Sprite> {
    let name = format!("{}{}", connector.kind, state.name());
    let abyss = connector
        .abyss
        .then(|| layout.assets.get(&format!("Abyss{name}")))
        .flatten();
    abyss.or_else(|| layout.assets.get(&name)).copied()
}

/// A connector's quad and texture coordinates in a state (PassiveTree:BuildConnector, BuildArc)
pub fn connector_quad(
    layout: &TreeLayout,
    connector: &Connector,
    state: LineState,
) -> Option<([Vec2; 4], [Vec2; 4])> {
    if let (Some(centre), Some(rel)) = (connector.centre, connector.quad) {
        // the arc's quad scales with the state's art (BuildArc: width * 2 * 1.33)
        let art = layout
            .assets
            .get(&format!("{}{}", connector.kind, state.name()))?;
        let size = art.rect[2] * 2.0 * ART_SCALE;
        let c = Vec2::from(centre);
        let p = connector.p;
        Some((
            [
                c,
                c + Vec2::from(rel[0]) * size,
                c + Vec2::from(rel[1]) * size,
                c + Vec2::from(rel[2]) * size,
            ],
            [
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, p),
                Vec2::new(0.0, 0.0),
                Vec2::new(p, 0.0),
            ],
        ))
    } else {
        // a line, as wide as the Normal art is tall; its art repeats along it
        let art = layout.assets.get("LineConnectorNormal")?;
        let (a, b) = (Vec2::from(connector.ends[0]), Vec2::from(connector.ends[1]));
        let v = b - a;
        let dist = v.length();
        if dist <= 0.0 {
            return None;
        }
        let n = v * (art.rect[3] * ART_SCALE / dist);
        let end = dist / (art.rect[2] * ART_SCALE);
        Some((
            [
                Vec2::new(a.x - n.y, a.y + n.x),
                Vec2::new(a.x + n.y, a.y - n.x),
                Vec2::new(b.x + n.y, b.y - n.x),
                Vec2::new(b.x - n.y, b.y + n.x),
            ],
            [
                Vec2::new(0.0, 1.0),
                Vec2::new(0.0, 0.0),
                Vec2::new(end, 0.0),
                Vec2::new(end, 1.0),
            ],
        ))
    }
}

/// How PoB picks a node's draw state (PassiveTreeView.lua:832-842)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Unallocated = 0,
    Path = 1,
    Allocated = 2,
}

pub fn node_state(id: u32, is_alloc: bool, look: &Look) -> NodeState {
    if look.heat.is_some() || is_alloc || look.hover == Some(id) || look.trace_end == Some(id) {
        NodeState::Allocated
    } else if look.path.is_some_and(|p| p.contains(&id)) {
        NodeState::Path
    } else {
        NodeState::Unallocated
    }
}

/// The colour PoB draws a node's base and effect with (PassiveTreeView.lua:906-946)
pub fn node_colour(node: &TreeNode, is_alloc: bool, sets: &DrawSets, look: &Look) -> [f32; 4] {
    let start = matches!(node.kind.as_str(), "ClassStart" | "AscendClassStart");
    if let Some(heat) = look.heat
        && !is_alloc
        && !start
    {
        return match heat {
            Some((power, index)) => rgba(overlays::heat_rgb(
                power,
                index.get(&node.id).map(|&i| &power.nodes[i]),
                look.power_theme,
            )),
            // nothing calculated yet: PoB's numbers are all 0
            None => [0.0, 0.0, 0.0, 1.0],
        };
    }
    match sets.compare_colour.get(&node.id) {
        Some(rgb) => rgba(*rgb),
        None if sets.split.contains(&node.id) => rgba(SPLIT_PERSONALITY),
        None => WHITE,
    }
}

/// The colour of the radius of the hovered socket that holds `node`, if any
/// (PassiveTreeView.lua:950-981)
fn radius_colour(
    layout: &TreeLayout,
    node: &TreeNode,
    socket: &TreeNode,
    variable: bool,
) -> Option<[f32; 4]> {
    if !node.in_radius {
        return None;
    }
    let d2 = (Vec2::from(node.pos) - Vec2::from(socket.pos)).length_sq();
    layout
        .jewel_radii
        .iter()
        .filter(|r| d2 <= r.outer * r.outer && r.inner * r.inner <= d2)
        .find(|r| (r.inner != 0.0) == variable)
        .map(|r| {
            let [red, g, b] = r.colour.map(|c| f32::from(c) / 255.0);
            [red, g, b, 1.0]
        })
}

/// Everything PoB draws for the tree, in its order and layers. `index` finds a node of the
/// layout by id.
pub fn build(
    layout: &TreeLayout,
    state: &TreeState,
    sets: &DrawSets,
    index: &HashMap<u32, usize>,
    look: &Look,
) -> Vec<Image> {
    let mut scene = Scene {
        images: Vec::with_capacity(layout.nodes.len() * 2 + layout.connectors.len()),
        layer: LAYER_BACK,
        pass: 0,
        view: look.view,
        _marker: std::marker::PhantomData,
    };

    // the background, repeating every 2.5 of its sizes, fixed to the tree's origin
    // (PassiveTreeView.lua:536-545)
    let background = layout
        .assets
        .get("Background2")
        .or_else(|| layout.assets.get("Background1"));
    if let Some(&bg) = background {
        let tile = bg.rect[2] * ART_SCALE * 2.5;
        if tile > 0.0 {
            let v = look.view;
            let corners = [
                v.left_top(),
                v.right_top(),
                v.right_bottom(),
                v.left_bottom(),
            ]
            .map(|p| p.to_vec2());
            scene.images.push(Image {
                layer: LAYER_BACK,
                pass: 0,
                sprite: bg,
                quad: corners,
                uv: corners.map(|c| c / tile),
                tint: WHITE,
                anchor: Vec2::ZERO,
                tag: Tag::Background,
            });
        }
    }
    // the class's background art
    if let Some(art) = layout.class_art.iter().find(|a| a.class == sets.class_id) {
        scene.draw_asset(Some(art.art), Vec2::from(art.pos), WHITE, Tag::ClassArt);
    }
    // the group backgrounds (renderGroup, PassiveTreeView.lua:566-660)
    let group = |scene: &mut Scene, g: &GroupArt, tint: [f32; 4]| {
        let at = Vec2::from(g.pos);
        if let Some(asc) = &g.ascendancy {
            let chosen =
                Some(asc) == sets.ascendancy.as_ref() || Some(asc) == sets.bloodline.as_ref();
            let tint = if chosen {
                tint
            } else {
                [tint[0], tint[1], tint[2], 0.5]
            };
            scene.draw_asset(Some(g.art), at, tint, Tag::Group);
        } else if g.half {
            scene.draw_half(g.art, at, tint, Tag::Group);
        } else {
            scene.draw_asset(Some(g.art), at, tint, Tag::Group);
        }
    };
    for g in &layout.groups {
        group(&mut scene, g, WHITE);
    }
    for g in &state.compare_only_groups {
        if let Some(g) = g.resolve(&layout.sheets) {
            group(&mut scene, &g, COMPARE_ONLY_GROUP);
        }
    }

    // the connectors
    scene.layer = LAYER_CONNECTOR;
    let connector = |scene: &mut Scene, c: &Connector, state: LineState, tint: [f32; 4]| {
        let (Some(art), Some((quad, uv))) = (
            connector_art(layout, c, state),
            connector_quad(layout, c, state),
        ) else {
            return;
        };
        let (a, b) = (Vec2::from(c.ends[0]), Vec2::from(c.ends[1]));
        let reach = (b - a).length() + art.rect[2].max(art.rect[3]) * ART_SCALE * 2.0;
        if !scene.visible((a + b) / 2.0, reach) {
            return;
        }
        scene.images.push(Image {
            layer: LAYER_CONNECTOR,
            pass: 0,
            sprite: art,
            quad,
            uv,
            tint,
            anchor: quad[0],
            tag: Tag::Connector(c.ids),
        });
    };
    for c in &layout.connectors {
        let (state, tint) = connector_look(c, sets, look);
        connector(&mut scene, c, state, tint);
    }
    for c in &state.compare_only_connectors {
        connector(&mut scene, c, LineState::Active, GREEN);
    }

    // the nodes (PassiveTreeView.lua:805-1009)
    let hovered = look
        .hover
        .and_then(|id| index.get(&id))
        .map(|&i| &layout.nodes[i]);
    let hovered_socket = hovered.filter(|n| n.kind == "Socket" && n.radius_socket);
    let ring_size = 175.0 * 2.0 / look.pob_zoom.max(f32::EPSILON).powf(0.4);
    for node in &layout.nodes {
        let at = Vec2::from(node.pos);
        let id = node.id;
        if !scene.visible(at, 400.0) {
            if sets.search.contains(&id) {
                scene.layer = LAYER_SEARCH;
                let ring = layout.assets.get("highlightRing").copied();
                scene.draw_square(ring, at, ring_size, 0.0, RED, Tag::SearchRing(id));
            }
            continue;
        }
        let is_alloc = sets.allocated.contains(&id)
            || sets.granted.contains(&id)
            || sets.compare_allocated.contains(&id);
        let state = node_state(id, is_alloc, look);
        let art = &node.art;
        let start = matches!(node.kind.as_str(), "ClassStart" | "AscendClassStart");
        let (base, overlay, effect) = match node.kind.as_str() {
            "ClassStart" | "AscendClassStart" => {
                (None, art.overlay[if is_alloc { 2 } else { 0 }], None)
            }
            "Socket" => {
                let overlay = sets
                    .socket_overlay
                    .get(&id)
                    .filter(|_| is_alloc)
                    .and_then(|name| layout.assets.get(name).copied());
                (art.base[state as usize], overlay, None)
            }
            "Mastery" => {
                let base = if is_alloc {
                    art.base[2]
                } else if look.hover == Some(id) && node.mastery_effects {
                    art.hover
                } else {
                    art.base[0]
                };
                (base, None, art.effect[usize::from(is_alloc)])
            }
            _ => (
                art.base[if is_alloc { 2 } else { 0 }],
                art.overlay[state as usize],
                art.effect[usize::from(is_alloc)],
            ),
        };
        let mut layer = if node.kind == "Mastery" {
            LAYER_MASTERY
        } else {
            LAYER_NODE
        };
        let colour = node_colour(node, is_alloc, sets, look);
        scene.pass = 0;
        if effect.is_some() {
            scene.layer = LAYER_MASTERY;
            scene.draw_asset(effect, at, colour, Tag::Node(id));
            layer = LAYER_NODE;
        }
        scene.layer = layer;
        // a mastery's icon over effects, a frame over icons
        scene.pass = 1;
        let mut current = colour;
        if base.is_some() {
            if node.kind == "Socket" && look.deps.contains(&id) {
                scene.draw_asset(base, at, RED, Tag::Node(id));
                current = WHITE;
            } else {
                scene.draw_asset(base, at, colour, Tag::Node(id));
            }
        }
        if overlay.is_some() {
            if !start && let Some(h) = hovered.filter(|h| h.id != id) {
                if look.deps.contains(&id) {
                    current = RED;
                } else if let Some(socket) = hovered_socket.filter(|s| s.id == h.id) {
                    let variable = sets.variable.contains(&socket.id);
                    if let Some(c) = radius_colour(layout, node, socket, variable) {
                        current = c;
                    }
                }
            }
            scene.pass = 2;
            scene.draw_asset(overlay, at, current, Tag::Node(id));
        }
        if sets.search.contains(&id) {
            scene.layer = LAYER_SEARCH;
            let ring = layout.assets.get("highlightRing").copied();
            scene.draw_square(ring, at, ring_size, 0.0, RED, Tag::SearchRing(id));
        }
    }

    // the compared spec's own nodes, green (PassiveTreeView.lua:1102-1141)
    scene.layer = LAYER_NODE;
    for node in &state.compare_only {
        let at = Vec2::new(node.x, node.y);
        let Some(art) = &node.art else {
            continue;
        };
        let base = art.base.and_then(|s| s.resolve(&layout.sheets));
        let overlay = art.overlay.and_then(|s| s.resolve(&layout.sheets));
        scene.pass = 3;
        scene.draw_asset(base, at, GREEN, Tag::CompareOnly(node.id));
        scene.pass = 4;
        scene.draw_asset(overlay, at, GREEN, Tag::CompareOnly(node.id));
    }

    // the sockets' radius rings (PassiveTreeView.lua:1198-1248)
    let time = look.time_ms.map(|ms| (ms * 0.00003) as f32);
    scene.pass = 5;
    for socket in &state.socket_rings {
        let Some(node) = index.get(&socket.node).map(|&i| &layout.nodes[i]) else {
            continue;
        };
        let at = Vec2::from(node.pos);
        if look.hover == Some(socket.node) {
            let ring = layout.assets.get("ring").copied();
            for r in &layout.jewel_radii {
                let [red, g, b] = r.colour.map(|c| f32::from(c) / 255.0);
                let tint = [red, g, b, 1.0];
                if socket.variable {
                    if r.inner != 0.0 {
                        let tag = Tag::HoverRing(socket.node);
                        scene.draw_square(ring, at, r.outer * 2.0, 0.0, tint, tag);
                        scene.draw_square(ring, at, r.inner * 2.0, 0.0, tint, tag);
                    }
                } else if r.inner == 0.0 {
                    let tag = Tag::HoverRing(socket.node);
                    scene.draw_square(ring, at, r.outer * 2.0, 0.0, tint, tag);
                }
            }
        }
        for ring in &socket.rings {
            let angle = time.map_or(0.0, |t| ring.spin * t);
            let sprite = layout.assets.get(&ring.image).copied();
            let at = Vec2::new(ring.x, ring.y);
            let tag = Tag::JewelRing(socket.node);
            scene.draw_square(sprite, at, ring.size, angle, ring.tint, tag);
        }
    }

    // PoB draws by layer, each in the order drawn
    scene.images.sort_by_key(|i| i.layer);
    scene.images
}

#[cfg(test)]
mod tests {
    use super::super::geometry::tests::{layout, node};
    use super::*;
    use crate::engine::{GroupArt, JewelRadius, LuaGroupArt, LuaSprite, SocketRings};

    /// A sprite of `w` x `h` pixels on sheet `sheet`
    fn art(sheet: usize, w: f32, h: f32) -> Sprite {
        Sprite {
            sheet,
            rect: [0.0, 0.0, w, h],
        }
    }

    fn look<'a>(empty: &'a HashSet<u32>) -> Look<'a> {
        Look {
            hover: None,
            path: None,
            deps: empty,
            trace_end: None,
            heat: None,
            power_theme: PowerTheme::RedBlue,
            view: Rect::from_min_max(Pos2::new(-1e5, -1e5), Pos2::new(1e5, 1e5)),
            pob_zoom: 1.0,
            time_ms: None,
        }
    }

    fn line(a: u32, b: u32, ascendancy: Option<&str>) -> Connector {
        Connector {
            ids: [a, b],
            kind: "LineConnector".into(),
            ascendancy: ascendancy.map(Into::into),
            ends: [[0.0, 0.0], [100.0, 0.0]],
            centre: None,
            quad: None,
            p: 0.0,
            abyss: false,
        }
    }

    fn sets(allocated: &[u32]) -> DrawSets {
        DrawSets {
            allocated: allocated.iter().copied().collect(),
            ascendancy: Some("Raider".into()),
            ..DrawSets::default()
        }
    }

    use eframe::egui::Pos2;

    /// renderConnector (PassiveTreeView.lua:661-712): the state, the compared spec's
    /// green and red, the dependents' red and the ascendancy fade
    #[test]
    fn connectors_take_pobs_state_and_colour() {
        let empty = HashSet::new();
        let l = look(&empty);
        let s = sets(&[1, 2]);
        assert_eq!(
            connector_look(&line(1, 2, None), &s, &l),
            (LineState::Active, WHITE)
        );
        assert_eq!(
            connector_look(&line(2, 3, None), &s, &l),
            (LineState::Normal, WHITE)
        );
        // on the hovered node's path: both ends allocated, hovered or on the path
        let path: HashSet<u32> = [4].into();
        let l = Look {
            hover: Some(3),
            path: Some(&path),
            ..look(&empty)
        };
        assert_eq!(
            connector_look(&line(2, 3, None), &s, &l).0,
            LineState::Intermediate
        );
        assert_eq!(
            connector_look(&line(3, 4, None), &s, &l).0,
            LineState::Intermediate
        );
        assert_eq!(
            connector_look(&line(4, 5, None), &s, &l).0,
            LineState::Normal
        );
        // a hovered node without a path: no Intermediate
        let l = Look {
            hover: Some(3),
            ..look(&empty)
        };
        assert_eq!(
            connector_look(&line(2, 3, None), &s, &l).0,
            LineState::Normal
        );
        // compared: Active there only is green and Active, Active here only is red
        let l = look(&empty);
        let mut c = sets(&[1, 2]);
        c.comparing = true;
        c.compare_allocated = [2, 3].into();
        assert_eq!(
            connector_look(&line(2, 3, None), &c, &l),
            (LineState::Active, GREEN)
        );
        assert_eq!(
            connector_look(&line(1, 2, None), &c, &l),
            (LineState::Active, RED)
        );
        // a node the compared spec lacks: its own state
        c.compare_missing = [1].into();
        assert_eq!(
            connector_look(&line(1, 2, None), &c, &l),
            (LineState::Active, WHITE)
        );
        // other ascendancies, the bloodline too, at 0.75; dependents red first
        let grey = [0.75, 0.75, 0.75, 1.0];
        assert_eq!(connector_look(&line(5, 6, Some("Raider")), &s, &l).1, WHITE);
        assert_eq!(connector_look(&line(5, 6, Some("Deadeye")), &s, &l).1, grey);
        let mut b = sets(&[]);
        b.bloodline = Some("Trialmaster".into());
        assert_eq!(
            connector_look(&line(5, 6, Some("Trialmaster")), &b, &l).1,
            grey
        );
        let deps: HashSet<u32> = [1, 2].into();
        let l = Look {
            deps: &deps,
            ..look(&empty)
        };
        assert_eq!(connector_look(&line(1, 2, Some("Deadeye")), &s, &l).1, RED);
        // Split Personality's path
        let mut sp = sets(&[]);
        sp.split = [7, 8].into();
        let l = look(&empty);
        assert_eq!(
            connector_look(&line(7, 8, None), &sp, &l).1,
            rgba(SPLIT_PERSONALITY)
        );
    }

    /// BuildConnector and BuildArc: a line is the Normal art's height (times 1.33) to each
    /// side, its art repeating along it; an arc's quad grows with the state's art
    #[test]
    fn connector_quads_follow_the_art() {
        let mut l = layout(vec![], vec![]);
        l.assets
            .insert("LineConnectorNormal".into(), art(0, 736.0, 16.0));
        l.assets.insert("Orbit2Normal".into(), art(0, 50.0, 50.0));
        l.assets.insert("Orbit2Active".into(), art(0, 60.0, 60.0));
        let (quad, uv) = connector_quad(&l, &line(1, 2, None), LineState::Active).unwrap();
        let half = 16.0 * ART_SCALE;
        assert_eq!(quad[0], Vec2::new(0.0, half));
        assert_eq!(quad[1], Vec2::new(0.0, -half));
        assert_eq!(quad[2], Vec2::new(100.0, -half));
        assert_eq!(uv[2], Vec2::new(100.0 / (736.0 * ART_SCALE), 0.0));
        let arc = Connector {
            kind: "Orbit2".into(),
            centre: Some([10.0, 20.0]),
            quad: Some([[0.5, 0.0], [0.5, -0.5], [0.0, -0.5]]),
            p: 0.25,
            ..line(1, 2, None)
        };
        let (quad, uv) = connector_quad(&l, &arc, LineState::Active).unwrap();
        let size = 60.0 * 2.0 * ART_SCALE;
        assert_eq!(quad[0], Vec2::new(10.0, 20.0));
        assert_eq!(quad[1], Vec2::new(10.0 + 0.5 * size, 20.0));
        assert_eq!(
            uv,
            [
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 0.25),
                Vec2::new(0.0, 0.0),
                Vec2::new(0.25, 0.0)
            ]
        );
        // without the state's art nothing is drawn
        assert!(connector_quad(&l, &arc, LineState::Intermediate).is_none());
    }

    fn of(images: &[Image], tag: Tag) -> Vec<&Image> {
        images.iter().filter(|i| i.tag == tag).collect()
    }

    /// The node's art in each state, as PoB picks it (PassiveTreeView.lua:805-904)
    #[test]
    fn nodes_draw_their_art_for_the_state() {
        let mut n = node(1, [0.0, 0.0], "Notable");
        n.art.base = [
            Some(art(0, 10.0, 10.0)),
            Some(art(0, 10.0, 10.0)),
            Some(art(1, 10.0, 10.0)),
        ];
        n.art.overlay = [
            Some(art(2, 20.0, 20.0)),
            Some(art(3, 20.0, 20.0)),
            Some(art(4, 20.0, 20.0)),
        ];
        let mut m = node(2, [500.0, 0.0], "Mastery");
        m.mastery_effects = true;
        m.art.base = [
            Some(art(5, 8.0, 8.0)),
            Some(art(5, 8.0, 8.0)),
            Some(art(6, 8.0, 8.0)),
        ];
        m.art.hover = Some(art(7, 8.0, 8.0));
        m.art.effect = [None, Some(art(8, 30.0, 30.0))];
        let mut start = node(3, [900.0, 0.0], "ClassStart");
        start.art.overlay = [
            Some(art(9, 5.0, 5.0)),
            Some(art(9, 5.0, 5.0)),
            Some(art(10, 5.0, 5.0)),
        ];
        let l = layout(vec![n, m, start], vec![]);
        let index: HashMap<u32, usize> = [(1, 0), (2, 1), (3, 2)].into();
        let state = crate::engine::fake::empty_tree_state();
        let empty = HashSet::new();
        let sheets = |images: &[Image], tag| -> Vec<(u8, usize)> {
            of(images, tag)
                .iter()
                .map(|i| (i.layer, i.sprite.sheet))
                .collect()
        };
        // nothing allocated or hovered
        let images = build(&l, &state, &sets(&[]), &index, &look(&empty));
        assert_eq!(
            sheets(&images, Tag::Node(1)),
            [(LAYER_NODE, 0), (LAYER_NODE, 2)]
        );
        // a mastery's base goes under the connectors
        assert_eq!(sheets(&images, Tag::Node(2)), [(LAYER_MASTERY, 5)]);
        assert_eq!(sheets(&images, Tag::Node(3)), [(LAYER_NODE, 9)]);
        // on the path: the CanAllocate frame
        let path: HashSet<u32> = [1].into();
        let l2 = Look {
            path: Some(&path),
            ..look(&empty)
        };
        let images = build(&l, &state, &sets(&[]), &index, &l2);
        assert_eq!(
            sheets(&images, Tag::Node(1)),
            [(LAYER_NODE, 0), (LAYER_NODE, 3)]
        );
        // hovered: drawn allocated, but the icon stays the unallocated one; a hovered mastery
        // shows its connected art
        for hover in [1, 2] {
            let images = build(
                &l,
                &state,
                &sets(&[]),
                &index,
                &Look {
                    hover: Some(hover),
                    ..look(&empty)
                },
            );
            if hover == 1 {
                assert_eq!(
                    sheets(&images, Tag::Node(1)),
                    [(LAYER_NODE, 0), (LAYER_NODE, 4)]
                );
            } else {
                assert_eq!(sheets(&images, Tag::Node(2)), [(LAYER_MASTERY, 7)]);
            }
        }
        // allocated: the active icon and frame; the mastery's effect below, its icon above
        let images = build(&l, &state, &sets(&[1, 2, 3]), &index, &look(&empty));
        assert_eq!(
            sheets(&images, Tag::Node(1)),
            [(LAYER_NODE, 1), (LAYER_NODE, 4)]
        );
        assert_eq!(
            sheets(&images, Tag::Node(2)),
            [(LAYER_MASTERY, 8), (LAYER_NODE, 6)]
        );
        assert_eq!(sheets(&images, Tag::Node(3)), [(LAYER_NODE, 10)]);
        // the compared spec's allocation counts as allocated
        let mut c = sets(&[]);
        c.compare_allocated = [1].into();
        let images = build(&l, &state, &c, &index, &look(&empty));
        assert_eq!(
            sheets(&images, Tag::Node(1)),
            [(LAYER_NODE, 1), (LAYER_NODE, 4)]
        );
        // the art is PoB's size: 1.33 tree units a pixel each side of the node
        let frame = of(&images, Tag::Node(1))[1];
        assert_eq!(frame.quad[0], Vec2::splat(-20.0 * ART_SCALE));
        assert_eq!(frame.quad[2], Vec2::splat(20.0 * ART_SCALE));
    }

    /// The node colours: the heat map's, the comparison's, the dependents' red on the frame
    #[test]
    fn nodes_are_tinted_as_pob_tints_them() {
        let mut n = node(1, [0.0, 0.0], "Normal");
        n.art.base = [Some(art(0, 10.0, 10.0)); 3];
        n.art.overlay = [Some(art(1, 20.0, 20.0)); 3];
        let l = layout(vec![n.clone()], vec![]);
        let index: HashMap<u32, usize> = [(1, 0)].into();
        let state = crate::engine::fake::empty_tree_state();
        let empty = HashSet::new();
        let tints = |s: &DrawSets, look: &Look| -> Vec<[f32; 4]> {
            build(&l, &state, s, &index, look)
                .iter()
                .filter(|i| i.tag == Tag::Node(1))
                .map(|i| i.tint)
                .collect()
        };
        assert_eq!(tints(&sets(&[]), &look(&empty)), [WHITE, WHITE]);
        let mut c = sets(&[]);
        c.compare_colour = [(1, [0.0, 1.0, 0.0])].into();
        assert_eq!(tints(&c, &look(&empty)), [GREEN, GREEN]);
        // the heat map: an unallocated node takes its power's colour, none calculated is black
        let power = NodePower {
            stat: Some("Life".into()),
            nodes: vec![crate::engine::NodePowerEntry {
                id: 1,
                single: 50.0,
                path: None,
                offence: None,
                defence: None,
            }],
            max_single: 100.0,
            max_offence: 0.0,
            max_defence: 0.0,
            finished: true,
        };
        let power_index: HashMap<u32, usize> = [(1, 0)].into();
        let heat = Look {
            heat: Some(Some((&power, &power_index))),
            ..look(&empty)
        };
        let red = (0.75_f32).sqrt();
        assert_eq!(tints(&sets(&[]), &heat), [[red, 0.0, 0.0, 1.0]; 2]);
        let heat_none = Look {
            heat: Some(None),
            ..look(&empty)
        };
        assert_eq!(tints(&sets(&[]), &heat_none), [[0.0, 0.0, 0.0, 1.0]; 2]);
        // allocated, the heat map leaves it alone
        assert_eq!(tints(&sets(&[1]), &heat), [WHITE, WHITE]);
        // a dependent of the hovered node: its frame red
        let mut l2 = layout(vec![n.clone(), node(2, [500.0, 0.0], "Normal")], vec![]);
        l2.nodes[1].art = n.art.clone();
        let index2: HashMap<u32, usize> = [(1, 0), (2, 1)].into();
        let deps: HashSet<u32> = [1, 2].into();
        let hovered = Look {
            hover: Some(2),
            deps: &deps,
            ..look(&empty)
        };
        let images = build(&l2, &state, &sets(&[1, 2]), &index2, &hovered);
        let t: Vec<_> = of(&images, Tag::Node(1)).iter().map(|i| i.tint).collect();
        assert_eq!(t, [WHITE, RED]);
        // ... but not the hovered node's own
        let t: Vec<_> = of(&images, Tag::Node(2)).iter().map(|i| i.tint).collect();
        assert_eq!(t, [WHITE, WHITE]);
    }

    /// A hovered socket colours the frames in its radius, and draws its radii's rings
    /// (PassiveTreeView.lua:950-981, 1209-1230)
    #[test]
    fn a_hovered_socket_shows_its_radius() {
        let mut socket = node(1, [0.0, 0.0], "Socket");
        socket.radius_socket = true;
        socket.art.base = [Some(art(0, 10.0, 10.0)); 3];
        let mut near = node(2, [500.0, 0.0], "Normal");
        near.in_radius = true;
        near.art.overlay = [Some(art(1, 20.0, 20.0)); 3];
        let mut far = near.clone();
        far.id = 3;
        far.pos = [1500.0, 0.0];
        let mut l = layout(vec![socket, near, far], vec![]);
        l.jewel_radii = vec![
            JewelRadius {
                inner: 0.0,
                outer: 960.0,
                colour: [255, 0, 0],
            },
            JewelRadius {
                inner: 0.0,
                outer: 1800.0,
                colour: [0, 0, 255],
            },
            JewelRadius {
                inner: 960.0,
                outer: 1320.0,
                colour: [0, 255, 0],
            },
        ];
        l.assets.insert("ring".into(), art(5, 64.0, 64.0));
        let index: HashMap<u32, usize> = [(1, 0), (2, 1), (3, 2)].into();
        let mut state = crate::engine::fake::empty_tree_state();
        state.socket_rings = vec![SocketRings {
            node: 1,
            variable: false,
            rings: vec![],
        }];
        let empty = HashSet::new();
        let hovered = Look {
            hover: Some(1),
            ..look(&empty)
        };
        let images = build(&l, &state, &sets(&[]), &index, &hovered);
        assert_eq!(of(&images, Tag::Node(2))[0].tint, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(of(&images, Tag::Node(3))[0].tint, [0.0, 0.0, 1.0, 1.0]);
        let rings: Vec<f32> = of(&images, Tag::HoverRing(1))
            .iter()
            .map(|i| i.quad[2].x - i.quad[0].x)
            .collect();
        assert_eq!(rings, [1920.0, 3600.0]);
        // Thread of Hope: the annuli
        let mut s = sets(&[]);
        s.variable = [1].into();
        state.socket_rings[0].variable = true;
        let images = build(&l, &state, &s, &index, &hovered);
        assert_eq!(of(&images, Tag::Node(2))[0].tint, WHITE);
        let far_ring = of(&images, Tag::Node(3))[0].tint;
        assert_eq!(far_ring, WHITE);
        assert_eq!(of(&images, Tag::HoverRing(1)).len(), 2);
    }

    /// The group art (renderGroup): a half background mirrored below; an ascendancy's emblem
    /// faded unless it is the build's or its bloodline; a compare-only cluster's green at 0.6
    #[test]
    fn groups_draw_their_backgrounds() {
        let mut l = layout(vec![], vec![]);
        l.sheets = vec![None, None];
        let g = |half, ascendancy: Option<&str>| GroupArt {
            pos: [0.0, 100.0],
            art: art(0, 10.0, 5.0),
            half,
            ascendancy: ascendancy.map(Into::into),
            flavour: None,
        };
        l.groups = vec![
            g(true, None),
            g(false, Some("Raider")),
            g(false, Some("Deadeye")),
            g(false, Some("Trialmaster")),
        ];
        let mut state = crate::engine::fake::empty_tree_state();
        state.compare_only_groups = vec![LuaGroupArt {
            x: 0.0,
            y: 0.0,
            art: LuaSprite {
                sheet: 1,
                x: Some(0.0),
                y: Some(0.0),
                w: Some(4.0),
                h: Some(4.0),
            },
            half: false,
            ascendancy: None,
            flavour: None,
        }];
        let empty = HashSet::new();
        let mut s = sets(&[]);
        s.bloodline = Some("Trialmaster".into());
        // the compare-only group's sheet isn't loaded: left out
        let images = build(&l, &state, &s, &HashMap::new(), &look(&empty));
        let groups = of(&images, Tag::Group);
        assert_eq!(groups.len(), 5);
        let (top, bottom) = (groups[0], groups[1]);
        assert_eq!(top.quad[2].y, 100.0);
        assert_eq!(bottom.quad[0].y, 100.0);
        assert_eq!(bottom.uv[0], Vec2::new(0.0, 1.0));
        let alphas: Vec<f32> = groups[2..].iter().map(|g| g.tint[3]).collect();
        assert_eq!(alphas, [1.0, 0.5, 1.0]);
        let sheet = std::sync::Arc::new(crate::engine::SheetImage::new(
            "x.png".into(),
            [4, 4],
            vec![0; 64],
        ));
        l.sheets = vec![Some(sheet.clone()), Some(sheet)];
        let images = build(&l, &state, &s, &HashMap::new(), &look(&empty));
        assert_eq!(
            of(&images, Tag::Group).last().unwrap().tint,
            COMPARE_ONLY_GROUP
        );
        // drawn first: the background layer
        assert!(images.iter().all(|i| i.layer == LAYER_BACK));
    }

    /// A cluster jewel only the compared spec has: its connectors in the Active art, green,
    /// whatever this spec allocates (PassiveTreeView.lua:747-770)
    #[test]
    fn compare_only_connectors_are_green_and_active() {
        let mut l = layout(vec![], vec![]);
        l.assets
            .insert("LineConnectorNormal".into(), art(0, 736.0, 16.0));
        l.assets
            .insert("LineConnectorActive".into(), art(1, 736.0, 16.0));
        let mut state = crate::engine::fake::empty_tree_state();
        state.compare_only_connectors = vec![line(70_000, 70_001, None)];
        let empty = HashSet::new();
        let images = build(&l, &state, &sets(&[]), &HashMap::new(), &look(&empty));
        let drawn = of(&images, Tag::Connector([70_000, 70_001]));
        assert_eq!(drawn.len(), 1);
        assert_eq!((drawn[0].sprite.sheet, drawn[0].tint), (1, GREEN));
        assert_eq!(drawn[0].layer, LAYER_CONNECTOR);
    }

    /// PoB's search ring: 175 / zoom^0.4 each side, red, above the nodes
    #[test]
    fn search_rings_follow_pobs_size() {
        let mut l = layout(vec![node(1, [0.0, 0.0], "Normal")], vec![]);
        l.assets.insert("highlightRing".into(), art(0, 32.0, 32.0));
        let index: HashMap<u32, usize> = [(1, 0)].into();
        let state = crate::engine::fake::empty_tree_state();
        let mut s = sets(&[]);
        s.search = [1].into();
        let empty = HashSet::new();
        let l2 = Look {
            pob_zoom: 2.0,
            ..look(&empty)
        };
        let images = build(&l, &state, &s, &index, &l2);
        let ring = of(&images, Tag::SearchRing(1))[0];
        assert_eq!(ring.layer, LAYER_SEARCH);
        assert_eq!(ring.tint, RED);
        let width = ring.quad[2].x - ring.quad[0].x;
        assert!((width - 350.0 / 2.0_f32.powf(0.4)).abs() < 1e-3, "{width}");
        assert_eq!(images.last().unwrap().tag, Tag::SearchRing(1));
    }
}
