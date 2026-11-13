//! The skill set selector and PoB's set manager as a window.

use super::{SkillsOutput, SkillsUi};
use crate::engine::Command;
use crate::engine::views::{SkillSetEntry, SkillsView};
use crate::ui::items::list::reorder_rows;
use crate::ui::name_box::{self, Filter};
use eframe::egui::{self, Color32};

/// What a dragged set row carries: its position
struct SetDrag(usize);

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
    selected: Option<u32>,
    naming: Option<(Naming, String, bool)>,
    confirm_delete: Option<u32>,
}

impl State {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn close(&mut self) {
        *self = State::default();
    }

    /// The typed name of the set being named
    #[cfg(test)]
    pub(super) fn naming_text(&self) -> Option<&str> {
        self.naming.as_ref().map(|(_, text, _)| text.as_str())
    }
}

/// PoB shows "Default" for a set with no title.
fn title(set: &SkillSetEntry) -> &str {
    if set.title.is_empty() {
        "Default"
    } else {
        &set.title
    }
}

impl SkillsUi {
    /// The row above the tab: `Set: [name v] [Manage...]`.
    pub(super) fn set_row(&mut self, ui: &mut egui::Ui, view: &SkillsView, out: &mut SkillsOutput) {
        ui.horizontal(|ui| {
            ui.label("Set:");
            let current = view
                .sets
                .iter()
                .find(|s| s.id == view.active_set)
                .map_or("", title);
            // PoB's selector takes a pick only with more than one set (SkillsTab.lua:99-101)
            let combo = ui
                .add_enabled_ui(view.sets.len() > 1, |ui| {
                    crate::widgets::dropdown::Dropdown::from_id_salt("skills_set")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (i, set) in view.sets.iter().enumerate() {
                                let item = crate::widgets::dropdown::item(
                                    ui,
                                    set.id == view.active_set,
                                    title(set),
                                );
                                self.rects.note(&item, || format!("set_item_{i}"));
                                if item.clicked() && set.id != view.active_set {
                                    out.commands.push(Command::SelectSkillSet { id: set.id });
                                }
                            }
                        })
                })
                .inner;
            self.rects.note(&combo.response, || "set".into());
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            let at = view
                .sets
                .iter()
                .position(|s| s.id == view.active_set)
                .unwrap_or(0);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, view.sets.len(), at)
            {
                out.commands.push(Command::SelectSkillSet {
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

    /// The manager window and its prompts.
    pub(super) fn sets_window(
        &mut self,
        ctx: &egui::Context,
        view: &SkillsView,
        out: &mut SkillsOutput,
    ) {
        if !self.sets.open {
            return;
        }
        let mut open = true;
        // Esc closes it as PoB's popup (PopupDialog.lua:72-77), once a box is left; a
        // question it asked takes Esc first, as PoB's popup over it would
        let window = egui::Id::unique("skills_sets");
        if crate::widgets::escape::escape(ctx, window) {
            if self.sets.naming.is_some() || self.sets.confirm_delete.is_some() {
                self.sets.naming = None;
                self.sets.confirm_delete = None;
            } else {
                open = false;
            }
        }
        egui::Window::new("Skill sets")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // the selected row may have been deleted
                if !view.sets.iter().any(|s| Some(s.id) == self.sets.selected) {
                    self.sets.selected = view.sets.first().map(|s| s.id);
                }
                let blocked = self.sets.naming.is_some() || self.sets.confirm_delete.is_some();
                // the list's keys (ListControl.lua:375-392)
                // a text field elsewhere keeps its keys
                if !blocked && !ui.ctx().text_edit_focused() {
                    let at = view
                        .sets
                        .iter()
                        .position(|s| Some(s.id) == self.sets.selected);
                    let len = view.sets.len();
                    if let Some(i) =
                        ui.input_mut(|inp| crate::ui::shortcuts::list_key(inp, len, at))
                    {
                        self.sets.selected = Some(view.sets[i].id);
                    }
                }
                ui.add_enabled_ui(!blocked, |ui| {
                    // PoB's list rows drag to a new place (R31)
                    let selected = self.sets.selected;
                    let rows = reorder_rows(
                        ui,
                        "skills_sets",
                        view.sets.len(),
                        true,
                        false,
                        SetDrag,
                        |p: &SetDrag| p.0,
                        |ui, i| {
                            let set = &view.sets[i];
                            let label = if set.id == view.active_set {
                                format!("{} (active)", title(set))
                            } else {
                                title(set).to_owned()
                            };
                            let fill = if selected == Some(set.id) {
                                ui.visuals().selection.bg_fill
                            } else {
                                Color32::TRANSPARENT
                            };
                            egui::Frame::NONE
                                .fill(fill)
                                .inner_margin(2.0)
                                .show(ui, |ui| {
                                    ui.set_min_width(240.0);
                                    ui.add(egui::Label::new(label).selectable(false));
                                });
                        },
                    );
                    for (i, row) in rows.rows.iter().enumerate() {
                        let set = &view.sets[i];
                        self.rects.note(row, || format!("sets_row_{i}"));
                        if row.clicked() {
                            self.sets.selected = Some(set.id);
                        }
                        if row.double_clicked() && set.id != view.active_set {
                            out.commands.push(Command::SelectSkillSet { id: set.id });
                        }
                    }
                    if let Some((from, to)) = rows.moved {
                        out.commands.push(Command::MoveSkillSet { from, to });
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
                            // F2 renames the selected set (SkillSetListControl.lua)
                            let f2 = self.sets.naming.is_none()
                                && !ui.ctx().text_edit_focused()
                                && ui.input(|i| i.key_pressed(egui::Key::F2));
                            if let Some(set) = sel {
                                if rename.clicked() || f2 {
                                    let t = title_text(set);
                                    self.sets.naming = Some((Naming::Rename(set.id), t, false));
                                }
                                if copy.clicked() {
                                    let t = title_text(set);
                                    self.sets.naming = Some((Naming::Copy(set.id), t, false));
                                }
                            }
                        });
                        // the last set can't be deleted
                        let can_delete = sel.is_some() && view.sets.len() > 1;
                        let delete = ui.add_enabled(can_delete, egui::Button::new("Delete"));
                        self.rects.note(&delete, || "sets_delete".into());
                        let key = can_delete
                            && !blocked
                            && !ui.ctx().text_edit_focused()
                            && ui.input(crate::ui::shortcuts::list_delete);
                        if (delete.clicked() || key)
                            && let Some(set) = sel
                        {
                            self.sets.confirm_delete = Some(set.id);
                        }
                    });
                });
                if let Some((kind, text, edited)) = &mut self.sets.naming {
                    ui.separator();
                    ui.label("Enter name for this skill set:");
                    // PoB's name box: its filter, 100 bytes, and a Save that waits for an edit
                    // and a visible character (SkillSetListControl.lua:53-78)
                    let (edit, changed) =
                        name_box::edit(ui, text, Filter::Plain, name_box::NAME_LIMIT, |t| {
                            t.desired_width(240.0)
                        });
                    self.rects.note(&edit, || "sets_name".into());
                    *edited |= changed;
                    let blank = !*edited || !name_box::has_visible(text);
                    let mut done = None;
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
                            Naming::New => Command::NewSkillSet { title },
                            Naming::Rename(id) => Command::RenameSkillSet { id: *id, title },
                            Naming::Copy(id) => Command::CopySkillSet { id: *id, title },
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
                    // PoB's question (SkillSetListControl.lua:102)
                    ui.label(format!("Are you sure you want to delete '{name}'?"));
                    ui.horizontal(|ui| {
                        let yes = ui.button("Delete");
                        self.rects.note(&yes, || "sets_confirm_delete".into());
                        if yes.clicked() {
                            out.commands.push(Command::DeleteSkillSet { id });
                            self.sets.confirm_delete = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.sets.confirm_delete = None;
                        }
                    });
                }
            });
        crate::widgets::escape::note(ctx, window);
        if !open {
            self.sets.close();
        }
    }
}

/// The title a rename or copy starts from.
fn title_text(set: &SkillSetEntry) -> String {
    set.name_box_text()
}
