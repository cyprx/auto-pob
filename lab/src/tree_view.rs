//! Draws the passive tree with egui's painter: pan by dragging, zoom with the scroll wheel.

use crate::engine::{Sprite, Tree, TreeNode};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

/// A node's node-power result, shown as a heatmap.
pub struct Heat {
    pub per_point: f64,
    pub distance: u32,
}

/// Camera over the tree.
#[derive(Default)]
pub struct TreeView {
    /// Tree position at the centre of the view
    center: Vec2,
    /// Screen points per tree unit
    zoom: f32,
    /// Fit the whole tree into the view on the next frame
    needs_fit: bool,
    /// Uploaded sprite sheets, by file
    textures: HashMap<PathBuf, TextureHandle>,
}

/// An uploaded sprite sheet: its texture and size in pixels.
type Sheet = (egui::TextureId, Vec2);

const ALLOCATED: Color32 = Color32::from_rgb(230, 185, 80);
const EDGE: Color32 = Color32::from_rgb(72, 62, 50);
const HOVER: Color32 = Color32::WHITE;
/// Heatmap colours for the lowest and highest positive score
const HEAT_LOW: Color32 = Color32::from_rgb(40, 70, 140);
const HEAT_HIGH: Color32 = Color32::from_rgb(255, 80, 40);
/// Below this many screen points, icons are unreadable and plain dots are drawn instead
const MIN_ICON_SIZE: f32 = 8.0;
/// PoB draws sprites at their pixel size times this, in tree units (`PassiveTreeView:DrawAsset`)
const SPRITE_SCALE: f32 = 1.33 * 2.0;

