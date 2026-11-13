//! A keyboard-navigable, header-sortable table on `egui_extras::TableBuilder`.
//!
//! Sorting: clicking a header cycles that column ascending, then descending; the state lives
//! in egui memory under the list's id. The widget does not reorder rows itself: the caller
//! reads [`list_sort`] before building its rows and supplies them already sorted.

use crate::theme;
use eframe::egui::{self, Key, Label, Sense};
use egui_extras::{Column, TableBuilder, TableRow};

pub struct ListColumn<'a> {
    pub title: &'a str,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAction {
    /// Double-click or Enter on a row.
    Activate(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSort {
    pub column: usize,
    pub ascending: bool,
}

/// Where a dragged row ends up, as `ListControl`'s drop does it. `hovered` is the row the
/// pointer is over and `below` whether it is in that row's lower half. `None` when the drop
/// changes nothing (the gap is right above or right below the dragged row).
pub fn drop_index(from: usize, hovered: usize, below: bool) -> Option<usize> {
    let gap = hovered + usize::from(below);
    if gap == from || gap == from + 1 {
        None
    } else if gap > from {
        Some(gap - 1)
    } else {
        Some(gap)
    }
}

const ROW_HEIGHT: f32 = 20.0;

/// The sort the user chose by clicking a header of the list with this `id`, if any.
pub fn list_sort(ctx: &egui::Context, id: egui::Id) -> Option<ListSort> {
    ctx.data(|d| d.get_temp(id))
}

/// Draws a list of `rows` rows. `row` fills each row's cells. Up/Down move `selected`.
/// Returns [`ListAction::Activate`] on double-click, or on Enter while the list has focus.
pub fn list(
    ui: &mut egui::Ui,
    id: egui::Id,
    columns: &[ListColumn],
    rows: usize,
    row: impl FnMut(usize, &mut TableRow),
    selected: &mut Option<usize>,
) -> Option<ListAction> {
    list_with(ui, id, columns, rows, row, selected, true)
}

/// [`list`], with header sorting switched off when `sortable` is false (the caller's order is
/// then final, and the headers are plain titles).
pub fn list_with(
    ui: &mut egui::Ui,
    id: egui::Id,
    columns: &[ListColumn],
    rows: usize,
    mut row: impl FnMut(usize, &mut TableRow),
    selected: &mut Option<usize>,
    sortable: bool,
) -> Option<ListAction> {
    let mut action = None;
    let mut scroll_to = None;

    // keyboard: only while this list has focus (clicking a row gives it focus)
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                vertical_arrows: true,
                ..Default::default()
            },
        )
    });
    if ui.memory(|m| m.has_focus(id)) && rows > 0 {
        let (down, up, enter) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
            )
        });
        let last = rows - 1;
        let new = match (*selected, down, up) {
            (None, true, _) | (None, _, true) => Some(0),
            (Some(i), true, _) => Some((i + 1).min(last)),
            (Some(i), _, true) => Some(i.saturating_sub(1)),
            (s, _, _) => s.map(|i| i.min(last)),
        };
        if new != *selected {
            *selected = new;
            scroll_to = new;
        }
        if enter && let Some(i) = *selected {
            action = Some(ListAction::Activate(i));
        }
    }

    let sort = list_sort(ui.ctx(), id);
    let mut clicked_row = false;
    let mut new_sort = None;
    // a hovered row gets the list rows' warm fill. That also tints a hovered widget's fill in
    // the table (the scroll bar's handle), so cells hold plain labels only.
    theme::table_rows(ui, |ui| {
        let mut table = TableBuilder::new(ui)
            .id_salt(id)
            .striped(true)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        for c in columns {
            table = table.column(Column::exact(c.width));
        }
        if let Some(i) = scroll_to {
            table = table.scroll_to_row(i, None);
        }
        table
            .header(ROW_HEIGHT, |mut header| {
                for (i, c) in columns.iter().enumerate() {
                    header.col(|ui| {
                        let arrow = match sort {
                            Some(s) if s.column == i => {
                                if s.ascending {
                                    " ^"
                                } else {
                                    " v"
                                }
                            }
                            _ => "",
                        };
                        let text = egui::RichText::new(format!("{}{arrow}", c.title))
                            .color(theme::HEADING)
                            .strong();
                        let sense = if sortable {
                            Sense::click()
                        } else {
                            Sense::hover()
                        };
                        if ui.add(Label::new(text).sense(sense)).clicked() {
                            new_sort = Some(ListSort {
                                column: i,
                                ascending: !matches!(sort, Some(s) if s.column == i && s.ascending),
                            });
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, rows, |mut r| {
                    let i = r.index();
                    r.set_selected(*selected == Some(i));
                    row(i, &mut r);
                    let resp = r.response();
                    if resp.clicked() {
                        *selected = Some(i);
                        clicked_row = true;
                    }
                    if resp.double_clicked() {
                        action = Some(ListAction::Activate(i));
                    }
                });
            });
    });

    if let Some(s) = new_sort {
        ui.ctx().data_mut(|d| d.insert_temp(id, s));
    }
    if clicked_row {
        ui.memory_mut(|m| m.request_focus(id));
    }
    // keep the focus alive across frames
    let layer = ui.layer_id();
    ui.memory_mut(|m| m.interested_in_focus(id, layer));
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Event;

    fn frame(
        ctx: &egui::Context,
        events: Vec<Event>,
        selected: &mut Option<usize>,
        focus: bool,
    ) -> Option<ListAction> {
        let id = egui::Id::unique("test-list");
        let mut out = None;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            if focus {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let cols = [ListColumn {
                title: "Name",
                width: 120.0,
            }];
            out = list(
                ui,
                id,
                &cols,
                3,
                |i, row| {
                    row.col(|ui| {
                        ui.label(format!("row {i}"));
                    });
                },
                selected,
            );
        })
        .drop_without_applying_deltas();
        out
    }

    fn key(k: Key) -> Vec<Event> {
        vec![Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    #[test]
    fn renders_and_navigates_with_keys() {
        let ctx = egui::Context::default();
        let mut sel = None;
        assert_eq!(frame(&ctx, vec![], &mut sel, true), None);
        assert_eq!(frame(&ctx, key(Key::ArrowDown), &mut sel, true), None);
        assert_eq!(sel, Some(0));
        frame(&ctx, key(Key::ArrowDown), &mut sel, true);
        assert_eq!(sel, Some(1));
        frame(&ctx, key(Key::ArrowUp), &mut sel, true);
        assert_eq!(sel, Some(0));
        assert_eq!(
            frame(&ctx, key(Key::Enter), &mut sel, true),
            Some(ListAction::Activate(0))
        );
    }

    #[test]
    fn ignores_keys_without_focus() {
        let ctx = egui::Context::default();
        let mut sel = None;
        frame(&ctx, vec![], &mut sel, false);
        frame(&ctx, key(Key::ArrowDown), &mut sel, false);
        assert_eq!(sel, None);
    }
}
