//! Draws the passive tree and turns pointer input into [`CanvasAction`]s: pan by dragging, zoom
//! with the scroll wheel, click to allocate.

use super::TreeTab;
use super::geometry::{MIN_HIT_POINTS, arc_points, node_radius};
use super::overlays::{self, ALLOCATED, Legend};
use super::scene;
use crate::engine::{
    CompareOnlyNode, NodePower, NodeTooltip, SheetPixels, TooltipLine, TreeLayout, TreeNode,
    TreeState,
};
use crate::theme;
use crate::widgets::tooltip_ui;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Tooltip, Vec2};
use std::collections::HashSet;

/// What the user did on the canvas this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasAction {
    Alloc(u32),
    Dealloc(u32),
    /// Open the effect picker of a mastery
    PickMastery(u32),
    /// Open the node replacement window of a node (R18)
    ReplaceNode(u32),
    /// Show the Items tab with the slot of an allocated jewel socket (R19)
    JumpToSocket(u32),
    /// The node under the pointer changed
    Hover(Option<u32>),
    /// A click on the end of the path traced with Shift: allocate that path (`AllocTracePath`)
    AllocTrace(u32, Vec<u32>),
}

/// An uploaded sprite sheet: its texture and size in pixels.
type Sheet = (egui::TextureId, Vec2);

/// A connection in the socket viewer's plain drawing
const EDGE: Color32 = Color32::from_rgb(72, 62, 50);
/// A search ring snapped to the view's edge is this much smaller (PoB's `scaled_down_ratio`)
const EDGE_RING_SCALE: f32 = 0.6667;
/// How far a snapped search ring may stick out past the view's edge (PoB's `peekaboo_ratio`)
const PEEKABOO: f32 = 1.15;
/// PoB shows the ascendancies' flavour text from this zoom on (PassiveTreeView.lua:612)
const FLAVOUR_MIN_ZOOM: f32 = 2.5;
/// PoB's flavour text size at a scale of 1 (PassiveTreeView.lua:611)
const FLAVOUR_SIZE: f32 = 52.0;
/// While jewel rings turn, the canvas is redrawn this often
const RING_FRAME_MS: u64 = 200;

