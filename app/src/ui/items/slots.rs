//! The slots column: a dropdown per slot, flask boxes and the jewel sockets.

use super::db::DbDrag;
use super::shared::SharedDrag;
use super::{ItemDrag, ItemsOutput, ItemsUi};
use crate::engine::views::{ItemsView, SlotKind, SlotView};
use crate::engine::{BuySource, Command, ItemTooltipQuery, PopupKind};
use crate::widgets::pob_text;
use eframe::egui::{self, text::LayoutJob};

/// The flask box's tooltip (ItemSlotControl.lua:41)
const FLASK_TIP: &str = "Activate this flask.";

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
        self.slot_header(ui, view, out);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut section = None;
                let mut spec_drawn = false;
                for row in rows(view) {
                    if section != Some(row.section) {
                        match row.section {
                            SlotSection::Equipment => {}
                            SlotSection::Flasks => {
                                ui.add_space(6.0);
                                ui.strong("Flasks");
                            }
                            SlotSection::Jewels => {
                                // PoB's tree dropdown sits between the flasks and the jewel
                                // sockets (ItemsTab.lua:193-216)
                                self.spec_select(ui, view, out);
                                spec_drawn = true;
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
                if !spec_drawn {
                    self.spec_select(ui, view, out);
                }
            });
    }

    /// PoB's "Passive tree:" dropdown and "Manage..." under the slots (ItemsTab.lua:193-206)
    fn spec_select(&mut self, ui: &mut egui::Ui, view: &ItemsView, out: &mut ItemsOutput) {
        let Some(specs) = &view.specs else {
            return;
        };
        let default = ui.visuals().text_color();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Passive tree:");
            let current = specs.labels.get(specs.selected).map_or("", String::as_str);
            let combo = ui.add_enabled_ui(specs.enabled, |ui| {
                crate::widgets::dropdown::Dropdown::from_id_salt("items_spec")
                    .width(200.0)
                    .selected_text(pob_text(current, 14.0, default))
                    .show_ui(ui, |ui| {
                        for (i, label) in specs.labels.iter().enumerate() {
                            let entry = crate::widgets::dropdown::item(
                                ui,
                                i == specs.selected,
                                pob_text(label, 14.0, default),
                            );
                            self.rects.note(&entry, || format!("spec_{i}"));
                            if entry.clicked() && i != specs.selected {
                                out.commands.push(Command::SelectSpec { index: i });
                            }
                        }
                    })
            });
            self.rects
                .note(&combo.inner.response, || "spec_combo".into());
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            if specs.enabled
                && let Some(index) = crate::widgets::scroll_step::combo_step(
                    ui,
                    &combo.inner.response,
                    specs.labels.len(),
                    specs.selected,
                )
            {
                out.commands.push(Command::SelectSpec { index });
            }
            let manage = ui.button("Manage\u{2026}");
            self.rects.note(&manage, || "spec_manage".into());
            if manage.clicked() {
                out.manage_trees = true;
            }
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
        // egui's drop zone takes whatever payload is released on it, so the zone is typed by
        // what is being dragged: a list row or a database row (R23)
        let db_dragging = egui::DragAndDrop::has_payload_of_type::<DbDrag>(ui.ctx());
        let shared_dragging = egui::DragAndDrop::has_payload_of_type::<SharedDrag>(ui.ctx());
        let contents = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                let pad = if indent { 16.0 } else { 0.0 };
                ui.add_space(pad);
                // PoB draws "^7<label>:" (ItemSlotControl.lua:139)
                let label = ui.add_sized(
                    [110.0 - pad, 18.0],
                    egui::Label::new(format!("{}:", slot.label))
                        .truncate()
                        .sense(egui::Sense::click()),
                );
                self.rects
                    .note(&label, || format!("slot_label_{}", slot.name));
                // PoB's trade search for the slot's item (R10); an empty slot has none. The menu
                // is the slot name's: the dropdown's own popup would share its id.
                if slot.selected != 0 {
                    label.context_menu(|ui| {
                        crate::widgets::scroll_step::note_tab_popup(ui.ctx());
                        let buy = ui.button("Buy similar\u{2026}");
                        self.rects.note(&buy, || format!("slot_buy_{}", slot.name));
                        if buy.clicked() {
                            out.commands.push(Command::OpenPopup(PopupKind::BuySimilar {
                                source: BuySource::Slot(slot.name.clone()),
                            }));
                            ui.close();
                        }
                    });
                }
                // a flask's box sits between its label and its dropdown, with PoB's tooltip
                // (ItemSlotControl.lua:32-43)
                if let Some(active) = slot.flask_active {
                    let mut on = active;
                    let check = ui
                        .add_enabled(slot.selected != 0, egui::Checkbox::without_text(&mut on))
                        .on_hover_text(FLASK_TIP)
                        .on_disabled_hover_text(FLASK_TIP);
                    self.rects
                        .note(&check, || format!("slot_flask_{}", slot.name));
                    if check.changed() {
                        out.commands.push(Command::SetFlaskActive {
                            slot: slot.name.clone(),
                            on,
                        });
                    }
                }
                let current = if slot.selected == 0 {
                    pob_text("None", 14.0, default)
                } else {
                    item_label(view, slot.selected, 14.0, default)
                };
                let combo = ui.add_enabled_ui(slot.enabled, |ui| {
                    crate::widgets::dropdown::Dropdown::from_id_salt(("items_slot", &slot.name))
                        .width(200.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            let none =
                                crate::widgets::dropdown::item(ui, slot.selected == 0, "None");
                            self.rects
                                .note(&none, || format!("slot_{}_none", slot.name));
                            if none.clicked() && slot.selected != 0 {
                                out.commands.push(Command::EquipItem {
                                    slot: slot.name.clone(),
                                    id: None,
                                });
                            }
                            for &id in &slot.choices {
                                let entry = crate::widgets::dropdown::item(
                                    ui,
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
                                        theirs: false,
                                        shift: false,
                                        shared: None,
                                    },
                                );
                            }
                        })
                });
                self.rects.note(&combo.inner.response, || {
                    format!("slot_combo_{}", slot.name)
                });
                // the wheel and the arrows step it: None, then its items
                // (DropDownControl.lua:476-516)
                let entries: Vec<u32> = std::iter::once(0)
                    .chain(slot.choices.iter().copied())
                    .collect();
                let at = entries
                    .iter()
                    .position(|&e| e == slot.selected)
                    .unwrap_or(0);
                if let Some(i) = crate::widgets::scroll_step::combo_step(
                    ui,
                    &combo.inner.response,
                    entries.len(),
                    at,
                ) {
                    out.commands.push(Command::EquipItem {
                        slot: slot.name.clone(),
                        id: (entries[i] != 0).then_some(entries[i]),
                    });
                }

                // the tree's right click on this slot's socket: scroll to it and drop it down
                if self.focus_slot.as_deref() == Some(&slot.name) {
                    self.focus_slot = None;
                    let button = &combo.inner.response;
                    button.scroll_to_me(Some(egui::Align::Center));
                    button.request_focus();
                    egui::Popup::open_id(ui.ctx(), button.id.with("popup"));
                }
                // a jewel socket's slot shows where the socket is while it is dropped or
                // pointed at (ItemSlotControl.lua:142-153)
                if let Some(node) = slot.node {
                    let button = &combo.inner.response;
                    let dropped = egui::Popup::is_id_open(ui.ctx(), button.id.with("popup"));
                    let dragging = egui::DragAndDrop::has_any_payload(ui.ctx());
                    if dropped || (button.hovered() && (dragging || !ui.ctx().text_edit_focused()))
                    {
                        // a list dropped upward puts the viewer under the slot (ItemSlotControl
                        // .lua:146-150)
                        let up = dropped
                            && ui
                                .ctx()
                                .memory(|m| m.area_rect(button.id.with("popup")))
                                .is_some_and(|r| r.center().y < button.rect.center().y);
                        self.socket_viewer(ui, button.rect, node, up);
                    }
                }
                // PoB's E reads the slot control under the mouse: its name or its dropdown
                if slot.selected != 0 && (label.hovered() || combo.inner.response.hovered()) {
                    self.hovered_slot = Some(slot.selected);
                }
                // PoB blanks a closed slot's tooltip while another control of the tab is
                // selected, a list excepted (ItemSlotControl.lua:47-52): a box with the
                // keyboard or another dropped list. A dropped list of Build's is Build's
                // selected control, not the tab's (ItemSlotControl.lua:51 reads only
                // itemsTab.selControl), and so is a menu of another host's
                let own_dropped =
                    egui::Popup::is_id_open(ui.ctx(), combo.inner.response.id.with("popup"));
                let tab_dropped = crate::widgets::scroll_step::tab_popup_open(ui.ctx());
                let blank = !own_dropped && (self.tab_box_selected || tab_dropped);
                if slot.selected != 0 && !blank {
                    self.note_hover(
                        &combo.inner.response,
                        ItemTooltipQuery {
                            id: slot.selected,
                            slot: Some(slot.name.clone()),
                            theirs: false,
                            shift: false,
                            shared: None,
                        },
                    );
                }
            });
        };
        let (zone, dropped, db_dropped, shared_dropped) = if db_dragging {
            let (zone, dropped) = ui.dnd_drop_zone::<DbDrag, _>(egui::Frame::NONE, contents);
            (zone, None, dropped, None)
        } else if shared_dragging {
            let (zone, dropped) = ui.dnd_drop_zone::<SharedDrag, _>(egui::Frame::NONE, contents);
            (zone, None, None, dropped)
        } else {
            let (zone, dropped) = ui.dnd_drop_zone::<ItemDrag, _>(egui::Frame::NONE, contents);
            (zone, dropped, None, None)
        };
        // a shared item dropped on the slot: PoB decides whether the slot takes it (NF-2)
        if let Some(drag) = shared_dropped {
            out.commands.push(Command::DropSharedItem {
                index: drag.index,
                slot: Some(slot.name.clone()),
                at: None,
            });
        }
        self.rects
            .note(&zone.response, || format!("slot_{}", slot.name));
        // a database row dropped on the slot: PoB decides whether the slot takes it (R23)
        if let Some(drag) = db_dropped {
            out.commands.push(Command::DropDbItem {
                db: drag.db,
                name: drag.name.clone(),
                slot: slot.name.clone(),
            });
        }
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

