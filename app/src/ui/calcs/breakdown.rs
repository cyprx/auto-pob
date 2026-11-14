//! The Calcs tab's breakdown panel: what PoB's breakdown control shows for the hovered or
//! pinned cell. The state here decides which cell's answer is on show; the drawing is below.

use crate::engine::Command;
use crate::engine::views::{BreakdownRow, BreakdownSection, BreakdownView, CellRef};
use crate::ui::tree::Rects;
use crate::widgets::{pob_text, tooltip_ui};
use eframe::egui::{self, Align, Color32, Layout, Sense, Vec2};
use std::{cmp::Ordering, collections::HashMap, sync::Arc};

/// How long the pointer rests on a cell before the engine is asked about it (seconds)
pub const HOVER_DELAY: f64 = 0.1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Asc,
    Desc,
}

/// A table's active sort: by the column key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: String,
    pub dir: Dir,
}

/// What clicking a column header does: ascending, then descending, then PoB's own order.
pub fn next_sort(current: Option<&Sort>, key: &str) -> Option<Sort> {
    match current {
        Some(s) if s.key == key && s.dir == Dir::Asc => Some(Sort {
            key: key.to_owned(),
            dir: Dir::Desc,
        }),
        Some(s) if s.key == key => None,
        _ => Some(Sort {
            key: key.to_owned(),
            dir: Dir::Asc,
        }),
    }
}

/// A cell's text without colour codes or thousands separators, as a number if it is one.
fn number(text: &str) -> Option<f64> {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '^' {
            match chars.peek() {
                Some('x' | 'X') => {
                    chars.next();
                    for _ in 0..6 {
                        chars.next();
                    }
                }
                Some(d) if d.is_ascii_digit() => {
                    chars.next();
                }
                _ => plain.push(c),
            }
        } else if c != ',' {
            plain.push(c);
        }
    }
    plain.trim().parse().ok().filter(|n: &f64| n.is_finite())
}

/// The order the rows are shown in. Numbers compare as numbers when both cells are numbers.
pub fn row_order(rows: &[BreakdownRow], sort: Option<&Sort>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let Some(sort) = sort else { return order };
    let text = |i: usize| rows[i].cells.get(&sort.key).map_or("", String::as_str);
    // numbers before text; each kind in its own total order, so mixed columns can't confuse
    // the sort
    let cmp = |a: usize, b: usize| match (number(text(a)), number(text(b))) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => text(a).cmp(text(b)),
    };
    order.sort_by(|&a, &b| {
        if sort.dir == Dir::Desc {
            cmp(b, a)
        } else {
            cmp(a, b)
        }
    });
    order
}

/// Which cell's breakdown is wanted and what was answered.
#[derive(Default)]
pub struct State {
    /// The cell under the pointer and since when
    hover: Option<(CellRef, f64)>,
    /// The cell the last query asked about; answers for other cells are stale
    requested: Option<CellRef>,
    /// The pinned cell and its row's label
    pub pinned: Option<(CellRef, String)>,
    pub shown: Option<Arc<BreakdownView>>,
    /// The pinned cell must be asked about again (new pin, or new numbers)
    requery: bool,
    failed: bool,
    sorts: HashMap<usize, Sort>,
    /// The tree the node viewer shows, as the Tree tab has it
    pub node_scene: Option<crate::ui::tree::SocketScene>,
}

impl State {
    #[cfg(test)]
    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn visible(&self) -> bool {
        self.shown.is_some() || self.pinned.is_some() || self.failed
    }

    /// New numbers from the engine: whatever is on show is out of date, so it is asked for
    /// again (the pinned cell, or else the one on show).
    pub fn calcs_changed(&mut self) {
        if self.pinned.is_some() || self.shown.is_some() {
            self.requery = true;
        } else {
            self.requested = None;
        }
    }

    /// A click on a cell: pins it, or unpins it if it is the pinned one.
    pub fn click(&mut self, cell: CellRef, label: String) {
        if self.pinned.as_ref().is_some_and(|(p, _)| *p == cell) {
            self.pinned = None;
            return;
        }
        self.requery = self.shown.as_ref().is_none_or(|v| v.cell != cell);
        self.requested = Some(cell.clone());
        self.pinned = Some((cell, label));
    }

