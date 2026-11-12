//! The Items view: PoB's slot rows for both builds, each item with its tooltip on hover, PoB's
//! difference label, and its Copy and Equip buttons (R8).

use super::{CompareOutput, CompareUi};
use crate::engine::Command;
use crate::engine::ItemTooltipQuery;
use crate::engine::views::{CompareItemRow, CompareView};
use crate::widgets::pob_text;
use eframe::egui;

/// PoB's warning after a jewel whose socket the tree doesn't allocate
const UNALLOCATED: &str = "^xFF9922  (tree missing allocated node)";
/// The comparison's item names are drawn in PoB's unique colour, as PoB has no rarity for them
/// in the row
const THEIRS_COLOUR: &str = "^xAF6025";

/// PoB's `getSlotDiffLabel` marker, as the engine sends it
pub(super) fn diff_label(row: &CompareItemRow) -> &str {
    &row.diff_label
}

pub(super) fn show(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    view: &CompareView,
    out: &mut CompareOutput,
) {
    let default = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    egui::ScrollArea::both()
        .id_salt("compare_items")
        .auto_shrink([false, false])
        .show(ui, |ui| {
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
                        show_row(ui, tab, row, out, default, weak);
                        ui.end_row();
                    }
                });
        });
}

fn show_row(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    row: &CompareItemRow,
    out: &mut CompareOutput,
    default: egui::Color32,
    weak: egui::Color32,
) {
    ui.label(pob_text(&row.label, 14.0, default));
    let ours = match row.ours {
        Some(id) => {
            let mut name = tab.item_names.get(&id).cloned().unwrap_or_default();
            if row.ours_unallocated {
                name.push_str(UNALLOCATED);
            }
            let r = ui.label(pob_text(&name, 14.0, default));
            tab.tip.note(
                &r,
                ItemTooltipQuery {
                    id,
                    slot: Some(row.slot.clone()),
                    theirs: false,
                },
            );
            r
        }
        None => ui.label(egui::RichText::new("(empty)").color(weak)),
    };
    tab.rects.note(&ours, || format!("ours:{}", row.slot));
    let theirs = match &row.theirs {
        Some(name) => {
            let mut name = format!("{THEIRS_COLOUR}{name}");
            if row.theirs_unallocated {
                name.push_str(UNALLOCATED);
            }
            let r = ui.label(pob_text(&name, 14.0, default));
            tab.tip.note(
                &r,
                ItemTooltipQuery {
                    id: 0,
                    slot: Some(row.slot.clone()),
                    theirs: true,
                },
            );
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
        let copy = ui.button("Copy");
        tab.rects.note(&copy, || format!("copy:{}", row.slot));
        if copy.clicked() {
            out.commands.push(Command::CopyCompareItem {
                slot: row.slot.clone(),
                and_use: false,
            });
        }
        if row.can_equip {
            let equip = ui.button("Equip");
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
