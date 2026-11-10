//! The power report: PoB's table of the nodes worth taking, sortable by column.

use super::Rects;
use crate::engine::{PowerReport, PowerReportRow};
use crate::theme;
use crate::widgets::{ListColumn, list, list_sort};
use eframe::egui;
use std::cmp::Ordering;

const COLUMNS: [ListColumn<'static>; 4] = [
    ListColumn {
        title: "Node",
        width: 170.0,
    },
    ListColumn {
        title: "Power",
        width: 70.0,
    },
    ListColumn {
        title: "Path power",
        width: 90.0,
    },
    ListColumn {
        title: "Points",
        width: 50.0,
    },
];

pub(super) struct ReportPanel {
    id: egui::Id,
    selected: Option<usize>,
}

impl Default for ReportPanel {
    fn default() -> Self {
        Self {
            id: egui::Id::unique("tree_power_report"),
            selected: None,
        }
    }
}

impl ReportPanel {
    /// Draws the report. Returns the node whose row was clicked.
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        report: Option<&PowerReport>,
        rects: &mut Rects,
    ) -> Option<u32> {
        theme::heading(ui, "Power report");
        let Some(report) = report else {
            ui.label("Waiting for the calculation…");
            return None;
        };
        ui.label(format!("Per point of {}", report.stat));
        let id = self.id;
        let order = sorted(&report.rows, list_sort(ui.ctx(), id));
        let before = self.selected;
        let action = list(
            ui,
            id,
            &COLUMNS,
            order.len(),
            |i, row| {
                let r = &report.rows[order[i]];
                let text = |s: &str| {
                    let text = egui::RichText::new(s.to_owned());
                    if r.allocated {
                        text.color(theme::TEXT_DIM)
                    } else {
                        text
                    }
                };
                let (_, name) = row.col(|ui| {
                    ui.label(text(&r.name));
                });
                rects.note(&name, || format!("report_row:{i}"));
                row.col(|ui| {
                    ui.label(text(&r.power_text));
                });
                row.col(|ui| {
                    ui.label(text(&r.path_power_text));
                });
                row.col(|ui| {
                    ui.label(text(&r.path_dist.to_string()));
                });
            },
            &mut self.selected,
        );
        let row = match action {
            Some(crate::widgets::ListAction::Activate(i)) => Some(i),
            None if self.selected != before => self.selected,
            None => None,
        };
        row.and_then(|i| order.get(i)).map(|&i| report.rows[i].id)
    }
}

/// The row indices in the order the user's sort chose; unsorted rows keep PoB's order.
fn sorted(rows: &[PowerReportRow], sort: Option<crate::widgets::ListSort>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if let Some(sort) = sort {
        let key = |a: &PowerReportRow, b: &PowerReportRow| match sort.column {
            0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            1 => a.power.partial_cmp(&b.power).unwrap_or(Ordering::Equal),
            2 => a
                .path_power
                .partial_cmp(&b.path_power)
                .unwrap_or(Ordering::Equal),
            _ => a.path_dist.cmp(&b.path_dist),
        };
        order.sort_by(|&a, &b| {
            let ordering = key(&rows[a], &rows[b]);
            if sort.ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::ListSort;

    fn row(name: &str, power: f64, path: f64, dist: u32) -> PowerReportRow {
        PowerReportRow {
            id: 0,
            name: name.into(),
            kind: "Notable".into(),
            power,
            power_text: String::new(),
            path_power: path,
            path_power_text: String::new(),
            allocated: false,
            path_dist: dist,
        }
    }

    #[test]
    fn sorts_by_the_chosen_column_either_way() {
        let rows = [
            row("b", 1.0, 9.0, 3),
            row("A", 3.0, 2.0, 1),
            row("c", 2.0, 5.0, 2),
        ];
        let by = |column, ascending| sorted(&rows, Some(ListSort { column, ascending }));
        assert_eq!(sorted(&rows, None), [0, 1, 2]);
        assert_eq!(by(0, true), [1, 0, 2]);
        assert_eq!(by(1, false), [1, 2, 0]);
        assert_eq!(by(2, true), [1, 2, 0]);
        assert_eq!(by(3, false), [0, 2, 1]);
    }
}