pub fn show(
    ui: &mut egui::Ui,
    tab: &mut TreeTab,
    layout: &TreeLayout,
    state: &TreeState,
    power: Option<&NodePower>,
) -> Vec<CanvasAction> {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    tab.rect = Some(rect);
    painter.rect_filled(rect, 0.0, theme::BACKGROUND);

    if tab.needs_fit || tab.camera.zoom == 0.0 {
        tab.camera.fit(rect, layout);
        tab.needs_fit = tab.camera.zoom == 0.0;
    }
    let valid_zoom = tab.camera.zoom > 0.0;
    if valid_zoom {
        // PoB's zoom range holds for the input too
        tab.camera.tree_size = layout.size;
        handle_input(ui, tab, &response);
        tab.camera.limit(rect, layout.size);
    }
    let camera = tab.camera;
    let zoom = camera.zoom;
    let to_screen = |p: [f32; 2]| camera.to_screen(rect, p);
    // drawing a little outside the view avoids popping at the edges
    let visible = rect.expand(50.0);

    // the node under the pointer; none while panning
    let under = response
        .hover_pos()
        .filter(|_| valid_zoom && !response.dragged())
        .and_then(|p| tab.grid.node_at(layout, camera.to_tree(rect, p), zoom));
    // Shift traces a path through the hovered nodes (PassiveTreeView.lua:283-363); a node
    // the trace can't take is not hovered
    let shift = ui.input(|i| i.modifiers.shift);
    let hovered_id = tab.trace_step(layout, under.map(|i| layout.nodes[i].id), shift);
    let hovered = hovered_id.and_then(|id| tab.index.get(&id).copied());
    // where none of ours is, a node of the compared spec only, Shift or not
    // (PassiveTreeView.lua:308-320)
    let compare_hover = response
        .hover_pos()
        .filter(|_| valid_zoom && !response.dragged() && under.is_none())
        .and_then(|p| compare_only_at(&state.compare_only, camera.to_tree(rect, p), zoom));
    let mut actions = Vec::new();
    let hover_id = hovered_id.or(compare_hover.map(|n| n.id));
    if hover_id != tab.hover {
        actions.push(CanvasAction::Hover(hover_id));
    }

    // what PoB draws with: the hovered node's path and dependents (PassiveTreeView.lua:323-376)
    let hover_sets = scene::hover_sets(
        hovered_id,
        shift.then_some(tab.trace.as_slice()),
        &tab.draw_sets,
        &tab.derived.paths,
        &tab.derived.depends,
    );
    // P shows the map before any stat was chosen: damage against defence
    let heat_on = state.heatmap.is_some() || state.heatmap_shown;
    let tree_view = Rect::from_min_max(
        camera.to_tree(rect, rect.min).into(),
        camera.to_tree(rect, rect.max).into(),
    );
    let now = ui.input(|i| i.time);
    let look = scene::Look {
        hover: hovered_id,
        path: hover_sets.path.as_ref(),
        deps: &hover_sets.deps,
        trace_end: shift.then(|| tab.trace.last().copied()).flatten(),
        heat: heat_on.then_some(power.map(|p| (p, &tab.power_index))),
        power_theme: tab.power_theme,
        view: tree_view,
        pob_zoom: pob_zoom(zoom, layout.size, rect),
        time_ms: (!tab.animations_off).then_some(now * 1000.0),
    };
    let images = scene::build(layout, state, &tab.draw_sets, &tab.index, &look);
    let pob_zoom = look.pob_zoom;
    // only the files this frame draws are decoded and uploaded
    let used: HashSet<usize> = images.iter().map(|i| i.sprite.sheet).collect();
    let sheets = upload_sheets(tab, ui.ctx(), layout, &used);
    paint(
        &painter,
        &sheets,
        &images,
        rect,
        camera,
        !tab.edge_search_off,
    );
    if !tab.animations_off
        && images
            .iter()
            .any(|i| matches!(i.tag, scene::Tag::JewelRing(_)))
    {
        // the jewel rings turn slowly; a few frames a second follow them
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(RING_FRAME_MS));
    }
    draw_flavour(&painter, layout, &tab.draw_sets, camera, rect, pob_zoom);
    // a node whose art couldn't be loaded is still shown, as a dot
    for node in &layout.nodes {
        let art = &node.art;
        if art.base.iter().chain(&art.overlay).any(Option::is_some) {
            continue;
        }
        let pos = to_screen(node.pos);
        if visible.contains(pos) {
            let colour = if tab.derived.allocated.contains(&node.id) {
                ALLOCATED
            } else {
                kind_color(&node.kind)
            };
            painter.circle_filled(pos, (node_radius(&node.kind) * zoom).max(1.5), colour);
        }
    }
    // a compare-only node whose art isn't on the layout's files (another tree version's) is
    // still shown, as a green dot
    for node in &state.compare_only {
        let drawn = node
            .art
            .as_ref()
            .and_then(|a| a.base.or(a.overlay))
            .and_then(|s| s.resolve(&layout.sheets))
            .is_some();
        let pos = to_screen([node.x, node.y]);
        if !drawn && visible.contains(pos) {
            let radius = (node_radius(&node.kind) * zoom).max(1.5);
            painter.circle_filled(pos, radius, Color32::from_rgb(0, 255, 0));
        }
    }
    // PoB draws no hover ring on a compare-only node, and no tooltip for a mastery without
    // effects or while Ctrl is held (PassiveTreeView.lua:1143)
    let ctrl_held = ui.input(|i| i.modifiers.command);
    if let Some(node) =
        compare_hover.filter(|n| !ctrl_held && (n.kind != "Mastery" || n.mastery_effects))
    {
        let reply = tab
            .tooltip
            .as_deref()
            .filter(|t| t.id == node.id && t.trace.is_empty() && !t.lines.is_empty());
        Tooltip::for_widget(&response)
            .at_pointer()
            .show(|ui| match reply {
                Some(t) => tooltip_ui(ui, &t.lines),
                None => {
                    ui.strong(if node.name.is_empty() {
                        &node.kind
                    } else {
                        &node.name
                    });
                }
            });
    }

    if let Some((id, start)) = tab.pulse {
        let age = now - start;
        match tab.index.get(&id) {
            Some(&i) if age < super::PULSE_SECONDS => {
                let node = &layout.nodes[i];
                let radius = (node_radius(&node.kind) * zoom).max(1.5);
                overlays::draw_pulse(&painter, to_screen(node.pos), radius, age);
                ui.ctx().request_repaint();
            }
            _ => tab.pulse = None,
        }
    }

    overlays::draw_legend(
        &painter,
        rect,
        &Legend {
            compare: !state.compare.is_empty(),
            searching: !state.search_matches.is_empty(),
            heat_stat: state.heatmap.as_deref(),
            heat: heat_on,
            progress: tab.progress,
            busy: tab.busy,
        },
    );

    let Some(i) = hovered else {
        return actions;
    };
    let node = &layout.nodes[i];

    let has_options = state
        .mastery_options
        .iter()
        .any(|m| m.node == node.id && !m.options.is_empty());
    let is_mastery = node.kind == "Mastery";
    let allocated = tab.derived.allocated.contains(&node.id);
    // a click with Ctrl held only zooms (PassiveTreeView.lua:259-263)
    let ctrl = ui.input(|i| i.modifiers.command);
    // as PoB's right click, in its order (PassiveTreeView.lua:510-533, R19)
    if response.secondary_clicked() && !ctrl {
        if allocated && node.kind == "Socket" {
            // only a slot PoB lets the user use
            if tab.sockets.contains(&node.id) {
                actions.push(CanvasAction::JumpToSocket(node.id));
            }
        } else if node.replaceable {
            actions.push(CanvasAction::ReplaceNode(node.id));
        } else if is_mastery && node.mastery_effects {
            actions.push(if allocated {
                CanvasAction::PickMastery(node.id)
            } else {
                CanvasAction::ReplaceNode(node.id)
            });
        }
    }
    if response.clicked() && !ctrl {
        // a left click on an allocated node removes it, a mastery included
        // (PassiveTreeView.lua:383); an unallocated mastery asks for its effect
        if allocated {
            actions.push(CanvasAction::Dealloc(node.id));
        } else if is_mastery {
            if has_options {
                actions.push(CanvasAction::PickMastery(node.id));
            }
        } else if tab.derived.paths.contains_key(&node.id)
            // a node of another ascendancy or bloodline has no path, but the click switches
            // to it (PassiveTreeView.lua:398-416); the engine says if that isn't possible
            || node
                .ascendancy
                .as_ref()
                .is_some_and(|a| !tab.derived.is_chosen(a))
        {
            // the trace's end allocates the trace (PassiveTreeView.lua:504)
            actions.push(match tab.trace.last() {
                Some(&end) if end == node.id => {
                    CanvasAction::AllocTrace(node.id, tab.trace.clone())
                }
                _ => CanvasAction::Alloc(node.id),
            });
        }
    }

    // PoB shows no tooltip while Ctrl is held, none for a socket while Shift is, and none for a
    // mastery without effects (PassiveTreeView.lua:1033)
    if ctrl || (node.kind == "Socket" && shift) || (is_mastery && !node.mastery_effects) {
        return actions;
    }
    let jewel = state.jewels.iter().find(|j| j.node == node.id);
    // a reply is the tooltip for this trace (or for none)
    let reply = tab.tooltip.as_deref().filter(|t| t.trace == tab.trace);
    let body = tooltip_body(node, reply);
    Tooltip::for_widget(&response)
        .at_pointer()
        .show(|ui| match body {
            // PoB's lines start with the name and hold the stats, and the jewel for a socket
            TooltipBody::Lines(lines) => tooltip_ui(ui, lines),
            TooltipBody::Instant => {
                ui.strong(if node.name.is_empty() {
                    &node.kind
                } else {
                    &node.name
                });
                for line in &node.stats {
                    ui.label(line);
                }
                if let Some(jewel) = jewel.filter(|j| !j.name.is_empty()) {
                    ui.colored_label(overlays::JEWEL_RING, &jewel.name);
                }
            }
        });
    actions
}

