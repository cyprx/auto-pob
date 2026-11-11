use super::list::delete_prompt;
use super::slots::{SlotRow, SlotSection, rows};
use super::*;
use crate::engine::views::{
    DisplayItemView, DropList, EquippedIn, ItemEntry, ItemSetEntry, ItemText, ItemTooltip,
    ItemTooltipLine, SlotKind, SlotView,
};
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
    copied: Vec<String>,
    /// A text field above the tab, with focus
    field: Option<String>,
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
            copied: Vec::new(),
            field: None,
        };
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) -> ItemsOutput {
        self.time += 0.2;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut out = ItemsOutput::default();
        let time = self.time;
        let output = self.ctx.run_ui(input, |ui| {
            if let Some(text) = &mut self.field {
                let field = ui.add(egui::TextEdit::singleline(text).id_salt("field"));
                field.request_focus();
            }
            // egui may run a frame twice; each pass's output counts
            let pass = self.ui.show(ui, time, false);
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
    assert_eq!(debug(&o.commands), ["DeleteItem { id: 2 }"]);

    // an abyss jewel used in another set asks too
    let mut h = Harness::new(sample());
    h.click("row_3");
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert!(o.commands.is_empty());

    // an unused item goes at once
    let mut h = Harness::new(sample());
    h.click("row_1");
    let o = h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(debug(&o.commands), ["DeleteItem { id: 1 }"]);
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
    assert_eq!(debug(&o.commands), ["CancelDisplayItem"]);
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
