//! The slots column: a dropdown per slot, flask boxes and the jewel sockets.

use super::{ItemDrag, ItemsOutput, ItemsUi};
use crate::engine::views::{ItemsView, SlotKind, SlotView};
use crate::engine::{Command, ItemTooltipQuery};
use crate::widgets::pob_text;
use eframe::egui::{self, text::LayoutJob};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotSection {
    Equipment,
    Flasks,
    Jewels,
}

/// One row of the column; `slot` indexes `ItemsView.slots`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotRow {
    pub slot: usize,
    pub section: SlotSection,
    pub indent: bool,
}

/// The shown slots in PoB's order, by section: equipment (an abyssal socket right after its
/// parent, indented), flasks, then jewels.
pub fn rows(view: &ItemsView) -> Vec<SlotRow> {
    let mut rows = Vec::new();
    let abyssal = |parent: &str| -> Vec<usize> {
        view.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.shown && s.kind == SlotKind::Abyssal)
            .filter(|(_, s)| s.parent.as_deref() == Some(parent))
            .map(|(i, _)| i)
            .collect()
    };
    for (section, wanted) in [
        (
            SlotSection::Equipment,
            &[SlotKind::Gear, SlotKind::Swap, SlotKind::Graft][..],
        ),
        (SlotSection::Flasks, &[SlotKind::Flask][..]),
        (SlotSection::Jewels, &[SlotKind::Jewel][..]),
    ] {
        for (i, s) in view.slots.iter().enumerate() {
            if s.shown && wanted.contains(&s.kind) {
                rows.push(SlotRow {
                    slot: i,
                    section,
                    indent: false,
                });
                for child in abyssal(&s.name) {
                    rows.push(SlotRow {
                        slot: child,
                        section,
                        indent: true,
                    });
                }
            }
        }
    }
    // an abyssal socket whose parent isn't listed still shows
    for (i, s) in view.slots.iter().enumerate() {
        if s.kind == SlotKind::Abyssal && s.shown && !rows.iter().any(|r| r.slot == i) {
            rows.push(SlotRow {
                slot: i,
                section: SlotSection::Equipment,
                indent: true,
            });
        }
    }
    rows.sort_by_key(|r| r.section as u8);
    rows
}

/// The item's name in its rarity colour.
pub(super) fn item_label(
    view: &ItemsView,
    id: u32,
    size: f32,
    default: egui::Color32,
) -> LayoutJob {
    match view.items.iter().find(|e| e.id == id) {
        Some(e) => pob_text(&format!("{}{}", e.colour, e.name), size, default),
        None => pob_text(&format!("(item {id})"), size, default),
    }
}

impl ItemsUi {
    pub(super) fn slots_panel(
        &mut self,
        ui: &mut egui::Ui,
        view: &ItemsView,
        out: &mut ItemsOutput,
    ) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut section = None;
                for row in rows(view) {
                    if section != Some(row.section) {
                        match row.section {
                            SlotSection::Equipment => {}
                            SlotSection::Flasks => {
                                ui.add_space(6.0);
                                ui.strong("Flasks");
                            }
                            SlotSection::Jewels => {
                                ui.add_space(6.0);
                                ui.strong("Jewels");
                            }
                        }
                        section = Some(row.section);
                    }
                    self.slot_row(ui, view, &view.slots[row.slot], row.indent, out);
                }
                // a slot that wasn't drawn (hidden) can't take the focus now, or later
                self.focus_slot = None;
            });
    }

    fn slot_row(
        &mut self,
        ui: &mut egui::Ui,
        view: &ItemsView,
        slot: &SlotView,
        indent: bool,
        out: &mut ItemsOutput,
    ) {
        let default = ui.visuals().text_color();
        let (zone, dropped) = ui.dnd_drop_zone::<ItemDrag, _>(egui::Frame::NONE, |ui| {
            ui.horizontal(|ui| {
                let pad = if indent { 16.0 } else { 0.0 };
                ui.add_space(pad);
                ui.add_sized(
                    [110.0 - pad, 18.0],
                    egui::Label::new(&slot.label).truncate(),
                );
                let current = if slot.selected == 0 {
                    pob_text("None", 14.0, default)
                } else {
                    item_label(view, slot.selected, 14.0, default)
                };
                let combo = ui.add_enabled_ui(slot.enabled, |ui| {
                    egui::ComboBox::from_id_salt(("items_slot", &slot.name))
                        .width(200.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            let none = ui.selectable_label(slot.selected == 0, "None");
                            self.rects
                                .note(&none, || format!("slot_{}_none", slot.name));
                            if none.clicked() && slot.selected != 0 {
                                out.commands.push(Command::EquipItem {
                                    slot: slot.name.clone(),
                                    id: None,
                                });
                            }
                            for &id in &slot.choices {
                                let entry = ui.selectable_label(
                                    slot.selected == id,
                                    item_label(view, id, 14.0, default),
                                );
                                self.rects
                                    .note(&entry, || format!("slot_{}_item_{id}", slot.name));
                                if entry.clicked() && slot.selected != id {
                                    out.commands.push(Command::EquipItem {
                                        slot: slot.name.clone(),
                                        id: Some(id),
                                    });
                                }
                                self.note_hover(
                                    &entry,
                                    ItemTooltipQuery {
                                        id,
                                        slot: Some(slot.name.clone()),
                                    },
                                );
                            }
                        })
                });
                self.rects.note(&combo.inner.response, || {
                    format!("slot_combo_{}", slot.name)
                });
                // the tree's right click on this slot's socket: scroll to it and drop it down
                if self.focus_slot.as_deref() == Some(&slot.name) {
                    self.focus_slot = None;
                    let button = &combo.inner.response;
                    button.scroll_to_me(Some(egui::Align::Center));
                    button.request_focus();
                    egui::Popup::open_id(ui.ctx(), button.id.with("popup"));
                }
                if slot.selected != 0 {
                    self.note_hover(
                        &combo.inner.response,
                        ItemTooltipQuery {
                            id: slot.selected,
                            slot: Some(slot.name.clone()),
                        },
                    );
                }
                if let Some(active) = slot.flask_active {
                    let mut on = active;
                    let check =
                        ui.add_enabled(slot.selected != 0, egui::Checkbox::without_text(&mut on));
                    self.rects
                        .note(&check, || format!("slot_flask_{}", slot.name));
                    if check.changed() {
                        out.commands.push(Command::SetFlaskActive {
                            slot: slot.name.clone(),
                            on,
                        });
                    }
                }
            });
        });
        self.rects
            .note(&zone.response, || format!("slot_{}", slot.name));
        // a dragged list row dropped on the slot equips it, if PoB's dropdown offers it
        if let Some(drag) = dropped
            && slot.choices.contains(&drag.id)
        {
            out.commands.push(Command::EquipItem {
                slot: slot.name.clone(),
                id: Some(drag.id),
            });
        }
    }
}