    pub fn on_view(&mut self, view: &Arc<BreakdownView>) {
        let wanted = match &self.pinned {
            Some((p, _)) => *p == view.cell,
            None => self.requested.as_ref() == Some(&view.cell),
        };
        if !wanted {
            return;
        }
        self.failed = view.error.is_some();
        if self.failed {
            self.shown = None;
            return;
        }
        if view.sections.is_empty() {
            // a cell with no breakdown (any more) can't stay pinned
            self.pinned = None;
            self.shown = None;
            return;
        }
        if self.shown.as_ref().is_none_or(|v| v.cell != view.cell) {
            self.sorts.clear();
        }
        self.shown = Some(view.clone());
    }

    /// Decides this frame's query. `hovered`: the cell under the pointer, if any.
    pub fn drive(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        hovered: Option<&CellRef>,
        queries: &mut Vec<Command>,
    ) {
        if let Some((cell, _)) = &self.pinned {
            self.hover = None;
            if std::mem::take(&mut self.requery) {
                self.requested = Some(cell.clone());
                queries.push(Command::Breakdown(cell.clone()));
            }
            return;
        }
        if std::mem::take(&mut self.requery)
            && let Some(shown) = &self.shown
        {
            // new numbers: the panel on show is refreshed even if the pointer has moved on
            self.requested = Some(shown.cell.clone());
            queries.push(Command::Breakdown(shown.cell.clone()));
        }
        let Some(cell) = hovered else {
            self.hover = None;
            if self.failed {
                // the pointer left the failed cell; hovering it again asks again
                self.requested = None;
            }
            return;
        };
        if self.hover.as_ref().is_none_or(|(h, _)| h != cell) {
            self.hover = Some((cell.clone(), now));
        }
        if self.requested.as_ref() == Some(cell) {
            return;
        }
        let waited = now - self.hover.as_ref().map_or(now, |(_, since)| *since);
        if waited >= HOVER_DELAY {
            self.requested = Some(cell.clone());
            queries.push(Command::Breakdown(cell.clone()));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY - waited));
        }
    }

    // ---- drawing -------------------------------------------------------------------------

    /// The panel's contents. Returns the node the user clicked, and whether the chip was.
    pub fn show(&mut self, ui: &mut egui::Ui, rects: &mut Rects) -> (Option<u32>, bool) {
        let mut focus = None;
        let mut unpin = false;
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            if let Some((_, label)) = &self.pinned {
                let chip = ui.button(format!("Pinned: {label} \u{2715}"));
                rects.note(&chip, || "bd_pin".into());
                unpin = chip.clicked();
            }
            if self.failed {
                ui.label("Breakdown unavailable");
            }
            let Some(view) = self.shown.clone() else {
                return;
            };
            let mut table = 0;
            for section in &view.sections {
                match section {
                    BreakdownSection::Text { lines, size } => {
                        let default = ui.visuals().text_color();
                        for line in lines {
                            ui.label(pob_text(line, *size as f32, default));
                        }
                    }
                    BreakdownSection::Table {
                        label,
                        footer,
                        columns,
                        rows,
                    } => {
                        let default = ui.visuals().text_color();
                        if let Some(label) = label {
                            ui.label(pob_text(label, 16.0, default));
                        }
                        let sort = self.sorts.get(&table).cloned();
                        let order = row_order(rows, sort.as_ref());
                        let mut clicked_head = None;
                        egui::Grid::new(("calcs_breakdown_table", table))
                            .striped(true)
                            .show(ui, |ui| {
                                for column in columns {
                                    let arrow = match sort.as_ref().filter(|s| s.key == column.key)
                                    {
                                        Some(Sort { dir: Dir::Asc, .. }) => " \u{25B2}",
                                        Some(_) => " \u{25BC}",
                                        None => "",
                                    };
                                    let text = format!("{}{arrow}", column.label);
                                    let head = ui.with_layout(align(column.right), |ui| {
                                        ui.add(egui::Button::new(text).frame(false))
                                    });
                                    rects.note(&head.inner, || {
                                        format!("bd_head_{table}_{}", column.key)
                                    });
                                    if head.inner.clicked() {
                                        clicked_head = Some(column.key.clone());
                                    }
                                }
                                ui.end_row();
                                for &ri in &order {
                                    let row = &rows[ri];
                                    for column in columns {
                                        let scene = self.node_scene.as_ref();
                                        let r = ui.with_layout(align(column.right), |ui| {
                                            cell(ui, row, &column.key, scene)
                                        });
                                        rects.note(&r.inner.0, || {
                                            format!("bd_cell_{table}_{ri}_{}", column.key)
                                        });
                                        if let Some(id) = r.inner.1 {
                                            focus = Some(id);
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                        if let Some(key) = clicked_head {
                            match next_sort(sort.as_ref(), &key) {
                                Some(s) => self.sorts.insert(table, s),
                                None => self.sorts.remove(&table),
                            };
                        }
                        if let Some(footer) = footer {
                            ui.label(pob_text(footer, 14.0, default));
                        }
                        table += 1;
                    }
                    BreakdownSection::Radius { radius } => radius_box(ui, *radius),
                }
                ui.add_space(6.0);
            }
        });
        (focus, unpin)
    }
}

fn align(right: bool) -> Layout {
    if right {
        Layout::top_down(Align::Max)
    } else {
        Layout::top_down(Align::Min)
    }
}

/// One table cell. A cell with a tooltip shows it on hover; one without but with a node is a
/// link to that node (PoB shows a node location only when there is no tooltip function).
fn cell(
    ui: &mut egui::Ui,
    row: &BreakdownRow,
    key: &str,
    scene: Option<&crate::ui::tree::SocketScene>,
) -> (egui::Response, Option<u32>) {
    let text = row.cells.get(key).map_or("", String::as_str);
    let tooltip = row.tooltips.get(key).filter(|t| !t.is_empty());
    let node = row.nodes.get(key).copied().filter(|_| tooltip.is_none());
    let colour = if node.is_some() {
        ui.visuals().hyperlink_color
    } else {
        ui.visuals().text_color()
    };
    let mut label = egui::Label::new(pob_text(text, 14.0, colour));
    if node.is_some() {
        label = label.sense(Sense::click());
    }
    let mut response = ui.add(label);
    if let Some(lines) = tooltip {
        response = response.on_hover_ui(|ui| tooltip_ui(ui, lines));
    }
    let mut focus = None;
    if let Some(id) = node {
        response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            focus = Some(id);
        }
        if response.hovered()
            && let Some(scene) = scene
        {
            node_viewer(ui, &response, scene, id);
        }
    }
    (response, focus)
}

/// The node viewer PoB shows by a hovered node cell: the node's place in the tree, 300 square
/// at zoom 5 inside a 2-point white border, right of the cell or else left of it, held inside
/// the screen, with a red ring on the node (CalcBreakdownControl.lua:594-611)
fn node_viewer(
    ui: &egui::Ui,
    cell: &egui::Response,
    scene: &crate::ui::tree::SocketScene,
    node: u32,
) {
    let rect = node_viewer_rect(cell.rect, ui.ctx().content_rect());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        ui.scope_id().with("calcs_node_viewer"),
    ));
    crate::ui::tree::canvas::paint_tree_viewer(
        ui.ctx(),
        &painter,
        rect,
        scene,
        node,
        NODE_VIEWER_ZOOM,
        2.0,
        crate::ui::tree::canvas::ViewerMark::Ring,
    );
}