/// What a node tooltip shows.
#[derive(Debug, PartialEq)]
pub enum TooltipBody<'a> {
    /// The name and stats the layout already holds, until the engine's tooltip arrives
    Instant,
    /// The engine's tooltip, which replaces the instant one
    Lines(&'a [TooltipLine]),
}

pub fn tooltip_body<'a>(node: &TreeNode, reply: Option<&'a NodeTooltip>) -> TooltipBody<'a> {
    match reply {
        Some(t) if t.id == node.id && !t.lines.is_empty() => TooltipBody::Lines(&t.lines),
        _ => TooltipBody::Instant,
    }
}

fn handle_input(ui: &egui::Ui, tab: &mut TreeTab, response: &egui::Response) {
    if response.dragged() && tab.camera.zoom > 0.0 {
        tab.camera.center -= response.drag_delta() / tab.camera.zoom;
    }
    // Ctrl+click zooms in two of PoB's 1.2 steps at the pointer, Ctrl+right-click out
    // (PassiveTreeView.lua:259-263)
    let ctrl = ui.input(|i| i.modifiers.command);
    let clicked = response.clicked() || response.secondary_clicked();
    if ctrl
        && clicked
        && tab.camera.zoom > 0.0
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let level = if response.secondary_clicked() { -2 } else { 2 };
        tab.camera
            .zoom_at(response.rect, pointer, 1.2f32.powi(level));
    }
    if let Some(pointer) = response.hover_pos() {
        // PoB zooms one of its 1.2 steps a wheel notch, three with Shift
        // (PassiveTreeView.lua:229-233); egui would turn Shift's wheel sideways
        let (notches, shift) = wheel_notches(ui);
        tab.wheel_rest += notches;
        let steps = tab.wheel_rest.trunc();
        tab.wheel_rest -= steps;
        let mut factor = 1.0;
        if steps != 0.0 {
            factor = 1.2f32.powi(steps as i32 * if shift { 3 } else { 1 });
        } else if notches == 0.0 {
            // a trackpad's pinch, or a touch screen's
            factor = ui.input(|i| {
                let pinch: f32 = i
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Zoom(f) => Some(*f),
                        _ => None,
                    })
                    .product();
                pinch * i.multi_touch().map_or(1.0, |t| t.zoom_delta)
            });
        }
        if factor != 1.0 {
            tab.camera.zoom_at(response.rect, pointer, factor);
        }
    }
}

