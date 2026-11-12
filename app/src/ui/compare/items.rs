//! The Items view: PoB's slot rows for both builds, each item with its tooltip on hover, PoB's
//! difference label, and its Copy and Equip buttons (R8). In PoB's expanded mode each row shows
//! both items' tooltips inline, asked for one at a time.

use super::{CompareOutput, CompareUi};
use crate::engine::Command;
use crate::engine::ItemTooltipQuery;
use crate::engine::views::{CompareItemRow, CompareView, ItemTooltipView};
use crate::widgets::{item_tooltip_ui, pob_text};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// PoB's warning after a jewel whose socket the tree doesn't allocate
const UNALLOCATED: &str = "^xFF9922  (tree missing allocated node)";
/// An inline tooltip not answered in this long (an item that vanished) is given up on
const EXPANDED_TIMEOUT: f64 = 2.0;

/// PoB's `getSlotDiffLabel` marker, as the engine sends it
pub(super) fn diff_label(row: &CompareItemRow) -> &str {
    &row.diff_label
}

/// The two tooltip questions of a row: ours and theirs, where there is an item
fn queries(row: &CompareItemRow) -> [Option<ItemTooltipQuery>; 2] {
    [
        row.ours.map(|id| ItemTooltipQuery {
            id,
            slot: Some(row.slot.clone()),
            theirs: false,
        }),
        row.theirs.as_ref().map(|_| ItemTooltipQuery {
            id: 0,
            slot: Some(row.slot.clone()),
            theirs: true,
        }),
    ]
}

/// The key of a question: the engine answers the latest one only, so they go one at a time
fn key(query: &ItemTooltipQuery) -> (u32, Option<String>, bool) {
    (query.id, query.slot.clone(), query.theirs)
}

/// The expanded mode's tooltips: answered ones, the one asked and when, and the ones the
/// engine never answered (gone)
#[derive(Default)]
pub(super) struct Expanded {
    cards: HashMap<(u32, Option<String>, bool), Arc<ItemTooltipView>>,
    asked: Option<(ItemTooltipQuery, f64)>,
    gone: HashSet<(u32, Option<String>, bool)>,
    /// The rows' questions still unanswered this frame, in order
    wanted: Vec<ItemTooltipQuery>,
}

impl Expanded {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn on_reply(&mut self, view: &Arc<ItemTooltipView>) {
        if self.asked.as_ref().is_some_and(|(q, _)| *q == view.query) {
            self.asked = None;
            self.cards.insert(key(&view.query), view.clone());
        }
    }

    /// The next question to send, once the last one is answered or given up on.
    pub(super) fn next_query(&mut self, now: f64) -> Option<Command> {
        let wanted = std::mem::take(&mut self.wanted);
        if let Some((query, since)) = &self.asked {
            if now - since < EXPANDED_TIMEOUT {
                return None;
            }
            self.gone.insert(key(query));
            self.asked = None;
        }
        let next = wanted
            .into_iter()
            .find(|q| !self.cards.contains_key(&key(q)) && !self.gone.contains(&key(q)))?;
        self.asked = Some((next.clone(), now));
        Some(Command::ItemTooltip(next))
    }

