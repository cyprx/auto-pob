//! The socket group list, its menu, drag reordering, the options below it and the confirms.

use super::{SkillsOutput, SkillsUi};
use crate::engine::views::{DropList, SkillsView, SocketGroupView};
use crate::engine::{Command, SkillOption};
use crate::widgets::{pob_text, tooltip_ui};
use eframe::egui::{self, Color32, Rect, Stroke};

pub use crate::widgets::drop_index;

/// The row's text: a star on the main skill, dimmed when disabled, italic for an item's group.
fn row_text(g: &SocketGroupView, default: Color32) -> egui::text::LayoutJob {
    let colour = if g.enabled {
        default
    } else {
        default.gamma_multiply(0.5)
    };
    let label = pob_text(&g.display_label, 14.0, colour);
    let mut job = if g.is_main {
        pob_text("^xFFD700\u{2605} ", 14.0, colour)
    } else {
        egui::text::LayoutJob::default()
    };
    for s in &label.sections {
        let range = usize::from(s.byte_range.start)..usize::from(s.byte_range.end);
        job.append(&label.text[range], 0.0, s.format.clone());
    }
    if g.source.is_some() {
        for s in &mut job.sections {
            s.format.italics = true;
        }
    }
    job
}

/// PoB's usage tips under the group list (SkillsTab.lua:111-117)
const GROUP_TIP: &str = "^7Usage Tips:
- You can copy/paste socket groups using Ctrl+C and Ctrl+V.
- Ctrl + Click to enable/disable socket groups.
- Ctrl + Right click to include/exclude in FullDPS calculations.
- Right click to set as the Main skill group.";

