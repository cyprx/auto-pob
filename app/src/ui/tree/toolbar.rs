//! The Tree tab's toolbar and the dialogs it opens: trees, reset, search,
//! the timeless jewel search, heat map, the power report toggle and the link menu.

use super::{Rects, TreeOutput};
use crate::engine::{Command, PopupKind, TreeState};
use crate::theme;
use crate::widgets::pob_text;
use eframe::egui;

/// A search edit is sent once typing has paused this long (seconds).
pub(super) const SEARCH_DEBOUNCE: f64 = 0.15;

/// The depths the heat map can be limited to, as the dropdown lists them.
const DEPTHS: [u32; 3] = [5, 10, 15];

/// What the tab knows about the node power calculation.
pub(super) struct PowerStatus {
    pub progress: Option<u32>,
    pub failed: bool,
}

/// A question the toolbar is waiting on.
#[derive(Debug, PartialEq)]
enum Dialog {
    NewSpec(String),
    RenameSpec {
        index: usize,
        title: String,
    },
    DeleteSpec(usize),
    Reset,
    /// PoB's "Remove All Tattoos" can't be undone
    RemoveTattoos,
    Import {
        title: String,
        url: String,
    },
}

#[derive(Default)]
pub(super) struct Toolbar {
    search: String,
    /// The engine's search text last frame, to notice when it changes by itself (undo)
    search_seen: String,
    /// When the search text was last edited, while that edit hasn't been sent
    search_edit: Option<f64>,
    pub(super) report_open: bool,
    dialog: Option<Dialog>,
    /// Choosing this class would drop points that don't connect to it: asked by the top bar's
    /// class dropdown, on every tab (id, name)
    class_dialog: Option<(u32, String)>,
    /// "Copy link" was clicked and the link has not arrived yet
    copy_pending: bool,
    link_fresh: bool,
    /// The tree selector drawn last: its id, and whether it was pointed at or open, when Up
    /// and Down leave the trees alone (TreeTab.lua:367-371)
    pub(super) spec_combo: Option<egui::Id>,
    pub(super) spec_busy: bool,
    /// Ctrl+F: the search box takes the keyboard when next drawn
    pub(super) focus_search: bool,
}