    /// Draws a row's inline tooltip, or notes that it is wanted.
    fn card(&mut self, ui: &mut egui::Ui, query: Option<ItemTooltipQuery>) {
        let Some(query) = query else {
            ui.weak("(empty)");
            return;
        };
        match self.cards.get(&key(&query)) {
            Some(card) => {
                item_tooltip_ui(ui, &card.tooltip);
            }
            None if self.gone.contains(&key(&query)) => {
                ui.weak("(no tooltip)");
            }
            None => {
                ui.spinner();
                self.wanted.push(query);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn has_card(&self, query: &ItemTooltipQuery) -> bool {
        self.cards.contains_key(&key(query))
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    view: &CompareView,
    out: &mut CompareOutput,
) {
    let default = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let expanded = CompareUi::expanded_on(view);
    egui::ScrollArea::both()
        .id_salt("compare_items")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if expanded {
                for row in &view.items {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        show_row(ui, tab, row, out, default, weak, false);
                    });
                    let [ours, theirs] = queries(row);
                    ui.columns(2, |columns| {
                        tab.expanded.card(&mut columns[0], ours);
                        tab.expanded.card(&mut columns[1], theirs);
                    });
                }
                return;
            }
            egui::Grid::new("compare_items_grid")
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("Slot");
                    ui.strong("This build");
                    ui.strong(super::plain(&view.headers[1]));
                    ui.label("");
                    ui.label("");
                    ui.end_row();
                    for row in &view.items {
                        show_row(ui, tab, row, out, default, weak, true);
                        ui.end_row();
                    }
                });
        });
}

/// This build's item name as PoB draws it: from the row, else from the Items view, else its id
fn ours_name(tab: &CompareUi, row: &CompareItemRow, id: u32) -> String {
    let colour = if row.ours_colour.is_empty() {
        "^7"
    } else {
        &row.ours_colour
    };
    match &row.ours_name {
        Some(name) => format!("{colour}{name}"),
        None => tab
            .item_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("^7Item {id}")),
    }
}

/// One row: label, both names (hover tooltips unless the tooltips are inline), PoB's marker and
/// the buttons
fn show_row(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    row: &CompareItemRow,
    out: &mut CompareOutput,
    default: egui::Color32,
    weak: egui::Color32,
    hover: bool,
) {
    let [our_query, their_query] = queries(row);
    ui.label(pob_text(&row.label, 14.0, default));
    let ours = match row.ours {
        Some(id) => {
            let mut name = ours_name(tab, row, id);
            if row.ours_unallocated {
                name.push_str(UNALLOCATED);
            }
            let r = ui.label(pob_text(&name, 14.0, default));
            if hover && let Some(query) = our_query {
                tab.tip.note(&r, query);
            }
            r
        }
        None => ui.label(egui::RichText::new("(empty)").color(weak)),
    };
    tab.rects.note(&ours, || format!("ours:{}", row.slot));
    let theirs = match &row.theirs {
        Some(name) => {
            let colour = if row.theirs_colour.is_empty() {
                "^7"
            } else {
                &row.theirs_colour
            };
            let mut name = format!("{colour}{name}");
            if row.theirs_unallocated {
                name.push_str(UNALLOCATED);
            }
            let r = ui.label(pob_text(&name, 14.0, default));
            if hover && let Some(query) = their_query {
                tab.tip.note(&r, query);
            }
            r
        }
        None => ui.label(egui::RichText::new("(empty)").color(weak)),
    };
    tab.rects.note(&theirs, || format!("theirs:{}", row.slot));
    ui.label(pob_text(diff_label(row), 14.0, default));
    ui.horizontal(|ui| {
        if !row.can_copy {
            return;
        }
        // a double click while the engine is behind would copy twice
        let busy = tab.busy;
        let copy = ui.add_enabled(!busy, egui::Button::new("Copy"));
        tab.rects.note(&copy, || format!("copy:{}", row.slot));
        if copy.clicked() {
            out.commands.push(Command::CopyCompareItem {
                slot: row.slot.clone(),
                and_use: false,
            });
        }
        if row.can_equip {
            let equip = ui.add_enabled(!busy, egui::Button::new("Equip"));
            tab.rects.note(&equip, || format!("equip:{}", row.slot));
            if equip.clicked() {
                out.commands.push(Command::CopyCompareItem {
                    slot: row.slot.clone(),
                    and_use: true,
                });
            }
        } else {
            // PoB draws this where this build lacks the slot (Ring 3)
            let r = ui.label(pob_text("^xBBBBBBMissing slot", 14.0, default));
            tab.rects.note(&r, || format!("missing:{}", row.slot));
        }
    });
}
