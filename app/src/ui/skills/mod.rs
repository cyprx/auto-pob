//! The Skills tab: PoB's skill sets, socket groups, gems and gem options. It takes engine
//! snapshots and returns commands; it knows nothing else about PoB.

pub mod group;
pub mod groups;
pub mod sets;

use crate::engine::views::{GroupTooltipView, SkillsView};
use crate::engine::{Command, GemTarget, Snapshot};
use crate::ui::tree::Rects;
use eframe::egui;
use std::{collections::HashMap, sync::Arc};

/// A group row must be hovered this long before its tooltip is asked for.
const HOVER_DELAY: f64 = 0.1;

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct SkillsOutput {
    pub commands: Vec<Command>,
    /// Questions (tooltips, copy); the app sends them without waiting for the answer.
    pub queries: Vec<Command>,
}

/// An edit typed into a field that still has focus.
struct PendingEdit {
    key: String,
    command: Command,
    text: String,
}

#[derive(Default)]
pub struct SkillsUi {
    view: Option<Arc<SkillsView>>,
    /// The selected group, 0-based; UI state, clamped as the group count shrinks
    selected: usize,
    /// The next view with more groups selects the last one (New, Paste)
    select_new: bool,
    /// Text being typed, by field key
    edits: HashMap<String, String>,
    /// What each field last sent (or showed)
    sent: HashMap<String, String>,
    pending: Option<PendingEdit>,
    frame: u64,
    pending_frame: u64,
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
    /// Where the gem picker was asked to open; Task 9 replaces this stub
    picker: Option<(usize, GemTarget)>,
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
        self.pending.is_some()
    }

    /// Drops the pending edit (the user chose not to save).
    pub fn discard(&mut self) {
        self.pending = None;
    }

    pub fn set_modal(&mut self, modal: bool) {
        self.modal = modal;
    }

    /// A confirm or the set manager is open, so shortcuts are left alone.
    pub fn has_dialog(&self) -> bool {
        self.confirm_delete.is_some() || self.confirm_delete_all || self.sets.is_open()
    }

    /// Closes the dialogs, as when the user leaves the tab.
    pub fn close_dialogs(&mut self) {
        self.confirm_delete = None;
        self.confirm_delete_all = false;
        self.copy_asked = None;
        self.clipboard = None;
        self.sets.close();
    }

    /// The edit typed into a field that hasn't lost focus yet, so a Save or a tab change
    /// doesn't leave it behind.
    pub fn flush(&mut self) -> Option<Command> {
        let p = self.pending.take()?;
        self.sent.insert(p.key, p.text);
        Some(p.command)
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        match snapshot {
            Snapshot::Skills(view) => {
                if self.view.as_deref() != Some(&**view) {
                    // what the engine says about a group may have changed
                    self.tooltip = None;
                    self.tooltip_asked = None;
                }
                let old = self.view.as_ref().map_or(0, |v| v.groups.len());
                let n = view.groups.len();
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
            _ => {}
        }
    }

    /// The gem picker is a later task; this records the request.
    pub(crate) fn open_picker(&mut self, group: usize, target: GemTarget) {
        self.picker = Some((group, target));
    }

    /// `busy`: the engine hasn't caught up with what was sent, so typed text stays.
    pub fn show(&mut self, ui: &mut egui::Ui, now: f64, busy: bool) -> SkillsOutput {
        let mut out = SkillsOutput::default();
        self.frame += 1;
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

        // an edit whose field wasn't drawn this frame (another group was selected) is sent now
        if self.pending.is_some() && self.pending_frame != self.frame {
            out.commands.extend(self.flush());
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
        let id = ui.scope_id().with(("skills_field", key));
        let mut buf = self.edits.remove(key).unwrap_or_default();
        let focused = ui.memory(|m| m.has_focus(id));
        if !focused && !busy {
            buf.clear();
            buf.push_str(current);
            self.sent.insert(key.to_owned(), current.to_owned());
        }
        let response = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .id(id)
                .desired_width(width),
        );
        self.rects.note(&response, || key.to_owned());
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Esc reverts the field and sends nothing
            buf.clear();
            buf.push_str(self.sent.get(key).map_or(current, String::as_str));
            if self.pending.as_ref().is_some_and(|p| p.key == key) {
                self.pending = None;
            }
        } else if response.lost_focus() {
            // an edit already noted for this field wins over what the buffer shows now
            if self.pending.as_ref().is_some_and(|p| p.key == key) {
                buf = self.pending.take().map(|p| p.text).unwrap_or_default();
            }
            if self.sent.get(key).is_none_or(|s| *s != buf) {
                self.sent.insert(key.to_owned(), buf.clone());
                out.push(make(buf.clone()));
            }
        } else if response.has_focus() && self.sent.get(key).is_none_or(|s| *s != buf) {
            self.pending = Some(PendingEdit {
                key: key.to_owned(),
                command: make(buf.clone()),
                text: buf.clone(),
            });
            self.pending_frame = self.frame;
        } else if self.pending.as_ref().is_some_and(|p| p.key == key) {
            // typed back to what was sent
            self.pending = None;
        }
        self.edits.insert(key.to_owned(), buf);
        response
    }

    #[cfg(test)]
    pub(crate) fn rect(&self, key: &str) -> egui::Rect {
        self.rects.map[key]
    }
}

#[cfg(test)]
pub(crate) mod tests;