/// This frame's wheel notches, up positive, and whether Shift was held. A line is a notch; a
/// trackpad's points count 50 to a notch.
fn wheel_notches(ui: &egui::Ui) -> (f32, bool) {
    ui.input(|i| {
        let mut notches = 0.0;
        let mut shift = false;
        for event in &i.events {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
            {
                // the vertical wheel only (PoB has WHEELUP and WHEELDOWN); with Shift some
                // platforms already send it sideways, so then the sideways part counts. A
                // sideways swipe or tilt without Shift zooms nothing.
                let d = if delta.y != 0.0 || !modifiers.shift {
                    delta.y
                } else {
                    delta.x
                };
                notches += match unit {
                    egui::MouseWheelUnit::Line => d,
                    egui::MouseWheelUnit::Point => d / 50.0,
                    egui::MouseWheelUnit::Page => d * 3.0,
                };
                shift |= modifiers.shift;
            }
        }
        (notches, shift)
    })
}

/// Uploads the images `used` draws that aren't on the GPU yet. Returns the uploaded ones
/// indexed like [`TreeLayout::sheets`]; one still decoding, or not used yet, is `None` for now. The pixels are decoded on a
/// thread of their own the first time they are wanted, and dropped once uploaded: the texture
/// is what gets drawn, and keeping the decoded copy would hold it twice.
fn upload_sheets(
    tab: &mut TreeTab,
    ctx: &egui::Context,
    layout: &TreeLayout,
    used: &HashSet<usize>,
) -> Vec<Option<Sheet>> {
    let mut to_decode = Vec::new();
    let mut waiting = false;
    let sheets = layout
        .sheets
        .iter()
        .enumerate()
        .map(|(i, sheet)| {
            let sheet = sheet.as_ref()?;
            let size = Vec2::new(sheet.size[0] as f32, sheet.size[1] as f32);
            if let Some(texture) = tab.textures.get(&sheet.path) {
                // a copy decoded again for this layout is dropped
                let _ = sheet.take_rgba();
                return Some((texture.id(), size));
            }
            // a file nothing draws yet stays on disk
            if !used.contains(&i) {
                return None;
            }
            let pixels = sheet.take_rgba();
            let rgba = match pixels {
                SheetPixels::Ready(rgba) => rgba,
                SheetPixels::Pending => {
                    if sheet.claim() {
                        to_decode.push(std::sync::Arc::clone(sheet));
                    }
                    waiting = true;
                    return None;
                }
                SheetPixels::Failed => return None,
            };
            let image = egui::ColorImage::from_rgba_unmultiplied(sheet.size, &rgba);
            let mut options =
                egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
            if repeats(&sheet.path) {
                options.wrap_mode = egui::TextureWrapMode::Repeat;
            }
            let texture = ctx.load_texture(sheet.path.display().to_string(), image, options);
            let id = texture.id();
            tab.textures.insert(sheet.path.clone(), texture);
            Some((id, size))
        })
        .collect();
    if !to_decode.is_empty() {
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("tree-art".into())
            .spawn(move || {
                for sheet in &to_decode {
                    sheet.decode_claimed();
                    ctx.request_repaint();
                }
            });
        if spawned.is_err() {
            eprintln!("Unable to start decoding the tree's art");
        }
    }
    if waiting {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    sheets
}

/// Whether a file's art repeats across what it is drawn on: the line connectors along a
/// line, the background across the view
fn repeats(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name.contains("LineConnector")
        || name.contains("line-connector")
        || name.starts_with("Background")
}

/// PoB's small view of a jewel socket's place in the tree (ItemSlotHelper.DrawViewer): the
/// tree at zoom 17 of a view `rect.min_side()` across, centred on `node`, with a faint
/// crosshair. Drawn in plain shapes: connections, and nodes in the allocated colour or grey.
pub fn paint_socket_viewer(
    painter: &egui::Painter,
    rect: Rect,
    layout: &TreeLayout,
    allocated: &HashSet<u32>,
    node: u32,
) {
    let Some(centre) = layout.nodes.iter().find(|n| n.id == node).map(|n| n.pos) else {
        return;
    };
    // PassiveTree.lua:93: the tree's size is its smaller extent, and 10% more
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for n in &layout.nodes {
        for k in 0..2 {
            lo[k] = lo[k].min(n.pos[k]);
            hi[k] = hi[k].max(n.pos[k]);
        }
    }
    let size = ((hi[0] - lo[0]).min(hi[1] - lo[1]) * 1.1).max(1.0);
    // PassiveTreeView.lua:271 with ItemSlotHelper's zoom of 17
    let zoom = rect.width().min(rect.height()) / size * 17.0;
    let to_screen = |p: [f32; 2]| rect.center() + (Vec2::from(p) - Vec2::from(centre)) * zoom;
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect.expand(1.0), 0.0, Color32::WHITE);
    painter.rect_filled(rect, 0.0, theme::BACKGROUND);
    let visible = rect.expand(40.0);
    for &(a, b) in &layout.edges {
        let (na, nb) = (&layout.nodes[a], &layout.nodes[b]);
        let (pa, pb) = (to_screen(na.pos), to_screen(nb.pos));
        if !visible.contains(pa) && !visible.contains(pb) {
            continue;
        }
        let on = allocated.contains(&na.id) && allocated.contains(&nb.id);
        let stroke = Stroke::new(
            (6.0 * zoom).clamp(1.0, 4.0),
            if on { ALLOCATED } else { EDGE },
        );
        match arc_center(na, nb) {
            Some(c) => painter.line(arc_points(c, na.pos, nb.pos, zoom, to_screen), stroke),
            None => painter.line_segment([pa, pb], stroke),
        };
    }
    for n in &layout.nodes {
        let pos = to_screen(n.pos);
        if !visible.contains(pos) {
            continue;
        }
        let colour = if allocated.contains(&n.id) {
            ALLOCATED
        } else {
            kind_color(&n.kind)
        };
        painter.circle_filled(pos, (node_radius(&n.kind) * zoom).max(1.5), colour);
    }
    // the crosshair (ItemSlotHelper.lua:33-35)
    let faint = Color32::from_white_alpha(51);
    painter.rect_filled(
        Rect::from_center_size(rect.center(), Vec2::new(2.0, rect.height())),
        0.0,
        faint,
    );
    painter.rect_filled(
        Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 2.0)),
        0.0,
        faint,
    );
}

