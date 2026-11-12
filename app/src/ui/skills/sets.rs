//! The skill set selector and PoB's set manager as a window.

use super::{SkillsOutput, SkillsUi};
use crate::engine::Command;
use crate::engine::views::{SkillSetEntry, SkillsView};
use eframe::egui;

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
    naming: Option<(Naming, String)>,
    confirm_delete: Option<u32>,
}

impl State {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn close(&mut self) {
        *self = State::default();
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
            let combo = egui::ComboBox::from_id_salt("skills_set")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, set) in view.sets.iter().enumerate() {
                        let item = ui.selectable_label(set.id == view.active_set, title(set));
                        self.rects.note(&item, || format!("set_item_{i}"));
                        if item.clicked() && set.id != view.active_set {
                            out.commands.push(Command::SelectSkillSet { id: set.id });
                        }
                    }
                });
            self.rects.note(&combo.response, || "set".into());
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
                if !blocked {
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
                    for (i, set) in view.sets.iter().enumerate() {
                        let label = if set.id == view.active_set {
                            format!("{} (active)", title(set))
                        } else {
                            title(set).to_owned()
                        };
                        let row = ui.selectable_label(self.sets.selected == Some(set.id), label);
                        self.rects.note(&row, || format!("sets_row_{i}"));
                        if row.clicked() {
                            self.sets.selected = Some(set.id);
                        }
                        if row.double_clicked() && set.id != view.active_set {
                            out.commands.push(Command::SelectSkillSet { id: set.id });
                        }
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
                            self.sets.naming = Some((Naming::New, String::new()));
                        }
                        ui.add_enabled_ui(sel.is_some(), |ui| {
                            let rename = ui.button("Rename");
                            self.rects.note(&rename, || "sets_rename".into());
                            let copy = ui.button("Copy");
                            self.rects.note(&copy, || "sets_copy".into());
                            // F2 renames the selected set (SkillSetListControl.lua)
                            let f2 = self.sets.naming.is_none()
                                && ui.input(|i| i.key_pressed(egui::Key::F2));
                            if let Some(set) = sel {
                                if rename.clicked() || f2 {
                                    let t = title_text(set);
                                    self.sets.naming = Some((Naming::Rename(set.id), t));
                                }
                                if copy.clicked() {
                                    let t = title_text(set);
                                    self.sets.naming = Some((Naming::Copy(set.id), t));
                                }
                            }
                        });
                        // the last set can't be deleted
                        let can_delete = sel.is_some() && view.sets.len() > 1;
                        let delete = ui.add_enabled(can_delete, egui::Button::new("Delete"));
                        self.rects.note(&delete, || "sets_delete".into());
                        let key =
                            can_delete && !blocked && ui.input(crate::ui::shortcuts::list_delete);
                        if (delete.clicked() || key)
                            && let Some(set) = sel
                        {
                            self.sets.confirm_delete = Some(set.id);
                        }
                    });
                });
                if let Some((kind, text)) = &mut self.sets.naming {
                    ui.separator();
                    ui.label("Enter name for this skill set:");
                    let edit = ui.add(egui::TextEdit::singleline(text).desired_width(240.0));
                    self.rects.note(&edit, || "sets_name".into());
                    let blank = text.trim().is_empty();
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
                    ui.label(format!("Delete the skill set \"{name}\"?"));
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
        if !open {
            self.sets.close();
        }
    }
}

/// The title a rename or copy starts from.
fn title_text(set: &SkillSetEntry) -> String {
    set.title.clone()
}
