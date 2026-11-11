//! The item list: rows, drag to equip or reorder, the "…" menu, keys and the confirms.

use super::{ItemDrag, ItemsOutput, ItemsUi};
use crate::engine::views::{ItemEntry, ItemsView};
use crate::engine::{Command, ItemTooltipQuery};
use crate::widgets::{drop_index, pob_text};
use eframe::egui::{self, Color32, Rect, Stroke};

/// PoB's question before deleting an item that is in use (ItemListControl.lua:331-372), or
/// `None` when the item is deleted at once.
pub fn delete_prompt(entry: &ItemEntry) -> Option<String> {
    let set = |title: &str| {
        let title = if title.is_empty() { "Default" } else { title };
        format!(" in set '{title}'")
    };
    let name = &entry.name;
    if let Some(slot) = &entry.equipped {
        let in_set = slot.set.as_deref().map(set).unwrap_or_default();
        Some(format!(
            "{name} is currently equipped in {}{in_set}.\nAre you sure you want to delete it?",
            slot.label
        ))
    } else if let Some(title) = &entry.abyss_set {
        Some(format!(
            "{name} is currently equipped in an Abyssal Socket{}.\nAre you sure you want to delete it?",
            set(title)
        ))
    } else {
        entry.tree.as_ref().map(|tree| {
            format!(
                "{name} is currently equipped in passive tree '{tree}'.\nAre you sure you want to delete it?"
            )
        })
    }
}

/// What a reorderable column of rows reports.
pub(super) struct Reordered {
    /// One click response per row
    pub rows: Vec<egui::Response>,
    /// `from`, and where it ends up (as `drop_index`)
    pub moved: Option<(usize, usize)>,
}

/// Draws `n` rows that can be dragged (carrying `payload(i)`) and, when `reorder` is on,
/// dropped on each other. `draw` fills a row.
#[allow(clippy::too_many_arguments)]
pub(super) fn reorder_rows<P: Send + Sync + 'static>(
    ui: &mut egui::Ui,
    salt: &str,
    n: usize,
    reorder: bool,
    scroll: bool,
    payload: impl Fn(usize) -> P,
    from_of: impl Fn(&P) -> usize,
    mut draw: impl FnMut(&mut egui::Ui, usize),
) -> Reordered {
    let ctx = ui.ctx().clone();
    let dragging = egui::DragAndDrop::has_payload_of_type::<P>(&ctx);
    let mut target: Option<(usize, bool)> = None;
    let mut indicator = None;
    let mut rows = Vec::with_capacity(n);
    let (_, dropped) = ui.dnd_drop_zone::<P, _>(egui::Frame::NONE, |ui| {
        let mut rows_ui = |ui: &mut egui::Ui| {
            for i in 0..n {
                let top = ui.cursor().min.y;
                let id = ui.scope_id().with((salt, i));
                let inner = ui.dnd_drag_source(id, payload(i), |ui| draw(ui, i));
                // the click goes over the drag source, which senses only drags
                let row = ui.interact(inner.response.rect, id.with("click"), egui::Sense::click());
                let rect = Rect::from_min_max(
                    egui::pos2(ui.min_rect().left(), top),
                    egui::pos2(ui.max_rect().right(), ui.cursor().min.y),
                );
                if dragging
                    && reorder
                    && let Some(p) = ui.input(|inp| inp.pointer.hover_pos())
                    && rect.contains(p)
                {
                    let below = p.y > rect.center().y;
                    target = Some((i, below));
                    let y = if below { rect.bottom() } else { rect.top() };
                    indicator = Some((rect.left(), rect.right(), y));
                }
                rows.push(row);
            }
        };
        if scroll {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .id_salt(salt)
                .show(ui, |ui| rows_ui(ui));
        } else {
            rows_ui(ui);
        }
    });
    if let Some((l, r, y)) = indicator {
        ui.painter()
            .hline(l..=r, y, Stroke::new(2.0, Color32::from_rgb(120, 180, 255)));
    }
    let moved = match (dropped, target) {
        (Some(p), Some((hover, below))) if reorder => {
            let from = from_of(&p);
            drop_index(from, hover, below).map(|to| (from, to))
        }
        _ => None,
    };
    Reordered { rows, moved }
}

