//! Draws the passive tree and turns pointer input into [`CanvasAction`]s: pan by dragging, zoom
//! with the scroll wheel, click to allocate.

use super::TreeTab;
use super::geometry::{arc_points, node_radius};
use super::overlays::{self, ALLOCATED, Legend};
use crate::engine::{NodePower, NodeTooltip, Sprite, TooltipLine, TreeLayout, TreeNode, TreeState};
use crate::theme;
use crate::widgets::tooltip_ui;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Tooltip, Vec2};
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

const EDGE: Color32 = Color32::from_rgb(72, 62, 50);
const HOVER: Color32 = Color32::WHITE;
/// A node allocated in the compared spec only
const COMPARE_ONLY: Color32 = Color32::from_rgb(0, 200, 0);
/// The nodes a click would allocate
const PATH: Color32 = Color32::from_rgb(120, 200, 255);
/// The nodes a click would deallocate
const DROP: Color32 = Color32::from_rgb(230, 80, 70);
/// Below this many screen points, icons are unreadable and plain dots are drawn instead
const MIN_ICON_SIZE: f32 = 8.0;
/// PoB draws sprites at their pixel size times this, in tree units (`PassiveTreeView:DrawAsset`)
const SPRITE_SCALE: f32 = 1.33 * 2.0;
/// A search ring snapped to the view's edge is this much smaller (PoB's `scaled_down_ratio`)
const EDGE_RING_SCALE: f32 = 0.6667;
/// Nodes of ascendancies other than the build's are drawn this faint, as PoB does
/// (`PassiveTreeView:Draw`, `renderGroup`: `SetDrawColor(1, 1, 1, 0.50)`)
const OTHER_ASCENDANCY: f32 = 0.5;

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
        handle_input(ui, tab, &response);
    }
    let sheets = upload_sheets(tab, ui.ctx(), layout);
    let camera = tab.camera;
    let zoom = camera.zoom;
    let to_screen = |p: [f32; 2]| camera.to_screen(rect, p);
    // drawing a little outside the view avoids popping at the edges
    let visible = rect.expand(50.0);

    // the node under the pointer; none while panning
    let hovered = response
        .hover_pos()
        .filter(|_| valid_zoom && !response.dragged())
        .and_then(|p| tab.grid.node_at(layout, camera.to_tree(rect, p), zoom));
    // Shift traces a path through the hovered nodes (PassiveTreeView.lua:283-363); a node
    // the trace can't take is not hovered
    let shift = ui.input(|i| i.modifiers.shift);
    let hovered_id = tab.trace_step(layout, hovered.map(|i| layout.nodes[i].id), shift);
    let hovered = hovered_id.and_then(|id| tab.index.get(&id).copied());
    // where none of ours is, a node of the compared spec only (PassiveTreeView.lua:308-320)
    let compare_hover = response
        .hover_pos()
        .filter(|_| valid_zoom && !response.dragged() && !shift && hovered_id.is_none())
        .and_then(|p| {
            let at = Vec2::from(camera.to_tree(rect, p));
            state
                .compare_only
                .iter()
                .find(|n| (Vec2::new(n.x, n.y) - at).length() <= node_radius(&n.kind))
        });
    let mut actions = Vec::new();
    let hover_id = hovered_id.or(compare_hover.map(|n| n.id));
    if hover_id != tab.hover {
        actions.push(CanvasAction::Hover(hover_id));
    }

    // the nodes a click on the hovered one would add or drop
    let (highlight, highlight_colour) = match hovered_id {
        // with Shift held the trace is the path, and nothing is a dependent
        // (PassiveTreeView.lua:328-363)
        _ if shift => (Some(&tab.trace), PATH),
        Some(id) if tab.derived.allocated.contains(&id) => (tab.derived.depends.get(&id), DROP),
        Some(id) => (tab.derived.paths.get(&id), PATH),
        None => (None, PATH),
    };
    let highlight: HashSet<u32> = highlight.into_iter().flatten().copied().collect();
    let fade = |node: &TreeNode| match &node.ascendancy {
        Some(a) if !tab.derived.is_chosen(a) => OTHER_ASCENDANCY,
        _ => 1.0,
    };

    // connections
    for &(a, b) in &layout.edges {
        let (na, nb) = (&layout.nodes[a], &layout.nodes[b]);
        let (pa, pb) = (to_screen(na.pos), to_screen(nb.pos));
        if !visible.contains(pa) && !visible.contains(pb) {
            continue;
        }
        let colour = if highlight.contains(&na.id) && highlight.contains(&nb.id) {
            highlight_colour
        } else if tab.derived.allocated.contains(&na.id) && tab.derived.allocated.contains(&nb.id) {
            ALLOCATED
        } else {
            EDGE
        };
        let stroke = Stroke::new(
            (6.0 * zoom).clamp(1.0, 4.0),
            colour.gamma_multiply(fade(na).min(fade(nb))),
        );
        match arc_center(na, nb) {
            Some(center) => {
                painter.line(arc_points(center, na.pos, nb.pos, zoom, to_screen), stroke)
            }
            None => painter.line_segment([pa, pb], stroke),
        };
    }

    // nodes
    // P shows the map before any stat was chosen: damage against defence
    let heat_on = state.heatmap.is_some() || state.heatmap_shown;
    for node in &layout.nodes {
        let pos = to_screen(node.pos);
        if !visible.contains(pos) {
            continue;
        }
        let radius = (node_radius(&node.kind) * zoom).max(1.5);
        let is_allocated = tab.derived.allocated.contains(&node.id);
        let fade = fade(node);
        let on_path = highlight.contains(&node.id);

        if let Some(rgb) = tab.derived.compare.get(&node.id) {
            overlays::draw_compare_tint(&painter, pos, radius, *rgb);
        }
        let glow = power
            .filter(|_| {
                heat_on
                    && !is_allocated
                    && !matches!(node.kind.as_str(), "ClassStart" | "AscendClassStart")
            })
            .and_then(|p| {
                let entry = &p.nodes[*tab.power_index.get(&node.id)?];
                overlays::heat_color(p, entry, tab.power_theme)
            });
        if let Some(colour) = glow {
            painter.circle_filled(pos, radius * 1.4, colour);
        }

        let state_ix = usize::from(is_allocated);
        let icon = node.art.icon[state_ix]
            .and_then(|sprite| Some((sprite, sheets.get(sprite.sheet).copied().flatten()?)));
        match icon {
            Some((sprite, sheet)) if sprite_size(sprite, zoom).x >= MIN_ICON_SIZE => {
                let tint = Color32::WHITE.gamma_multiply(fade);
                draw_sprite(&painter, sheet, sprite, pos, zoom, tint);
                let frame = node.art.frame[state_ix]
                    .and_then(|s| Some((s, sheets.get(s.sheet).copied().flatten()?)));
                if let Some((frame, sheet)) = frame {
                    draw_sprite(&painter, sheet, frame, pos, zoom, tint);
                }
            }
            _ => {
                let fill = if is_allocated {
                    ALLOCATED
                } else if on_path {
                    highlight_colour
                } else {
                    kind_color(&node.kind)
                };
                painter.circle_filled(pos, radius, fill.gamma_multiply(fade));
                if matches!(node.kind.as_str(), "Notable" | "Keystone") {
                    painter.circle_stroke(pos, radius, Stroke::new(1.0, Color32::from_gray(160)));
                }
            }
        }
        if on_path {
            painter.circle_stroke(pos, radius + 1.5, Stroke::new(1.5, highlight_colour));
        }
        // with the edge option on, a match outside the view is drawn snapped, below
        if tab.derived.matches.contains(&node.id) && (tab.edge_search_off || rect.contains(pos)) {
            overlays::draw_search_ring(&painter, pos, radius);
        }
    }
    // the compared spec's own nodes, green as PoB tints them (PassiveTreeView.lua:1102-1140)
    for node in &state.compare_only {
        let pos = to_screen([node.x, node.y]);
        if !visible.contains(pos) {
            continue;
        }
        let radius = (node_radius(&node.kind) * zoom).max(1.5);
        painter.circle_filled(pos, radius, COMPARE_ONLY);
        if compare_hover.is_some_and(|n| n.id == node.id) {
            painter.circle_stroke(pos, radius + 2.0, Stroke::new(2.0, HOVER));
        }
    }
    if let Some(node) = compare_hover {
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
    // PoB's "Show search circles at viewport edge": a match outside the view shows as a
    // smaller ring at the edge nearest to it (PassiveTreeView.lua:1017-1031)
    if !tab.edge_search_off {
        for &id in &tab.derived.matches {
            let Some(&i) = tab.index.get(&id) else {
                continue;
            };
            let node = &layout.nodes[i];
            let pos = to_screen(node.pos);
            if rect.contains(pos) {
                continue;
            }
            let radius = (node_radius(&node.kind) * zoom).max(1.5) * EDGE_RING_SCALE;
            // PoB lets the ring overhang the edge a little (its `peekaboo_ratio` 1.15)
            let inset = radius * 0.13;
            let at = egui::pos2(
                pos.x.clamp(rect.left() + inset, rect.right() - inset),
                pos.y.clamp(rect.top() + inset, rect.bottom() - inset),
            );
            overlays::draw_search_ring(&painter, at, radius);
        }
    }

    for jewel in &state.jewels {
        if let Some(&i) = tab.index.get(&jewel.node) {
            let pos = to_screen(layout.nodes[i].pos);
            overlays::draw_jewel_ring(&painter, visible, pos, jewel, zoom);
        }
    }

    if let Some((id, start)) = tab.pulse {
        let age = ui.input(|i| i.time) - start;
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
    let radius = (node_radius(&node.kind) * zoom).max(1.5);
    painter.circle_stroke(to_screen(node.pos), radius + 2.0, Stroke::new(2.0, HOVER));

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

/// Uploads sprite sheets that aren't on the GPU yet. Returns them indexed like
/// [`TreeLayout::sheets`]. Every sheet's pixels are taken and dropped, uploaded or not: the
/// texture is what gets drawn, and keeping the decoded copy would hold it twice.
fn upload_sheets(
    tab: &mut TreeTab,
    ctx: &egui::Context,
    layout: &TreeLayout,
) -> Vec<Option<Sheet>> {
    layout
        .sheets
        .iter()
        .map(|sheet| {
            let sheet = sheet.as_ref()?;
            let rgba = sheet.take_rgba();
            let texture: &TextureHandle = match tab.textures.entry(sheet.path.clone()) {
                std::collections::hash_map::Entry::Occupied(held) => held.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    // no pixels: this UI took them before and has the texture (an engine
                    // serves one UI), so this can't happen; the sprites would be left out, as
                    // for a sheet that couldn't be loaded
                    let image = egui::ColorImage::from_rgba_unmultiplied(sheet.size, &rgba?);
                    let options = egui::TextureOptions::LINEAR
                        .with_mipmap_mode(Some(egui::TextureFilter::Linear));
                    slot.insert(ctx.load_texture(sheet.path.display().to_string(), image, options))
                }
            };
            Some((
                texture.id(),
                Vec2::new(sheet.size[0] as f32, sheet.size[1] as f32),
            ))
        })
        .collect()
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

/// On-screen size of a sprite at the given zoom.
fn sprite_size(sprite: Sprite, zoom: f32) -> Vec2 {
    Vec2::new(sprite.rect[2], sprite.rect[3]) * SPRITE_SCALE * zoom
}

fn draw_sprite(
    painter: &egui::Painter,
    (texture, sheet_size): Sheet,
    sprite: Sprite,
    pos: Pos2,
    zoom: f32,
    tint: Color32,
) {
    let [x, y, w, h] = sprite.rect;
    let uv = Rect::from_min_size(
        Pos2::new(x / sheet_size.x, y / sheet_size.y),
        Vec2::new(w / sheet_size.x, h / sheet_size.y),
    );
    let rect = Rect::from_center_size(pos, sprite_size(sprite, zoom));
    painter.image(texture, rect, uv, tint);
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

    fn sheet() -> std::sync::Arc<crate::engine::SheetImage> {
        std::sync::Arc::new(crate::engine::SheetImage::new(
            "TreeData/3_29/skills-3.jpg".into(),
            [2, 1],
            vec![255; 8],
        ))
    }

    #[test]
    fn uploaded_sheets_drop_their_pixels() {
        let ctx = egui::Context::default();
        let mut tab = TreeTab::default();
        let first = sheet();
        let up = upload_sheets(&mut tab, &ctx, &sheet_layout(&first));
        assert_eq!(up.len(), 2);
        let (id, size) = up[0].expect("uploaded");
        assert_eq!(size, Vec2::new(2.0, 1.0));
        assert!(up[1].is_none());
        // the texture holds the sheet now; the decoded copy is gone
        assert!(!first.has_rgba());
        // later frames draw from the texture
        let again = upload_sheets(&mut tab, &ctx, &sheet_layout(&first));
        assert_eq!(again[0].map(|s| s.0), Some(id));
        // a later layout's fresh copy of a sheet already up is dropped too, not kept
        let second = sheet();
        let next = upload_sheets(&mut tab, &ctx, &sheet_layout(&second));
        assert_eq!(next[0].map(|s| s.0), Some(id));
        assert!(!second.has_rgba());
        // without pixels nor a texture the sheet's sprites are left out, not a panic
        let mut other = TreeTab::default();
        assert!(upload_sheets(&mut other, &ctx, &sheet_layout(&first))[0].is_none());
        assert!(other.textures.is_empty());
    }
}