/// If both nodes sit on the same orbit of the same group, PoB connects them with an arc
/// around the group's centre.
fn arc_center(a: &TreeNode, b: &TreeNode) -> Option<[f32; 2]> {
    match (a.orbit, b.orbit, a.group_center) {
        (Some(oa), Some(ob), Some(center)) if oa == ob && oa.1 > 0 => Some(center),
        _ => None,
    }
}

/// The compare-only node at `at`, the nearest one when several are in reach, with the same
/// least reach in screen points as our own nodes ([`super::geometry::HitGrid::node_at`])
fn compare_only_at(nodes: &[CompareOnlyNode], at: [f32; 2], zoom: f32) -> Option<&CompareOnlyNode> {
    let floor = MIN_HIT_POINTS / zoom;
    let at = Vec2::from(at);
    nodes
        .iter()
        .map(|n| (n, (Vec2::new(n.x, n.y) - at).length()))
        .filter(|(n, d)| *d <= node_radius(&n.kind).max(floor))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, _)| n)
}

/// PoB's `zoom` for a view: its scale is the view's smaller side over `tree.size`, times the
/// zoom (PassiveTreeView.lua:271)
pub fn pob_zoom(zoom: f32, tree_size: f32, view: Rect) -> f32 {
    let side = view.width().min(view.height());
    if side > 0.0 {
        zoom * tree_size / side
    } else {
        1.0
    }
}

/// Where PoB draws a search ring of `size` (half its width) around `at` with "Show search
/// circles at viewport edge" on: unmoved while it fits, else held at the edge and smaller
/// (PassiveTreeView.lua:1017-1031). Returns the ring's top-left corner and half width.
pub fn snap_search_ring(at: Pos2, size: f32, view: Rect) -> (Pos2, f32) {
    let clamp = |v: f32, lo: f32, hi: f32| v.max(lo).min(hi);
    let x = clamp(
        at.x - size,
        view.left() - size / PEEKABOO,
        view.right() - size * PEEKABOO,
    );
    let y = clamp(
        at.y - size,
        view.top() - size / PEEKABOO,
        view.bottom() - size * PEEKABOO,
    );
    if x != at.x - size || y != at.y - size {
        let size = size * EDGE_RING_SCALE;
        (Pos2::new(x + size / 2.0, y + size / 2.0), size)
    } else {
        (Pos2::new(x, y), size)
    }
}

