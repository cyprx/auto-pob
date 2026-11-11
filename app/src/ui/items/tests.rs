use super::list::delete_prompt;
use super::slots::{SlotRow, SlotSection, rows};
use super::*;
use crate::engine::PopupKind;
use crate::engine::views::ItemDbView;
use crate::engine::views::{
    DisplayItemView, DropList, EquippedIn, ItemEntry, ItemSetEntry, ItemText, ItemTooltip,
    ItemTooltipLine, ModLineList, ModLineRef, RangeLineView, SlotKind, SlotView,
};
use crate::engine::views::{SocketView, VariantView};
use crate::engine::{ItemDb, ItemDbQuery};
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

fn item(id: u32, name: &str) -> ItemEntry {
    ItemEntry {
        id,
        name: name.into(),
        base: "Base".into(),
        rarity: "RARE".into(),
        colour: "^xFFFF77".into(),
        display_label: format!("^xFFFF77{name} ^x7F7F7F(Unused)"),
        ..ItemEntry::default()
    }
}

fn slot(name: &str, kind: SlotKind, choices: &[u32]) -> SlotView {
    SlotView {
        name: name.into(),
        label: name.into(),
        kind,
        shown: true,
        enabled: !choices.is_empty(),
        choices: choices.to_vec(),
        ..SlotView::default()
    }
}

/// Items: 1 unused ring, 2 equipped body armour, 3 an abyss jewel used in another set, 4 a
/// jewel socketed in another tree.
pub(crate) fn sample() -> ItemsView {
    let mut body = item(2, "Kaom's Heart");
    body.equipped = Some(EquippedIn {
        slot: "Body Armour".into(),
        label: "Body Armour".into(),
        set: None,
    });
    let mut abyss = item(3, "Lightless Gaze");
    abyss.abyss_set = Some("Alt".into());
    let mut tree = item(4, "Prismatic Jewel");
    tree.tree = Some("Level 90".into());
    let mut body_slot = slot("Body Armour", SlotKind::Gear, &[2]);
    body_slot.selected = 2;
    let mut flask = slot("Flask 1", SlotKind::Flask, &[]);
    flask.flask_active = Some(false);
    let mut hidden = slot("Jewel 7", SlotKind::Jewel, &[4]);
    hidden.shown = false;
    ItemsView {
        items: vec![item(1, "Doom Loop"), body, abyss, tree],
        list_filter: DropList {
            labels: vec!["Any".into(), "Current".into()],
            selected: 0,
            enabled: true,
        },
        listed: vec![1, 2, 3, 4],
        can_reorder: true,
        slots: vec![
            slot("Weapon 1", SlotKind::Gear, &[]),
            slot("Helmet", SlotKind::Gear, &[]),
            body_slot,
            slot("Ring 1", SlotKind::Gear, &[1]),
            slot("Ring 2", SlotKind::Gear, &[1]),
            slot("Belt", SlotKind::Gear, &[]),
            SlotView {
                parent: Some("Belt".into()),
                ..slot("Belt Abyssal Socket 1", SlotKind::Abyssal, &[3])
            },
            flask,
            hidden,
            slot("Jewel 5", SlotKind::Jewel, &[4]),
        ],
        sets: vec![
            ItemSetEntry {
                id: 1,
                title: "Default".into(),
            },
            ItemSetEntry {
                id: 2,
                title: "Alt".into(),
            },
        ],
        active_set: 1,
        ..ItemsView::default()
    }
}

struct Harness {
    ui: ItemsUi,
    ctx: egui::Context,
    time: f64,
    step: f64,
    copied: Vec<String>,
    /// A text field above the tab, with focus
    field: Option<String>,
    /// The engine is behind
    busy: bool,
    /// The keys held down
    modifiers: Modifiers,
}

fn debug(commands: &[Command]) -> Vec<String> {
    commands.iter().map(|c| format!("{c:?}")).collect()
}

