//! The set bar and PoB's item set manager as a window.

use super::list::reorder_rows;
use super::shared::released_over;
use super::{ItemsOutput, ItemsUi};
use crate::engine::Command;
use crate::engine::views::{ItemSetEntry, ItemsView};
use crate::ui::name_box::{self, Filter};
use crate::widgets::pob_text;
use eframe::egui::{self, Color32};

/// What the name box of the manager is asking for.
#[derive(Debug, Clone, PartialEq)]
enum Naming {
    New,
    Rename(u32),
    Copy(u32),
}

#[derive(Debug, Default)]
pub(super) struct State {
    open: bool,
    /// The row the manager's buttons act on
    pub(super) selected: Option<u32>,
    naming: Option<(Naming, String, bool)>,
    confirm_delete: Option<u32>,
    /// The shared set row the shared buttons act on (NF-2)
    shared_selected: Option<usize>,
    /// The shared set being renamed, with the typed name and whether it was edited: PoB's
    /// Save is enabled only by an edit (SharedItemSetListControl.lua:31-38)
    shared_naming: Option<(usize, String, bool)>,
    /// A shared set the user asked to delete, until PoB's question is answered
    confirm_shared_delete: Option<usize>,
    /// A shared set row had the keyboard last frame: the build list's keys wait, as PoB gives
    /// keys only to its selected list
    shared_focused: bool,
}

impl State {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// The typed name of the shared set being renamed
    #[cfg(test)]
    pub(super) fn shared_naming_text(&self) -> Option<&str> {
        self.shared_naming
            .as_ref()
            .map(|(_, text, _)| text.as_str())
    }

    /// The typed name of the set being named
    #[cfg(test)]
    pub(super) fn naming_text(&self) -> Option<&str> {
        self.naming.as_ref().map(|(_, text, _)| text.as_str())
    }

    pub(super) fn close(&mut self) {
        *self = State::default();
    }
}

/// PoB shows "Default" for a set with no title.
fn title(set: &ItemSetEntry) -> &str {
    if set.title.is_empty() {
        "Default"
    } else {
        &set.title
    }
}

/// PoB's tooltip for an item set: a "Slot: Item" line per equipped slot; none when it is empty
fn set_tooltip(response: &egui::Response, set: &ItemSetEntry, default: Color32) {
    if set.lines.is_empty() {
        return;
    }
    response.clone().on_hover_ui(|ui| {
        for line in &set.lines {
            ui.label(pob_text(line, 14.0, default));
        }
    });
}

struct SetDrag(usize);

/// A shared set row being dragged: within its list, or onto the build's sets
struct SharedSetDrag(usize);