/// Draws the scene's images, each run of one texture in one mesh
fn paint(
    painter: &egui::Painter,
    sheets: &[Option<Sheet>],
    images: &[scene::Image],
    rect: Rect,
    camera: super::geometry::Camera,
    edge_search: bool,
) {
    let mut mesh = egui::Mesh::default();
    let flush = |mesh: &mut egui::Mesh| {
        if !mesh.is_empty() {
            painter.add(egui::Shape::mesh(std::mem::take(mesh)));
        }
    };
    // PoB's layers, and within one each pass a file at a time: one mesh per file, not one per
    // switch between a node's icon and its frame
    let mut order: Vec<&scene::Image> = images.iter().collect();
    order.sort_by_key(|i| (i.layer, i.pass, i.sprite.sheet));
    for image in order {
        let Some((texture, size)) = sheets.get(image.sprite.sheet).copied().flatten() else {
            continue;
        };
        let mut quad = image.quad.map(|c| camera.to_screen(rect, c.into()));
        if edge_search && matches!(image.tag, scene::Tag::SearchRing(_)) {
            let half = (quad[1].x - quad[0].x) / 2.0;
            let (min, half) =
                snap_search_ring(camera.to_screen(rect, image.anchor.into()), half, rect);
            let max = min + Vec2::splat(half * 2.0);
            quad = [min, Pos2::new(max.x, min.y), max, Pos2::new(min.x, max.y)];
        }
        let bounds = Rect::from_points(&quad);
        if !bounds.intersects(rect) {
            continue;
        }
        if mesh.texture_id != texture {
            flush(&mut mesh);
            mesh.texture_id = texture;
        }
        let [x, y, w, h] = image.sprite.rect;
        let uv = |u: Vec2| Pos2::new((x + u.x * w) / size.x, (y + u.y * h) / size.y);
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let [r, g, b, a] = image.tint;
        let colour = Color32::from_rgba_unmultiplied(c(r), c(g), c(b), c(a));
        let first = mesh.vertices.len() as u32;
        for (pos, u) in quad.iter().zip(image.uv) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: *pos,
                uv: uv(u),
                color: colour,
            });
        }
        mesh.indices
            .extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    flush(&mut mesh);
}

/// The ascendancies' flavour text, zoomed in far enough, at half strength for one that isn't
/// the build's (PassiveTreeView.lua:597-621)
fn draw_flavour(
    painter: &egui::Painter,
    layout: &TreeLayout,
    sets: &scene::DrawSets,
    camera: super::geometry::Camera,
    rect: Rect,
    pob_zoom: f32,
) {
    if pob_zoom < FLAVOUR_MIN_ZOOM {
        return;
    }
    for group in &layout.groups {
        let (Some(flavour), Some(asc)) = (&group.flavour, &group.ascendancy) else {
            continue;
        };
        let at = camera.to_screen(rect, [flavour.x, flavour.y]);
        if !rect.expand(rect.width()).contains(at) {
            continue;
        }
        let chosen = Some(asc) == sets.ascendancy.as_ref() || Some(asc) == sets.bloodline.as_ref();
        let channel = |i: usize| {
            let v = flavour
                .colour
                .get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .unwrap_or(255);
            if chosen { v } else { v / 2 }
        };
        let colour = Color32::from_rgb(channel(0), channel(2), channel(4));
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &flavour.text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(FLAVOUR_SIZE * camera.zoom),
                color: colour,
                italics: true,
                ..Default::default()
            },
        );
        let galley = painter.layout_job(job);
        painter.galley(at, galley, colour);
    }
}