impl Harness {
    fn new(view: ItemsView) -> Self {
        let mut ui = ItemsUi::default();
        ui.on_snapshot(&Snapshot::Items(Arc::new(view)));
        let mut h = Self {
            ui,
            ctx: egui::Context::default(),
            time: 1.0,
            step: 0.2,
            copied: Vec::new(),
            field: None,
            busy: false,
            modifiers: Modifiers::NONE,
        };
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, mut events: Vec<Event>) -> ItemsOutput {
        self.time += self.step;
        events.insert(0, Event::ModifiersChanged(self.modifiers));
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut out = ItemsOutput::default();
        let time = self.time;
        let busy = self.busy;
        let output = self.ctx.run_ui(input, |ui| {
            if let Some(text) = &mut self.field {
                let field = ui.add(egui::TextEdit::singleline(text).id_salt("field"));
                field.request_focus();
            }
            // egui may run a frame twice; each pass's output counts
            let pass = self.ui.show(ui, time, busy);
            out.commands.extend(pass.commands);
            out.queries.extend(pass.queries);
        });
        self.copied.extend(
            output
                .platform_output
                .commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t.clone()),
                    _ => None,
                }),
        );
        output.drop_without_applying_deltas();
        out
    }

    fn click(&mut self, key: &str) -> ItemsOutput {
        let pos = self.ui.rect(key).center();
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut all = ItemsOutput::default();
        for events in [
            vec![Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            let o = self.frame(events);
            all.commands.extend(o.commands);
            all.queries.extend(o.queries);
        }
        all
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> ItemsOutput {
        self.frame(vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }])
    }

    /// Presses at `from`, drags to `to` and releases; every command sent on the way.
    fn drag_between(&mut self, from: Pos2, to: Pos2) -> Vec<Command> {
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mid = Pos2::new(from.x + 30.0, from.y + 6.0);
        let mut all = Vec::new();
        for events in [
            vec![Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![Event::PointerMoved(mid)],
            vec![Event::PointerMoved(to)],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
            vec![],
        ] {
            all.extend(self.frame(events).commands);
        }
        all
    }

    fn double_click(&mut self, key: &str) -> Vec<Command> {
        let pos = self.ui.rect(key).center();
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.step = 0.05;
        let mut all = Vec::new();
        for events in [
            vec![Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            all.extend(self.frame(events).commands);
        }
        self.step = 0.2;
        all
    }

    /// The engine's next view has the items in this order (ids), as after a move or a sort.
    fn reorder(&mut self, order: &[u32]) {
        let mut view = (*self.view()).clone();
        view.items = order
            .iter()
            .map(|id| view.items.iter().find(|e| e.id == *id).unwrap().clone())
            .collect();
        view.listed = order.to_vec();
        self.ui.on_snapshot(&Snapshot::Items(Arc::new(view)));
        self.frame(vec![]);
    }

    fn view(&self) -> Arc<ItemsView> {
        self.ui.view.clone().unwrap()
    }
}

fn names(view: &ItemsView, rows: &[SlotRow]) -> Vec<(String, SlotSection, bool)> {
    rows.iter()
        .map(|r| (view.slots[r.slot].name.clone(), r.section, r.indent))
        .collect()
}

#[test]
fn slot_rows_group_abyssal_under_parent_and_jewels_last() {
    let mut view = sample();
    // the abyssal socket sits after the flask and a jewel in PoB's order
    let abyssal = view.slots.remove(6);
    view.slots.push(abyssal);
    let got = names(&view, &rows(&view));
    let eq = SlotSection::Equipment;
    let expected = vec![
        ("Weapon 1".to_owned(), eq, false),
        ("Helmet".to_owned(), eq, false),
        ("Body Armour".to_owned(), eq, false),
        ("Ring 1".to_owned(), eq, false),
        ("Ring 2".to_owned(), eq, false),
        ("Belt".to_owned(), eq, false),
        ("Belt Abyssal Socket 1".to_owned(), eq, true),
        ("Flask 1".to_owned(), SlotSection::Flasks, false),
        ("Jewel 5".to_owned(), SlotSection::Jewels, false),
    ];
    assert_eq!(got, expected, "hidden Jewel 7 is dropped");
}

#[test]
fn drag_to_slot_sends_equip_item() {
    let mut h = Harness::new(sample());
    let drag = |h: &mut Harness, to: &str| {
        let from = h.ui.rect("row_1").center();
        let to = h.ui.rect(to).center();
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mid = Pos2::new(from.x + 30.0, from.y + 6.0);
        let mut all = Vec::new();
        for events in [
            vec![Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![Event::PointerMoved(mid)],
            vec![Event::PointerMoved(to)],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
            vec![],
        ] {
            all.extend(h.frame(events).commands);
        }
        debug(&all)
    };
    assert_eq!(
        drag(&mut h, "slot_Ring 2"),
        ["EquipItem { slot: \"Ring 2\", id: Some(1) }"]
    );
    assert!(
        drag(&mut h, "slot_Helmet").is_empty(),
        "the Helmet dropdown doesn't offer the ring"
    );
}

#[test]
fn choosing_from_a_slot_dropdown_equips() {
    let mut h = Harness::new(sample());
    h.click("slot_combo_Ring 1");
    let o = h.click("slot_Ring 1_item_1");
    assert_eq!(
        debug(&o.commands),
        ["EquipItem { slot: \"Ring 1\", id: Some(1) }"]
    );
    h.click("slot_combo_Body Armour");
    let o = h.click("slot_Body Armour_none");
    assert_eq!(
        debug(&o.commands),
        ["EquipItem { slot: \"Body Armour\", id: None }"]
    );
}

#[test]
fn flask_box_is_disabled_while_the_slot_is_empty() {
    let mut view = sample();
    let o = Harness::new(view.clone()).click("slot_flask_Flask 1");
    assert!(o.commands.is_empty());
    view.slots[7].selected = 4;
    view.slots[7].choices = vec![4];
    let o = Harness::new(view).click("slot_flask_Flask 1");
    assert_eq!(
        debug(&o.commands),
        ["SetFlaskActive { slot: \"Flask 1\", on: true }"]
    );
}

#[test]
fn ctrl_v_outside_text_sends_paste_item() {
    let mut h = Harness::new(sample());
    let o = h.frame(vec![Event::Paste("Rarity: Rare".into())]);
    assert_eq!(debug(&o.commands), ["PasteItem { text: \"Rarity: Rare\" }"]);
}

#[test]
fn ctrl_v_in_a_text_field_does_not() {
    let mut h = Harness::new(sample());
    h.field = Some(String::new());
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.ctx.text_edit_focused(), "the field has focus");
    let o = h.frame(vec![Event::Paste("Rarity: Rare".into())]);
    assert!(o.commands.is_empty(), "the field takes the paste");
}

#[test]
fn delete_equipped_item_asks_first() {
    let mut h = Harness::new(sample());
    h.click("row_2");
    let o = h.key(Key::Delete, Modifiers::NONE);
    h.frame(vec![]);
    assert!(o.commands.is_empty(), "the first press sends nothing");
    assert!(
        h.ui.rects.map.contains_key("confirm_delete"),
        "a confirm opened"
    );
    let o = h.click("confirm_delete");
    assert_eq!(
        debug(&o.commands),
        ["DeleteItem { id: 2, confirmed: true }"]
    );

    // an abyss jewel used in another set asks too
    let mut h = Harness::new(sample());
    h.click("row_3");
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert!(o.commands.is_empty());

    // an unused item goes at once
    let mut h = Harness::new(sample());
    h.click("row_1");
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(
        debug(&o.commands),
        ["DeleteItem { id: 1, confirmed: false }"]
    );
}

#[test]
fn delete_prompts_are_pobs_three_texts() {
    let view = sample();
    let by = |id| view.items.iter().find(|e| e.id == id).unwrap();
    assert_eq!(delete_prompt(by(1)), None);
    assert_eq!(
        delete_prompt(by(2)).unwrap(),
        "Kaom's Heart is currently equipped in Body Armour.\nAre you sure you want to delete it?"
    );
    assert_eq!(
        delete_prompt(by(3)).unwrap(),
        "Lightless Gaze is currently equipped in an Abyssal Socket in set 'Alt'.\nAre you sure you want to delete it?"
    );
    assert_eq!(
        delete_prompt(by(4)).unwrap(),
        "Prismatic Jewel is currently equipped in passive tree 'Level 90'.\nAre you sure you want to delete it?"
    );
    let mut other = by(2).clone();
    other.equipped.as_mut().unwrap().set = Some("Alt".into());
    assert!(
        delete_prompt(&other)
            .unwrap()
            .contains("in Body Armour in set 'Alt'.")
    );
}

#[test]
fn delete_unused_does_not_ask_and_delete_all_does() {
    let mut h = Harness::new(sample());
    h.click("menu");
    let o = h.click("menu_delete_unused");
    assert_eq!(debug(&o.commands), ["DeleteUnusedItems"]);
    assert!(!h.ui.has_dialog());

    h.click("menu");
    let o = h.click("menu_delete_all");
    assert!(o.commands.is_empty(), "Delete all asks first");
    assert!(h.ui.has_dialog());
    let o = h.click("confirm_delete_all");
    assert_eq!(debug(&o.commands), ["DeleteAllItems"]);
    assert!(!h.ui.has_dialog());
}

#[test]
fn copy_reply_reaches_clipboard_only_when_asked() {
    let mut h = Harness::new(sample());
    let reply = |id, text: &str| {
        Snapshot::ItemText(Arc::new(ItemText {
            id,
            text: text.into(),
        }))
    };
    h.ui.on_snapshot(&reply(1, "unasked"));
    h.frame(vec![]);
    assert!(h.copied.is_empty(), "nothing was asked for");
    h.click("row_1");
    let o = h.frame(vec![Event::Copy]);
    assert!(debug(&o.queries).contains(&"CopyItem { id: 1 }".to_owned()));
    h.ui.on_snapshot(&reply(2, "other item"));
    h.frame(vec![]);
    assert!(h.copied.is_empty(), "a reply for another item is ignored");
    h.ui.on_snapshot(&reply(1, "Rarity: Rare\r\n"));
    h.frame(vec![]);
    assert_eq!(h.copied, ["Rarity: Rare\r\n"]);
    h.ui.on_snapshot(&reply(1, "again"));
    h.frame(vec![]);
    assert_eq!(h.copied.len(), 1, "one copy per ask");
}

fn tip(id: u32, text: &str) -> Snapshot {
    Snapshot::ItemTooltip(Arc::new(ItemTooltipView {
        query: ItemTooltipQuery { id, slot: None },
        tooltip: ItemTooltip {
            lines: vec![ItemTooltipLine::Line {
                size: 14,
                text: text.into(),
                font: None,
                center: false,
                background: None,
                mod_line: None,
            }],
            ..ItemTooltip::default()
        },
    }))
}

#[test]
fn hovering_a_row_asks_for_its_tooltip_after_the_delay() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("row_1").center();
    let o = h.frame(vec![Event::PointerMoved(pos)]);
    assert!(o.queries.is_empty(), "too early");
    let o = h.frame(vec![]);
    assert_eq!(
        debug(&o.queries),
        ["ItemTooltip(ItemTooltipQuery { id: 1, slot: None })"]
    );
    let o = h.frame(vec![]);
    assert!(o.queries.is_empty(), "asked once");
}

#[test]
fn hovering_a_dropdown_entry_asks_with_the_slot() {
    let mut h = Harness::new(sample());
    h.click("slot_combo_Ring 1");
    let pos = h.ui.rect("slot_Ring 1_item_1").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let o = h.frame(vec![]);
    let o2 = h.frame(vec![]);
    let all: Vec<_> = o.queries.iter().chain(&o2.queries).collect();
    assert_eq!(
        all.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(),
        ["ItemTooltip(ItemTooltipQuery { id: 1, slot: Some(\"Ring 1\") })"]
    );
}

#[test]
fn stale_item_tooltip_reply_ignored() {
    let mut h = Harness::new(sample());
    let first = h.ui.rect("row_1").center();
    h.frame(vec![Event::PointerMoved(first)]);
    let o = h.frame(vec![]);
    assert_eq!(o.queries.len(), 1);
    // the pointer moves on and the second question goes out
    let second = h.ui.rect("row_2").center();
    h.frame(vec![Event::PointerMoved(second)]);
    let o = h.frame(vec![]);
    assert_eq!(
        debug(&o.queries),
        ["ItemTooltip(ItemTooltipQuery { id: 2, slot: None })"]
    );
    h.ui.on_snapshot(&tip(1, "old"));
    assert!(
        h.ui.tooltip.is_none(),
        "the reply to the first question is stale"
    );
    h.ui.on_snapshot(&tip(2, "new"));
    assert_eq!(h.ui.tooltip.as_ref().unwrap().query.id, 2);
}

#[test]
fn last_item_set_cannot_be_deleted() {
    let mut v = sample();
    v.sets.truncate(1);
    let mut h = Harness::new(v);
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_0");
    let o = h.click("sets_delete");
    assert!(o.commands.is_empty());
    assert!(
        !h.ui.rects.map.contains_key("sets_confirm_delete"),
        "no confirm for the last set"
    );
}

#[test]
fn deleting_a_set_asks_first() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    let o = h.click("sets_delete");
    assert!(o.commands.is_empty());
    let o = h.click("sets_confirm_delete");
    assert_eq!(debug(&o.commands), ["DeleteItemSet { id: 2 }"]);
}

#[test]
fn copying_a_set_prefills_its_title() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    h.click("sets_copy");
    let o = h.click("sets_save");
    assert_eq!(
        debug(&o.commands),
        ["CopyItemSet { id: 2, title: \"Alt\" }"]
    );
}

