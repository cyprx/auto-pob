//! The Skills tab: PoB's skill sets, socket groups, gems and gem options. It takes engine
//! snapshots and returns commands; it knows nothing else about PoB.

pub mod group;
pub mod groups;
mod keys;
pub mod picker;
pub mod sets;

use crate::engine::views::{GemTooltipView, GroupTooltipView, SkillsView};
use crate::engine::{Command, GemTarget, GemTooltipQuery, Snapshot};
use crate::ui::tree::Rects;
use crate::widgets::{EditBoxOptions, EditBoxes};
use eframe::egui;
use picker::GemPicker;
use std::{collections::HashMap, sync::Arc};

/// A group row must be hovered this long before its tooltip is asked for.
const HOVER_DELAY: f64 = 0.1;

/// The field PoB doesn't count as an unsaved change
const DEFAULT_QUALITY: &str = "default_quality";

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct SkillsOutput {
    pub commands: Vec<Command>,
    /// Questions (tooltips, copy); the app sends them without waiting for the answer.
    pub queries: Vec<Command>,
}

#[derive(Default)]
pub struct SkillsUi {
    view: Option<Arc<SkillsView>>,
    /// The selected group, 0-based; UI state, clamped as the group count shrinks
    selected: usize,
    /// The next view with more groups selects the last one (New, Paste)
    select_new: bool,
    /// The text fields, by key; they send on Enter or blur
    boxes: EditBoxes<String>,
    /// The command each field with a pending edit would send, made when the edit was last seen
    pending_commands: HashMap<String, (String, Command)>,
    /// A group with gems the user asked to delete
    confirm_delete: Option<usize>,
    confirm_delete_all: bool,
    /// The group whose copy text we asked for
    copy_asked: Option<usize>,
    /// Text to hand to the system clipboard next frame
    clipboard: Option<String>,
    /// The last text copied, which the row menu's Paste uses (egui can't read the clipboard)
    last_copied: Option<String>,
    tooltip: Option<Arc<GroupTooltipView>>,
    hover: Option<(usize, f64)>,
    tooltip_asked: Option<usize>,
    sets: sets::State,
    picker: Option<GemPicker>,
    /// A snapshot changed the groups under the open picker
    close_picker: bool,
    /// The frame's time, for what opens a picker
    now: f64,
    /// A gem name in the table hovered since this time
    gem_hover: Option<((usize, usize), f64)>,
    gem_tooltip_asked: Option<(usize, usize)>,
    gem_tooltip: Option<Arc<GemTooltipView>>,
    /// An app-level dialog is open, so Ctrl+V is left alone
    modal: bool,
    pub(crate) rects: Rects,
}

impl SkillsUi {
    /// Forgets the build's state, as for another build.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The undo and redo buttons' enablement.
    pub fn history(&self) -> (bool, bool) {
        self.view
            .as_ref()
            .map_or((false, false), |v| (v.can_undo, v.can_redo))
    }

    /// An edit typed into a field is waiting to be sent, so the build has unsaved changes the
    /// engine doesn't know about yet.
    pub fn has_pending(&self) -> bool {
        // the default quality option never marks the build unsaved in PoB
        self.boxes.pending_keys().any(|k| k != DEFAULT_QUALITY)
    }

    /// Drops the pending edit (the user chose not to save).
    pub fn discard(&mut self) {
        self.boxes.discard();
        self.pending_commands.clear();
    }

    pub fn set_modal(&mut self, modal: bool) {
        self.modal = modal;
    }

    /// A confirm or the set manager is open, so shortcuts are left alone.
    pub fn has_dialog(&self) -> bool {
        self.confirm_delete.is_some()
            || self.confirm_delete_all
            || self.sets.is_open()
            || self.picker.is_some()
    }

    /// Closes the dialogs, as when the user leaves the tab.
    /// Returns the command that closes the gem picker's search, if one was open.
    pub fn close_dialogs(&mut self) -> Option<Command> {
        self.confirm_delete = None;
        self.confirm_delete_all = false;
        self.copy_asked = None;
        self.clipboard = None;
        self.sets.close();
        self.picker.take().map(|_| Command::CloseGemSearch)
    }

    /// The edit typed into a field that hasn't lost focus yet, so a Save or a tab change
    /// doesn't leave it behind.
    pub fn flush(&mut self) -> Option<Command> {
        let (key, text) = self.boxes.take_pending()?;
        self.pending_command(&key, &text)
    }

