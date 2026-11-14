//! Draws the passive tree and turns pointer input into [`CanvasAction`]s: pan by dragging, zoom
//! with the scroll wheel, click to allocate.

use super::TreeTab;
use super::geometry::{MIN_HIT_POINTS, node_radius};
use super::overlays::{self, ALLOCATED, Legend};
use super::scene;
use crate::engine::{
    CompareOnlyNode, NodePower, NodeTooltip, SheetPixels, TooltipLine, TreeLayout, TreeNode,
    TreeState,
};
use crate::theme;
use crate::widgets::{Look, art, styled_tooltip_ui};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
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

/// A search ring snapped to the view's edge is this much smaller (PoB's `scaled_down_ratio`)
const EDGE_RING_SCALE: f32 = 0.6667;
/// How far a snapped search ring may stick out past the view's edge (PoB's `peekaboo_ratio`)
const PEEKABOO: f32 = 1.15;
/// PoB shows the ascendancies' flavour text from this zoom on (PassiveTreeView.lua:612)
const FLAVOUR_MIN_ZOOM: f32 = 2.5;
/// PoB's flavour text size at a scale of 1 (PassiveTreeView.lua:611)
const FLAVOUR_SIZE: f32 = 52.0;
/// While jewel rings turn, the canvas is redrawn when their edge has moved about a pixel, but
/// no more often than this ...
const RING_FRAME_MIN_MS: f32 = 100.0;
/// ... nor less often than this
const RING_FRAME_MAX_MS: f32 = 2000.0;

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
    let sheets = upload_sheets(&tab.textures, ui.ctx(), layout, &used);
    paint(
        &painter,
        &sheets,
        &images,
        rect,
        camera,
        !tab.edge_search_off,
    );
    if !tab.animations_off {
        let shown: HashSet<u32> = images
            .iter()
            .filter_map(|i| match i.tag {
                scene::Tag::JewelRing(node) => Some(node),
                _ => None,
            })
            .collect();
        let rings = state
            .socket_rings
            .iter()
            .filter(|s| shown.contains(&s.node))
            .flat_map(|s| &s.rings);
        if let Some(wait) = ring_frame_interval(rings, zoom) {
            ui.ctx().request_repaint_after(wait);
        }
    }
    draw_flavour(&painter, layout, &tab.draw_sets, camera, rect, pob_zoom);
    // a node whose art couldn't be loaded is still shown, as a dot
    for node in &layout.nodes {
        if !art_missing(layout, &node.art) {
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
        // PoB's hover form, beside the node's square (PassiveTreeView.lua:1148-1150)
        let at = to_screen([node.x, node.y]);
        let size = (node_radius(&node.kind) * zoom).floor();
        let corner = Pos2::new((at.x - size).floor(), (at.y - size).floor());
        let id = egui::Id::NULL.with(("tree-compare-tip", node.id, reply.is_some()));
        let tip = area_size(ui.ctx(), id);
        let pos = hover_tooltip_place(corner, size * 2.0, tip.unwrap_or_default(), rect);
        tooltip_area(ui.ctx(), id, pos, |ui| match reply {
            Some(t) => node_tooltip_ui(ui, t),
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
    let skill = reply
        .filter(|t| t.id == node.id)
        .and_then(|t| t.skill.as_ref());
    // PoB anchors the tooltip at the node's top right, and the granted skill's beside it or
    // under it (PassiveTreeView.lua:1044-1097); the sizes are the last frame's
    let has_lines = matches!(body, TooltipBody::Lines(_));
    let main_id = egui::Id::NULL.with(("tree-tip", node.id, has_lines));
    let skill_id = egui::Id::NULL.with(("tree-skill-tip", node.id));
    let main_size = area_size(ui.ctx(), main_id);
    let skill_size = skill.and_then(|_| area_size(ui.ctx(), skill_id));
    let size = (node_radius(&node.kind) * zoom).floor();
    let (main_at, skill_at) = node_tooltip_places(
        to_screen(node.pos),
        size,
        main_size.unwrap_or_default(),
        skill.map(|_| skill_size.unwrap_or_default()),
        rect,
    );
    if let (Some(skill), Some(at)) = (skill, skill_at) {
        tooltip_area(ui.ctx(), skill_id, at, |ui| {
            styled_tooltip_ui(
                ui,
                &skill.lines,
                &Look {
                    header: skill.header.as_deref(),
                    ..Default::default()
                },
            )
        });
    }
    tooltip_area(ui.ctx(), main_id, main_at, |ui| match body {
        // PoB's lines start with the name and hold the stats, and the jewel for a socket
        TooltipBody::Lines(_) => {
            if let Some(t) = reply {
                node_tooltip_ui(ui, t);
            }
        }
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

/// A node's tooltip as PoB draws it: centred, under its header art, a notable's oils after its
/// name (PassiveTreeView.lua:1398-1420)
fn node_tooltip_ui(ui: &mut egui::Ui, tip: &NodeTooltip) {
    styled_tooltip_ui(
        ui,
        &tip.lines,
        &Look {
            header: tip.header.as_deref(),
            recipe: &tip.recipe,
            center: true,
        },
    );
}

/// The size a tooltip area had last frame; `None` on its first, which egui draws invisibly to
/// measure it
fn area_size(ctx: &egui::Context, id: egui::Id) -> Option<Vec2> {
    ctx.memory(|m| m.area_rect(id)).map(|r| r.size())
}

/// A tooltip at `pos`, its top left, above everything and taking no input
fn tooltip_area(ctx: &egui::Context, id: egui::Id, pos: Pos2, add: impl FnOnce(&mut egui::Ui)) {
    egui::Area::new(id)
        .order(egui::Order::Tooltip)
        .fixed_pos(pos)
        .interactable(false)
        .constrain(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(ui.spacing().tooltip_width);
                add(ui);
            });
        });
}

/// Where PoB draws a node's tooltip and its granted skill's, as top-left corners, for a node
/// at `node` drawn `size` each side (`node.size * scale`), tooltips of `tip` and `skill` and the
/// view (PassiveTreeView.lua:1044-1097). A skill wider than half again its height goes under the
/// node's tooltip, else beside it; both move left and up to stay in the view.
pub fn node_tooltip_places(
    node: Pos2,
    size: f32,
    tip: Vec2,
    skill: Option<Vec2>,
    view: Rect,
) -> (Pos2, Option<Pos2>) {
    let skill_size = skill.unwrap_or_default();
    let fat = skill.is_some() && skill_size.x > skill_size.y * 1.5;
    let total = if fat {
        Vec2::new(tip.x.max(skill_size.x), tip.y + skill_size.y)
    } else {
        Vec2::new(tip.x + skill_size.x, tip.y.max(skill_size.y))
    };
    let node_x = (node.x + size).floor();
    let node_y = (node.y - size).floor();
    let (mut x, mut y) = (node_x, node_y);
    let right = x + total.x - view.left() - view.width();
    if right > 0.0 {
        x -= right;
    }
    let below = y + total.y - view.top() - view.height();
    if below > 0.0 {
        y -= below;
    }
    let main = if fat {
        Pos2::new(
            node_x.min(view.width() - tip.x + view.left()),
            y.max(view.top()),
        )
    } else {
        Pos2::new(
            x.max(view.left()),
            node_y.min(view.height() - tip.y + view.top()),
        )
    };
    let skill_at = skill.map(|_| {
        if fat {
            Pos2::new(x, y + tip.y)
        } else {
            Pos2::new(x + tip.x + 5.0, y)
        }
    });
    // PoB wraps a tooltip taller than the view into columns, so it never starts above the view
    // (Tooltip.lua:212-298); ours is one column, held at the view's top and left instead
    let keep = |p: Pos2| Pos2::new(p.x.max(view.left()), p.y.max(view.top()));
    (keep(main), skill_at.map(keep))
}

/// Where PoB's `Tooltip:Draw` puts a tooltip of `tip` for a hovered square at `corner`, `side`
/// across: right of it, else left, and under it when that overlaps; held up inside the view
/// (Tooltip.lua's hover form)
pub fn hover_tooltip_place(corner: Pos2, side: f32, tip: Vec2, view: Rect) -> Pos2 {
    let mut x = corner.x + side + 5.0;
    let mut y = corner.y;
    if x + tip.x > view.right() {
        x = view.left().max(corner.x - 5.0 - tip.x);
        if x + tip.x > corner.x {
            y += side;
        }
    }
    if y + tip.y > view.bottom() {
        y = view.top().max(corner.y + side - tip.y);
    }
    Pos2::new(x, y)
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
        tab.camera.zoom_at(response.rect, pointer, level);
    }
    if let Some(pointer) = response.hover_pos() {
        // PoB zooms one of its levels a wheel notch, three with Shift
        // (PassiveTreeView.lua:229-233); egui would turn Shift's wheel sideways. A trackpad's
        // pinch, or a touch screen's, counts a level for each 1.2 it zooms.
        let (notches, shift) = wheel_notches(ui);
        let pinch = ui.input(|i| {
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
        let pinch_levels = if pinch > 0.0 && pinch != 1.0 {
            pinch.ln() / 1.2f32.ln()
        } else {
            0.0
        };
        tab.wheel_rest += notches * if shift { 3.0 } else { 1.0 } + pinch_levels;
        let steps = tab.wheel_rest.trunc();
        tab.wheel_rest -= steps;
        if steps != 0.0 {
            tab.camera.zoom_at(response.rect, pointer, steps as i32);
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
    textures: &super::Textures,
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
            if let Some(texture) = super::lock(textures).get(&sheet.path) {
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
            super::lock(textures).insert(sheet.path.clone(), texture);
            Some((id, size))
        })
        .collect();
    // if nothing can decode them, they are drawn as dots and the canvas doesn't wait
    if !to_decode.is_empty() && !decode_in_background(to_decode, ctx, art::send_to_art_thread) {
        waiting = false;
    }
    if waiting {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    sheets
}

/// Decodes the claimed `sheets` on the thread `spawn` hands the work to, asking for a frame
/// after each. If it can't take the work the files are marked failed, so nothing waits for
/// them; returns whether it took it.
fn decode_in_background(
    sheets: Vec<std::sync::Arc<crate::engine::SheetImage>>,
    ctx: &egui::Context,
    spawn: impl FnOnce(art::ArtJob) -> std::io::Result<()>,
) -> bool {
    let ctx = ctx.clone();
    let failed = sheets.clone();
    let started = spawn(Box::new(move || {
        for sheet in &sheets {
            sheet.decode_claimed();
            ctx.request_repaint();
        }
    }));
    if started.is_err() {
        eprintln!("Unable to start decoding the tree's art");
        for sheet in &failed {
            sheet.fail();
        }
    }
    started.is_ok()
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

/// PoB's zoom for the socket viewer (ItemSlotHelper.lua:21)
pub const SOCKET_VIEWER_ZOOM: f32 = 17.0;

/// The socket viewer's camera: the tree at PoB's zoom 17 of a view `rect.min_side()` across,
/// centred on the socket at `at` (ItemSlotHelper.lua:21-26, PassiveTreeView.lua:271)
pub fn socket_viewer_camera(rect: Rect, tree_size: f32, at: [f32; 2]) -> super::geometry::Camera {
    let side = rect.width().min(rect.height());
    super::geometry::Camera {
        center: at.into(),
        zoom: side / tree_size.max(1.0) * SOCKET_VIEWER_ZOOM,
        tree_size,
        level: 0,
    }
}

/// What a small tree view marks its node with
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewerMark {
    /// The socket viewer's faint crosshair (ItemSlotHelper.lua:30-35)
    Crosshair,
    /// The Calcs node viewer's red highlight ring, 30 across (CalcBreakdownControl.lua:607-609)
    Ring,
}

/// PoB's small view of a jewel socket's place in the tree (ItemSlotHelper.DrawViewer): a white
/// 1-point border, then the viewer's own PassiveTreeView:Draw at zoom 17 centred on `node`,
/// with nothing hovered, and a faint crosshair over it. The art is the Tree tab's.
pub fn paint_socket_viewer(
    ctx: &egui::Context,
    painter: &egui::Painter,
    rect: Rect,
    scene_of: &super::SocketScene,
    node: u32,
) {
    paint_tree_viewer(
        ctx,
        painter,
        rect,
        scene_of,
        node,
        SOCKET_VIEWER_ZOOM,
        1.0,
        ViewerMark::Crosshair,
    );
}

/// A small view of the tree around `node` in `rect`, inside a white border `border` wide: the
/// tree as a PassiveTreeView of its own draws it (no comparison, search, heat map or hover) at
/// PoB's `zoom`, marked as `mark` says
#[allow(clippy::too_many_arguments)]
pub fn paint_tree_viewer(
    ctx: &egui::Context,
    painter: &egui::Painter,
    rect: Rect,
    scene_of: &super::SocketScene,
    node: u32,
    zoom: f32,
    border: f32,
    mark: ViewerMark,
) {
    let layout = &scene_of.layout;
    let Some(centre) = scene_of.index.get(&node).map(|&i| layout.nodes[i].pos) else {
        return;
    };
    let painter = painter.with_clip_rect(rect.expand(border));
    painter.rect_filled(rect.expand(border), 0.0, Color32::WHITE);
    painter.rect_filled(rect, 0.0, theme::BACKGROUND);
    let painter = painter.with_clip_rect(rect);
    let mut camera = socket_viewer_camera(rect, layout.size, centre);
    camera.zoom *= zoom / SOCKET_VIEWER_ZOOM;
    let now = ctx.input(|i| i.time);
    let none = HashSet::new();
    let look = scene::Look {
        hover: None,
        path: None,
        deps: &none,
        trace_end: None,
        heat: None,
        power_theme: Default::default(),
        view: Rect::from_min_max(
            camera.to_tree(rect, rect.min).into(),
            camera.to_tree(rect, rect.max).into(),
        ),
        pob_zoom: zoom,
        time_ms: (!scene_of.animations_off).then_some(now * 1000.0),
    };
    let images = scene::build(
        layout,
        &scene_of.state,
        &scene_of.sets,
        &scene_of.index,
        &look,
    );
    let mut used: HashSet<usize> = images.iter().map(|i| i.sprite.sheet).collect();
    if mark == ViewerMark::Ring
        && let Some(ring) = layout.assets.get("highlightRing")
    {
        used.insert(ring.sheet);
    }
    let sheets = upload_sheets(&scene_of.textures, ctx, layout, &used);
    paint(&painter, &sheets, &images, rect, camera, false);
    if !scene_of.animations_off {
        let rings = scene_of.state.socket_rings.iter().flat_map(|s| &s.rings);
        if let Some(wait) = ring_frame_interval(rings, camera.zoom) {
            ctx.request_repaint_after(wait);
        }
    }
    match mark {
        ViewerMark::Crosshair => {
            // ItemSlotHelper.lua:30-35
            let faint = Color32::from_white_alpha(51);
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(rect.left() + (rect.width() / 2.0).floor() - 1.0, rect.top()),
                    Vec2::new(2.0, rect.height()),
                ),
                0.0,
                faint,
            );
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(
                        rect.left(),
                        rect.top() + (rect.height() / 2.0).floor() - 1.0,
                    ),
                    Vec2::new(rect.width(), 2.0),
                ),
                0.0,
                faint,
            );
        }
        ViewerMark::Ring => {
            // the view's highlight ring, red (CalcBreakdownControl.lua:607-609)
            if let Some((&ring, Some((texture, size)))) = layout
                .assets
                .get("highlightRing")
                .map(|r| (r, sheets.get(r.sheet).copied().flatten()))
            {
                let [x, y, w, h] = ring.rect;
                let uv = Rect::from_min_max(
                    Pos2::new(x / size.x, y / size.y),
                    Pos2::new((x + w) / size.x, (y + h) / size.y),
                );
                painter.image(
                    texture,
                    Rect::from_center_size(rect.center(), Vec2::splat(30.0)),
                    uv,
                    Color32::from_rgb(255, 0, 0),
                );
            }
        }
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

/// How long the turning jewel rings can wait for a frame: until the fastest edge on screen has
/// moved about a pixel (PoB turns a ring `spin * 0.03` radians a second, PassiveTreeView.lua:
/// 1250-1274), within [`RING_FRAME_MIN_MS`] and [`RING_FRAME_MAX_MS`]. `None` without rings.
fn ring_frame_interval<'a>(
    rings: impl Iterator<Item = &'a crate::engine::RingImage>,
    zoom: f32,
) -> Option<std::time::Duration> {
    let fastest = rings
        .map(|r| r.size / 2.0 * zoom * r.spin.abs() * 0.03)
        .reduce(f32::max)?;
    let ms = if fastest > 0.0 {
        1000.0 / fastest
    } else {
        RING_FRAME_MAX_MS
    };
    Some(std::time::Duration::from_secs_f32(
        ms.clamp(RING_FRAME_MIN_MS, RING_FRAME_MAX_MS) / 1000.0,
    ))
}

/// Whether none of a node's art can be drawn: it has none, or its files couldn't be loaded
fn art_missing(layout: &TreeLayout, art: &crate::engine::NodeArt) -> bool {
    art.base.iter().chain(&art.overlay).flatten().all(|s| {
        layout
            .sheets
            .get(s.sheet)
            .and_then(Option::as_ref)
            .is_none_or(|f| f.is_failed())
    })
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
            ..Default::default()
        }
    }

    /// PoB's node tooltip sits at the node's top right; the skill's beside it, or under it when
    /// wide; both are pushed back into the view (PassiveTreeView.lua:1044-1097)
    #[test]
    fn node_tooltips_sit_where_pobs_do() {
        let view = Rect::from_min_size(Pos2::new(0.0, 30.0), Vec2::new(1000.0, 700.0));
        let tip = Vec2::new(300.0, 200.0);
        // alone, with room: at the node's top right
        let (main, skill) = node_tooltip_places(Pos2::new(100.5, 300.0), 20.0, tip, None, view);
        assert_eq!((main, skill), (Pos2::new(120.0, 280.0), None));
        // a tall skill goes beside it, 5 apart
        let tall = Vec2::new(200.0, 250.0);
        let (main, skill) =
            node_tooltip_places(Pos2::new(100.0, 300.0), 20.0, tip, Some(tall), view);
        assert_eq!(main, Pos2::new(120.0, 280.0));
        assert_eq!(skill, Some(Pos2::new(425.0, 280.0)));
        // a wide one under it
        let wide = Vec2::new(400.0, 100.0);
        let (main, skill) =
            node_tooltip_places(Pos2::new(100.0, 300.0), 20.0, tip, Some(wide), view);
        assert_eq!(main, Pos2::new(120.0, 280.0));
        assert_eq!(skill, Some(Pos2::new(120.0, 480.0)));
        // near the right and bottom edges both move in; the node's tooltip keeps its own clamp
        let (main, skill) =
            node_tooltip_places(Pos2::new(900.0, 650.0), 20.0, tip, Some(tall), view);
        // the pair is 500 wide (PoB leaves out the 5 between): 920 + 500 is 420 past the right
        // edge, and 630 + 250 is 150 past the bottom
        assert_eq!(skill, Some(Pos2::new(805.0, 480.0)));
        assert_eq!(main, Pos2::new(500.0, 530.0));
        // taller than the view: its top stays in it, and the skill's too
        let tall = Vec2::new(300.0, 900.0);
        let (main, skill) =
            node_tooltip_places(Pos2::new(100.0, 300.0), 20.0, tall, Some(tall), view);
        assert_eq!(main.y, view.top());
        assert_eq!(skill.unwrap().y, view.top());
        let (main, _) = node_tooltip_places(Pos2::new(100.0, 300.0), 20.0, tall, None, view);
        assert!(main.y >= view.top() && main.x >= view.left());
        // the compare-only tooltip: right of the square, else left of it and under it
        let corner = Pos2::new(100.0, 100.0);
        assert_eq!(
            hover_tooltip_place(corner, 40.0, tip, view),
            Pos2::new(145.0, 100.0)
        );
        assert_eq!(
            hover_tooltip_place(Pos2::new(900.0, 100.0), 40.0, tip, view),
            Pos2::new(595.0, 100.0)
        );
        assert_eq!(
            hover_tooltip_place(Pos2::new(100.0, 600.0), 40.0, Vec2::new(950.0, 200.0), view),
            Pos2::new(0.0, 440.0)
        );
    }

    /// The socket viewer is PoB's zoom 17 over the view's smaller side, on the socket
    /// (ItemSlotHelper.lua:21-26)
    #[test]
    fn the_socket_viewer_zooms_17_on_the_socket() {
        let rect = Rect::from_min_size(Pos2::new(50.0, 60.0), Vec2::new(308.0, 280.0));
        let camera = socket_viewer_camera(rect, 20_000.0, [1000.0, -500.0]);
        assert!((camera.zoom - 280.0 / 20_000.0 * 17.0).abs() < 1e-6);
        assert_eq!(camera.to_screen(rect, [1000.0, -500.0]), rect.center());
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

    /// A file that fails to decode, the decoder panicking included, stops the waiting, and its
    /// nodes' art counts as missing (drawn as dots)
    #[test]
    fn a_failed_decode_ends_the_wait_and_leaves_dots() {
        let dir = crate::test_dir::TestDir::new("bad-sheet");
        let path = dir.join("art.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot a png").unwrap();
        let bad = std::sync::Arc::new(crate::engine::SheetImage::lazy(path, [1, 1]));
        assert!(bad.claim());
        bad.decode_claimed();
        assert!(bad.is_failed());
        // a decoder that panics
        let panicked = crate::engine::SheetImage::lazy(dir.join("x.png"), [1, 1]);
        assert!(panicked.claim());
        panicked.decode_with(|_| panic!("a damaged file"));
        assert!(panicked.is_failed());
        // no thread to decode on
        let orphan =
            std::sync::Arc::new(crate::engine::SheetImage::lazy(dir.join("y.png"), [1, 1]));
        assert!(orphan.claim());
        let ctx = egui::Context::default();
        let started = decode_in_background(vec![orphan.clone()], &ctx, |_| {
            Err(std::io::Error::other("no threads"))
        });
        assert!(!started);
        assert!(orphan.is_failed());
        assert!(!bad.claim(), "a failed file isn't decoded again");
        let mut layout = sheet_layout(&bad);
        let sprite = crate::engine::Sprite {
            sheet: 0,
            rect: [0.0, 0.0, 1.0, 1.0],
        };
        let art = crate::engine::NodeArt {
            base: [Some(sprite); 3],
            ..Default::default()
        };
        assert!(art_missing(&layout, &art));
        layout.sheets[0] = Some(sheet());
        assert!(!art_missing(&layout, &art));
        // a failed file asks for no more frames
        let tab = TreeTab::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let up = upload_sheets(&tab.textures, ui.ctx(), &sheet_layout(&bad), &all());
            assert!(up[0].is_none());
        })
        .textures_delta
        .clear();
        // egui itself asks for the first few frames
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        for _ in 0..4 {
            out.textures_delta.clear();
            out = ctx.run_ui(egui::RawInput::default(), |ui| {
                upload_sheets(&tab.textures, ui.ctx(), &sheet_layout(&bad), &all());
            });
        }
        out.textures_delta.clear();
        assert!(
            out.viewport_output
                .values()
                .all(|v| v.repaint_delay == std::time::Duration::MAX),
            "the canvas still waits for a failed file: {:?}",
            out.viewport_output
                .values()
                .map(|v| v.repaint_delay)
                .collect::<Vec<_>>()
        );
    }

    /// Every batch of art is decoded on the same thread
    #[test]
    fn the_art_is_decoded_on_one_thread() {
        let (send, receive) = std::sync::mpsc::channel();
        for _ in 0..3 {
            let send = send.clone();
            art::send_to_art_thread(Box::new(move || {
                let me = std::thread::current();
                send.send((me.id(), me.name().map(String::from))).unwrap();
            }))
            .unwrap();
        }
        let ran: Vec<_> = (0..3).map(|_| receive.recv().unwrap()).collect();
        assert!(ran.iter().all(|r| r == &ran[0]), "{ran:?}");
        assert_eq!(ran[0].1.as_deref(), Some("tree-art"));
    }

    /// The rings' frames follow how fast their edge moves on screen
    #[test]
    fn ring_frames_follow_the_rings_on_screen() {
        let ring = |size: f32, spin: f32| crate::engine::RingImage {
            image: "jewelShadedOuterRing".into(),
            size,
            spin,
            x: 0.0,
            y: 0.0,
            tint: [1.0; 4],
        };
        let ms = |rings: &[crate::engine::RingImage], zoom| {
            ring_frame_interval(rings.iter(), zoom).map(|d| d.as_millis())
        };
        assert_eq!(ms(&[], 1.0), None);
        // a 2400 radius ring at fit zoom (0.03): its edge moves 1.5 px a second
        let big = ring(4800.0, -0.7);
        assert_eq!(ms(std::slice::from_ref(&big), 0.03), Some(661));
        // a small one barely moves: a frame every 2 s
        assert_eq!(ms(&[ring(100.0, 0.7)], 0.03), Some(2000));
        // zoomed in, a frame per pixel, but no more than 10 a second
        assert_eq!(ms(std::slice::from_ref(&big), 0.1), Some(198));
        assert_eq!(ms(std::slice::from_ref(&big), 0.27), Some(100));
        // the fastest ring counts
        assert_eq!(ms(&[ring(100.0, 0.7), big], 0.1), Some(198));
    }

    /// A file no image of the frame draws is neither decoded nor uploaded
    #[test]
    fn only_the_files_drawn_are_decoded() {
        let ctx = egui::Context::default();
        let tab = TreeTab::default();
        let dir = crate::test_dir::TestDir::new("unused-sheet");
        let path = dir.join("art.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let lazy = std::sync::Arc::new(crate::engine::SheetImage::lazy(path, [1, 1]));
        let up = upload_sheets(&tab.textures, &ctx, &sheet_layout(&lazy), &HashSet::new());
        assert!(up[0].is_none());
        assert!(
            lazy.is_unloaded(),
            "decoding started for a file nothing draws"
        );
        let ready = sheet();
        let up = upload_sheets(&tab.textures, &ctx, &sheet_layout(&ready), &[1].into());
        assert!(up[0].is_none());
        assert!(super::super::lock(&tab.textures).is_empty());
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
        let tab = TreeTab::default();
        let first = sheet();
        let up = upload_sheets(&tab.textures, &ctx, &sheet_layout(&first), &all());
        assert_eq!(up.len(), 2);
        let (id, size) = up[0].expect("uploaded");
        assert_eq!(size, Vec2::new(2.0, 1.0));
        assert!(up[1].is_none());
        // the texture holds the sheet now; the decoded copy is gone
        assert!(!first.has_rgba());
        // later frames draw from the texture
        let again = upload_sheets(&tab.textures, &ctx, &sheet_layout(&first), &all());
        assert_eq!(again[0].map(|s| s.0), Some(id));
        // a later layout's fresh copy of a sheet already up is dropped too, not kept
        let second = sheet();
        let next = upload_sheets(&tab.textures, &ctx, &sheet_layout(&second), &all());
        assert_eq!(next[0].map(|s| s.0), Some(id));
        assert!(!second.has_rgba());
        // without pixels nor a texture the sheet's sprites are left out, not a panic
        let other = TreeTab::default();
        assert!(upload_sheets(&other.textures, &ctx, &sheet_layout(&first), &all())[0].is_none());
        assert!(super::super::lock(&other.textures).is_empty());
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
        let tab = TreeTab::default();
        // the first frame starts the decoding and draws without it
        assert!(upload_sheets(&tab.textures, &ctx, &sheet_layout(&lazy), &all())[0].is_none());
        assert!(!lazy.is_unloaded());
        let start = std::time::Instant::now();
        let up = loop {
            if let Some(up) = upload_sheets(&tab.textures, &ctx, &sheet_layout(&lazy), &all())[0] {
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
        assert!(upload_sheets(&tab.textures, &ctx, &sheet_layout(&broken), &all())[0].is_none());
    }
}