#[test]
fn set_bar_selects_sets_and_weapon_sets() {
    let mut view = sample();
    view.sets.truncate(1);
    let mut h = Harness::new(view);
    let o = h.click("weapons_I");
    assert!(o.commands.is_empty(), "the current set's button is locked");
    let o = h.click("weapons_II");
    assert_eq!(debug(&o.commands), ["SelectWeaponSet { second: true }"]);
    h.click("set");
    assert!(
        !h.ui.rects.map.contains_key("set_item_0"),
        "the dropdown is disabled with one set"
    );
}

#[test]
fn ctrl_d_toggles_stat_differences() {
    let mut view = sample();
    let mut h = Harness::new(view.clone());
    let o = h.key(Key::D, Modifiers::COMMAND);
    assert_eq!(debug(&o.commands), ["SetShowStatDifferences { on: true }"]);
    view.show_stat_differences = true;
    h.ui.on_snapshot(&Snapshot::Items(Arc::new(view)));
    let o = h.key(Key::D, Modifiers::COMMAND);
    assert_eq!(debug(&o.commands), ["SetShowStatDifferences { on: false }"]);
    assert!(h.view().show_stat_differences);
}

#[test]
fn deleting_the_displayed_item_cancels_the_display() {
    let mut h = Harness::new(sample());
    let displayed = |id| {
        Snapshot::DisplayItem(Some(Arc::new(DisplayItemView {
            id,
            ..DisplayItemView::default()
        })))
    };
    h.ui.on_snapshot(&displayed(Some(2)));
    let o = h.frame(vec![]);
    assert!(o.commands.is_empty(), "the item is still in the list");
    // Delete: the items arrive without it, then the unchanged display item
    let mut gone = sample();
    gone.items.retain(|e| e.id != 2);
    gone.listed.retain(|id| *id != 2);
    h.ui.on_snapshot(&Snapshot::Items(Arc::new(gone)));
    h.ui.on_snapshot(&displayed(Some(2)));
    let o = h.frame(vec![]);
    assert_eq!(debug(&o.commands), ["CancelDisplayItem { if_id: Some(2) }"]);
    let o = h.frame(vec![]);
    assert!(o.commands.is_empty(), "cancelled once");
    h.ui.on_snapshot(&displayed(Some(2)));
    let o = h.frame(vec![]);
    assert!(
        o.commands.is_empty(),
        "a late copy of the same snapshot cancels nothing"
    );
    // a new paste has no id, and an item still in the build is left alone
    h.ui.on_snapshot(&displayed(None));
    h.ui.on_snapshot(&displayed(Some(1)));
    assert!(h.frame(vec![]).commands.is_empty());
}

#[test]
fn leaving_the_tab_closes_dialogs() {
    let mut h = Harness::new(sample());
    h.click("manage");
    assert!(h.ui.has_dialog());
    h.ui.close_dialogs();
    assert!(!h.ui.has_dialog());
}

#[test]
fn double_clicking_a_row_edits_the_item() {
    let mut h = Harness::new(sample());
    let sent = h.double_click("row_2");
    assert!(
        debug(&sent).contains(&"EditItem { id: 2 }".to_owned()),
        "{sent:?}"
    );
}

#[test]
fn double_clicking_a_set_selects_it() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    let sent = h.double_click("sets_row_1");
    assert_eq!(debug(&sent), ["SelectItemSet { id: 2 }"]);
}

#[test]
fn dragging_a_row_reorders_the_list() {
    let mut h = Harness::new(sample());
    let from = h.ui.rect("row_1").center();
    let r = h.ui.rect("row_3");
    let below = Pos2::new(r.center().x, r.bottom() - 2.0);
    assert_eq!(
        debug(&h.drag_between(from, below)),
        ["MoveItem { from: 0, to: 2 }"]
    );
}

#[test]
fn a_filtered_list_does_not_reorder() {
    let mut view = sample();
    view.can_reorder = false;
    let mut h = Harness::new(view);
    let from = h.ui.rect("row_1").center();
    let r = h.ui.rect("row_3");
    let below = Pos2::new(r.center().x, r.bottom() - 2.0);
    assert!(h.drag_between(from, below).is_empty());
}

#[test]
fn dragging_a_set_reorders_the_sets() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    let from = h.ui.rect("sets_row_0").center();
    let r = h.ui.rect("sets_row_1");
    let below = Pos2::new(r.center().x, r.bottom() - 2.0);
    assert_eq!(
        debug(&h.drag_between(from, below)),
        ["MoveItemSet { from: 0, to: 1 }"]
    );
}

#[test]
fn keys_act_on_the_selected_item_after_a_reorder() {
    let mut h = Harness::new(sample());
    h.click("row_2");
    let from = h.ui.rect("row_1").center();
    let r = h.ui.rect("row_3");
    let sent = h.drag_between(from, Pos2::new(r.center().x, r.bottom() - 2.0));
    assert_eq!(debug(&sent), ["MoveItem { from: 0, to: 2 }"]);
    // the engine's new order puts a different item under the focused row
    h.reorder(&[2, 3, 1, 4]);
    let o = h.frame(vec![Event::Copy]);
    assert!(
        debug(&o.queries).contains(&"CopyItem { id: 1 }".to_owned()),
        "{:?}",
        o.queries
    );
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(
        debug(&o.commands),
        ["DeleteItem { id: 1, confirmed: false }"]
    );
}

#[test]
fn a_second_delete_with_nothing_selected_does_nothing() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(
        debug(&o.commands),
        ["DeleteItem { id: 1, confirmed: false }"]
    );
    // the engine's view no longer has it; the equipped item slid under the focused row
    let mut view = (*h.view()).clone();
    view.items.retain(|e| e.id != 1);
    view.listed.retain(|id| *id != 1);
    h.ui.on_snapshot(&Snapshot::Items(Arc::new(view)));
    h.frame(vec![]);
    let o = h.key(Key::Delete, Modifiers::NONE);
    h.frame(vec![]);
    assert!(o.commands.is_empty());
    assert!(!h.ui.has_dialog(), "no prompt for an item nobody selected");
}

#[test]
fn keys_act_on_the_selected_item_after_a_sort() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    h.reorder(&[4, 3, 2, 1]);
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(
        debug(&o.commands),
        ["DeleteItem { id: 1, confirmed: false }"]
    );
}

#[test]
fn the_sort_button_sends_sort_items() {
    let mut h = Harness::new(sample());
    let o = h.click("sort");
    assert_eq!(debug(&o.commands), ["SortItems"]);
}

#[test]
fn flush_sends_a_pending_cancel_once() {
    let mut h = Harness::new(sample());
    h.ui.on_snapshot(&Snapshot::DisplayItem(Some(Arc::new(DisplayItemView {
        id: Some(2),
        ..DisplayItemView::default()
    }))));
    let mut gone = sample();
    gone.items.retain(|e| e.id != 2);
    h.ui.on_snapshot(&Snapshot::Items(Arc::new(gone)));
    assert_eq!(
        debug(&h.ui.flush()),
        ["CancelDisplayItem { if_id: Some(2) }"]
    );
    assert!(h.ui.flush().is_empty());
    assert!(h.frame(vec![]).commands.is_empty(), "not sent twice");
}

// ---- the display panel ------------------------------------------------------------------

fn tip_line(text: &str, mod_line: Option<ModLineRef>) -> ItemTooltipLine {
    ItemTooltipLine::Line {
        size: 14,
        text: text.into(),
        font: None,
        center: false,
        background: None,
        mod_line,
    }
}

fn display_view() -> DisplayItemView {
    DisplayItemView {
        id: Some(2),
        name: "Kaom's Heart".into(),
        save_label: "Save".into(),
        edit_rarity: 3,
        edit_text: "Kaom's Heart".into(),
        quality: Some("20".into()),
        range_lines: vec![
            RangeLineView {
                label: "+10 to Life".into(),
                range: Some(0.5),
                slider: true,
                ..RangeLineView::default()
            },
            RangeLineView {
                label: "+20 to Mana".into(),
                range: Some(0.25),
                slider: true,
                mutable: true,
                ..RangeLineView::default()
            },
        ],
        tooltip: ItemTooltip {
            lines: vec![
                tip_line(
                    "Clickable",
                    Some(ModLineRef {
                        list: ModLineList::Explicit,
                        index: 1,
                    }),
                ),
                tip_line("Plain", None),
            ],
            ..ItemTooltip::default()
        },
        ..DisplayItemView::default()
    }
}

impl Harness {
    fn show_display(&mut self, view: DisplayItemView) {
        self.ui
            .on_snapshot(&Snapshot::DisplayItem(Some(Arc::new(view))));
        self.frame(vec![]);
    }

    fn drect(&self, key: &str) -> egui::Rect {
        self.ui.rect(key)
    }