impl SkillsUi {
    pub(super) fn group_list(
        &mut self,
        ui: &mut egui::Ui,
        view: &SkillsView,
        now: f64,
        out: &mut SkillsOutput,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Socket groups");
            let new = ui.button("+ New");
            self.rects.note(&new, || "new_group".into());
            if new.clicked() {
                self.select_new = true;
                out.commands.push(Command::AddSocketGroup);
            }
            let all = ui.add_enabled(!view.groups.is_empty(), egui::Button::new("Delete All"));
            self.rects.note(&all, || "delete_all".into());
            if all.clicked() {
                self.confirm_delete_all = true;
            }
            // PoB's Delete acts on the selected group, not one of an item (SkillListControl.lua:
            // 33-38)
            // nothing is selected after a delete, as PoB's `selValue = nil` (:176, 187)
            let selected = view.groups.get(self.selected).filter(|_| !self.unselected);
            let delete = ui.add_enabled(
                selected.is_some_and(|g| g.source.is_none()),
                egui::Button::new("Delete"),
            );
            self.rects.note(&delete, || "delete".into());
            if delete.clicked()
                && let Some(g) = selected
            {
                self.ask_delete(self.selected, g, out);
            }
        });
        let default = ui.visuals().text_color();
        // PoB's usage tips under its list (SkillsTab.lua:110-118)
        egui::Panel::bottom("skills_group_tip").show(ui, |ui| {
            ui.label(pob_text(GROUP_TIP, 13.0, default));
        });
        let ctx = ui.ctx().clone();
        let dragging = egui::DragAndDrop::has_payload_of_type::<usize>(&ctx);
        let mut target: Option<(usize, bool)> = None;
        let mut hovered_row = None;
        let mut indicator = None;
        let (_, dropped) = ui.dnd_drop_zone::<usize, _>(egui::Frame::NONE, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    for (i, g) in view.groups.iter().enumerate() {
                        let top = ui.cursor().min.y;
                        let id = ui.scope_id().with(("skills_row", i));
                        let selected = i == self.selected;
                        let inner = ui.dnd_drag_source(id, i, |ui| {
                            let fill = if selected {
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
                                        egui::Label::new(row_text(g, default)).selectable(false),
                                    );
                                });
                        });
                        // the click goes over the drag source, which senses only drags
                        let row = ui.interact(
                            inner.response.rect,
                            id.with("click"),
                            egui::Sense::click(),
                        );
                        self.rects.note(&row, || format!("row_{i}"));
                        let rect = Rect::from_min_max(
                            egui::pos2(ui.min_rect().left(), top),
                            egui::pos2(ui.max_rect().right(), ui.cursor().min.y),
                        );
                        // Ctrl+click enables or disables, Ctrl+right-click includes or
                        // excludes from Full DPS (SkillListControl.lua:202-225)
                        let ctrl = ui.input(|inp| inp.modifiers.command);
                        // the click selects the row first (ListControl.lua:338-370)
                        if row.clicked() {
                            self.selected = i;
                            self.unselected = false;
                            row.request_focus();
                            if ctrl {
                                out.commands.push(Command::SetGroupEnabled {
                                    group: i,
                                    on: !g.enabled,
                                });
                            }
                        }
                        let full_dps = row.secondary_clicked() && ctrl;
                        if full_dps {
                            out.commands.push(Command::SetGroupFullDps {
                                group: i,
                                on: !g.full_dps,
                            });
                        }
                        if row.hovered() {
                            hovered_row = Some(i);
                        }
                        if dragging
                            && let Some(p) = ui.input(|inp| inp.pointer.hover_pos())
                            && rect.contains(p)
                        {
                            let below = p.y > rect.center().y;
                            target = Some((i, below));
                            let y = if below { rect.bottom() } else { rect.top() };
                            indicator = Some((rect.left(), rect.right(), y));
                        }
                        if self.focus_group == Some(i) {
                            self.focus_group = None;
                            row.request_focus();
                        } else if row.has_focus() {
                            crate::ui::shortcuts::keep_arrows(ui.ctx(), row.id);
                            self.row_keys(ui, i, view.groups.len(), g, out);
                        }
                        if !full_dps {
                            self.row_menu(&row, i, g, out);
                        }
                        if !dragging
                            && let Some(t) = &self.tooltip
                            && t.group == i
                            && !t.lines.is_empty()
                        {
                            row.on_hover_ui(|ui| tooltip_ui(ui, &t.lines));
                        }
                    }
                });
        });
        if let Some((l, r, y)) = indicator {
            ui.painter()
                .hline(l..=r, y, Stroke::new(2.0, Color32::from_rgb(120, 180, 255)));
        }
        if let (Some(from), Some((hover, below))) = (dropped, target)
            && let Some(to) = drop_index(*from, hover, below)
        {
            self.move_group(*from, to, out);
        }
        // the hover timer restarts when the pointer moves to another row
        match (hovered_row, self.hover) {
            (Some(i), Some((j, _))) if i == j => {}
            (Some(i), _) => {
                self.hover = Some((i, now));
                self.tooltip_asked = None;
                self.tooltip = None;
            }
            (None, _) => {
                self.hover = None;
                self.tooltip_asked = None;
            }
        }
    }

    fn move_group(&mut self, from: usize, to: usize, out: &mut SkillsOutput) {
        self.selected = to;
        out.commands.push(Command::MoveSocketGroup { from, to });
    }

    /// Ctrl+C, Delete or Backspace, and Up, Down, Home and End on the focused row
    /// (ListControl.lua:375-392).
    fn row_keys(
        &mut self,
        ui: &egui::Ui,
        i: usize,
        len: usize,
        g: &SocketGroupView,
        out: &mut SkillsOutput,
    ) {
        let copy = ui.input(|inp| inp.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if copy && g.source.is_none() {
            self.ask_copy(i, out);
        }
        if ui.input(crate::ui::shortcuts::list_delete) && !self.unselected {
            self.ask_delete(i, g, out);
        }
        if let Some(to) = ui.input_mut(|inp| crate::ui::shortcuts::list_key(inp, len, Some(i))) {
            self.selected = to;
            self.unselected = false;
            self.focus_group = Some(to);
        }
    }

    fn ask_copy(&mut self, group: usize, out: &mut SkillsOutput) {
        self.copy_asked = Some(group);
        out.queries.push(Command::CopySocketGroup { group });
    }

    /// A group with gems is deleted only after the user confirms, as in PoB; an item's group
    /// is not deleted, PoB says why (SkillListControl.lua:165-189).
    fn ask_delete(&mut self, group: usize, g: &SocketGroupView, out: &mut SkillsOutput) {
        if g.source.is_some() {
            self.item_group_message = true;
        } else if g.gems.is_empty() {
            self.delete_group(group, g.name.clone(), out);
        } else {
            self.confirm_delete = Some(group);
            self.confirm_name = g.name.clone();
        }
    }

    /// PoB clears the list's selection after a delete (SkillListControl.lua:176, 187): Delete
    /// and the Delete key do nothing until a group is selected again. `name` is the group's
    /// `displayLabel` as shown, which the engine checks.
    fn delete_group(&mut self, group: usize, name: String, out: &mut SkillsOutput) {
        if group < self.selected {
            self.selected -= 1;
        }
        self.unselected = true;
        out.commands.push(Command::DeleteSocketGroup {
            group,
            name: Some(name),
        });
    }

    fn row_menu(
        &mut self,
        row: &egui::Response,
        i: usize,
        g: &SocketGroupView,
        out: &mut SkillsOutput,
    ) {
        row.context_menu(|ui| {
            self.selected = i;
            self.unselected = false;
            let main = ui.button("Set as main skill");
            self.rects.note(&main, || "menu_main".into());
            if main.clicked() {
                out.commands.push(Command::SetMainSocketGroup { group: i });
                ui.close();
            }
            let editable = g.source.is_none();
            let copy = ui.add_enabled(editable, egui::Button::new("Copy"));
            self.rects.note(&copy, || "menu_copy".into());
            if copy.clicked() {
                self.ask_copy(i, out);
                ui.close();
            }
            let paste = ui.add_enabled(
                self.last_copied.is_some(),
                egui::Button::new("Paste last copied group"),
            );
            self.rects.note(&paste, || "menu_paste".into());
            if paste.clicked()
                && let Some(text) = self.last_copied.clone()
            {
                self.select_new = true;
                out.commands.push(Command::PasteSocketGroup { text });
                ui.close();
            }
            let delete = ui.add_enabled(editable, egui::Button::new("Delete"));
            self.rects.note(&delete, || "menu_delete".into());
            if delete.clicked() {
                self.ask_delete(i, g, out);
                ui.close();
            }
            let all = ui.button("Delete All");
            self.rects.note(&all, || "menu_delete_all".into());
            if all.clicked() {
                self.confirm_delete_all = true;
                ui.close();
            }
        });
    }

    /// The confirms for Delete and Delete All.
    pub(super) fn dialogs(
        &mut self,
        ctx: &egui::Context,
        view: &SkillsView,
        out: &mut SkillsOutput,
    ) {
        if self.item_group_message {
            let window = egui::Id::unique("skills_item_group_message");
            let mut close = crate::widgets::escape::escape(ctx, window);
            egui::Window::new("Delete Socket Group")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(
                        "This socket group cannot be deleted as it is created by an equipped item.",
                    );
                    let ok = ui.button("Ok");
                    self.rects.note(&ok, || "item_group_ok".into());
                    close |= ok.clicked();
                });
            crate::widgets::escape::note(ctx, window);
            if close {
                self.item_group_message = false;
            }
        }
        if let Some(group) = self.confirm_delete {
            let name = view
                .groups
                .get(group)
                .map_or(String::new(), |g| g.name.clone());
            // Esc is PoB's Cancel (PopupDialog.lua:72-77)
            let window = egui::Id::unique("skills_delete");
            let mut choice = crate::widgets::escape::escape(ctx, window).then_some(false);
            // PoB's text (SkillListControl.lua:178)
            egui::Window::new("Delete Socket Group")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Are you sure you want to delete '{name}'?"));
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
                let name = std::mem::take(&mut self.confirm_name);
                self.delete_group(group, name, out);
            }
            if choice.is_some() {
                self.confirm_delete = None;
            }
        }
        if self.confirm_delete_all {
            // Esc is PoB's Cancel (PopupDialog.lua:72-77)
            let window = egui::Id::unique("skills_delete_all");
            let mut choice = crate::widgets::escape::escape(ctx, window).then_some(false);
            // PoB's text (SkillListControl.lua:40)
            egui::Window::new("Delete All")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Are you sure you want to delete all socket groups in this build?");
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
                self.selected = 0;
                out.commands.push(Command::DeleteAllSocketGroups);
            }
            if choice.is_some() {
                self.confirm_delete_all = false;
            }
        }
    }

    /// The gem options under the list.
    pub(super) fn options(
        &mut self,
        ui: &mut egui::Ui,
        view: &SkillsView,
        busy: bool,
        out: &mut SkillsOutput,
    ) {
        let o = &view.options;
        ui.strong("Options");
        ui.horizontal_wrapped(|ui| {
            ui.label("Sort gems by:");
            if let Some(i) = self.drop_list(ui, "sort_field", &o.sort_field, &[]) {
                out.commands
                    .push(Command::SetSkillOption(SkillOption::SortField(i)));
            }
            let mut on = o.sort_by_dps;
            let check = ui.checkbox(&mut on, "DPS");
            self.rects.note(&check, || "sort_by_dps".into());
            if check.changed() {
                out.commands
                    .push(Command::SetSkillOption(SkillOption::SortByDps(on)));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Default gem level:");
            if let Some(i) =
                self.drop_list(ui, "default_level", &o.default_level, &o.default_level_help)
            {
                out.commands
                    .push(Command::SetSkillOption(SkillOption::DefaultLevel(i)));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Default gem quality:");
            self.field(
                ui,
                "default_quality",
                &o.default_quality,
                40.0,
                busy,
                |text| Command::SetSkillOption(SkillOption::DefaultQuality(text)),
                &mut out.commands,
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Show support gems:");
            if let Some(i) = self.drop_list(ui, "support_types", &o.support_types, &[]) {
                out.commands
                    .push(Command::SetSkillOption(SkillOption::SupportTypes(i)));
            }
            let mut on = o.show_legacy;
            let check = ui.checkbox(&mut on, "Legacy");
            self.rects.note(&check, || "show_legacy".into());
            if check.changed() {
                out.commands
                    .push(Command::SetSkillOption(SkillOption::ShowLegacy(on)));
            }
        });
    }

    /// A dropdown showing the view's selected entry as given; returns a newly chosen index.
    fn drop_list(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        list: &DropList,
        help: &[String],
    ) -> Option<usize> {
        let default = ui.visuals().text_color();
        let current = list.labels.get(list.selected).map_or("", String::as_str);
        let mut chosen = None;
        let combo = ui.add_enabled_ui(list.enabled, |ui| {
            crate::widgets::dropdown::Dropdown::from_id_salt(("skills_option", key))
                .selected_text(pob_text(current, 14.0, default))
                .show_ui(ui, |ui| {
                    for (i, text) in list.labels.iter().enumerate() {
                        let item = crate::widgets::dropdown::item(
                            ui,
                            i == list.selected,
                            pob_text(text, 14.0, default),
                        );
                        self.rects.note(&item, || format!("{key}_item_{i}"));
                        if item.clicked() && i != list.selected {
                            chosen = Some(i);
                        }
                        // each entry's own description (SkillsTab.lua:132-137)
                        if let Some(line) = help.get(i).filter(|l| !l.is_empty()) {
                            item.on_hover_ui(|ui| {
                                ui.label(pob_text(&format!("^7{line}"), 14.0, default));
                            });
                        }
                    }
                })
        });
        self.rects.note(&combo.inner.response, || key.to_owned());
        // the wheel and the arrows step it (DropDownControl.lua:476-516)
        chosen = chosen.or_else(|| {
            crate::widgets::scroll_step::combo_step(
                ui,
                &combo.inner.response,
                list.labels.len(),
                list.selected,
            )
        });
        // the closed dropdown: the chosen entry's description
        if let Some(line) = help.get(list.selected).filter(|l| !l.is_empty()) {
            combo.inner.response.on_hover_ui(|ui| {
                ui.label(pob_text(&format!("^7{line}"), 14.0, default));
            });
        }
        chosen
    }
}
