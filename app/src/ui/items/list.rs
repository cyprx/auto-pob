//! The item list: rows, drag to equip or reorder, the "…" menu, keys and the confirms.

use super::db::DbDrag;
use super::shared::{SharedDrag, drop_row, released_over};
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

/// The empty list's text (ItemListControl.lua:13)
const EMPTY_TEXT: &str = "^x7F7F7FThis is the list of items that have been added to this build.\nYou can add items to this list by dragging them from\none of the other lists, or by clicking 'Add to build' when\nviewing an item.";

/// What a reorderable column of rows reports.
pub(crate) struct Reordered {
    /// One click response per row
    pub rows: Vec<egui::Response>,
    /// `from`, and where it ends up (as `drop_index`)
    pub moved: Option<(usize, usize)>,
}

/// Draws `n` rows that can be dragged (carrying `payload(i)`) and, when `reorder` is on,
/// dropped on each other. `draw` fills a row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reorder_rows<P: Send + Sync + 'static>(
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
                crate::widgets::dropdown::Dropdown::from_id_salt("items_filter")
                    .selected_text(pob_text(current, 14.0, default))
                    .show_ui(ui, |ui| {
                        for (i, text) in f.labels.iter().enumerate() {
                            let item = crate::widgets::dropdown::item(
                                ui,
                                i == f.selected,
                                pob_text(text, 14.0, default),
                            );
                            self.rects.note(&item, || format!("filter_item_{i}"));
                            if item.clicked() && i != f.selected {
                                out.commands.push(Command::SetItemListFilter { index: i });
                            }
                        }
                    })
            });
            self.rects.note(&combo.inner.response, || "filter".into());
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            if let Some(index) = crate::widgets::scroll_step::combo_step(
                ui,
                &combo.inner.response,
                f.labels.len(),
                f.selected,
            ) {
                out.commands.push(Command::SetItemListFilter { index });
            }
            let sort = ui.button("Sort");
            self.rects.note(&sort, || "sort".into());
            if sort.clicked() {
                out.commands.push(Command::SortItems);
            }
            // PoB's buttons beside Sort (ItemListControl.lua:22-68). Del Unused deletes without
            // asking; both are enabled while the shown (filtered) list has a row
            let any = !view.listed.is_empty();
            let unused = ui.add_enabled(any, egui::Button::new("Del Unused"));
            self.rects.note(&unused, || "delete_unused".into());
            if unused.clicked() {
                out.commands.push(Command::DeleteUnusedItems);
            }
            let all = ui.add_enabled(any, egui::Button::new("Del All"));
            self.rects.note(&all, || "delete_all".into());
            if all.clicked() {
                self.confirm_delete_all = true;
            }
            // Delete acts on the selected row, with PoB's question for an item in use
            let selected = self
                .selected
                .and_then(|id| view.items.iter().find(|e| e.id == id))
                .filter(|e| view.listed.contains(&e.id));
            let delete = ui.add_enabled(selected.is_some(), egui::Button::new("Delete"));
            self.rects.note(&delete, || "delete".into());
            if delete.clicked()
                && let Some(entry) = selected
            {
                self.delete_entry(entry, out);
            }
        });

        let rows: Vec<&ItemEntry> = view
            .listed
            .iter()
            .filter_map(|id| view.items.iter().find(|e| e.id == *id))
            .collect();
        let selected = self.selected;
        // a shared item or a database row dropped on the list is added to the build (NF-2,
        // ItemsTab.lua:1184-1191); peeked before the rows' own drop zone takes it
        let shared = egui::DragAndDrop::payload::<SharedDrag>(ui.ctx());
        let db = egui::DragAndDrop::payload::<DbDrag>(ui.ctx());
        let top = ui.cursor().min;
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
        if rows.is_empty() {
            // PoB's text for the empty list (ItemListControl.lua:13)
            ui.label(pob_text(EMPTY_TEXT, 14.0, default));
        }
        let area = Rect::from_min_max(top, ui.max_rect().max);
        // at the row under the cursor, as PoB's list marks it; a filtered list doesn't take a
        // position (ListControl.lua:157-164), so the item is appended
        let at = view.can_reorder.then(|| {
            drop_row(
                &reordered.rows,
                ui.input(|i| i.pointer.interact_pos()).map(|p| p.y),
            )
        });
        if let Some(drag) = released_over(ui, shared, area) {
            out.commands.push(Command::DropSharedItem {
                index: drag.index,
                slot: None,
                at,
            });
        }
        if let Some(drag) = released_over(ui, db, area) {
            out.commands.push(Command::DropDbItemOnList {
                db: drag.db,
                name: drag.name.clone(),
                at,
            });
        }
        let mut list_focus = false;
        for (i, row) in reordered.rows.iter().enumerate() {
            let entry = rows[i];
            self.rects.note(row, || format!("row_{}", entry.id));
            let (ctrl, shift) = ui.input(|inp| {
                (
                    inp.modifiers.ctrl || inp.modifiers.command,
                    inp.modifiers.shift,
                )
            });
            if row.clicked() {
                self.selected = Some(entry.id);
                row.request_focus();
                // PoB's Ctrl+click equips the item into its slot or takes it out, Ctrl+Shift
                // into the second slot (ItemListControl.lua:290-314); once per double click
                if ctrl && !row.double_clicked() {
                    out.commands.push(Command::QuickEquipItem {
                        id: entry.id,
                        second_slot: shift,
                    });
                }
            }
            if row.double_clicked() && !ctrl {
                out.commands.push(Command::EditItem { id: entry.id });
            }
            self.note_hover(
                row,
                ItemTooltipQuery {
                    id: entry.id,
                    slot: None,
                    theirs: false,
                    shift: false,
                    shared: None,
                },
            );
            if row.has_focus() {
                crate::ui::shortcuts::keep_arrows(ui.ctx(), row.id);
            }
            list_focus |= row.has_focus();
        }
        // Up, Down, Home and End move the selection (ListControl.lua:375-382)
        if list_focus && !self.has_dialog() && !self.modal && !ui.ctx().text_edit_focused() {
            let at = self
                .selected
                .and_then(|id| rows.iter().position(|e| e.id == id));
            if let Some(i) = ui.input_mut(|inp| crate::ui::shortcuts::list_key(inp, rows.len(), at))
            {
                self.selected = Some(rows[i].id);
                reordered.rows[i].request_focus();
            }
        }
        // keys act on the selected item (by id), whichever index-keyed row holds the focus
        if list_focus
            && let Some(entry) = self
                .selected
                .and_then(|id| rows.iter().find(|e| e.id == id))
        {
            self.row_keys(ui, entry, out);
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
        if ui.input(crate::ui::shortcuts::list_delete) {
            self.delete_entry(entry, out);
        }
    }

    /// PoB's OnSelDelete: the item goes at once, or after PoB's question when it is in use
    fn delete_entry(&mut self, entry: &ItemEntry, out: &mut ItemsOutput) {
        match delete_prompt(entry) {
            Some(text) => self.confirm_delete = Some((entry.id, text)),
            None => out.commands.push(Command::DeleteItem {
                id: entry.id,
                confirmed: false,
            }),
        }
    }

    /// The confirms for Delete and Delete all.
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, out: &mut ItemsOutput) {
        if let Some((id, text)) = self.confirm_delete.clone() {
            // Esc is PoB's Cancel (PopupDialog.lua:72-77)
            let window = egui::Id::unique("items_delete");
            let mut choice = crate::widgets::escape::escape(ctx, window).then_some(false);
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
            crate::widgets::escape::note(ctx, window);
            if choice == Some(true) {
                out.commands.push(Command::DeleteItem {
                    id,
                    confirmed: true,
                });
            }
            if choice.is_some() {
                self.confirm_delete = None;
            }
        }
        if self.confirm_delete_all {
            // Esc is PoB's Cancel (PopupDialog.lua:72-77)
            let window = egui::Id::unique("items_delete_all");
            let mut choice = crate::widgets::escape::escape(ctx, window).then_some(false);
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
            crate::widgets::escape::note(ctx, window);
            if choice == Some(true) {
                out.commands.push(Command::DeleteAllItems);
            }
            if choice.is_some() {
                self.confirm_delete_all = false;
            }
        }
    }
}