impl ItemsUi {
    pub(super) fn item_list(&mut self, ui: &mut egui::Ui, view: &ItemsView, out: &mut ItemsOutput) {
        let default = ui.visuals().text_color();
        ui.horizontal_wrapped(|ui| {
            ui.strong("Items");
            let f = &view.list_filter;
            let current = f.labels.get(f.selected).map_or("", String::as_str);
            let combo = ui.add_enabled_ui(f.enabled, |ui| {
                egui::ComboBox::from_id_salt("items_filter")
                    .selected_text(pob_text(current, 14.0, default))
                    .show_ui(ui, |ui| {
                        for (i, text) in f.labels.iter().enumerate() {
                            let item =
                                ui.selectable_label(i == f.selected, pob_text(text, 14.0, default));
                            self.rects.note(&item, || format!("filter_item_{i}"));
                            if item.clicked() && i != f.selected {
                                out.commands.push(Command::SetItemListFilter { index: i });
                            }
                        }
                    })
            });
            self.rects.note(&combo.inner.response, || "filter".into());
            let menu = ui.menu_button("\u{2026}", |ui| {
                let sort = ui.button("Sort");
                self.rects.note(&sort, || "menu_sort".into());
                if sort.clicked() {
                    out.commands.push(Command::SortItems);
                    ui.close();
                }
                // PoB deletes the unused items without asking
                let unused = ui.button("Delete unused");
                self.rects.note(&unused, || "menu_delete_unused".into());
                if unused.clicked() {
                    out.commands.push(Command::DeleteUnusedItems);
                    ui.close();
                }
                let all = ui.add_enabled(!view.items.is_empty(), egui::Button::new("Delete all"));
                self.rects.note(&all, || "menu_delete_all".into());
                if all.clicked() {
                    self.confirm_delete_all = true;
                    ui.close();
                }
            });
            self.rects.note(&menu.response, || "menu".into());
        });

        let rows: Vec<&ItemEntry> = view
            .listed
            .iter()
            .filter_map(|id| view.items.iter().find(|e| e.id == *id))
            .collect();
        let selected = self.selected;
        let reordered = reorder_rows(
            ui,
            "items_row",
            rows.len(),
            view.can_reorder,
            true,
            |i| ItemDrag {
                index: i,
                id: rows[i].id,
            },
            |p: &ItemDrag| p.index,
            |ui, i| {
                let fill = if selected == Some(rows[i].id) {
                    ui.visuals().selection.bg_fill
                } else {
                    Color32::TRANSPARENT
                };
                egui::Frame::NONE
                    .fill(fill)
                    .inner_margin(2.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            egui::Label::new(pob_text(&rows[i].display_label, 14.0, default))
                                .selectable(false),
                        );
                    });
            },
        );
        for (i, row) in reordered.rows.iter().enumerate() {
            let entry = rows[i];
            self.rects.note(row, || format!("row_{}", entry.id));
            if row.clicked() {
                self.selected = Some(entry.id);
                row.request_focus();
            }
            if row.double_clicked() {
                out.commands.push(Command::EditItem { id: entry.id });
            }
            self.note_hover(
                row,
                ItemTooltipQuery {
                    id: entry.id,
                    slot: None,
                },
            );
            if row.has_focus() {
                self.row_keys(ui, entry, out);
            }
        }
        if let Some((from, to)) = reordered.moved {
            if let Some(entry) = rows.get(from) {
                self.selected = Some(entry.id);
            }
            out.commands.push(Command::MoveItem { from, to });
        }
    }

    /// Ctrl+C and Delete on the focused row.
    fn row_keys(&mut self, ui: &egui::Ui, entry: &ItemEntry, out: &mut ItemsOutput) {
        if self.has_dialog() || self.modal || ui.ctx().text_edit_focused() {
            return;
        }
        if ui.input(|inp| inp.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
            self.copy_asked = Some(entry.id);
            out.queries.push(Command::CopyItem { id: entry.id });
        }
        if ui.input(|inp| inp.key_pressed(egui::Key::Delete)) {
            match delete_prompt(entry) {
                Some(text) => self.confirm_delete = Some((entry.id, text)),
                None => out.commands.push(Command::DeleteItem { id: entry.id }),
            }
        }
    }

    /// The confirms for Delete and Delete all.
    pub(super) fn dialogs(
        &mut self,
        ctx: &egui::Context,
        _view: &ItemsView,
        out: &mut ItemsOutput,
    ) {
        if let Some((id, text)) = self.confirm_delete.clone() {
            let mut choice = None;
            egui::Window::new("Delete Item")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(&text);
                    ui.horizontal(|ui| {
                        let yes = ui.button("Delete");
                        self.rects.note(&yes, || "confirm_delete".into());
                        if yes.clicked() {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(false);
                        }
                    });
                });
            if choice == Some(true) {
                out.commands.push(Command::DeleteItem { id });
            }
            if choice.is_some() {
                self.confirm_delete = None;
            }
        }
        if self.confirm_delete_all {
            let mut choice = None;
            egui::Window::new("Delete All")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Are you sure you want to delete all items in this build?");
                    ui.horizontal(|ui| {
                        let yes = ui.button("Delete");
                        self.rects.note(&yes, || "confirm_delete_all".into());
                        if yes.clicked() {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(false);
                        }
                    });
                });
            if choice == Some(true) {
                out.commands.push(Command::DeleteAllItems);
            }
            if choice.is_some() {
                self.confirm_delete_all = false;
            }
        }
    }
}