    fn button(&mut self, pos: Pos2, pressed: bool) -> ItemsOutput {
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }])
    }

    /// Focuses the box and types into it.
    fn type_into(&mut self, key: &str, text: &str) -> Vec<Command> {
        let pos = self.drect(key).center();
        let mut all = Vec::new();
        for events in [
            vec![Event::PointerMoved(pos)],
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            vec![Event::Key {
                key: Key::A,
                physical_key: Some(Key::A),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }],
            vec![Event::Text(text.into())],
            vec![],
        ] {
            all.extend(self.frame(events).commands);
        }
        all
    }
}

#[test]
fn clicking_a_mod_line_sends_toggle_mod_line() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    let o = h.click("tip_line_0");
    assert_eq!(
        debug(&o.commands),
        ["ToggleModLine { line: ModLineRef { list: Explicit, index: 1 } }"]
    );
    let o = h.click("tip_line_1");
    assert!(o.commands.is_empty(), "{:?}", o.commands);
}

#[test]
fn hidden_editors_are_not_drawn() {
    let mut h = Harness::new(sample());
    let mut view = display_view();
    view.quality = None;
    view.range_lines.clear();
    h.show_display(view);
    for key in [
        "quality",
        "catalyst",
        "catalyst_quality",
        "range_lines",
        "range_slider",
        "add_socket",
        "version",
    ] {
        assert!(!h.ui.display.rects.map.contains_key(key), "{key} was drawn");
    }
    assert!(h.ui.display.rects.map.contains_key("save"));
    h.show_display(display_view());
    assert!(h.ui.display.rects.map.contains_key("quality"));
}

#[test]
fn save_flushes_a_focused_quality_box_first() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    let sent = h.type_into("quality", "15");
    assert!(sent.is_empty(), "still typing: {sent:?}");
    // pressing Save: the box is still focused when the click is handled
    let pos = h.drect("save").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let mut commands = h.button(pos, true).commands;
    commands.extend(h.button(pos, false).commands);
    commands.extend(h.frame(vec![]).commands);
    assert_eq!(
        debug(&commands),
        ["SetItemQuality { text: \"15\" }", "SaveDisplayItem"]
    );
}

#[test]
fn flush_sends_a_focused_quality_box_once() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "15");
    assert_eq!(debug(&h.ui.flush()), ["SetItemQuality { text: \"15\" }"]);
    assert!(h.ui.flush().is_empty());
}

#[test]
fn enter_sends_the_quality_and_escape_reverts() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "15");
    let o = h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(debug(&o.commands), ["SetItemQuality { text: \"15\" }"]);
    h.type_into("quality", "99");
    let o = h.key(Key::Escape, Modifiers::NONE);
    assert!(o.commands.is_empty());
    assert!(h.frame(vec![]).commands.is_empty());
    assert!(h.ui.flush().is_empty());
    assert_eq!(h.ui.display.quality.text, "20");
}

#[test]
fn text_window_sends_set_display_item_text_with_rarity() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.click("edit");
    let w = h.ui.display.text_window.clone().expect("window open");
    assert_eq!(
        (w.rarity, w.text.as_str(), w.creating),
        (3, "Kaom's Heart", false)
    );
    h.click("text_rarity");
    h.click("text_rarity_1");
    h.type_into("text_edit", "Typed text");
    let o = h.click("text_save");
    assert_eq!(
        debug(&o.commands),
        ["SetDisplayItemText { rarity: 1, text: \"Typed text\" }"]
    );
    assert!(h.ui.display.text_window.is_some(), "kept until the view");
}

#[test]
fn text_window_stays_after_an_error_and_closes_on_the_next_view() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.click("edit");
    h.click("text_save");
    h.ui.text_refused();
    h.ui.on_snapshot(&Snapshot::DisplayItem(Some(Arc::new(display_view()))));
    assert!(
        h.ui.display.text_window.is_some(),
        "an unrelated view keeps it"
    );
    h.click("text_save");
    h.ui.on_snapshot(&Snapshot::DisplayItem(Some(Arc::new(display_view()))));
    assert!(h.ui.display.text_window.is_none());
}

#[test]
fn text_window_closes_when_the_item_changes() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.click("edit");
    let mut other = display_view();
    other.id = Some(3);
    h.show_display(other);
    assert!(h.ui.display.text_window.is_none());
}

#[test]
fn create_custom_opens_a_rare_empty_window() {
    let mut h = Harness::new(sample());
    h.frame(vec![]);
    h.click("create");
    let w = h.ui.display.text_window.clone().expect("window open");
    assert_eq!((w.rarity, w.text.as_str(), w.creating), (2, "", true));
}

#[test]
fn slider_sends_on_release_only() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    let r = h.drect("range_slider");
    let from = r.center();
    let to = Pos2::new(r.left() + 5.0, from.y);
    let mut during = Vec::new();
    during.extend(h.frame(vec![Event::PointerMoved(from)]).commands);
    during.extend(h.button(from, true).commands);
    during.extend(
        h.frame(vec![Event::PointerMoved(Pos2::new(from.x - 10.0, from.y))])
            .commands,
    );
    during.extend(h.frame(vec![Event::PointerMoved(to)]).commands);
    assert!(during.is_empty(), "nothing while dragging: {during:?}");
    let released = h.button(to, false).commands;
    assert_eq!(released.len(), 1, "{released:?}");
    assert!(
        matches!(released[0], Command::SetRangeLine { line: 0, value } if value < 0.2),
        "{released:?}"
    );
    assert!(h.frame(vec![]).commands.is_empty());
}

#[test]
fn the_range_combo_picks_the_line_for_the_slider() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    assert!(!h.ui.display.rects.map.contains_key("range_mutated"));
    h.click("range_lines");
    h.click("range_line_1");
    h.frame(vec![]);
    let o = h.click("range_mutated");
    assert_eq!(
        debug(&o.commands),
        ["SetRangeLineMutated { line: 1, on: true }"]
    );
}

#[test]
fn focused_quality_box_survives_incoming_view() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "15");
    let mut view = display_view();
    view.quality = Some("20".into());
    view.name = "Kaom's Heart".into();
    h.show_display(view);
    assert_eq!(h.ui.display.quality.text, "15");
    h.key(Key::Enter, Modifiers::NONE);
    let mut view = display_view();
    view.quality = Some("15".into());
    h.show_display(view);
    assert_eq!(h.ui.display.quality.text, "15");
}

fn droplist(labels: &[&str], selected: usize, enabled: bool) -> DropList {
    DropList {
        labels: labels.iter().map(|l| (*l).into()).collect(),
        selected,
        enabled,
    }
}

/// Every editor present; socket 1 is hidden, so the original index of the last two differs
/// from their position among the shown ones.
fn rich_view() -> DisplayItemView {
    let socket = |colour: &str, shown, linked| SocketView {
        colour: colour.into(),
        shown,
        linked,
    };
    DisplayItemView {
        version: Some(droplist(&["Old", "New", "Newer"], 0, true)),
        variants: vec![
            VariantView {
                control: 3,
                list: droplist(&["A", "B", "C"], 0, true),
            },
            VariantView {
                control: 4,
                list: droplist(&["X", "Y"], 0, false),
            },
        ],
        sockets: vec![
            socket("R", true, Some(true)),
            socket("G", false, Some(false)),
            socket("G", true, Some(false)),
            socket("B", true, None),
        ],
        can_add_socket: true,
        influences: vec![
            droplist(&["None", "Shaper", "Elder"], 0, true),
            droplist(&["None", "Crusader", "Hunter"], 0, true),
        ],
        catalyst: Some(droplist(&["None", "Abrasive", "Fertile"], 0, true)),
        catalyst_quality: Some("10".into()),
        ..display_view()
    }
}

fn rich() -> Harness {
    let mut h = Harness::new(sample());
    h.show_display(rich_view());
    h
}

fn pick(h: &mut Harness, combo: &str, item: &str) -> Vec<String> {
    h.click(combo);
    debug(&h.click(item).commands)
}

#[test]
fn hidden_editors_are_not_drawn_and_present_ones_are() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    for key in [
        "version",
        "variant_3",
        "socket_0",
        "link_0",
        "add_socket",
        "influence_0",
        "catalyst",
        "catalyst_quality",
    ] {
        assert!(!h.ui.display.rects.map.contains_key(key), "{key} drawn");
    }
    h.show_display(rich_view());
    for key in [
        "version",
        "variant_3",
        "variant_4",
        "socket_0",
        "socket_2",
        "socket_3",
        "link_0",
        "link_2",
        "add_socket",
        "influence_0",
        "influence_1",
        "catalyst",
        "catalyst_quality",
    ] {
        assert!(h.ui.display.rects.map.contains_key(key), "{key} missing");
    }
    assert!(!h.ui.display.rects.map.contains_key("socket_1"));
    assert!(!h.ui.display.rects.map.contains_key("link_1"));
}