impl ItemsUi {
    /// PoB's 308 by 280 view of the socket's place in the tree, above the slot, or under it
    /// when its list drops upward or there is no room above (ItemSlotControl.lua:142-153,
    /// ItemSlotHelper.DrawViewer)
    fn socket_viewer(&mut self, ui: &egui::Ui, slot: egui::Rect, node: u32, below: bool) {
        let Some(scene) = &self.socket_scene else {
            return;
        };
        let size = egui::vec2(308.0, 280.0);
        let screen = ui.ctx().content_rect();
        let mut top = slot.top() - size.y - 4.0;
        if below || top < screen.top() {
            top = slot.bottom() + 4.0;
        }
        let rect = egui::Rect::from_min_size(egui::pos2(slot.left(), top), size);
        self.rects.note_rect(rect, "socket_viewer");
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            ui.scope_id().with("socket_viewer"),
        ));
        crate::ui::tree::canvas::paint_socket_viewer(
            &painter,
            rect,
            &scene.layout,
            &scene.allocated,
            node,
        );
    }
}

/// The Items tab's keys (R25, ItemsTab.lua:1454-1490).
impl ItemsUi {
    /// E over a slot holding an item: the item list's double click on it, which edits it
    /// (ItemsTab.lua:1459-1464)
    pub fn edit_hovered_slot(&self) -> Option<Command> {
        self.hovered_slot.map(|id| Command::EditItem { id })
    }

    /// The wiki page of the item under the pointer: a unique's or relic's name, else its
    /// base (ItemTools.lua:395-399)
    pub fn hovered_wiki(&self) -> Option<String> {
        if let Some(name) = &self.db.hovered_wiki {
            return Some(name.clone());
        }
        let (query, _) = self.hover.as_ref()?;
        let item = self
            .view
            .as_ref()?
            .items
            .iter()
            .find(|i| i.id == query.id)?;
        Some(if matches!(item.rarity.as_str(), "UNIQUE" | "RELIC") {
            item.name.clone()
        } else {
            item.base.clone()
        })
    }

    /// Ctrl+F: the item database's search box (ItemsTab.lua:1471-1480); returns the query to
    /// resend if the database changed
    pub fn focus_db_search(&mut self, ctx: &egui::Context) -> Option<Command> {
        self.db.focus_search(ctx)
    }
}