fn kind_color(kind: &str) -> Color32 {
    match kind {
        "Notable" => Color32::from_gray(130),
        "Keystone" => Color32::from_gray(170),
        "Mastery" => Color32::from_gray(45),
        "Socket" => Color32::from_rgb(60, 110, 110),
        "ClassStart" => Color32::from_gray(200),
        _ => Color32::from_gray(90),
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::tests::node;
    use super::*;

    fn reply(id: u32, lines: usize) -> NodeTooltip {
        NodeTooltip {
            id,
            trace: vec![],
            lines: (0..lines)
                .map(|_| TooltipLine::Line {
                    size: 14,
                    text: "x".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn tooltip_body_prefers_the_engines_lines_for_the_hovered_node() {
        let n = node(5, [0.0, 0.0], "Normal");
        assert_eq!(tooltip_body(&n, None), TooltipBody::Instant);
        let r = reply(5, 2);
        assert_eq!(tooltip_body(&n, Some(&r)), TooltipBody::Lines(&r.lines));
        assert_eq!(tooltip_body(&n, Some(&reply(6, 2))), TooltipBody::Instant);
        assert_eq!(tooltip_body(&n, Some(&reply(5, 0))), TooltipBody::Instant);
    }

    fn sheet_layout(sheet: &std::sync::Arc<crate::engine::SheetImage>) -> TreeLayout {
        let mut layout = super::super::geometry::tests::layout(vec![], vec![]);
        layout.sheets = vec![Some(std::sync::Arc::clone(sheet)), None];
        layout
    }

    /// Both sheets of `sheet_layout`
    fn all() -> HashSet<usize> {
        [0, 1].into()
    }

    /// A file no image of the frame draws is neither decoded nor uploaded
    #[test]
    fn only_the_files_drawn_are_decoded() {
        let ctx = egui::Context::default();
        let mut tab = TreeTab::default();
        let dir = crate::test_dir::TestDir::new("unused-sheet");
        let path = dir.join("art.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let lazy = std::sync::Arc::new(crate::engine::SheetImage::lazy(path, [1, 1]));
        let up = upload_sheets(&mut tab, &ctx, &sheet_layout(&lazy), &HashSet::new());
        assert!(up[0].is_none());
        assert!(
            lazy.is_unloaded(),
            "decoding started for a file nothing draws"
        );
        let ready = sheet();
        let up = upload_sheets(&mut tab, &ctx, &sheet_layout(&ready), &[1].into());
        assert!(up[0].is_none());
        assert!(tab.textures.is_empty());
        assert!(ready.has_rgba());
    }

    fn sheet() -> std::sync::Arc<crate::engine::SheetImage> {
        std::sync::Arc::new(crate::engine::SheetImage::new(
            "TreeData/3_29/skills-3.jpg".into(),
            [2, 1],
            vec![255; 8],
        ))
    }

    /// The background tile goes under the class art and the groups, whatever its file's index
    /// (PassiveTreeView.lua:536-660); a frame goes over its icon
    #[test]
    fn paint_keeps_pobs_order_within_a_layer() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let camera = super::super::geometry::Camera {
            center: Vec2::ZERO,
            zoom: 1.0,
            ..Default::default()
        };
        let image = |sheet: usize, layer: u8, pass: u8, tag| scene::Image {
            layer,
            pass,
            sprite: crate::engine::Sprite {
                sheet,
                rect: [0.0, 0.0, 1.0, 1.0],
            },
            quad: [
                Vec2::new(-10.0, -10.0),
                Vec2::new(10.0, -10.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(-10.0, 10.0),
            ],
            uv: [Vec2::ZERO; 4],
            tint: [1.0; 4],
            anchor: Vec2::ZERO,
            tag,
        };
        // the scene's own passes for the background, class art and a group, the background's
        // file last in the layout; and a node's icon and frame, the frame's file first
        let mut l = super::super::geometry::tests::layout(
            vec![super::super::geometry::tests::node(1, [0.0, 0.0], "Normal")],
            vec![],
        );
        let tex = |i| egui::TextureId::User(i);
        let sheets: Vec<Option<Sheet>> = (0..5).map(|i| Some((tex(i), Vec2::ONE))).collect();
        for (i, name) in ["Group", "Class", "Frame", "Icon", "Background2"]
            .iter()
            .enumerate()
        {
            l.assets.insert(
                (*name).into(),
                crate::engine::Sprite {
                    sheet: i,
                    rect: [0.0, 0.0, 1.0, 1.0],
                },
            );
        }
        l.class_art = vec![crate::engine::ClassArt {
            class: 0,
            art: l.assets["Class"],
            pos: [0.0, 0.0],
        }];
        l.groups = vec![crate::engine::GroupArt {
            pos: [0.0, 0.0],
            art: l.assets["Group"],
            half: false,
            ascendancy: None,
            flavour: None,
        }];
        let state = crate::engine::fake::empty_tree_state();
        let sets = scene::DrawSets::new(&state, None);
        let empty = HashSet::new();
        let look = scene::Look {
            hover: None,
            path: None,
            deps: &empty,
            trace_end: None,
            heat: None,
            power_theme: Default::default(),
            view: Rect::from_min_max(Pos2::new(-1e4, -1e4), Pos2::new(1e4, 1e4)),
            pob_zoom: 1.0,
            time_ms: None,
        };
        let mut images = scene::build(&l, &state, &sets, &Default::default(), &look);
        images.push(image(3, scene::LAYER_NODE, 1, scene::Tag::Node(1)));
        images.push(image(2, scene::LAYER_NODE, 2, scene::Tag::Node(1)));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let (_, painter) = ui.allocate_painter(rect.size(), Sense::hover());
            paint(&painter, &sheets, &images, rect, camera, false);
        });
        output.textures_delta.clear();
        let order: Vec<egui::TextureId> = output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Mesh(m) => Some(m.texture_id),
                _ => None,
            })
            .collect();
        assert_eq!(order, [tex(4), tex(1), tex(0), tex(3), tex(2)]);
    }

    /// PassiveTreeView.lua:1017-1031: a ring that fits stays; one past an edge is held there,
    /// two thirds the size, sticking out by PoB's `peekaboo_ratio`
    #[test]
    fn search_rings_snap_to_the_edge_as_pobs() {
        let view = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        assert_eq!(
            snap_search_ring(Pos2::new(400.0, 300.0), 20.0, view),
            (Pos2::new(380.0, 280.0), 20.0)
        );
        // off the right bottom
        let (at, size) = snap_search_ring(Pos2::new(1000.0, 900.0), 30.0, view);
        assert!((size - 20.001).abs() < 1e-3, "{size}");
        assert!(
            (at.x - (800.0 - 30.0 * PEEKABOO + size / 2.0)).abs() < 1e-3,
            "{at:?}"
        );
        assert!(
            (at.y - (600.0 - 30.0 * PEEKABOO + size / 2.0)).abs() < 1e-3,
            "{at:?}"
        );
        // off the left only
        let (at, size) = snap_search_ring(Pos2::new(-200.0, 300.0), 30.0, view);
        assert!(
            (at.x - (-30.0 / PEEKABOO + size / 2.0)).abs() < 1e-3,
            "{at:?}"
        );
        assert!((at.y - (270.0 + size / 2.0)).abs() < 1e-3, "{at:?}");
        // a view smaller than the ring doesn't panic
        snap_search_ring(
            Pos2::new(5.0, 5.0),
            30.0,
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(2.0)),
        );
        assert!((pob_zoom(0.5, 20_000.0, view) - 0.5 * 20_000.0 / 600.0).abs() < 1e-3);
    }

    fn compare_only(id: u32, x: f32, kind: &str) -> CompareOnlyNode {
        CompareOnlyNode {
            id,
            x,
            y: 0.0,
            kind: kind.into(),
            name: String::new(),
            mastery_effects: false,
            art: None,
        }
    }

    /// The nearest compare-only node in reach, with our nodes' least reach on screen
    #[test]
    fn compare_only_nodes_are_hit_like_ours() {
        let nodes = [
            compare_only(1, 0.0, "Normal"),
            compare_only(2, 40.0, "Normal"),
        ];
        let at = |x, zoom| compare_only_at(&nodes, [x, 0.0], zoom).map(|n| n.id);
        assert_eq!(at(5.0, 1.0), Some(1));
        assert_eq!(at(30.0, 1.0), Some(2));
        assert_eq!(at(100.0, 1.0), None);
        // zoomed out, 6 points reach 300 tree units
        assert_eq!(at(250.0, 0.02), Some(2));
        assert_eq!(at(400.0, 0.02), None);
    }

    #[test]
    fn uploaded_sheets_drop_their_pixels() {
        let ctx = egui::Context::default();
        let mut tab = TreeTab::default();
        let first = sheet();
        let up = upload_sheets(&mut tab, &ctx, &sheet_layout(&first), &all());
        assert_eq!(up.len(), 2);
        let (id, size) = up[0].expect("uploaded");
        assert_eq!(size, Vec2::new(2.0, 1.0));
        assert!(up[1].is_none());
        // the texture holds the sheet now; the decoded copy is gone
        assert!(!first.has_rgba());
        // later frames draw from the texture
        let again = upload_sheets(&mut tab, &ctx, &sheet_layout(&first), &all());
        assert_eq!(again[0].map(|s| s.0), Some(id));
        // a later layout's fresh copy of a sheet already up is dropped too, not kept
        let second = sheet();
        let next = upload_sheets(&mut tab, &ctx, &sheet_layout(&second), &all());
        assert_eq!(next[0].map(|s| s.0), Some(id));
        assert!(!second.has_rgba());
        // without pixels nor a texture the sheet's sprites are left out, not a panic
        let mut other = TreeTab::default();
        assert!(upload_sheets(&mut other, &ctx, &sheet_layout(&first), &all())[0].is_none());
        assert!(other.textures.is_empty());
    }

    /// A file is decoded off the frame the first time it is wanted, then uploaded; the layout
    /// only read its size (crate::engine::tree::load_sheet)
    #[test]
    fn a_files_pixels_are_decoded_when_first_drawn() {
        let dir = crate::test_dir::TestDir::new("lazy-sheet");
        let path = dir.join("art.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        let lazy = std::sync::Arc::new(crate::engine::SheetImage::lazy(path, [3, 2]));
        assert!(lazy.is_unloaded());
        let ctx = egui::Context::default();
        let mut tab = TreeTab::default();
        // the first frame starts the decoding and draws without it
        assert!(upload_sheets(&mut tab, &ctx, &sheet_layout(&lazy), &all())[0].is_none());
        assert!(!lazy.is_unloaded());
        let start = std::time::Instant::now();
        let up = loop {
            if let Some(up) = upload_sheets(&mut tab, &ctx, &sheet_layout(&lazy), &all())[0] {
                break up;
            }
            assert!(start.elapsed().as_secs() < 10, "never decoded");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(up.1, Vec2::new(3.0, 2.0));
        // uploaded: the pixels are dropped
        assert!(!lazy.has_rgba());
        // a file that can't be decoded is left out for good
        let broken = std::sync::Arc::new(crate::engine::SheetImage::lazy(
            dir.join("none.png"),
            [1, 1],
        ));
        broken.decode();
        assert_eq!(broken.take_rgba(), crate::engine::SheetPixels::Failed);
        assert!(upload_sheets(&mut tab, &ctx, &sheet_layout(&broken), &all())[0].is_none());
    }
}