#[test]
fn version_and_variant_dropdowns_send_their_indices() {
    let mut h = rich();
    assert_eq!(
        pick(&mut h, "version", "version_2"),
        ["SetItemVersion { index: 2 }"]
    );
    assert_eq!(
        pick(&mut h, "variant_3", "variant_3_1"),
        ["SetItemVariant { control: 3, index: 1 }"]
    );
}

#[test]
fn a_disabled_dropdown_sends_nothing_and_does_not_open() {
    let mut h = rich();
    h.click("variant_4");
    assert!(!h.ui.display.rects.map.contains_key("variant_4_1"));
    let o = h.click("variant_4");
    assert!(o.commands.is_empty());
}

#[test]
fn sockets_use_the_original_index_not_the_shown_position() {
    let mut h = rich();
    assert_eq!(
        pick(&mut h, "socket_2", "socket_2_B"),
        ["SetItemSocket { index: 2, colour: B }"]
    );
    assert_eq!(
        pick(&mut h, "socket_3", "socket_3_W"),
        ["SetItemSocket { index: 3, colour: W }"]
    );
    assert_eq!(
        debug(&h.click("link_2").commands),
        ["SetItemSocketLink { index: 2, linked: true }"]
    );
    assert_eq!(
        debug(&h.click("link_0").commands),
        ["SetItemSocketLink { index: 0, linked: false }"]
    );
    assert_eq!(debug(&h.click("add_socket").commands), ["AddItemSocket"]);
}

#[test]
fn influence_and_catalyst_dropdowns_send_their_indices() {
    let mut h = rich();
    assert_eq!(
        pick(&mut h, "influence_0", "influence_0_2"),
        ["SetItemInfluence { which: 0, index: 2 }"]
    );
    assert_eq!(
        pick(&mut h, "influence_1", "influence_1_1"),
        ["SetItemInfluence { which: 1, index: 1 }"]
    );
    assert_eq!(
        pick(&mut h, "catalyst", "catalyst_2"),
        ["SetItemCatalyst { index: 2 }"]
    );
}

#[test]
fn the_catalyst_quality_box_sends_on_enter_reverts_on_escape_and_flushes() {
    let mut h = rich();
    h.type_into("catalyst_quality", "15");
    let o = h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        debug(&o.commands),
        ["SetItemCatalystQuality { text: \"15\" }"]
    );
    h.type_into("catalyst_quality", "99");
    let o = h.key(Key::Escape, Modifiers::NONE);
    assert!(o.commands.is_empty());
    assert!(h.ui.flush().is_empty());
    assert_eq!(h.ui.display.catalyst_quality.text, "10");
    h.type_into("catalyst_quality", "7");
    assert_eq!(
        debug(&h.ui.flush()),
        ["SetItemCatalystQuality { text: \"7\" }"]
    );
    assert!(h.ui.flush().is_empty());
}

#[test]
fn cancel_sends_cancel_display_item() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    assert_eq!(
        debug(&h.click("cancel").commands),
        ["CancelDisplayItem { if_id: None }"]
    );
}

#[test]
fn edit_prefills_the_views_rarity_and_text() {
    let mut h = Harness::new(sample());
    let mut view = display_view();
    view.edit_rarity = 4;
    view.edit_text = "Relic text".into();
    h.show_display(view);
    h.click("edit");
    let w = h.ui.display.text_window.clone().expect("open");
    assert_eq!(
        (w.rarity, w.text.as_str(), w.creating),
        (4, "Relic text", false)
    );
}

fn saved_window() -> Harness {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.click("edit");
    h.click("text_save");
    h.ui.text_sent(10);
    h
}

#[test]
fn a_view_from_before_the_text_command_leaves_the_window_open() {
    let mut h = saved_window();
    h.ui.on_snapshot_at(&Snapshot::DisplayItem(Some(Arc::new(display_view()))), 9);
    assert!(h.ui.display.text_window.is_some());
    h.ui.on_snapshot_at(&Snapshot::DisplayItem(Some(Arc::new(display_view()))), 10);
    assert!(h.ui.display.text_window.is_none(), "its own view closes it");
}

#[test]
fn the_error_for_the_text_command_keeps_the_window_over_its_views() {
    let mut h = saved_window();
    h.ui.text_refused();
    h.ui.on_snapshot_at(&Snapshot::DisplayItem(Some(Arc::new(display_view()))), 10);
    h.ui.on_snapshot_at(&Snapshot::DisplayItem(Some(Arc::new(display_view()))), 11);
    assert!(h.ui.display.text_window.is_some());
}

#[test]
fn text_window_closes_on_a_name_change_and_when_the_display_goes() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.click("edit");
    let mut renamed = display_view();
    renamed.name = "Other name".into();
    h.show_display(renamed);
    assert!(h.ui.display.text_window.is_none());
    h.click("edit");
    assert!(h.ui.display.text_window.is_some());
    h.ui.on_snapshot(&Snapshot::DisplayItem(None));
    assert!(h.ui.display.text_window.is_none());
}

#[test]
fn the_panel_stays_inside_the_window() {
    let mut h = Harness::new(sample());
    let mut view = rich_view();
    view.sockets = (0..6)
        .map(|_| SocketView {
            colour: "R".into(),
            shown: true,
            linked: Some(false),
        })
        .collect();
    h.show_display(view);
    h.frame(vec![]);
    for (key, r) in &h.ui.display.rects.map {
        assert!(r.right() <= 1200.0, "{key} ends at {}", r.right());
    }
}

// ---- the final review's fixes ---------------------------------------------------------------

#[test]
fn an_orphan_cancel_is_dropped_when_another_item_is_displayed() {
    let displayed = |id| {
        Snapshot::DisplayItem(Some(Arc::new(DisplayItemView {
            id,
            ..DisplayItemView::default()
        })))
    };
    let mut gone = sample();
    gone.items.retain(|e| e.id != 2);
    gone.listed.retain(|id| *id != 2);
    // item 2 is deleted, then the user pastes: the batch ends with the new item (no id)
    let mut h = Harness::new(sample());
    h.ui.on_snapshot(&displayed(Some(2)));
    h.ui.on_snapshot(&Snapshot::Items(Arc::new(gone.clone())));
    h.ui.on_snapshot(&displayed(None));
    assert!(h.ui.flush().is_empty());
    assert!(h.frame(vec![]).commands.is_empty(), "the paste is kept");
    // another list item displayed instead
    let mut h = Harness::new(sample());
    h.ui.on_snapshot(&displayed(Some(2)));
    h.ui.on_snapshot(&Snapshot::Items(Arc::new(gone)));
    h.ui.on_snapshot(&displayed(Some(1)));
    assert!(h.frame(vec![]).commands.is_empty());
}

#[test]
fn the_raw_text_window_counts_as_a_dialog_and_blocks_the_shortcuts() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    h.show_display(display_view());
    // without the window, the keys act
    let o = h.frame(vec![Event::Paste("Rarity: Rare".into())]);
    assert_eq!(debug(&o.commands), ["PasteItem { text: \"Rarity: Rare\" }"]);
    let o = h.key(Key::D, Modifiers::COMMAND);
    assert_eq!(debug(&o.commands), ["SetShowStatDifferences { on: true }"]);
    h.click("row_1");
    assert!(!h.ui.has_dialog());

    h.ui.display.text_window = Some(text_window::TextWindow::create());
    assert!(h.ui.has_dialog());
    let o = h.frame(vec![Event::Paste("Rarity: Rare".into())]);
    assert!(o.commands.is_empty(), "{:?}", o.commands);
    let o = h.key(Key::D, Modifiers::COMMAND);
    assert!(o.commands.is_empty(), "{:?}", o.commands);
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert!(o.commands.is_empty(), "{:?}", o.commands);
}

#[test]
fn the_panel_editors_do_nothing_while_the_raw_text_window_is_open() {
    let mut h = rich();
    h.ui.display.text_window = Some(text_window::TextWindow::create());
    // park the window out of the way so a click lands on the editor itself
    h.frame(vec![]);
    for key in ["save", "cancel", "tip_line_0", "add_socket", "edit"] {
        let o = h.click(key);
        assert!(o.commands.is_empty(), "{key}: {:?}", o.commands);
    }
    h.ui.display.text_window = None;
    let o = h.click("cancel");
    assert_eq!(debug(&o.commands), ["CancelDisplayItem { if_id: None }"]);
}

#[test]
fn a_quality_typed_and_then_hidden_is_not_flushed() {
    // the box vanished because the item changed: the edit must not follow to another item
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "15");
    let mut view = display_view();
    view.quality = None;
    h.show_display(view);
    assert!(h.ui.flush().is_empty(), "{:?}", h.ui.flush());

    let mut h = rich();
    h.type_into("catalyst_quality", "7");
    let mut view = rich_view();
    view.catalyst_quality = None;
    h.show_display(view);
    assert!(h.ui.flush().is_empty());
}