impl TreeView {
    pub fn fit(&mut self) {
        self.needs_fit = true;
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        tree: &Tree,
        allocated: &HashSet<u32>,
        heat: &HashMap<u32, Heat>,
    ) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, crate::theme::BACKGROUND);

        if self.needs_fit || self.zoom == 0.0 {
            self.fit_to(rect, tree);
        }
        self.handle_input(ui, &response);
        let sheets = self.upload_sheets(ui.ctx(), tree);

        let to_screen = |p: [f32; 2]| rect.center() + (Vec2::from(p) - self.center) * self.zoom;
        // drawing a little outside the view avoids popping at the edges
        let visible = rect.expand(50.0);

        // connections
        for &(a, b) in &tree.edges {
            let (na, nb) = (&tree.nodes[a], &tree.nodes[b]);
            let (pa, pb) = (to_screen(na.pos), to_screen(nb.pos));
            if !visible.contains(pa) && !visible.contains(pb) {
                continue;
            }
            let color = if allocated.contains(&na.id) && allocated.contains(&nb.id) {
                ALLOCATED
            } else {
                EDGE
            };
            let stroke = Stroke::new((6.0 * self.zoom).clamp(1.0, 4.0), color);
            match arc_center(na, nb) {
                Some(center) => painter.line(
                    arc_points(center, na.pos, nb.pos, self.zoom, to_screen),
                    stroke,
                ),
                None => painter.line_segment([pa, pb], stroke),
            };
        }

        // nodes, remembering the one under the pointer
        let pointer = response.hover_pos();
        let mut hovered: Option<(&TreeNode, f32)> = None;
        let max_heat = heat.values().map(|h| h.per_point).fold(0.0_f64, f64::max);
        for node in &tree.nodes {
            let pos = to_screen(node.pos);
            if !visible.contains(pos) {
                continue;
            }
            let radius = (node_radius(&node.kind) * self.zoom).max(1.5);
            let is_allocated = allocated.contains(&node.id);
            let heat_fill = heat
                .get(&node.id)
                .filter(|h| h.per_point > 0.0 && !is_allocated)
                .map(|h| heat_color((h.per_point / max_heat) as f32));

            let state = usize::from(is_allocated);
            let icon = node.art.icon[state]
                .and_then(|sprite| Some((sprite, sheets.get(sprite.sheet).copied().flatten()?)));
            match icon {
                Some((sprite, sheet)) if sprite_size(sprite, self.zoom).x >= MIN_ICON_SIZE => {
                    if let Some(fill) = heat_fill {
                        // a glow behind the icon, so the heatmap stays visible
                        painter.circle_filled(pos, radius * 1.4, fill);
                    }
                    draw_sprite(&painter, sheet, sprite, pos, self.zoom);
                    let frame = node.art.frame[state].and_then(|sprite| {
                        Some((sprite, sheets.get(sprite.sheet).copied().flatten()?))
                    });
                    if let Some((frame, sheet)) = frame {
                        draw_sprite(&painter, sheet, frame, pos, self.zoom);
                    }
                }
                _ => {
                    let fill = if is_allocated {
                        ALLOCATED
                    } else {
                        heat_fill.unwrap_or_else(|| kind_color(&node.kind))
                    };
                    painter.circle_filled(pos, radius, fill);
                    if matches!(node.kind.as_str(), "Notable" | "Keystone") {
                        painter.circle_stroke(
                            pos,
                            radius,
                            Stroke::new(1.0, Color32::from_gray(160)),
                        );
                    }
                }
            }

            if let Some(pointer) = pointer {
                let distance = pointer.distance(pos);
                if distance <= radius.max(6.0) && hovered.is_none_or(|(_, d)| distance < d) {
                    hovered = Some((node, distance));
                }
            }
        }

        painter.text(
            rect.left_top() + Vec2::splat(8.0),
            egui::Align2::LEFT_TOP,
            "Drag to pan, scroll to zoom",
            egui::FontId::proportional(12.0),
            Color32::from_gray(120),
        );

        if let Some((node, _)) = hovered {
            let pos = to_screen(node.pos);
            let radius = (node_radius(&node.kind) * self.zoom).max(1.5);
            painter.circle_stroke(pos, radius + 2.0, Stroke::new(2.0, HOVER));
            response.on_hover_ui_at_pointer(|ui| {
                ui.strong(if node.name.is_empty() {
                    &node.kind
                } else {
                    &node.name
                });
                for line in &node.stats {
                    ui.label(line);
                }
                if allocated.contains(&node.id) {
                    ui.colored_label(ALLOCATED, "Allocated");
                } else if let Some(h) = heat.get(&node.id) {
                    ui.separator();
                    ui.label(format!(
                        "{:+.2} per point ({} point{} away)",
                        h.per_point,
                        h.distance,
                        if h.distance == 1 { "" } else { "s" }
                    ));
                }
            });
        }
    }

    /// Uploads sprite sheets that aren't on the GPU yet. Returns them indexed like
    /// [`Tree::sheets`].
    fn upload_sheets(&mut self, ctx: &egui::Context, tree: &Tree) -> Vec<Option<Sheet>> {
        tree.sheets
            .iter()
            .map(|sheet| {
                let sheet = sheet.as_ref()?;
                let texture = self.textures.entry(sheet.path.clone()).or_insert_with(|| {
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

    fn fit_to(&mut self, rect: Rect, tree: &Tree) {
        let bounds = tree.nodes.iter().fold(Rect::NOTHING, |r, n| {
            r.union(Rect::from_pos(Pos2::from(n.pos)))
        });
        if bounds.is_positive() {
            self.center = bounds.center().to_vec2();
            self.zoom = (rect.width() / bounds.width()).min(rect.height() / bounds.height()) * 0.95;
            self.needs_fit = false;
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.dragged() {
            self.center -= response.drag_delta() / self.zoom;
        }
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let factor = (scroll * 0.002).exp() * ui.input(|i| i.zoom_delta());
            if factor != 1.0 {
                // zoom around the pointer: keep the tree point under it in place
                let offset = pointer - response.rect.center();
                let under_pointer = self.center + offset / self.zoom;
                self.zoom = (self.zoom * factor).clamp(0.01, 2.0);
                self.center = under_pointer - offset / self.zoom;
            }
        }
    }
}

/// If both nodes sit on the same orbit of the same group, PoB connects them with an arc
/// around the group's centre.
fn arc_center(a: &TreeNode, b: &TreeNode) -> Option<[f32; 2]> {
    match (a.orbit, b.orbit, a.group_center) {
        (Some(oa), Some(ob), Some(center)) if oa == ob && oa.1 > 0 => Some(center),
        _ => None,
    }
}

/// Points along the shorter arc from `a` to `b` around `center`, in screen space.
fn arc_points(
    center: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
    zoom: f32,
    to_screen: impl Fn([f32; 2]) -> Pos2,
) -> Vec<Pos2> {
    let c = Vec2::from(center);
    let (va, vb) = (Vec2::from(a) - c, Vec2::from(b) - c);
    let radius = va.length();
    let start = va.angle();
    let mut sweep = vb.angle() - start;
    // take the short way round
    if sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    } else if sweep < -std::f32::consts::PI {
        sweep += std::f32::consts::TAU;
    }
    let segments = ((sweep.abs() * radius * zoom / 6.0) as usize).clamp(2, 48);
    (0..=segments)
        .map(|i| {
            let angle = start + sweep * i as f32 / segments as f32;
            to_screen((c + Vec2::angled(angle) * radius).into())
        })
        .collect()
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
) {
    let [x, y, w, h] = sprite.rect;
    let uv = Rect::from_min_size(
        Pos2::new(x / sheet_size.x, y / sheet_size.y),
        Vec2::new(w / sheet_size.x, h / sheet_size.y),
    );
    let rect = Rect::from_center_size(pos, sprite_size(sprite, zoom));
    painter.image(texture, rect, uv, Color32::WHITE);
}

/// Approximate node size in tree units, by PoB node type.
fn node_radius(kind: &str) -> f32 {
    match kind {
        "Notable" => 38.0,
        "Keystone" => 55.0,
        "Mastery" => 30.0,
        "Socket" => 35.0,
        "ClassStart" => 60.0,
        _ => 25.0,
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

/// `t` in 0..=1, from the lowest to the highest score.
fn heat_color(t: f32) -> Color32 {
    HEAT_LOW.lerp_to_gamma(HEAT_HIGH, t.clamp(0.0, 1.0))
}
