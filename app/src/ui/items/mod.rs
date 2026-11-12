//! The Items tab: PoB's equipment slots, item list and item sets. It takes engine snapshots
//! and returns commands; it knows nothing else about PoB.

pub mod crafting;
pub mod db;
pub mod display;
pub mod list;
pub mod sets;
pub mod shared;
pub mod slots;
pub mod text_window;

use crate::engine::views::{DisplayItemView, ItemDbView, ItemTooltipView, ItemsView};
use crate::engine::{Command, ItemTooltipQuery, Snapshot};
use crate::ui::tree::Rects;
use crate::widgets::item_tooltip_ui;
use eframe::egui;
use std::sync::Arc;

/// A row must be hovered this long before its tooltip is asked for.
const HOVER_DELAY: f64 = 0.1;

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct ItemsOutput {
    pub commands: Vec<Command>,
    /// Questions (tooltips, copy); the app sends them without waiting for the answer.
    pub queries: Vec<Command>,
    /// The tree dropdown's "Manage..." (PoB's spec manager): the app shows the Tree tab, where
    /// ours lives
    pub manage_trees: bool,
}

/// The payload of a list row being dragged: onto a slot to equip, onto a row to reorder.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ItemDrag {
    pub index: usize,
    pub id: u32,
}

#[derive(Default)]
pub struct ItemsUi {
    view: Option<Arc<ItemsView>>,
    /// The selected item's id
    selected: Option<u32>,
    /// An item in use the user asked to delete, with PoB's question
    confirm_delete: Option<(u32, String)>,
    confirm_delete_all: bool,
    /// The item whose copy text we asked for
    copy_asked: Option<u32>,
    /// The shared item whose copy text we asked for (Ctrl+C on a shared row)
    shared_copy_asked: Option<usize>,
    /// Text to hand to the system clipboard next frame
    clipboard: Option<String>,
    tooltip: Option<Arc<ItemTooltipView>>,
    /// What the pointer rests on, and since when
    hover: Option<(ItemTooltipQuery, f64)>,
    /// The latest tooltip query sent; only its reply is kept
    tooltip_asked: Option<ItemTooltipQuery>,
    /// What the pointer is over this frame
    hovered_now: Option<ItemTooltipQuery>,
    sets: sets::State,
    db: db::DbState,
    /// The open database's latest view
    db_view: Option<Arc<ItemDbView>>,
    /// The jewel slot the tree's right click asked for, until its row is drawn
    focus_slot: Option<String>,
    /// The id of the item on the display panel, if it is one of the build's
    display_id: Option<u32>,
    /// What the display panel shows, if anything
    display_view: Option<Arc<DisplayItemView>>,
    display: display::DisplayState,
    /// The display item whose list entry vanished, already cancelled
    cancelled: Option<u32>,
    /// The display item to cancel: its list entry has gone. The cancel names it, so it does
    /// nothing if another item is displayed by then.
    cancel_display: Option<u32>,
    /// An app-level dialog is open, so shortcuts are left alone
    modal: bool,
    pub(crate) rects: Rects,
    /// The slot under the pointer last frame holds this item: E edits it (ItemsTab.lua:1460)
    hovered_slot: Option<u32>,
    /// The shared item row the Delete button acts on
    shared_selected: Option<usize>,
    /// A shared item the user asked to delete, until PoB's question is answered
    confirm_shared_delete: Option<usize>,
}

impl ItemsUi {
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

    pub fn set_modal(&mut self, modal: bool) {
        self.modal = modal;
    }

    /// A confirm or the set manager is open, so shortcuts are left alone.
    pub fn has_dialog(&self) -> bool {
        self.confirm_delete.is_some()
            || self.confirm_delete_all
            || self.sets.is_open()
            || self.display.text_window.is_some()
            || self.confirm_shared_delete.is_some()
    }

    /// Closes the dialogs, as when the user leaves the tab.
    pub fn close_dialogs(&mut self) {
        self.confirm_delete = None;
        self.confirm_delete_all = false;
        self.copy_asked = None;
        self.shared_copy_asked = None;
        self.clipboard = None;
        self.sets.close();
        self.display.text_window = None;
        self.display.submitted = None;
        self.confirm_shared_delete = None;
    }

    /// What must go out before Save or a tab change, even while the tab isn't drawn: a quality
    /// box still being typed in (R22: not counted as unsaved, PoB never counts display edits),
    /// then the cancel of a display item whose list entry has gone.
    pub fn flush(&mut self) -> Vec<Command> {
        let mut commands = self.display.take_pending();
        if let Some(id) = self.cancel_display.take() {
            commands.push(Command::CancelDisplayItem { if_id: Some(id) });
        }
        // the database search still waiting out its pause (R4)
        commands.extend(self.db.flush());
        commands
    }

    /// Leaving the tab closes the engine's database; the panel is as it was on the return.
    pub fn leave(&mut self) -> Option<Command> {
        self.focus_slot = None;
        self.db_view = None;
        self.db.leave()
    }

