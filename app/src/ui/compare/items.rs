//! The Items view: PoB's slot rows for both builds, each item with its tooltip on hover, PoB's
//! difference label, and its Buy (R10), Copy and Equip buttons (R8). In PoB's expanded mode
//! each row shows both items' cards as PoB draws them (`DrawItemExpanded`), each mod marked
//! against the other side's.

use super::{CompareOutput, CompareUi};
use crate::engine::ItemTooltipQuery;
use crate::engine::views::{CompareItemRow, CompareView};
use crate::engine::{BuySource, Command, PopupKind};
use crate::widgets::pob_text;
use eframe::egui;

/// PoB's warning after a jewel whose socket the tree doesn't allocate
const UNALLOCATED: &str = "^xFF9922  (tree missing allocated node)";
/// The room PoB gives a name in a row: its 310 px box less 8 px of padding, for 16 px text
/// (TradeHelpers.lua:465-509), scaled to the 14 px the names are drawn at here
const NAME_ROOM: f32 = (310.0 - 8.0) * 14.0 / 16.0;

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
            shift: false,
            shared: None,
        }),
        row.theirs.as_ref().map(|_| ItemTooltipQuery {
            id: 0,
            slot: Some(row.slot.clone()),
            theirs: true,
            shift: false,
            shared: None,
        }),
    ]
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
                    // PoB's line above the cards: the slot, our warning, the marker and the
                    // buttons; the names are the cards' own (CompareTab.lua:3819-3846)
                    ui.horizontal_wrapped(|ui| {
                        let mut label = format!("^7{}:", row.label);
                        if row.ours_unallocated {
                            label.push_str(UNALLOCATED);
                        }
                        let r = ui.label(pob_text(&label, 14.0, default));
                        tab.rects.note(&r, || format!("label:{}", row.slot));
                        ui.label(pob_text(diff_label(row), 14.0, default));
                        buttons(ui, tab, row, out, default);
                    });
                    ui.columns(2, |columns| {
                        card(
                            &mut columns[0],
                            tab,
                            &row.ours_card,
                            &format!("card:ours:{}", row.slot),
                            default,
                        );
                        card(
                            &mut columns[1],
                            tab,
                            &row.theirs_card,
                            &format!("card:theirs:{}", row.slot),
                            default,
                        );
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
                        show_row(ui, tab, row, out, default, weak);
                        ui.end_row();
                    }
                });
        });
}

/// This build's item name as PoB draws it, colour code apart: from the row, else from the
/// Items view, else its id
fn ours_name(tab: &CompareUi, row: &CompareItemRow, id: u32) -> (String, String) {
    let colour = if row.ours_colour.is_empty() {
        "^7"
    } else {
        &row.ours_colour
    };
    match &row.ours_name {
        Some(name) => (colour.to_owned(), name.clone()),
        // the Items view's names carry their colour already
        None => match tab.item_names.get(&id) {
            Some(name) => (String::new(), name.clone()),
            None => ("^7".to_owned(), format!("Item {id}")),
        },
    }
}

/// One side's card: PoB's lines in order, the name at PoB's larger size; "(empty)" for no item
fn card(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    lines: &[String],
    key: &str,
    default: egui::Color32,
) {
    let empty = matches!(lines, [only] if only == "^8(empty)");
    let r = ui
        .vertical(|ui| {
            for (i, line) in lines.iter().enumerate() {
                let size = if i == 0 && !empty { 16.0 } else { 14.0 };
                ui.label(pob_text(line, size, default));
            }
        })
        .response;
    tab.rects.note(&r, || key.to_owned());
}

/// A name as PoB fits it into its box: cut with "..." when too wide (`fitItemName`), the
/// warning included, as PoB fits `name .. warning`
fn fitted(ui: &egui::Ui, colour: &str, name: &str, default: egui::Color32) -> String {
    let painter = ui.painter();
    crate::widgets::fit_with_dots(colour, name, |text| {
        painter.layout_job(pob_text(text, 14.0, default)).size().x <= NAME_ROOM
    })
}

/// One row: label, both names (their tooltips on hover), PoB's marker and the buttons
fn show_row(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    row: &CompareItemRow,
    out: &mut CompareOutput,
    default: egui::Color32,
    weak: egui::Color32,
) {
    let [our_query, their_query] = queries(row);
    ui.label(pob_text(&row.label, 14.0, default));
    let ours = match row.ours {
        Some(id) => {
            let (colour, mut name) = ours_name(tab, row, id);
            if row.ours_unallocated {
                name.push_str(UNALLOCATED);
            }
            let name = fitted(ui, &colour, &name, default);
            let r = ui.label(pob_text(&name, 14.0, default));
            if let Some(query) = our_query {
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
            let mut name = name.clone();
            if row.theirs_unallocated {
                name.push_str(UNALLOCATED);
            }
            let name = fitted(ui, colour, &name, default);
            let r = ui.label(pob_text(&name, 14.0, default));
            if let Some(query) = their_query {
                tab.tip.note(&r, query);
            }
            r
        }
        None => ui.label(egui::RichText::new("(empty)").color(weak)),
    };
    tab.rects.note(&theirs, || format!("theirs:{}", row.slot));
    ui.label(pob_text(diff_label(row), 14.0, default));
    buttons(ui, tab, row, out, default);
}

/// A row's Buy, Copy and Equip (or "Missing slot"), where the comparison has an item
fn buttons(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    row: &CompareItemRow,
    out: &mut CompareOutput,
    default: egui::Color32,
) {
    ui.horizontal(|ui| {
        if !row.can_copy {
            return;
        }
        // a double click while the engine is behind would copy twice
        let busy = tab.busy;
        // PoB's Buy, first of the row's buttons: the trade search for their item (R10)
        let buy = ui.button("Buy");
        tab.rects.note(&buy, || format!("buy:{}", row.slot));
        if buy.clicked() {
            out.commands.push(Command::OpenPopup(PopupKind::BuySimilar {
                source: BuySource::CompareSlot(row.slot.clone()),
            }));
        }
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
