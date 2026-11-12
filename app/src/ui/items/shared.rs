//! PoB's shared items (NF-2): the list under "Craft item..." that every build shares, kept in
//! Settings.xml. Rows drag onto slots and the item list; build items and database rows drag
//! onto it; Delete asks first, as PoB does.

use super::db::DbDrag;
use super::list::reorder_rows;
use super::{ItemDrag, ItemsOutput, ItemsUi};
use crate::engine::views::ItemsView;
use crate::engine::{Command, ItemTooltipQuery, SharedItemSource};
use crate::widgets::pob_text;
use eframe::egui::{self, Color32, Rect};
use std::sync::Arc;

/// The payload of a shared item row being dragged: onto a slot, the item list or another row
#[derive(Debug, Clone, Copy)]
pub(crate) struct SharedDrag {
    pub index: usize,
}

/// A payload of another kind than a drop zone's own, released over `rect` this frame. egui's
/// drop zone takes whatever is released on it, so the payload is peeked before the zone draws.
pub(super) fn released_over<T: Send + Sync + 'static>(
    ui: &egui::Ui,
    peeked: Option<Arc<T>>,
    rect: Rect,
) -> Option<Arc<T>> {
    let peeked = peeked?;
    let (released, pos) = ui.input(|i| (i.pointer.any_released(), i.pointer.interact_pos()));
    (released && pos.is_some_and(|p| rect.contains(p))).then_some(peeked)
}

/// The row a drop at `y` lands on: before the first row whose middle is below it
pub(super) fn drop_row(rows: &[egui::Response], y: Option<f32>) -> usize {
    y.and_then(|y| rows.iter().position(|r| r.rect.center().y > y))
        .unwrap_or(rows.len())
}

impl ItemsUi {
    /// The shared list, while PoB shows it (no item displayed)
    pub(super) fn shared_panel(
        &mut self,
        ui: &mut egui::Ui,
        view: &ItemsView,
        out: &mut ItemsOutput,
    ) {
        if !view.shared_shown {
            return;
        }
        let default = ui.visuals().text_color();
        let n = view.shared_items.len();
        if self.shared_selected.is_some_and(|i| i >= n) {
            self.shared_selected = None;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.strong("Shared items:");
            let delete =
                ui.add_enabled(self.shared_selected.is_some(), egui::Button::new("Delete"));
            self.rects.note(&delete, || "shared_delete".into());
            if delete.clicked() {
                self.confirm_shared_delete = self.shared_selected;
            }
        });
        let ctx = ui.ctx().clone();
        let item = egui::DragAndDrop::payload::<ItemDrag>(&ctx);
        let db = egui::DragAndDrop::payload::<DbDrag>(&ctx);
        let selected = self.shared_selected;
        let area = ui.scope(|ui| {
            ui.set_min_height(120.0);
            let rows = reorder_rows(
                ui,
                "items_shared",
                n,
                true,
                false,
                |i| SharedDrag { index: i },
                |p: &SharedDrag| p.index,
                |ui, i| {
                    let fill = if selected == Some(i) {
                        ui.visuals().selection.bg_fill
                    } else {
                        Color32::TRANSPARENT
                    };
                    egui::Frame::NONE.fill(fill).inner_margin(2.0).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            egui::Label::new(pob_text(&view.shared_items[i].label, 14.0, default))
                                .selectable(false),
                        );
                    });
                },
            );
            if n == 0 {
                // PoB's own hint for the empty list (SharedItemListControl.lua:14)
                ui.weak(
                    "This is a list of items that will be shared between all of your builds.\nYou can add items to this list by dragging them from one of the other lists.",
                );
            }
            rows
        });
        let rows = area.inner;
        let mut focused = false;
        for (i, row) in rows.rows.iter().enumerate() {
            self.rects.note(row, || format!("shared_row_{i}"));
            if row.clicked() {
                self.shared_selected = Some(i);
                row.request_focus();
            }
            if row.has_focus() {
                crate::ui::shortcuts::keep_arrows(ui.ctx(), row.id);
            }
            focused |= row.has_focus();
            if row.double_clicked() {
                out.commands.push(Command::ViewSharedItem { index: i });
            }
            self.note_hover(
                row,
                ItemTooltipQuery {
                    id: 0,
                    slot: None,
                    theirs: false,
                    shift: false,
                    shared: Some(i),
                },
            );
        }
        self.rects.note(&area.response, || "shared_list".into());
        // PoB's Ctrl+C on the selected row (SharedItemListControl.lua:62-64)
        if focused
            && let Some(index) = self.shared_selected
            && !(self.has_dialog() || self.modal || ui.ctx().text_edit_focused())
            && ui.input(|inp| inp.events.iter().any(|e| matches!(e, egui::Event::Copy)))
        {
            self.shared_copy_asked = Some(index);
            out.queries.push(Command::CopySharedItem { index });
        }
        // the list's keys (ListControl.lua:375-392): the arrows, Home and End pick a row, and
        // Delete or Backspace asks to delete it (SharedItemListControl's OnSelDelete)
        if focused && !(self.has_dialog() || self.modal || ui.ctx().text_edit_focused()) {
            if let Some(i) =
                ui.input_mut(|inp| crate::ui::shortcuts::list_key(inp, n, self.shared_selected))
            {
                self.shared_selected = Some(i);
                rows.rows[i].request_focus();
            } else if self.shared_selected.is_some() && ui.input(crate::ui::shortcuts::list_delete)
            {
                self.confirm_shared_delete = self.shared_selected;
            }
        }
        if let Some((from, to)) = rows.moved {
            self.shared_selected = Some(to);
            out.commands.push(Command::MoveSharedItem { from, to });
        }
        // a build item or a database row dropped on the list, at the row under the pointer
        let at = drop_row(
            &rows.rows,
            ui.input(|i| i.pointer.interact_pos()).map(|p| p.y),
        );
        if let Some(drag) = released_over(ui, item, area.response.rect) {
            out.commands.push(Command::AddSharedItem {
                source: SharedItemSource::Item { id: drag.id },
                index: at,
            });
        }
        if let Some(drag) = released_over(ui, db, area.response.rect) {
            out.commands.push(Command::AddSharedItem {
                source: SharedItemSource::Db {
                    db: drag.db,
                    name: drag.name.clone(),
                },
                index: at,
            });
        }
    }

    /// PoB's question before a shared item goes (SharedItemListControl.lua:70-76)
    pub(super) fn shared_dialog(
        &mut self,
        ctx: &egui::Context,
        view: &ItemsView,
        out: &mut ItemsOutput,
    ) {
        let Some(index) = self.confirm_shared_delete else {
            return;
        };
        let Some(entry) = view.shared_items.get(index) else {
            self.confirm_shared_delete = None;
            return;
        };
        let mut choice = None;
        egui::Window::new("Delete Item")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Are you sure you want to remove '{}' from the shared item list?",
                    entry.name
                ));
                ui.horizontal(|ui| {
                    let yes = ui.button("Delete");
                    self.rects.note(&yes, || "confirm_shared_delete".into());
                    if yes.clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
        if choice == Some(true) {
            out.commands.push(Command::DeleteSharedItem { index });
            self.shared_selected = None;
        }
        if choice.is_some() {
            self.confirm_shared_delete = None;
        }
    }
}