#[test]
fn cancel_drops_a_quality_still_being_typed() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "15");
    let pos = h.drect("cancel").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let mut commands = h.button(pos, true).commands;
    commands.extend(h.button(pos, false).commands);
    commands.extend(h.frame(vec![]).commands);
    assert_eq!(debug(&commands), ["CancelDisplayItem { if_id: None }"]);
    assert!(h.ui.flush().is_empty(), "nothing is left pending");

    let mut h = rich();
    h.type_into("catalyst_quality", "7");
    let pos = h.drect("cancel").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let mut commands = h.button(pos, true).commands;
    commands.extend(h.button(pos, false).commands);
    commands.extend(h.frame(vec![]).commands);
    assert_eq!(debug(&commands), ["CancelDisplayItem { if_id: None }"]);
    assert!(h.ui.flush().is_empty());
}

#[test]
fn edit_sends_a_quality_still_being_typed_and_opens_on_the_next_click() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "15");
    let pos = h.drect("edit").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let mut commands = h.button(pos, true).commands;
    commands.extend(h.button(pos, false).commands);
    commands.extend(h.frame(vec![]).commands);
    assert_eq!(debug(&commands), ["SetItemQuality { text: \"15\" }"]);
    assert!(
        h.ui.display.text_window.is_none(),
        "the window would show the old quality"
    );
    // the engine's view has the quality now
    let mut view = display_view();
    view.quality = Some("15".into());
    view.edit_text = "Quality: +15%".into();
    h.show_display(view);
    let o = h.click("edit");
    assert!(o.commands.is_empty());
    assert_eq!(
        h.ui.display.text_window.as_ref().map(|w| w.text.as_str()),
        Some("Quality: +15%")
    );
}

#[test]
fn save_is_disabled_while_the_engine_is_busy() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.busy = true;
    h.frame(vec![]);
    assert!(h.click("save").commands.is_empty());
    h.busy = false;
    h.frame(vec![]);
    assert_eq!(debug(&h.click("save").commands), ["SaveDisplayItem"]);
}

#[test]
fn the_quality_boxes_take_two_digits_only() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    h.type_into("quality", "2a05");
    assert_eq!(h.ui.display.quality.text, "20");
    h.type_into("quality", "x");
    assert_eq!(h.ui.display.quality.text, "20", "a letter is ignored");
    let mut h = rich();
    h.type_into("catalyst_quality", "123");
    assert_eq!(h.ui.display.catalyst_quality.text, "12");
}

#[test]
fn delete_all_and_unused_follow_the_shown_list() {
    let mut view = sample();
    view.listed.clear();
    let mut h = Harness::new(view);
    h.click("menu");
    assert!(h.click("menu_delete_unused").commands.is_empty());
    h.click("menu");
    h.click("menu_delete_all");
    assert!(
        !h.ui.has_dialog(),
        "Delete all is disabled for an empty list"
    );
}

// ---- crafting sections (Task 8) ----

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn crafted_view() -> DisplayItemView {
    use crate::engine::views::{AffixView, ClusterView, CustomModView};
    DisplayItemView {
        popups: vec![
            PopupKind::Craft,
            PopupKind::Enchant { second: false },
            PopupKind::Anoint { slot: 1 },
            PopupKind::AddModifier,
            PopupKind::EditText,
        ],
        affixes: vec![
            AffixView {
                slot: 0,
                label: "Prefix:".into(),
                list: DropList {
                    labels: labels(&["None", "^7+# to Life", "^xFF0000+# to Mana"]),
                    selected: 0,
                    enabled: true,
                },
                range: Some(0.25),
                tiers: Some(4),
            },
            AffixView {
                slot: 1,
                label: "Suffix:".into(),
                list: DropList {
                    labels: labels(&["None", "Of Haste"]),
                    selected: 1,
                    enabled: true,
                },
                range: None,
                tiers: None,
            },
        ],
        crafting_sort: Some(DropList {
            labels: labels(&["By tier", "By level", "Alphabetical"]),
            selected: 0,
            enabled: true,
        }),
        cluster: Some(ClusterView {
            skill: DropList {
                labels: labels(&["Fire Damage", "Cold Damage"]),
                selected: 0,
                enabled: true,
            },
            count: 8,
            min: 8,
            max: 12,
        }),
        custom_mods: vec![
            CustomModView {
                kind: "Crafted".into(),
                line: "+10 to Life".into(),
            },
            CustomModView {
                kind: "Custom".into(),
                line: "+1 to Strength".into(),
            },
        ],
        ..display_view()
    }
}

fn has_rect(h: &Harness, key: &str) -> bool {
    h.ui.display.rects.map.contains_key(key) || h.ui.display.crafting.rects.map.contains_key(key)
}

impl Harness {
    /// Forgets where widgets were, so only what the next frame draws has a rect.
    fn redraw(&mut self) {
        self.ui.display.rects.map.clear();
        self.ui.display.crafting.rects.map.clear();
        self.frame(vec![]);
    }

    /// The resizable panels reach their width over a few frames.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(vec![]);
        }
    }

    /// Presses the slider's middle, drags to `fraction` of its width and lets go.
    fn drag_slider(&mut self, key: &str, fraction: f32) -> Vec<Command> {
        let rect = self.ui.rect(key);
        let y = rect.center().y;
        let at = |t: f32| Pos2::new(rect.min.x + rect.width() * t, y);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut sent = Vec::new();
        sent.extend(self.frame(vec![Event::PointerMoved(at(0.02))]).commands);
        sent.extend(self.frame(vec![button(at(0.02), true)]).commands);
        sent.extend(self.frame(vec![Event::PointerMoved(at(fraction))]).commands);
        sent.extend(self.frame(vec![Event::PointerMoved(at(fraction))]).commands);
        assert!(sent.is_empty(), "nothing is sent while dragging: {sent:?}");
        sent.extend(self.frame(vec![button(at(fraction), false)]).commands);
        sent.extend(self.frame(vec![]).commands);
        sent
    }
}

#[test]
fn openers_follow_view_popups() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    assert!(has_rect(&h, "open_Change Enchantment..."));
    assert!(has_rect(&h, "open_Anoint 2..."));
    assert!(has_rect(&h, "open_Add modifier..."));
    assert!(!has_rect(&h, "open_Corrupt..."), "PoB doesn't offer it");
    assert!(!has_rect(&h, "open_Anoint..."), "slot 0 isn't offered");
    assert_eq!(
        debug(&h.click("open_Anoint 2...").commands),
        ["OpenPopup(Anoint { slot: 1 })"]
    );
    assert_eq!(
        debug(&h.click("open_Add modifier...").commands),
        ["OpenPopup(AddModifier)"]
    );
    // the offer changes with the view
    let mut view = crafted_view();
    view.popups = vec![
        PopupKind::Corrupt,
        PopupKind::AddImplicit,
        PopupKind::AddCrucible,
    ];
    h.show_display(view);
    h.redraw();
    assert!(!has_rect(&h, "open_Anoint 2..."));
    for label in ["Corrupt...", "Add Implicit...", "Add Crucible mod..."] {
        assert!(has_rect(&h, &format!("open_{label}")), "{label}");
    }
    assert_eq!(
        debug(&h.click("open_Corrupt...").commands),
        ["OpenPopup(Corrupt)"]
    );
}

#[test]
fn craft_button_only_without_display_item() {
    let mut h = Harness::new(sample());
    h.settle();
    assert!(has_rect(&h, "craft"), "nothing is displayed");
    assert_eq!(debug(&h.click("craft").commands), ["OpenPopup(Craft)"]);
    // Craft is in the view's list, but the item panel doesn't draw it
    h.show_display(crafted_view());
    h.settle();
    h.redraw();
    assert!(!has_rect(&h, "craft"));
    assert!(!has_rect(&h, "open_Craft item..."));
}

#[test]
fn affix_pick_sends_set_affix_with_label() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    h.click("affix_0");
    h.frame(vec![]);
    let sent = h.click("affix_0_2").commands;
    assert_eq!(
        debug(&sent),
        ["SetAffix { slot: 0, index: 2, label: \"^xFF0000+# to Mana\" }"]
    );
    // the other affix row is its own slot
    h.click("affix_1");
    h.frame(vec![]);
    let sent = h.click("affix_1_0").commands;
    assert_eq!(
        debug(&sent),
        ["SetAffix { slot: 1, index: 0, label: \"None\" }"]
    );
}

#[test]
fn modifier_sorting_sends_its_index() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    h.click("crafting_sort");
    h.frame(vec![]);
    assert_eq!(
        debug(&h.click("crafting_sort_2").commands),
        ["SetCraftingSort { index: 2 }"]
    );
}

