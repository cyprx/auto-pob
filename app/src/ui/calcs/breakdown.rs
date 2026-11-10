//! The Calcs tab's breakdown panel: what PoB's breakdown control shows for the hovered or
//! pinned cell. The state here decides which cell's answer is on show; the drawing is below.

use crate::engine::Command;
use crate::engine::views::{BreakdownRow, BreakdownSection, BreakdownView, CellRef};
use crate::ui::tree::Rects;
use crate::widgets::{pob_text, tooltip_ui};
use eframe::egui::{self, Align, Color32, Layout, Pos2, Sense, Stroke, StrokeKind, Vec2};
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
}

impl State {
    #[cfg(test)]
    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn visible(&self) -> bool {
        self.shown.is_some() || self.pinned.is_some() || self.failed
    }

    /// New numbers from the engine: whatever is on show is out of date.
    pub fn calcs_changed(&mut self) {
        if self.pinned.is_some() {
            self.requery = true;
        } else {
            self.requested = None;
        }
    }

    pub fn query_failed(&mut self) {
        self.failed = true;
        self.shown = None;
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
        self.failed = false;
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
                                        let r = ui.with_layout(align(column.right), |ui| {
                                            cell(ui, row, &column.key)
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
fn cell(ui: &mut egui::Ui, row: &BreakdownRow, key: &str) -> (egui::Response, Option<u32>) {
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
    }
    (response, focus)
}

/// R12: the radius as a circle on a dark 16:9 box standing for 1920 by 1080 world units.
fn radius_box(ui: &mut egui::Ui, radius: f64) {
    let width = ui.available_width().clamp(120.0, 360.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, width * 9.0 / 16.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(12));
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(70)),
        StrokeKind::Inside,
    );
    let scale = width / 1920.0;
    let centre: Pos2 = rect.center();
    painter.circle_stroke(
        centre,
        radius as f32 * scale,
        Stroke::new(1.5, Color32::from_rgb(255, 255, 255)),
    );
    painter.circle_filled(centre, 2.0, Color32::from_rgb(255, 255, 255));
}