    /// The command for a flushed edit, which was made from the same text.
    fn pending_command(&mut self, key: &str, text: &str) -> Option<Command> {
        let (made_from, command) = self.pending_commands.remove(key)?;
        debug_assert_eq!(made_from, text, "the command for {key} is stale");
        Some(command)
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        match snapshot {
            Snapshot::Skills(view) => {
                if self.view.as_deref() != Some(&**view) {
                    // what the engine says about a group may have changed
                    self.tooltip = None;
                    self.tooltip_asked = None;
                    self.gem_tooltip = None;
                    self.gem_tooltip_asked = None;
                }
                let old = self.view.as_ref().map_or(0, |v| v.groups.len());
                let n = view.groups.len();
                // the picker's group index may mean another group now (or none)
                if self.view.is_some()
                    && self
                        .picker
                        .as_ref()
                        .is_some_and(|p| n != old || p.group() >= n)
                {
                    self.close_picker = true;
                }
                if std::mem::take(&mut self.select_new) && n > old {
                    self.selected = n - 1;
                }
                self.selected = self.selected.min(n.saturating_sub(1));
                self.view = Some(view.clone());
            }
            Snapshot::SocketGroupText(t) => {
                if self.copy_asked == Some(t.group) {
                    self.copy_asked = None;
                    self.last_copied = Some(t.text.clone());
                    self.clipboard = Some(t.text.clone());
                }
            }
            Snapshot::GroupTooltip(t) => self.tooltip = Some(t.clone()),
            Snapshot::GemSearch(v) => {
                if let Some(p) = &mut self.picker {
                    p.on_search(v.clone());
                }
            }
            Snapshot::GemTooltip(t) => match (&mut self.picker, &t.query.gem_id) {
                (Some(p), Some(_)) => p.on_tooltip(t.clone()),
                (_, None) if self.gem_tooltip_asked == Some((t.query.group, slot_of(&t.query))) => {
                    self.gem_tooltip = Some(t.clone());
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// The DPS sort reported progress.
    pub fn on_gem_progress(&mut self, percent: u32) {
        if let Some(p) = &mut self.picker {
            p.on_progress(percent);
        }
    }

    /// The DPS sort failed; the picker offers Retry.
    pub fn on_gem_failed(&mut self) {
        if let Some(p) = &mut self.picker {
            p.on_failed();
        }
    }

    pub(crate) fn open_picker(&mut self, group: usize, target: GemTarget) {
        self.picker = Some(GemPicker::open(group, target, self.now));
    }

    /// `busy`: the engine hasn't caught up with what was sent, so typed text stays.
    pub fn show(&mut self, ui: &mut egui::Ui, now: f64, busy: bool) -> SkillsOutput {
        let mut out = SkillsOutput::default();
        self.now = now;
        let Some(view) = self.view.clone() else {
            ui.spinner();
            return out;
        };
        if let Some(text) = self.clipboard.take() {
            ui.ctx().copy_text(text);
        }
        // Ctrl+V outside a text field and any dialog
        if !self.has_dialog() && !self.modal && !ui.ctx().text_edit_focused() {
            let pasted = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
            });
            if let Some(text) = pasted {
                self.select_new = true;
                out.commands.push(Command::PasteSocketGroup { text });
            }
        }

        self.set_row(ui, &view, &mut out);
        ui.separator();
        egui::Panel::left("skills_groups")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::Panel::bottom("skills_options")
                    .resizable(false)
                    .show(ui, |ui| self.options(ui, &view, busy, &mut out));
                self.group_list(ui, &view, now, &mut out);
            });
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| self.group_pane(ui, &view, busy, &mut out));

        let ctx = ui.ctx().clone();
        self.dialogs(&ctx, &view, &mut out);
        self.sets_window(&ctx, &view, &mut out);
        if std::mem::take(&mut self.close_picker) && self.picker.take().is_some() {
            out.commands.push(Command::CloseGemSearch);
        }
        if let Some(picker) = &mut self.picker {
            let shown = picker.show(ui, now);
            out.commands.extend(shown.commands);
            out.queries.extend(shown.queries);
            if shown.close {
                self.picker = None;
                out.commands.push(Command::CloseGemSearch);
            }
        }

        // an edit whose field wasn't drawn this frame (another group was selected) is sent now
        for (key, text) in self.boxes.end_frame() {
            out.commands.extend(self.pending_command(&key, &text));
        }
        // a group row hovered long enough asks for its tooltip
        match self.hover {
            Some((group, since)) if self.tooltip_asked != Some(group) => {
                if now - since >= HOVER_DELAY {
                    self.tooltip_asked = Some(group);
                    out.queries.push(Command::GroupTooltip { group });
                } else {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            _ => {}
        }
        out
    }

    /// A text field that sends on Enter or when it loses focus. While it has focus an
    /// incoming view doesn't replace what is typed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn field(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        current: &str,
        width: f32,
        busy: bool,
        make: impl Fn(String) -> Command,
        out: &mut Vec<Command>,
    ) -> egui::Response {
        let opts = EditBoxOptions {
            width,
            ..EditBoxOptions::default()
        };
        let (response, sent) = self.boxes.show(ui, key.to_owned(), current, busy, &opts);
        self.rects.note(&response, || key.to_owned());
        match self.boxes.pending_text(&key.to_owned()) {
            Some(text) => {
                self.pending_commands
                    .insert(key.to_owned(), (text.to_owned(), make(text.to_owned())));
            }
            None => {
                self.pending_commands.remove(key);
            }
        }
        out.extend(sent.map(make));
        response
    }

    #[cfg(test)]
    pub(crate) fn rect(&self, key: &str) -> egui::Rect {
        self.picker
            .as_ref()
            .and_then(|p| p.rects.map.get(key))
            .copied()
            .unwrap_or_else(|| self.rects.map[key])
    }
}

/// The slot a table gem's tooltip query is for.
fn slot_of(query: &GemTooltipQuery) -> usize {
    match query.target {
        GemTarget::Slot(s) => s,
        GemTarget::Imbued => usize::MAX,
    }
}

#[cfg(test)]
pub(crate) mod tests;