/// Where the node viewer's tree goes for a cell at `cell` (inside its 2-point border): right of
/// the cell, else left of it; no lower than the screen allows (CalcBreakdownControl.lua:595-600)
pub fn node_viewer_rect(cell: egui::Rect, screen: egui::Rect) -> egui::Rect {
    let mut x = cell.right() + 5.0;
    if screen.right() < x + 304.0 {
        x = cell.left() - 309.0;
    }
    let y = cell.top().min(screen.bottom() - 304.0);
    egui::Rect::from_min_size(egui::pos2(x + 2.0, y + 2.0), Vec2::splat(300.0))
}

/// The node viewer's zoom (CalcBreakdownControl.lua:602)
const NODE_VIEWER_ZOOM: f32 = 5.0;

/// PoB's radius visual's size: 1920 by 1080 at a quarter (CalcBreakdownControl.lua:140-147)
const RADIUS_SIZE: Vec2 = Vec2::new(480.0, 270.0);

/// PoB's isometric camera, world units to the visual's pixels, rounded (Main.lua:1486-1499)
fn world_to_screen(x: f64, y: f64, width: f64, height: f64) -> (i64, i64) {
    let cos45 = (std::f64::consts::PI / 4.0).cos();
    let cos35 = (std::f64::consts::PI * 0.195).cos();
    let sin35 = (std::f64::consts::PI * 0.195).sin();
    let cx = (x - y) * cos45;
    let cy = -5.33 - (y + x) * cos45 * cos35;
    let cz = 122.0 + (y + x) * cos45 * sin35;
    let sx = width * 0.5 + cx / cz * 1.27 * height;
    let sy = height * 0.5 + cy / cz * 1.27 * height;
    ((sx + 0.5).floor() as i64, (sy + 0.5).floor() as i64)
}