#[test]
fn affix_slider_sends_on_release() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    let sent = h.drag_slider("affix_range_0", 0.9);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let Command::SetAffixRange { slot, value } = &sent[0] else {
        panic!("{sent:?}");
    };
    assert_eq!(*slot, 0);
    assert!(*value > 0.7, "{value}");
    assert!(!has_rect(&h, "affix_range_1"), "slot 1 has no slider");
}

#[test]
fn cluster_count_slider_sends_whole_count() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    let sent = h.drag_slider("cluster_count", 0.5);
    assert_eq!(debug(&sent), ["SetClusterNodeCount { count: 10 }"]);
    // the skill dropdown is its own command
    h.click("cluster_skill");
    h.frame(vec![]);
    assert_eq!(
        debug(&h.click("cluster_skill_1").commands),
        ["SetClusterSkill { index: 1 }"]
    );
}

#[test]
fn remove_custom_mod_sends_index() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    assert_eq!(
        debug(&h.click("remove_1").commands),
        ["RemoveCustomMod { index: 1 }"]
    );
    assert_eq!(
        debug(&h.click("remove_0").commands),
        ["RemoveCustomMod { index: 0 }"]
    );
}

#[test]
fn crafting_sections_show_only_what_the_view_has() {
    let mut h = Harness::new(sample());
    h.show_display(display_view());
    for key in [
        "affix_0",
        "crafting_sort",
        "cluster_skill",
        "cluster_count",
        "remove_0",
    ] {
        assert!(!has_rect(&h, key), "{key}");
    }
}

#[test]
fn hovering_an_affix_entry_asks_for_its_tooltip_after_100ms() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    h.step = 0.06;
    h.click("affix_0");
    h.frame(vec![]);
    let pos = h.ui.rect("affix_0_1").center();
    let mut queries = h.frame(vec![Event::PointerMoved(pos)]).queries;
    assert!(queries.is_empty(), "too early");
    for _ in 0..3 {
        queries.extend(h.frame(vec![]).queries);
    }
    assert_eq!(
        debug(&queries),
        ["ControlTooltip(Affix { slot: 0, index: Some(1) })"]
    );
}

// ---- the item databases panel -------------------------------------------------------------

fn drop_list(labels: &[&str]) -> DropList {
    DropList {
        labels: labels.iter().map(|s| (*s).to_owned()).collect(),
        selected: 0,
        enabled: true,
    }
}

fn db_row(name: &str) -> crate::engine::views::DbRow {
    crate::engine::views::DbRow {
        name: name.into(),
        label: format!("^xAF6025{name}"),
        base: "Base".into(),
        sort_value: Some(12.5),
    }
}

fn db_view(db: ItemDb) -> ItemDbView {
    let uniques = db == ItemDb::Uniques;
    ItemDbView {
        db,
        loading: false,
        sorting: false,
        percent: None,
        slot: drop_list(&["Any slot", "Helmet", "Ring"]),
        kind: drop_list(&["Any type", "Iron Hat", "Gold Ring"]),
        league: uniques.then(|| drop_list(&["Any league", "Heist", "Sanctum"])),
        requirement: uniques.then(|| drop_list(&["Any requirements", "Level 60"])),
        obtainable: uniques.then(|| drop_list(&["Any obtainability", "Obtainable"])),
        sort: uniques.then(|| drop_list(&["Sort by Name", "Sort by Life", "Sort by DPS"])),
        search: String::new(),
        search_mode: drop_list(&["Anywhere", "Names", "Modifiers"]),
        rows: vec![
            db_row("Goldrim"),
            db_row("Tabula Rasa"),
            db_row("Kaom's Heart"),
        ],
        note: String::new(),
    }
}

impl Harness {
    fn show_db(&mut self, view: ItemDbView) {
        self.ui.on_snapshot(&Snapshot::ItemDb(Some(Arc::new(view))));
        self.frame(vec![]);
    }

    /// Opens the panel; returns what the click sent.
    fn open_db(&mut self) -> Vec<String> {
        // the resizable panels reach their place over a few frames
        self.settle();
        let out = self.click("db_header");
        self.show_db(db_view(ItemDb::Uniques));
        // the body opens over a few frames
        self.settle();
        debug(&out.commands)
    }

    fn pick(&mut self, key: &str, index: usize) -> Vec<Command> {
        self.click(key);
        self.frame(vec![]);
        let sent = self.click(&format!("{key}_{index}")).commands;
        self.settle();
        sent
    }

    /// Whether a frame draws the widget.
    fn drawn(&mut self, key: &str) -> bool {
        self.ui.db.rects.map.clear();
        self.frame(vec![]);
        self.ui.db.rects.map.contains_key(key)
    }

    fn type_in(&mut self, key: &str, text: &str) {
        self.click(key);
        self.frame(vec![Event::Text(text.into())]);
    }
}

fn unique_query() -> ItemDbQuery {
    ItemDbQuery {
        db: ItemDb::Uniques,
        slot: 0,
        kind: 0,
        league: 0,
        requirement: 0,
        obtainable: 0,
        sort: 0,
        search: String::new(),
        search_mode: 0,
    }
}

#[test]
fn opening_the_panel_sends_the_default_query_and_collapsing_closes_it() {
    let mut h = Harness::new(sample());
    assert!(h.drawn("db_header"));
    assert!(!h.drawn("db_uniques"), "collapsed");
    assert_eq!(h.open_db(), [format!("SetItemDb({:?})", unique_query())]);
    assert!(h.drawn("db_uniques") && h.drawn("db_row_0"));
    assert_eq!(debug(&h.click("db_header").commands), ["CloseItemDb"]);
}

#[test]
fn db_toggle_to_rares_zeroes_unique_fields() {
    let mut h = Harness::new(sample());
    h.open_db();
    // the uniques' own filters and the sort are set...
    assert_eq!(debug(&h.pick("db_league", 2)).len(), 1);
    assert_eq!(debug(&h.pick("db_requirement", 1)).len(), 1);
    assert_eq!(debug(&h.pick("db_obtainable", 1)).len(), 1);
    assert_eq!(debug(&h.pick("db_sort", 2)).len(), 1);
    h.pick("db_slot", 1);
    h.pick("db_search_mode", 2);
    assert_eq!(
        h.ui.db.query(),
        &ItemDbQuery {
            slot: 1,
            league: 2,
            requirement: 1,
            obtainable: 1,
            sort: 2,
            search_mode: 2,
            ..unique_query()
        }
    );
    // ...and the rares have none of them (R13)
    let sent = h.click("db_rares").commands;
    assert_eq!(
        debug(&sent),
        [format!(
            "SetItemDb({:?})",
            ItemDbQuery {
                db: ItemDb::Rares,
                search_mode: 2,
                ..unique_query()
            }
        )]
    );
    // until the rares' lists arrive, the uniques' controls are gone, and the unique-only
    // ones stay hidden for the rares
    h.show_db(db_view(ItemDb::Rares));
    for key in ["db_league", "db_requirement", "db_obtainable", "db_sort"] {
        assert!(!has_rect(&h, key), "{key}");
    }
    assert!(h.drawn("db_slot") && h.drawn("db_search_mode"));
    // back to the uniques: nothing of the earlier choices is kept
    let sent = h.click("db_uniques").commands;
    assert_eq!(
        debug(&sent),
        [format!(
            "SetItemDb({:?})",
            ItemDbQuery {
                search_mode: 2,
                ..unique_query()
            }
        )]
    );
}

#[test]
fn db_search_sends_after_300ms_of_no_typing() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.step = 0.05;
    h.type_in("db_search", "gold");
    let mut sent = Vec::new();
    // 0.2 s on: still waiting
    for _ in 0..4 {
        sent.extend(h.frame(vec![]).commands);
    }
    assert!(sent.is_empty(), "{sent:?}");
    // more typing restarts the wait
    sent.extend(h.frame(vec![Event::Text("r".into())]).commands);
    for _ in 0..4 {
        sent.extend(h.frame(vec![]).commands);
    }
    assert!(sent.is_empty(), "{sent:?}");
    for _ in 0..4 {
        sent.extend(h.frame(vec![]).commands);
    }
    assert_eq!(
        debug(&sent),
        [format!(
            "SetItemDb({:?})",
            ItemDbQuery {
                search: "goldr".into(),
                ..unique_query()
            }
        )]
    );
    // once: Enter after it, or leaving the box, sends nothing more
    let after = h.key(Key::Enter, Modifiers::NONE);
    assert!(after.commands.is_empty(), "{:?}", after.commands);
}

#[test]
fn db_search_sends_at_once_on_enter() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.type_in("db_search", "ring");
    let out = h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        debug(&out.commands),
        [format!(
            "SetItemDb({:?})",
            ItemDbQuery {
                search: "ring".into(),
                ..unique_query()
            }
        )]
    );
}

