//! The Items tab: PoB's equipment slots, item list and item sets. It takes engine snapshots
//! and returns commands; it knows nothing else about PoB.

pub mod list;
pub mod sets;
pub mod slots;

use crate::engine::views::{ItemTooltipView, ItemsView};
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
    /// The id of the item on the display panel, if it is one of the build's
    display_id: Option<u32>,
    /// The display item whose list entry vanished, already cancelled
    cancelled: Option<u32>,
    cancel_display: bool,
    /// An app-level dialog is open, so shortcuts are left alone
    modal: bool,
    pub(crate) rects: Rects,
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

    /// An edit still being typed that must go out before Save or a tab change. The tab has
    /// none yet.
    pub fn flush(&mut self) -> Option<Command> {
        None
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
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
                self.display_id = display.as_ref().and_then(|d| d.id);
                if self.cancelled != self.display_id {
                    self.cancelled = None;
                }
                self.check_orphan();
            }
            Snapshot::ItemTooltip(t) => {
                // only the latest question is answered for; an earlier one is stale
                if self.tooltip_asked.as_ref() == Some(&t.query) {
                    self.tooltip = Some(t.clone());
                }
            }
            Snapshot::ItemText(t) if self.copy_asked == Some(t.id) => {
                self.copy_asked = None;
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
            self.cancel_display = true;
        }
    }

    /// Notes what the pointer is over, and draws the tooltip once its reply is here.
    pub(crate) fn note_hover(&mut self, response: &egui::Response, query: ItemTooltipQuery) {
        let dragging = egui::DragAndDrop::has_payload_of_type::<ItemDrag>(&response.ctx);
        if dragging {
            return;
        }
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
    pub fn show(&mut self, ui: &mut egui::Ui, now: f64, _busy: bool) -> ItemsOutput {
        let mut out = ItemsOutput::default();
        let Some(view) = self.view.clone() else {
            ui.spinner();
            return out;
        };
        if let Some(text) = self.clipboard.take() {
            ui.ctx().copy_text(text);
        }
        if std::mem::take(&mut self.cancel_display) {
            out.commands.push(Command::CancelDisplayItem);
        }
        self.hovered_now = None;
        self.shortcuts(ui, &view, &mut out);

        self.set_bar(ui, &view, &mut out);
        ui.separator();
        egui::Panel::left("items_slots")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| self.slots_panel(ui, &view, &mut out));
        egui::CentralPanel::default().show(ui, |ui| self.item_list(ui, &view, &mut out));

        let ctx = ui.ctx().clone();
        self.dialogs(&ctx, &view, &mut out);
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
            .unwrap_or_else(|| panic!("no rect {key}; have {:?}", self.rects.map.keys()))
    }
}

#[cfg(test)]
pub(crate) mod tests;