impl Toolbar {
    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some() || self.class_dialog.is_some()
    }

    /// Drops the tab's own dialog; the class question belongs to the top bar and stays.
    pub(super) fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// F2: the active tree's Rename box
    pub(super) fn rename_active(&mut self, state: &TreeState) {
        if let Some(spec) = state.specs.get(state.active_spec) {
            self.dialog = Some(Dialog::RenameSpec {
                index: state.active_spec,
                title: spec.title.clone(),
            });
        }
    }

    /// A link snapshot arrived.
    pub(super) fn link_arrived(&mut self) {
        self.link_fresh = true;
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        power: &PowerStatus,
        link: Option<&str>,
        rects: &mut Rects,
        out: &mut TreeOutput,
    ) {
        if self.copy_pending
            && self.link_fresh
            && let Some(link) = link
        {
            ui.ctx().copy_text(link.to_owned());
            self.copy_pending = false;
        }
        let commands = &mut out.commands;
        ui.horizontal_wrapped(|ui| {
            self.specs(ui, state, rects, commands);
            ui.separator();
            let reset = ui.button("Reset").on_hover_text("Reset the passive tree");
            rects.note(&reset, || "reset".into());
            if reset.clicked() {
                self.dialog = Some(Dialog::Reset);
            }
            let tattoos = ui
                .button("Remove all tattoos")
                .on_hover_text("Remove every tattoo from the tree");
            rects.note(&tattoos, || "remove_tattoos".into());
            if tattoos.clicked() {
                self.dialog = Some(Dialog::RemoveTattoos);
            }
        });
        ui.horizontal_wrapped(|ui| {
            self.search(ui, state, rects, commands);
            // PoB's "Find Timeless Jewel", next to the search as PoB places it (R17)
            let timeless = ui
                .button("Find timeless jewel\u{2026}")
                .on_hover_text("Search the seeds of a timeless jewel for the nodes you want");
            rects.note(&timeless, || "find_timeless".into());
            if timeless.clicked() {
                commands.push(Command::OpenPopup(PopupKind::TimelessJewel));
            }
            ui.separator();
            self.heatmap(ui, state, power, rects, commands);
            ui.separator();
            self.link_menu(ui, state, rects, commands, &mut out.queries);
        });
        self.dialogs(ui.ctx(), state, rects, commands);
    }

    pub(super) fn specs(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let active = state.specs.get(state.active_spec);
        let combo = egui::ComboBox::from_id_salt("tree_spec")
            .selected_text(active.map_or("", |s| s.title.as_str()))
            .show_ui(ui, |ui| {
                for (i, spec) in state.specs.iter().enumerate() {
                    let item = ui.selectable_label(
                        i == state.active_spec,
                        format!("{} ({} points)", spec.title, spec.points),
                    );
                    rects.note(&item, || format!("spec:{i}"));
                    if item.clicked() && i != state.active_spec {
                        commands.push(Command::SelectSpec { index: i });
                    }
                }
            });
        rects.note(&combo.response, || "spec_combo".into());
        self.spec_combo = Some(combo.response.id);
        self.spec_busy = combo.response.hovered()
            || egui::Popup::is_id_open(ui.ctx(), combo.response.id.with("popup"));
        let add = ui.button("+").on_hover_text("Add a tree");
        rects.note(&add, || "spec_new".into());
        if add.clicked() {
            self.dialog = Some(Dialog::NewSpec(String::new()));
        }
        let rename = ui.add_enabled(active.is_some(), egui::Button::new("Rename"));
        rects.note(&rename, || "spec_rename".into());
        if rename.clicked()
            && let Some(spec) = active
        {
            self.dialog = Some(Dialog::RenameSpec {
                index: state.active_spec,
                title: spec.title.clone(),
            });
        }
        let delete = ui.add_enabled(state.specs.len() > 1, egui::Button::new("Delete"));
        rects.note(&delete, || "spec_delete".into());
        if delete.clicked() {
            self.dialog = Some(Dialog::DeleteSpec(state.active_spec));
        }
        ui.label("Compare");
        let compare = egui::ComboBox::from_id_salt("tree_compare")
            .selected_text(
                state
                    .compare_spec
                    .and_then(|i| state.specs.get(i))
                    .map_or("None", |s| s.title.as_str()),
            )
            .show_ui(ui, |ui| {
                let none = ui.selectable_label(state.compare_spec.is_none(), "None");
                rects.note(&none, || "compare:none".into());
                if none.clicked() && state.compare_spec.is_some() {
                    commands.push(Command::SetCompareSpec { index: None });
                }
                for (i, spec) in state.specs.iter().enumerate() {
                    if i == state.active_spec {
                        continue;
                    }
                    let item = ui.selectable_label(state.compare_spec == Some(i), &spec.title);
                    rects.note(&item, || format!("compare:{i}"));
                    if item.clicked() && state.compare_spec != Some(i) {
                        commands.push(Command::SetCompareSpec { index: Some(i) });
                    }
                }
            });
        rects.note(&compare.response, || "compare_combo".into());
    }

    /// PoB's point display (`pointDisplay`), in the top bar; `progress` is its tooltip
    pub(super) fn points(&self, ui: &mut egui::Ui, state: &TreeState, progress: &str) {
        let p = state.points;
        let colour = |over: bool| {
            if over {
                theme::NEGATIVE
            } else {
                theme::TEXT_DIM
            }
        };
        let points = ui.label(
            egui::RichText::new(format!("Points {}/{}", p.used, p.available))
                .color(colour(p.used > p.available)),
        );
        let ascendancy = ui.label(
            egui::RichText::new(format!(
                "Ascendancy {}/{}",
                p.ascendancy_used, p.ascendancy_available
            ))
            .color(colour(p.ascendancy_used > p.ascendancy_available)),
        );
        if !progress.is_empty() {
            let default = ui.visuals().text_color();
            for response in [points, ascendancy] {
                response.on_hover_ui(|ui| {
                    ui.label(pob_text(progress, 14.0, default));
                });
            }
        }
    }

    /// PoB's class and ascendancy dropdowns (`classDrop`, `ascendDrop`), in the top bar
    pub(super) fn class(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let class = state.classes.iter().find(|c| c.id == state.class_id);
        let combo = egui::ComboBox::from_id_salt("tree_class")
            .selected_text(class.map_or("Class", |c| c.name.as_str()))
            .show_ui(ui, |ui| {
                for c in &state.classes {
                    let item = ui.selectable_label(c.id == state.class_id, &c.name);
                    rects.note(&item, || format!("class:{}", c.id));
                    if item.clicked() && c.id != state.class_id {
                        if c.needs_confirm {
                            self.class_dialog = Some((c.id, c.name.clone()));
                        } else {
                            commands.push(Command::SelectClass {
                                id: c.id,
                                connect: false,
                            });
                        }
                    }
                }
            });
        rects.note(&combo.response, || "class_combo".into());
        let ascendancies = class.map_or(&[][..], |c| c.ascendancies.as_slice());
        let current = ascendancies.iter().find(|a| a.id == state.ascendancy_id);
        let combo = egui::ComboBox::from_id_salt("tree_ascendancy")
            .selected_text(current.map_or("Ascendancy", |a| a.name.as_str()))
            .show_ui(ui, |ui| {
                for a in ascendancies {
                    let item = ui.selectable_label(a.id == state.ascendancy_id, &a.name);
                    rects.note(&item, || format!("ascendancy:{}", a.id));
                    if item.clicked() && a.id != state.ascendancy_id {
                        commands.push(Command::SelectAscendancy { id: a.id });
                    }
                }
            });
        rects.note(&combo.response, || "ascendancy_combo".into());
    }

    pub(super) fn search(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let now = ui.input(|i| i.time);
        // the engine changed the search itself (an undo): show it, unless an edit is on its way
        if state.search != self.search_seen {
            self.search_seen = state.search.clone();
            if self.search_edit.is_none() {
                self.search = state.search.clone();
            }
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search nodes")
                .desired_width(160.0),
        );
        rects.note(&field, || "search".into());
        if std::mem::take(&mut self.focus_search) {
            field.request_focus();
        }
        if field.changed() {
            self.search_edit = Some(now);
        }
        if !self.search.is_empty() && ui.small_button("x").clicked() {
            self.search.clear();
            self.search_edit = Some(f64::NEG_INFINITY);
        }
        if let Some(edited) = self.search_edit {
            let waited = now - edited;
            if waited >= SEARCH_DEBOUNCE {
                self.search_edit = None;
                if self.search != state.search {
                    commands.push(Command::SetTreeSearch {
                        text: self.search.clone(),
                    });
                }
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(
                        SEARCH_DEBOUNCE - waited,
                    ));
            }
        }
    }

    fn heatmap(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        power: &PowerStatus,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let set =
            |stat: Option<String>, max_depth: Option<u32>| Command::SetHeatmap { stat, max_depth };
        ui.label("Heat map");
        let combo = egui::ComboBox::from_id_salt("tree_heat_stat")
            .selected_text(state.heatmap.as_deref().unwrap_or(if state.heatmap_shown {
                "Damage / defence"
            } else {
                "Off"
            }))
            .show_ui(ui, |ui| {
                let shown = state.heatmap.is_some() || state.heatmap_shown;
                let off = ui.selectable_label(!shown, "Off");
                rects.note(&off, || "heat:off".into());
                if off.clicked() && shown {
                    commands.push(set(None, state.max_depth));
                }
                for stat in &state.power_stats {
                    let selected = state.heatmap.as_ref() == Some(stat);
                    let item = ui.selectable_label(selected, stat);
                    rects.note(&item, || format!("heat:{stat}"));
                    if item.clicked() && !selected {
                        commands.push(set(Some(stat.clone()), state.max_depth));
                    }
                }
            });
        rects.note(&combo.response, || "heat_stat".into());
        ui.add_enabled_ui(state.heatmap.is_some(), |ui| {
            let combo = egui::ComboBox::from_id_salt("tree_heat_depth")
                .selected_text(
                    state
                        .max_depth
                        .map_or("All".to_owned(), |d| format!("Within {d}")),
                )
                .show_ui(ui, |ui| {
                    let all = ui.selectable_label(state.max_depth.is_none(), "All");
                    rects.note(&all, || "depth:all".into());
                    if all.clicked() && state.max_depth.is_some() {
                        commands.push(set(state.heatmap.clone(), None));
                    }
                    for depth in DEPTHS {
                        let item =
                            ui.selectable_label(state.max_depth == Some(depth), depth.to_string());
                        rects.note(&item, || format!("depth:{depth}"));
                        if item.clicked() && state.max_depth != Some(depth) {
                            commands.push(set(state.heatmap.clone(), Some(depth)));
                        }
                    }
                });
            rects.note(&combo.response, || "heat_depth".into());
        });
        if power.failed {
            ui.colored_label(theme::NEGATIVE, "calculation failed");
            let retry = ui.button("Retry");
            rects.note(&retry, || "heat_retry".into());
            if retry.clicked() {
                commands.push(set(state.heatmap.clone(), state.max_depth));
            }
        } else if let Some(percent) = power.progress {
            ui.add(
                egui::ProgressBar::new(percent as f32 / 100.0)
                    .desired_width(100.0)
                    .text(format!("{percent}%")),
            );
        }
        let before = self.report_open;
        let toggle = ui.toggle_value(&mut self.report_open, "Power report");
        rects.note(&toggle, || "report_toggle".into());
        // the report is built from the heat map's calculation
        if self.report_open
            && !before
            && state.heatmap.is_none()
            && let Some(stat) = state.power_stats.first()
        {
            commands.push(set(Some(stat.clone()), state.max_depth));
        }
    }

    fn link_menu(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
        queries: &mut Vec<Command>,
    ) {
        let menu = ui.menu_button("Link…", |ui| {
            let import = ui.button("Import link…");
            rects.note(&import, || "link_import".into());
            if import.clicked() {
                self.dialog = Some(Dialog::Import {
                    title: String::new(),
                    url: String::new(),
                });
            }
            let copy = ui.button("Copy link");
            rects.note(&copy, || "link_copy".into());
            if copy.clicked() {
                self.copy_pending = true;
                self.link_fresh = false;
                queries.push(Command::ExportTreeLink);
            }
            if let Some(version) = &state.convertible_to {
                ui.separator();
                for (key, label, keep_copy) in [
                    ("link_convert", "Convert this tree", false),
                    ("link_convert_copy", "Convert this tree (keep copy)", true),
                ] {
                    let item = ui.button(label);
                    rects.note(&item, || key.into());
                    if item.clicked() {
                        commands.push(Command::ConvertSpec {
                            version: version.clone(),
                            keep_copy,
                        });
                    }
                }
                let all = ui.button("Convert all trees");
                rects.note(&all, || "link_convert_all".into());
                if all.clicked() {
                    commands.push(Command::ConvertAllSpecs {
                        version: version.clone(),
                    });
                }
            }
        });
        rects.note(&menu.response, || "link_menu".into());
    }

    /// The class question, asked from the top bar on any tab
    pub(super) fn class_dialog(
        &mut self,
        ctx: &egui::Context,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let Some((id, name)) = &self.class_dialog else {
            return;
        };
        let (id, name) = (*id, name.clone());
        let mut close = false;
        egui::Window::new("Change class")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Some of your points don't connect to {name}. Reset the tree, or keep the \
                     points that can connect to it?"
                ));
                ui.horizontal(|ui| {
                    if button(ui, rects, "class_reset", "Reset tree", true) {
                        commands.push(Command::SelectClass { id, connect: false });
                        close = true;
                    }
                    if button(ui, rects, "class_connect", "Connect path", true) {
                        commands.push(Command::SelectClass { id, connect: true });
                        close = true;
                    }
                    close |= ui.button("Cancel").clicked();
                });
            });
        if close {
            self.class_dialog = None;
        }
    }

    fn dialogs(
        &mut self,
        ctx: &egui::Context,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut close = false;
        let title = match dialog {
            Dialog::NewSpec(_) => "New tree",
            Dialog::RenameSpec { .. } => "Rename tree",
            Dialog::DeleteSpec(_) => "Delete tree",
            Dialog::Reset => "Reset tree",
            Dialog::RemoveTattoos => "Remove all tattoos",
            Dialog::Import { .. } => "Import tree link",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| match dialog {
                Dialog::NewSpec(name) => {
                    let field = ui.add(egui::TextEdit::singleline(name).hint_text("Title"));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let valid = !name.trim().is_empty();
                        if (button(ui, rects, "spec_new_ok", "Add", valid) || (enter && valid))
                            && valid
                        {
                            commands.push(Command::NewSpec {
                                title: name.trim().to_owned(),
                            });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::RenameSpec { index, title } => {
                    let field = ui.add(egui::TextEdit::singleline(title));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let valid = !title.trim().is_empty();
                        if (button(ui, rects, "spec_rename_ok", "Rename", valid)
                            || (enter && valid))
                            && valid
                        {
                            commands.push(Command::RenameSpec {
                                index: *index,
                                title: title.trim().to_owned(),
                            });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::DeleteSpec(index) => {
                    let name = state.specs.get(*index).map_or("", |s| s.title.as_str());
                    ui.label(format!("Delete the tree {name}?"));
                    ui.horizontal(|ui| {
                        if button(ui, rects, "spec_delete_yes", "Delete", true) {
                            commands.push(Command::DeleteSpec { index: *index });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::Reset => {
                    ui.label("Remove every allocated passive point from this tree?");
                    ui.horizontal(|ui| {
                        if button(ui, rects, "reset_yes", "Reset", true) {
                            commands.push(Command::ResetTree);
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::RemoveTattoos => {
                    // PoB adds no undo state for this, so the question is the only safeguard
                    ui.label("Remove every tattoo from this tree? This can't be undone.");
                    ui.horizontal(|ui| {
                        if button(ui, rects, "remove_tattoos_yes", "Remove", true) {
                            commands.push(Command::RemoveAllTattoos);
                            close = true;
                        }
                        let cancel = ui.button("Cancel");
                        rects.note(&cancel, || "remove_tattoos_no".into());
                        close |= cancel.clicked();
                    });
                }
                Dialog::Import { title, url } => {
                    ui.label("Adds the link's tree as a new tree.");
                    ui.add(egui::TextEdit::singleline(title).hint_text("Title"));
                    ui.add(
                        egui::TextEdit::singleline(url)
                            .hint_text("Tree link")
                            .desired_width(400.0),
                    );
                    ui.horizontal(|ui| {
                        let valid = !title.trim().is_empty() && !url.trim().is_empty();
                        if button(ui, rects, "import_ok", "Import", valid) {
                            commands.push(Command::ImportTreeLink {
                                title: title.trim().to_owned(),
                                url: url.trim().to_owned(),
                            });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
            });
        if close {
            self.dialog = None;
        }
    }
}

/// A dialog's primary button; its rect is recorded under `key` for tests.
fn button(ui: &mut egui::Ui, rects: &mut Rects, key: &str, label: &str, enabled: bool) -> bool {
    let response = ui.add_enabled(enabled, theme::primary_button(label));
    rects.note(&response, || key.to_owned());
    response.clicked()
}