    /// The tree's right click on a jewel socket: shows the slot of the socket `node` and
    /// opens its dropdown, as PoB does (`slot.dropped`, `SelectControl`).
    pub fn focus_slot(&mut self, node: u32) {
        self.focus_slot = self
            .view
            .as_ref()
            .and_then(|v| v.slots.iter().find(|s| s.node == Some(node)))
            .map(|s| s.name.clone());
    }

    /// The jewel sockets whose slot PoB lets the user use: the tree's right click jumps to
    /// these only.
    pub fn enabled_sockets(&self) -> Vec<u32> {
        self.view.as_ref().map_or_else(Vec::new, |v| {
            v.slots
                .iter()
                .filter(|s| s.enabled)
                .filter_map(|s| s.node)
                .collect()
        })
    }

    #[cfg(test)]
    pub(crate) fn quality_text(&self) -> &str {
        &self.display.quality.text
    }

    #[cfg(test)]
    pub fn focused_slot(&self) -> Option<&str> {
        self.focus_slot.as_deref()
    }

    /// The database sort's progress.
    pub fn on_db_progress(&mut self, percent: u32) {
        self.db.on_progress(percent);
    }

    /// The database load or sort failed.
    pub fn on_db_failed(&mut self, message: String) {
        self.db.on_failed(message);
    }

    #[cfg(test)]
    pub fn text_window_open(&self) -> bool {
        self.display.text_window.is_some()
    }

    /// The engine refused the raw text: the window stays open for another try.
    pub fn text_refused(&mut self) {
        self.display.submitted = None;
    }

    /// The raw text went to the engine as command `seq`; the first view from `seq` on closes
    /// the window.
    pub fn text_sent(&mut self, seq: u64) {
        self.display.submitted = Some(seq);
    }