impl ItemsUi {
    /// The row above the tab: `Set: [name v] [Manage...]  Weapons: [I] [II]`.
    pub(super) fn set_bar(&mut self, ui: &mut egui::Ui, view: &ItemsView, out: &mut ItemsOutput) {
        ui.horizontal(|ui| {
            // PoB's labels (ItemsTab.lua:112, 254)
            ui.label("Item set:");
            let current = view
                .sets
                .iter()
                .find(|s| s.id == view.active_set)
                .map_or("", title);
            let combo = ui.add_enabled_ui(view.sets.len() > 1, |ui| {
                crate::widgets::dropdown::Dropdown::from_id_salt("items_set")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (i, set) in view.sets.iter().enumerate() {
                            let item = crate::widgets::dropdown::item(
                                ui,
                                set.id == view.active_set,
                                title(set),
                            );
                            self.rects.note(&item, || format!("set_item_{i}"));
                            // an entry's tooltip lists the set's items (ItemsTab.lua:106-111)
                            set_tooltip(&item, set, ui.visuals().text_color());
                            if item.clicked() && set.id != view.active_set {
                                out.commands.push(Command::SelectItemSet { id: set.id });
                            }
                        }
                    })
            });
            self.rects.note(&combo.inner.response, || "set".into());
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            let at = view
                .sets
                .iter()
                .position(|s| s.id == view.active_set)
                .unwrap_or(0);
            if let Some(i) = crate::widgets::scroll_step::combo_step(
                ui,
                &combo.inner.response,
                view.sets.len(),
                at,
            ) {
                out.commands.push(Command::SelectItemSet {
                    id: view.sets[i].id,
                });
            }
            let manage = ui.button("Manage\u{2026}");
            self.rects.note(&manage, || "manage".into());
            if manage.clicked() {
                self.sets.open = true;
                self.sets.selected = Some(view.active_set);
            }
        });
    }

    /// The slot column's header: "Equipped items:" and, at its right, the weapon set buttons
    /// (ItemsTab.lua:217-254)
    pub(super) fn slot_header(
        &mut self,
        ui: &mut egui::Ui,
        view: &ItemsView,
        out: &mut ItemsOutput,
    ) {
        ui.horizontal(|ui| {
            ui.label("Equipped items:");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (second, text) in [(true, "II"), (false, "I")] {
                    let on = view.second_weapon_set == second;
                    let button = ui.selectable_label(on, text);
                    self.rects.note(&button, || format!("weapons_{text}"));
                    // PoB locks the current set's button
                    if button.clicked() && !on {
                        out.commands.push(Command::SelectWeaponSet { second });
                    }
                }
                ui.label("Weapon Set:");
            });
        });
    }

    /// The manager window and its prompts.
    pub(super) fn sets_window(
        &mut self,
        ctx: &egui::Context,
        view: &ItemsView,
        out: &mut ItemsOutput,
    ) {
        if !self.sets.open {
            return;
        }
        let mut open = true;
        // Esc closes it as PoB's popup (PopupDialog.lua:72-77), once a box is left; a
        // question it asked takes Esc first, as PoB's popup over it would
        let window = egui::Id::unique("items_sets");
        if crate::widgets::escape::escape(ctx, window) {
            if self.sets.naming.is_some()
                || self.sets.confirm_delete.is_some()
                || self.sets.shared_naming.is_some()
                || self.sets.confirm_shared_delete.is_some()
            {
                self.sets.naming = None;
                self.sets.confirm_delete = None;
                self.sets.shared_naming = None;
                self.sets.confirm_shared_delete = None;
            } else {
                open = false;
            }
        }
        let mut done_clicked = false;
        egui::Window::new("Manage Item Sets")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // the selected row may have been deleted
                if !view.sets.iter().any(|s| Some(s.id) == self.sets.selected) {
                    self.sets.selected = view.sets.first().map(|s| s.id);
                }
                let blocked = self.sets.naming.is_some()
                    || self.sets.confirm_delete.is_some()
                    || self.sets.shared_naming.is_some()
                    || self.sets.confirm_shared_delete.is_some();
                // the list's keys (ListControl.lua:375-392)
                // a text field elsewhere keeps its keys
                if !blocked && !ui.ctx().text_edit_focused() && !self.sets.shared_focused {
                    let at = view.sets.iter().position(|s| Some(s.id) == self.sets.selected);
                    let len = view.sets.len();
                    if let Some(i) =
                        ui.input_mut(|inp| crate::ui::shortcuts::list_key(inp, len, at))
                    {
                        self.sets.selected = Some(view.sets[i].id);
                    }
                }
                ui.add_enabled_ui(!blocked, |ui| {
                    let selected = self.sets.selected;
                    // a shared set dropped on the build's sets is imported (NF-2)
                    let shared = egui::DragAndDrop::payload::<SharedSetDrag>(ui.ctx());
                    let top = ui.cursor().min;
                    let rows = reorder_rows(
                        ui,
                        "items_sets",
                        view.sets.len(),
                        true,
                        false,
                        SetDrag,
                        |p: &SetDrag| p.0,
                        |ui, i| {
                            let set = &view.sets[i];
                            // PoB's row: "  ^9(Current)" after the active set's title
                            // (ItemSetListControl.lua:79-83)
                            let row = if set.row.is_empty() { title(set) } else { &set.row };
                            let label = pob_text(row, 14.0, ui.visuals().text_color());
                            let fill = if selected == Some(set.id) {
                                ui.visuals().selection.bg_fill
                            } else {
                                Color32::TRANSPARENT
                            };
                            egui::Frame::NONE.fill(fill).inner_margin(2.0).show(ui, |ui| {
                                ui.set_min_width(240.0);
                                ui.add(egui::Label::new(label).selectable(false));
                            });
                        },
                    );
                    for (i, row) in rows.rows.iter().enumerate() {
                        let set = &view.sets[i];
                        self.rects.note(row, || format!("sets_row_{i}"));
                        // the row's tooltip lists the set's items (ItemSetListControl.lua:86-90)
                        set_tooltip(row, set, ui.visuals().text_color());
                        if row.clicked() {
                            self.sets.selected = Some(set.id);
                            // the build list becomes the selected control
                            row.request_focus();
                        }
                        if row.double_clicked() && set.id != view.active_set {
                            out.commands.push(Command::SelectItemSet { id: set.id });
                        }
                    }
                    if let Some((from, to)) = rows.moved {
                        out.commands.push(Command::MoveItemSet { from, to });
                    }
                    let area = egui::Rect::from_min_max(
                        top,
                        egui::pos2(ui.max_rect().right(), ui.cursor().min.y),
                    );
                    self.rects.note_rect(area, "sets_list");
                    if let Some(drag) = released_over(ui, shared, area) {
                        out.commands
                            .push(Command::ImportSharedItemSet { index: drag.0 });
                    }
                    ui.separator();
                    let sel = self
                        .sets
                        .selected
                        .and_then(|id| view.sets.iter().find(|s| s.id == id));
                    ui.horizontal(|ui| {
                        let new = ui.button("New");
                        self.rects.note(&new, || "sets_new".into());
                        if new.clicked() {
                            self.sets.naming = Some((Naming::New, String::new(), false));
                        }
                        ui.add_enabled_ui(sel.is_some(), |ui| {
                            let rename = ui.button("Rename");
                            self.rects.note(&rename, || "sets_rename".into());
                            let copy = ui.button("Copy");
                            self.rects.note(&copy, || "sets_copy".into());
                            // F2 renames the selected set (ItemSetListControl.lua)
                            let f2 = self.sets.naming.is_none() && !ui.ctx().text_edit_focused() && !self.sets.shared_focused && ui.input(|i| i.key_pressed(egui::Key::F2));
                            if let Some(set) = sel {
                                if rename.clicked() || f2 {
                                    self.sets.naming =
                                        Some((Naming::Rename(set.id), set.name_box_text(), false));
                                }
                                if copy.clicked() {
                                    self.sets.naming =
                                        Some((Naming::Copy(set.id), set.name_box_text(), false));
                                }
                            }
                        });
                        // the last set can't be deleted
                        let can_delete = sel.is_some() && view.sets.len() > 1;
                        let delete = ui.add_enabled(can_delete, egui::Button::new("Delete"));
                        self.rects.note(&delete, || "sets_delete".into());
                        let key = can_delete && !blocked && !ui.ctx().text_edit_focused() && !self.sets.shared_focused && ui.input(crate::ui::shortcuts::list_delete);
                        if (delete.clicked() || key)
                            && let Some(set) = sel
                        {
                            self.sets.confirm_delete = Some(set.id);
                        }
                    });
                });
                self.shared_sets(ui, view, blocked, out);
                if let Some((kind, text, edited)) = &mut self.sets.naming {
                    ui.separator();
                    ui.label("Enter name for this item set:");
                    // PoB's name box: its filter, 100 bytes, and a Save that waits for an edit
                    // and a visible character (ItemSetListControl.lua:44-69)
                    let (edit, changed) = name_box::edit(
                        ui,
                        text,
                        Filter::Plain,
                        name_box::NAME_LIMIT,
                        |t| t.desired_width(240.0),
                    );
                    self.rects.note(&edit, || "sets_name".into());
                    *edited |= changed;
                    let blank = !*edited || !name_box::has_visible(text);
                    // Enter presses an enabled Save, as PoB's popup's enter control
                    // (ItemSetListControl.lua:70)
                    let mut done = (!blank
                        && edit.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    .then_some(true);
                    ui.horizontal(|ui| {
                        let save = ui.add_enabled(!blank, egui::Button::new("Save"));
                        self.rects.note(&save, || "sets_save".into());
                        if save.clicked() {
                            done = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            done = Some(false);
                        }
                    });
                    if done == Some(true) {
                        let title = text.clone();
                        out.commands.push(match kind {
                            Naming::New => Command::NewItemSet { title },
                            Naming::Rename(id) => Command::RenameItemSet { id: *id, title },
                            Naming::Copy(id) => Command::CopyItemSet { id: *id, title },
                        });
                    }
                    if done.is_some() {
                        self.sets.naming = None;
                    }
                }
                if let Some(id) = self.sets.confirm_delete {
                    ui.separator();
                    let name = view
                        .sets
                        .iter()
                        .find(|s| s.id == id)
                        .map_or("", title)
                        .to_owned();
                    ui.label(format!(
                        "Are you sure you want to delete '{name}'?\nThis will not delete any items used by the set."
                    ));
                    ui.horizontal(|ui| {
                        let yes = ui.button("Delete");
                        self.rects.note(&yes, || "sets_confirm_delete".into());
                        if yes.clicked() {
                            out.commands.push(Command::DeleteItemSet { id });
                            self.sets.confirm_delete = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.sets.confirm_delete = None;
                        }
                    });
                }
                // PoB's "Done" closes the window (ItemsTab.lua:2512-2514)
                ui.separator();
                let done = ui.add_enabled(
                    self.sets.naming.is_none()
                        && self.sets.confirm_delete.is_none()
                        && self.sets.shared_naming.is_none()
                        && self.sets.confirm_shared_delete.is_none(),
                    egui::Button::new("Done"),
                );
                self.rects.note(&done, || "sets_done".into());
                if done.clicked() {
                    done_clicked = true;
                }
            });
        crate::widgets::escape::note(ctx, window);
        if !open || done_clicked {
            self.sets.close();
        }
    }

    /// PoB's shared set list beside the build's (ItemsTab.lua:2506-2516): sets shared by every
    /// build, kept in Settings.xml (NF-2)
    fn shared_sets(
        &mut self,
        ui: &mut egui::Ui,
        view: &ItemsView,
        blocked: bool,
        out: &mut ItemsOutput,
    ) {
        let n = view.shared_sets.len();
        if self.sets.shared_selected.is_some_and(|i| i >= n) {
            self.sets.shared_selected = None;
        }
        ui.separator();
        ui.add_enabled_ui(!blocked, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Shared sets");
                let sel = self.sets.shared_selected;
                let rename = ui.add_enabled(sel.is_some(), egui::Button::new("Rename"));
                self.rects.note(&rename, || "shared_sets_rename".into());
                let delete = ui.add_enabled(sel.is_some(), egui::Button::new("Delete"));
                self.rects.note(&delete, || "shared_sets_delete".into());
                if let Some(i) = sel {
                    if rename.clicked() {
                        self.sets.shared_naming = Some(rename_start(view, i));
                    }
                    if delete.clicked() {
                        self.sets.confirm_shared_delete = Some(i);
                    }
                }
            });
            // a build set dropped here is shared
            let build_set = egui::DragAndDrop::payload::<SetDrag>(ui.ctx());
            let top = ui.cursor().min;
            let selected = self.sets.shared_selected;
            let rows = reorder_rows(
                ui,
                "items_shared_sets",
                n,
                true,
                false,
                SharedSetDrag,
                |p: &SharedSetDrag| p.0,
                |ui, i| {
                    let fill = if selected == Some(i) {
                        ui.visuals().selection.bg_fill
                    } else {
                        Color32::TRANSPARENT
                    };
                    egui::Frame::NONE.fill(fill).inner_margin(2.0).show(ui, |ui| {
                        ui.set_min_width(240.0);
                        ui.add(egui::Label::new(&view.shared_sets[i].title).selectable(false));
                    });
                },
            );
            if n == 0 {
                // PoB's hint for the empty list (SharedItemSetListControl.lua:13)
                ui.weak("This is a list of item sets that will be shared between all of your builds.\nYou can add sets to this list by dragging them from the build's set list.");
            }
            let area = egui::Rect::from_min_max(
                top,
                egui::pos2(ui.max_rect().right(), ui.cursor().min.y),
            );
            self.rects.note_rect(area, "shared_sets_list");
            for (i, row) in rows.rows.iter().enumerate() {
                self.rects.note(row, || format!("shared_sets_row_{i}"));
                if row.clicked() {
                    self.sets.shared_selected = Some(i);
                    row.request_focus();
                }
                let lines = &view.shared_sets[i].lines;
                if !lines.is_empty() {
                    let default = ui.visuals().text_color();
                    row.clone().on_hover_ui(|ui| {
                        for line in lines {
                            ui.label(pob_text(line, 14.0, default));
                        }
                    });
                }
                if row.has_focus() {
                    crate::ui::shortcuts::keep_arrows(ui.ctx(), row.id);
                }
                // PoB's F2 renames the selected shared set
                if row.has_focus() && ui.input(|inp| inp.key_pressed(egui::Key::F2)) {
                    self.sets.shared_naming = Some(rename_start(view, i));
                }
            }
            self.sets.shared_focused = rows.rows.iter().any(|r| r.has_focus());
            // the shared list's own keys (ListControl.lua:375-392): the arrows, Home and End
            // pick a row, Delete or Backspace asks to delete it (SharedItemSetListControl's
            // OnSelDelete)
            if self.sets.shared_focused && !blocked && !ui.ctx().text_edit_focused() {
                if let Some(i) = ui.input_mut(|inp| {
                    crate::ui::shortcuts::list_key(inp, n, self.sets.shared_selected)
                }) {
                    self.sets.shared_selected = Some(i);
                    rows.rows[i].request_focus();
                } else if let Some(i) = self.sets.shared_selected
                    && ui.input(crate::ui::shortcuts::list_delete)
                {
                    self.sets.confirm_shared_delete = Some(i);
                }
            }
            if let Some((from, to)) = rows.moved {
                self.sets.shared_selected = Some(to);
                out.commands.push(Command::MoveSharedItemSet { from, to });
            }
            if let Some(drag) = released_over(ui, build_set, area)
                && let Some(set) = view.sets.get(drag.0)
            {
                out.commands.push(Command::AddSharedItemSet { id: set.id });
            }
        });
        if let Some((index, text, edited)) = &mut self.sets.shared_naming {
            ui.separator();
            ui.label("Enter name for this item set:");
            let (edit, changed) =
                name_box::edit(ui, text, Filter::Plain, name_box::NAME_LIMIT, |t| {
                    t.desired_width(240.0)
                });
            self.rects.note(&edit, || "shared_sets_name".into());
            *edited |= changed;
            let blank = !name_box::has_visible(text);
            // Enter presses an enabled Save (SharedItemSetListControl.lua:41)
            let mut done = (*edited
                && !blank
                && edit.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            .then_some(true);
            ui.horizontal(|ui| {
                let save = ui.add_enabled(*edited && !blank, egui::Button::new("Save"));
                self.rects.note(&save, || "shared_sets_save".into());
                if save.clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
            if done == Some(true) {
                out.commands.push(Command::RenameSharedItemSet {
                    index: *index,
                    title: text.clone(),
                });
            }
            if done.is_some() {
                self.sets.shared_naming = None;
            }
        }
        if let Some(index) = self.sets.confirm_shared_delete {
            ui.separator();
            let name = view
                .shared_sets
                .get(index)
                .map_or("", |s| s.title.as_str())
                .to_owned();
            ui.label(format!(
                "Are you sure you want to delete '{name}' from the shared item set list?"
            ));
            ui.horizontal(|ui| {
                let yes = ui.button("Delete");
                self.rects
                    .note(&yes, || "shared_sets_confirm_delete".into());
                if yes.clicked() {
                    out.commands.push(Command::DeleteSharedItemSet { index });
                    self.sets.confirm_shared_delete = None;
                    self.sets.shared_selected = None;
                }
                if ui.button("Cancel").clicked() {
                    self.sets.confirm_shared_delete = None;
                }
            });
        }
    }
}

/// PoB's name box for a shared set: the set's own title (empty when it has none), Save
/// disabled until the text is edited (SharedItemSetListControl.lua:29-39)
fn rename_start(view: &ItemsView, index: usize) -> (usize, String, bool) {
    let title = view.shared_sets[index]
        .raw_title
        .clone()
        .unwrap_or_default();
    (index, title, false)
}