/// The rows of PoB's filled circle of `radius` on the ground (main:RenderCircle, Main.lua:
/// 1502-1523): each pixel row's `(y, left, right)`, top to bottom
pub fn radius_rows(width: f64, height: f64, radius: f64) -> Vec<(i64, i64, i64)> {
    let mut rows: std::collections::BTreeMap<i64, (i64, i64)> = Default::default();
    // PoB's `for d = 0, 360, 0.15`, the step added each time as LuaJIT does
    let mut d = 0.0f64;
    while d <= 360.0 {
        let r = d / 180.0 * std::f64::consts::PI;
        let (px, py) = world_to_screen(r.sin() * radius, r.cos() * radius, width, height);
        if py >= 0 && (py as f64) < height {
            let px = px.clamp(0, width as i64);
            let row = rows.entry(py).or_insert((px, px));
            row.0 = row.0.min(px);
            row.1 = row.1.max(px);
        }
        d += 0.15;
    }
    rows.into_iter().map(|(y, (l, r))| (y, l, r)).collect()
}

/// PoB's radius visual (CalcBreakdownControl:DrawRadiusVisual): the range guide at 3/4
/// brightness, the radius as a translucent green disc in the game's perspective, and the game's
/// UI over it. 480 by 270, or smaller to fit.
fn radius_box(ui: &mut egui::Ui, radius: f64) {
    let width = ui.available_width().clamp(120.0, RADIUS_SIZE.x);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, width * RADIUS_SIZE.y / RADIUS_SIZE.x),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(12));
    let ctx = ui.ctx().clone();
    crate::widgets::art::paint(
        &ctx,
        &painter,
        "Assets/range_guide.png",
        rect,
        Color32::from_gray(191),
    );
    let scale = width / RADIUS_SIZE.x;
    let green = Color32::from_rgba_unmultiplied(128, 255, 128, 84);
    for (y, left, right) in radius_rows(f64::from(RADIUS_SIZE.x), f64::from(RADIUS_SIZE.y), radius)
    {
        let min = rect.min + Vec2::new(left as f32, y as f32) * scale;
        let size = Vec2::new((right - left + 1) as f32, 1.0) * scale;
        painter.rect_filled(egui::Rect::from_min_size(min, size), 0.0, green);
    }
    crate::widgets::art::paint(
        &ctx,
        &painter,
        "Assets/game_ui_small.png",
        rect,
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PoB's node viewer sits right of the cell, or left when the screen ends first, and never
    /// below the screen (CalcBreakdownControl.lua:595-600)
    #[test]
    fn the_node_viewer_sits_where_pobs_does() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let cell =
            |x: f32, y: f32| egui::Rect::from_min_size(egui::pos2(x, y), Vec2::new(80.0, 14.0));
        assert_eq!(
            node_viewer_rect(cell(100.0, 50.0), screen).min,
            egui::pos2(187.0, 52.0)
        );
        // 700 + 5 + 304 is past 1000: on the left, 309 from the cell
        assert_eq!(
            node_viewer_rect(cell(620.0, 50.0), screen).min,
            egui::pos2(313.0, 52.0)
        );
        // low on the screen: held up
        assert_eq!(
            node_viewer_rect(cell(100.0, 600.0), screen).min,
            egui::pos2(187.0, 398.0)
        );
        assert_eq!(
            node_viewer_rect(cell(100.0, 50.0), screen).size(),
            Vec2::splat(300.0)
        );
    }
}