    /// A snapshot with no seq of its own (tests): it counts as the latest.
    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        self.on_snapshot_at(snapshot, u64::MAX);
    }

    pub fn on_snapshot_at(&mut self, snapshot: &Snapshot, seq: u64) {
        match snapshot {
            Snapshot::Items(view) => {
                if self.view.as_deref() != Some(&**view) {
                    // what the engine says about an item may have changed
                    self.tooltip = None;
                    self.tooltip_asked = None;
                }
                if self
                    .selected
                    .is_some_and(|id| !view.items.iter().any(|e| e.id == id))
                {
                    self.selected = None;
                }
                self.view = Some(view.clone());
                self.check_orphan();
            }
            Snapshot::DisplayItem(display) => {
                let old = self.display_view.as_ref().map(|d| (d.id, d.name.clone()));
                let new = display.as_ref().map(|d| (d.id, d.name.clone()));
                // a window opened on another item is stale; one just saved is done
                if self.display.submitted.is_some_and(|s| seq >= s)
                    || (old != new && self.display.text_window.is_some())
                {
                    self.display.text_window = None;
                    self.display.submitted = None;
                }
                if display.is_none() || old != new {
                    self.display.on_gone();
                }
                self.display.on_view();
                self.display_view = display.clone();
                self.display_id = display.as_ref().and_then(|d| d.id);
                if self.cancelled != self.display_id {
                    self.cancelled = None;
                }
                // a different item is displayed now: the cancel was for the old one
                if self.cancel_display.is_some() && self.cancel_display != self.display_id {
                    self.cancel_display = None;
                }
                self.check_orphan();
            }
            Snapshot::ControlTooltip(view) => {
                self.display.crafting.on_snapshot(snapshot);
                self.db.on_tooltip(view);
            }
            Snapshot::ItemDb(view) => {
                self.db.on_view(view.as_deref());
                self.db_view = view.clone();
            }
            Snapshot::ItemTooltip(t) => {
                // only the latest question is answered for; an earlier one is stale
                if self.tooltip_asked.as_ref() == Some(&t.query) {
                    self.tooltip = Some(t.clone());
                }
            }
            Snapshot::ItemText(t) if !t.shared && self.copy_asked == Some(t.id) => {
                self.copy_asked = None;
                self.clipboard = Some(t.text.clone());
            }
            Snapshot::ItemText(t) if t.shared && self.shared_copy_asked == Some(t.id as usize) => {
                self.shared_copy_asked = None;
                self.clipboard = Some(t.text.clone());
            }
            _ => {}
        }
    }

    /// PoB orphans a displayed item whose list entry was deleted if it is then saved, so the
    /// display is cancelled once the item has gone.
    fn check_orphan(&mut self) {
        if let (Some(id), Some(view)) = (self.display_id, &self.view)
            && self.cancelled != Some(id)
            && !view.items.iter().any(|e| e.id == id)
        {
            self.cancelled = Some(id);
            self.cancel_display = Some(id);
        }
    }

    /// Notes what the pointer is over, and draws the tooltip once its reply is here.
    pub(crate) fn note_hover(&mut self, response: &egui::Response, mut query: ItemTooltipQuery) {
        let dragging = egui::DragAndDrop::has_payload_of_type::<ItemDrag>(&response.ctx)
            || egui::DragAndDrop::has_payload_of_type::<db::DbDrag>(&response.ctx)
            || egui::DragAndDrop::has_payload_of_type::<shared::SharedDrag>(&response.ctx);
        if dragging {
            return;
        }
        // Shift held asks for PoB's child tooltips (R34); a change of key is a new question
        query.shift = response.ctx.input(|i| i.modifiers.shift);
        if response.hovered() {
            self.hovered_now = Some(query.clone());
        }
        if let Some(t) = self.tooltip.clone()
            && t.query == query
        {
            response.clone().on_hover_ui(|ui| {
                item_tooltip_ui(ui, &t.tooltip);
            });
        }
    }

    /// `busy`: the engine hasn't caught up with what was sent.
    pub fn show(&mut self, ui: &mut egui::Ui, now: f64, busy: bool) -> ItemsOutput {
        let mut out = ItemsOutput::default();
        let Some(view) = self.view.clone() else {
            ui.spinner();
            return out;
        };
        if let Some(text) = self.clipboard.take() {
            ui.ctx().copy_text(text);
        }
        if let Some(id) = self.cancel_display.take() {
            out.commands
                .push(Command::CancelDisplayItem { if_id: Some(id) });
        }
        self.hovered_now = None;
        self.hovered_slot = None;
        self.shortcuts(ui, &view, &mut out);

        self.set_bar(ui, &view, &mut out);
        ui.separator();
        egui::Panel::left("items_slots")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| self.slots_panel(ui, &view, &mut out));
        egui::Panel::right("items_display")
            .resizable(true)
            .default_size(380.0)
            .show(ui, |ui| match self.display_view.clone() {
                Some(d) => out
                    .commands
                    .extend(display::show(ui, &d, &mut self.display, busy)),
                None => {
                    out.commands
                        .extend(display::show_empty(ui, &mut self.display));
                    // PoB's shared list hangs below "Craft item..."
                    self.shared_panel(ui, &view, &mut out);
                }
            });
        out.queries.extend(self.display.crafting.take_queries());
        egui::CentralPanel::default().show(ui, |ui| {
            // PoB's databases sit below the item list
            egui::Panel::bottom("items_db")
                .resizable(self.db.expanded)
                .default_size(300.0)
                .show(ui, |ui| {
                    let o = db::show(ui, self.db_view.as_deref(), &mut self.db, now);
                    out.commands.extend(o.commands);
                    out.queries.extend(o.queries);
                });
            egui::CentralPanel::default().show(ui, |ui| self.item_list(ui, &view, &mut out));
        });

        let ctx = ui.ctx().clone();
        self.dialogs(&ctx, &mut out);
        self.shared_dialog(&ctx, &view, &mut out);
        out.commands.extend(self.display.text_window(&ctx));
        self.sets_window(&ctx, &view, &mut out);

        // the hover timer restarts when the pointer moves to something else
        match (self.hovered_now.take(), &self.hover) {
            (Some(q), Some((p, _))) if q == *p => {}
            (Some(q), _) => {
                self.hover = Some((q, now));
                self.tooltip_asked = None;
                self.tooltip = None;
            }
            (None, _) => {
                self.hover = None;
                self.tooltip_asked = None;
                self.tooltip = None;
            }
        }
        if let Some((query, since)) = &self.hover
            && self.tooltip_asked.as_ref() != Some(query)
        {
            if now - since >= HOVER_DELAY {
                self.tooltip_asked = Some(query.clone());
                out.queries.push(Command::ItemTooltip(query.clone()));
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        out
    }

    /// Ctrl+V (outside a text field), Ctrl+D.
    fn shortcuts(&mut self, ui: &egui::Ui, view: &ItemsView, out: &mut ItemsOutput) {
        if self.has_dialog() || self.modal || ui.ctx().text_edit_focused() {
            return;
        }
        let pasted = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(text) => Some(text.clone()),
                _ => None,
            })
        });
        if let Some(text) = pasted {
            out.commands.push(Command::PasteItem { text });
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D)) {
            out.commands.push(Command::SetShowStatDifferences {
                on: !view.show_stat_differences,
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn rect(&self, key: &str) -> egui::Rect {
        *self
            .rects
            .map
            .get(key)
            .or_else(|| self.display.rects.map.get(key))
            .or_else(|| self.display.crafting.rects.map.get(key))
            .or_else(|| self.db.rects.map.get(key))
            .unwrap_or_else(|| panic!("no rect {key}; have {:?}", self.rects.map.keys()))
    }

    #[cfg(test)]
    pub(crate) fn has_rect(&self, key: &str) -> bool {
        self.rects.map.contains_key(key) || self.display.rects.map.contains_key(key)
    }
}

#[cfg(test)]
pub(crate) mod tests;
