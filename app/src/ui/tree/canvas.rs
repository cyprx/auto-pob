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
/// The nodes a click would allocate
const PATH: Color32 = Color32::from_rgb(120, 200, 255);
/// The nodes a click would deallocate
const DROP: Color32 = Color32::from_rgb(230, 80, 70);
/// Below this many screen points, icons are unreadable and plain dots are drawn instead
const MIN_ICON_SIZE: f32 = 8.0;
/// PoB draws sprites at their pixel size times this, in tree units (`PassiveTreeView:DrawAsset`)
const SPRITE_SCALE: f32 = 1.33 * 2.0;
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
    let mut actions = Vec::new();
    if hovered_id != tab.hover {
        actions.push(CanvasAction::Hover(hovered_id));
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
    let current = tab.derived.current_ascendancy.as_deref();
    let fade = |node: &TreeNode| match &node.ascendancy {
        Some(a) if Some(a.as_str()) != current => OTHER_ASCENDANCY,
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
                overlays::heat_color(p, entry)
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
        if tab.derived.matches.contains(&node.id) {
            overlays::draw_search_ring(&painter, pos, radius);
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
            // a node of another ascendancy has no path, but the click switches to it
            // (PassiveTreeView.lua:416); the engine says if that isn't possible
            || node
                .ascendancy
                .as_ref()
                .is_some_and(|a| tab.derived.current_ascendancy.as_ref() != Some(a))
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
/// [`TreeLayout::sheets`].
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
            let texture: &TextureHandle =
                tab.textures.entry(sheet.path.clone()).or_insert_with(|| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(sheet.size, &sheet.rgba);
                    let options = egui::TextureOptions::LINEAR
                        .with_mipmap_mode(Some(egui::TextureFilter::Linear));
                    ctx.load_texture(sheet.path.display().to_string(), image, options)
                });
            Some((
                texture.id(),
                Vec2::new(sheet.size[0] as f32, sheet.size[1] as f32),
            ))
        })
        .collect()
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
}