#[test]
fn db_search_still_pending_is_flushed_once() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.type_in("db_search", "ring");
    let flushed = h.ui.flush();
    assert_eq!(
        debug(&flushed),
        [format!(
            "SetItemDb({:?})",
            ItemDbQuery {
                search: "ring".into(),
                ..unique_query()
            }
        )]
    );
    assert!(h.ui.flush().is_empty(), "once");
    // the blur that follows sends nothing
    let out = h.click("db_header");
    assert_eq!(debug(&out.commands), ["CloseItemDb"]);
}

#[test]
fn db_progress_shown_while_sorting() {
    let mut h = Harness::new(sample());
    h.open_db();
    assert!(!h.drawn("db_progress"));
    let mut sorting = db_view(ItemDb::Uniques);
    sorting.sorting = true;
    sorting.rows.clear();
    h.show_db(sorting);
    // sorting, no progress yet: no bar
    assert!(!h.drawn("db_progress"));
    h.ui.on_db_progress(40);
    h.frame(vec![]);
    assert!(h.drawn("db_progress"));
    // the rows come with the end of the sort and the bar goes
    h.show_db(db_view(ItemDb::Uniques));
    assert!(!h.drawn("db_progress"));
    assert!(h.drawn("db_row_0"));
}

#[test]
fn db_failure_shows_retry_and_retry_resends_query() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.pick("db_sort", 1);
    h.ui.on_db_failed("boom".into());
    h.frame(vec![]);
    assert!(h.drawn("db_retry"));
    let sent = h.click("db_retry").commands;
    assert_eq!(
        debug(&sent),
        [format!(
            "SetItemDb({:?})",
            ItemDbQuery {
                sort: 1,
                ..unique_query()
            }
        )]
    );
    assert!(!h.drawn("db_retry"), "the error is gone");
}

#[test]
fn a_refused_db_query_is_reset_so_retry_can_succeed() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.pick("db_sort", 1);
    // the engine refused the query: it said why, and closed the database
    h.ui.on_db_failed("that league isn't listed".into());
    h.ui.on_snapshot(&Snapshot::ItemDb(None));
    h.frame(vec![]);
    assert!(h.drawn("db_retry"));
    let sent = h.click("db_retry").commands;
    assert_eq!(
        debug(&sent),
        [format!("SetItemDb({:?})", unique_query())],
        "Retry sends the query the filters were reset to, not the refused one"
    );
}

#[test]
fn a_closed_db_without_a_failure_keeps_the_query() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.pick("db_sort", 1);
    h.ui.on_snapshot(&Snapshot::ItemDb(None));
    h.frame(vec![]);
    assert_eq!(h.ui.db.query().sort, 1);
}

#[test]
fn db_double_click_views_and_ctrl_click_adds() {
    let mut h = Harness::new(sample());
    h.open_db();
    // a single click only selects
    assert!(h.click("db_row_1").commands.is_empty());
    h.step = 0.05;
    let pos = h.ui.rect("db_row_1").center();
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut sent = Vec::new();
    for events in [
        vec![Event::PointerMoved(pos)],
        vec![button(true)],
        vec![button(false)],
        vec![button(true)],
        vec![button(false)],
        vec![],
    ] {
        sent.extend(h.frame(events).commands);
    }
    assert_eq!(
        debug(&sent),
        ["ViewDbItem { db: Uniques, name: \"Tabula Rasa\" }"]
    );
    // Ctrl+click adds and equips
    h.step = 0.2;
    h.modifiers = Modifiers::CTRL;
    let sent = h.click("db_row_2").commands;
    assert_eq!(
        debug(&sent),
        ["AddDbItem { db: Uniques, name: \"Kaom's Heart\" }"]
    );
    h.modifiers = Modifiers::NONE;
    // so does the button, for the selected row
    let sent = h.click("db_add").commands;
    assert_eq!(
        debug(&sent),
        ["AddDbItem { db: Uniques, name: \"Kaom's Heart\" }"]
    );
}

#[test]
fn ctrl_double_click_on_a_db_row_adds_once() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.step = 0.05;
    h.modifiers = Modifiers::CTRL;
    let pos = h.ui.rect("db_row_1").center();
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::CTRL,
    };
    let mut sent = Vec::new();
    for events in [
        vec![Event::PointerMoved(pos)],
        vec![button(true)],
        vec![button(false)],
        vec![button(true)],
        vec![button(false)],
        vec![],
    ] {
        sent.extend(h.frame(events).commands);
    }
    assert_eq!(
        debug(&sent),
        ["AddDbItem { db: Uniques, name: \"Tabula Rasa\" }"],
        "one add for the gesture"
    );
}

#[test]
fn db_row_tooltip_replies_are_kept_only_for_the_latest_question() {
    use crate::engine::ControlTooltipQuery;
    use crate::engine::views::ControlTooltipView;
    let mut h = Harness::new(sample());
    h.open_db();
    h.step = 0.06;
    let pos = h.ui.rect("db_row_1").center();
    let mut queries = h.frame(vec![Event::PointerMoved(pos)]).queries;
    for _ in 0..3 {
        queries.extend(h.frame(vec![]).queries);
    }
    assert_eq!(queries.len(), 1);
    let reply = |name: &str| {
        Snapshot::ControlTooltip(Arc::new(ControlTooltipView {
            query: ControlTooltipQuery::DbItem {
                db: ItemDb::Uniques,
                name: name.into(),
            },
            tooltip: crate::engine::views::ItemTooltip::default(),
        }))
    };
    // a reply to a row the pointer left is ignored
    h.ui.on_snapshot(&reply("Goldrim"));
    assert!(h.ui.db.tip_query().is_none());
    h.ui.on_snapshot(&reply("Tabula Rasa"));
    assert_eq!(
        h.ui.db.tip_query(),
        Some(&ControlTooltipQuery::DbItem {
            db: ItemDb::Uniques,
            name: "Tabula Rasa".into()
        })
    );
    // moving off the row drops it
    h.frame(vec![Event::PointerMoved(Pos2::new(-5.0, -5.0))]);
    assert!(h.ui.db.tip_query().is_none());
}

#[test]
fn db_rows_ask_for_their_tooltip_after_100ms() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.step = 0.06;
    let pos = h.ui.rect("db_row_1").center();
    let mut queries = h.frame(vec![Event::PointerMoved(pos)]).queries;
    assert!(queries.is_empty(), "too early");
    for _ in 0..3 {
        queries.extend(h.frame(vec![]).queries);
    }
    assert_eq!(
        debug(&queries),
        ["ControlTooltip(DbItem { db: Uniques, name: \"Tabula Rasa\" })"]
    );
}

#[test]
fn db_closed_on_tab_leave() {
    let mut h = Harness::new(sample());
    h.open_db();
    assert_eq!(
        debug(&h.ui.leave().into_iter().collect::<Vec<_>>()),
        ["CloseItemDb"]
    );
    assert!(h.ui.leave().is_none(), "once");
    // back on the tab the panel is still open and asks again
    let out = h.frame(vec![]);
    assert_eq!(
        debug(&out.commands),
        [format!("SetItemDb({:?})", unique_query())]
    );
}

#[test]
fn a_view_of_the_other_database_is_not_drawn() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.click("db_rares");
    // the uniques' late view must not show under the rares toggle
    h.show_db(db_view(ItemDb::Uniques));
    assert!(!h.drawn("db_row_0"));
    assert!(!h.drawn("db_league"));
}

#[test]
fn focus_slot_finds_the_slot_of_a_socket_and_drops_it_down() {
    let mut view = sample();
    view.slots[9].node = Some(26725);
    let mut h = Harness::new(view);
    assert_eq!(h.ui.enabled_sockets(), [26725]);
    h.ui.focus_slot(99);
    assert_eq!(h.ui.focused_slot(), None, "no such socket");
    h.ui.focus_slot(26725);
    assert_eq!(h.ui.focused_slot(), Some("Jewel 5"));
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.ui.focused_slot(), None, "taken when drawn");
    assert!(egui::Popup::is_any_open(&h.ctx), "the dropdown is open");
}

#[test]
fn focus_slot_of_a_hidden_slot_does_not_linger() {
    let mut view = sample();
    view.slots[9].node = Some(26725);
    view.slots[9].shown = false;
    let mut h = Harness::new(view);
    h.ui.focus_slot(26725);
    assert_eq!(h.ui.focused_slot(), Some("Jewel 5"));
    h.frame(vec![]);
    assert_eq!(
        h.ui.focused_slot(),
        None,
        "dropped after a pass that didn't draw it"
    );
    assert!(!egui::Popup::is_any_open(&h.ctx));
}

/// A database view for the app's tests.
pub(crate) fn db_view_for_app() -> ItemDbView {
    db_view(ItemDb::Uniques)
}
