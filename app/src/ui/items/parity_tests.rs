//! Task 13b's Items tab gestures: Ctrl+Shift+click and drag in the databases (R23), Ctrl+click
//! on the item list, Shift's child tooltips (R34), the modal raw-text window (R34), the stacked
//! range sliders and the tree dropdown. A child of `tests`, for its harness.

use super::*;

#[test]
fn ctrl_shift_click_adds_second_slot() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.modifiers = Modifiers::CTRL | Modifiers::SHIFT;
    let sent = h.click("db_row_2").commands;
    assert_eq!(
        debug(&sent),
        ["AddDbItem { db: Uniques, name: \"Kaom's Heart\", second_slot: true }"]
    );
    // Shift alone is no add
    h.modifiers = Modifiers::SHIFT;
    assert!(h.click("db_row_1").commands.is_empty());
}

#[test]
fn db_row_drop_on_slot_sends_drop() {
    let mut h = Harness::new(sample());
    h.open_db();
    let from = h.ui.rect("db_row_1").center();
    let to = h.ui.rect("slot_Ring 1").center();
    let sent = h.drag_between(from, to);
    assert_eq!(
        debug(&sent),
        ["DropDbItem { db: Uniques, name: \"Tabula Rasa\", slot: \"Ring 1\" }"]
    );
    // dropped on the item list, it is added at the row under the cursor, unequipped
    // (ItemsTab.lua:1184-1187)
    let to = h.ui.rect("row_2").center();
    assert_eq!(
        debug(&h.drag_between(from, to)),
        ["DropDbItemOnList { db: Uniques, name: \"Tabula Rasa\", at: Some(2) }"]
    );
    // dropped anywhere else, nothing is sent
    let to = h.ui.rect("db_row_2").center();
    assert!(h.drag_between(from, to).is_empty());
}

#[test]
fn shift_hover_on_a_db_row_asks_child_tooltips() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.step = 0.06;
    let pos = h.ui.rect("db_row_1").center();
    h.modifiers = Modifiers::SHIFT;
    let mut asked = h.frame(vec![Event::PointerMoved(pos)]).queries;
    for _ in 0..3 {
        asked.extend(h.frame(vec![]).queries);
    }
    assert_eq!(
        debug(&asked),
        ["ControlTooltip(DbItem { db: Uniques, name: \"Tabula Rasa\", shift: true })"]
    );
}

#[test]
fn ctrl_click_on_a_list_row_quick_equips() {
    let mut h = Harness::new(sample());
    h.modifiers = Modifiers::CTRL;
    assert_eq!(
        debug(&h.click("row_1").commands),
        ["QuickEquipItem { id: 1, second_slot: false }"]
    );
    h.modifiers = Modifiers::CTRL | Modifiers::SHIFT;
    assert_eq!(
        debug(&h.click("row_1").commands),
        ["QuickEquipItem { id: 1, second_slot: true }"]
    );
    // a plain click only selects
    h.modifiers = Modifiers::NONE;
    assert!(h.click("row_1").commands.is_empty());
}

#[test]
fn shift_hover_asks_child_tooltip() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("row_1").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let asked = h.frame(vec![]).queries;
    assert_eq!(
        debug(&asked),
        [
            "ItemTooltip(ItemTooltipQuery { id: 1, slot: None, theirs: false, shift: false, shared: None })"
        ]
    );
    // holding Shift is a new question, asked once after the delay
    h.modifiers = Modifiers::SHIFT;
    let mut asked = h.frame(vec![]).queries;
    asked.extend(h.frame(vec![]).queries);
    asked.extend(h.frame(vec![]).queries);
    assert_eq!(
        debug(&asked),
        [
            "ItemTooltip(ItemTooltipQuery { id: 1, slot: None, theirs: false, shift: true, shared: None })"
        ]
    );
}

#[test]
fn text_window_blocks_list_clicks() {
    let mut h = Harness::new(sample());
    h.frame(vec![]);
    let row = h.ui.rect("row_1").center();
    h.click("create");
    assert!(h.ui.display.text_window.is_some());
    // a click on the list behind the window reaches nothing
    let pos = row;
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
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(h.ui.selected, None, "the row behind was selected");
    assert!(h.ui.display.text_window.is_some(), "the window stays");
    // its own close button still works
    h.click("text_close");
    assert!(h.ui.display.text_window.is_none());
}

#[test]
fn text_window_closes_on_escape_but_not_on_a_click_outside() {
    let mut h = Harness::new(sample());
    h.frame(vec![]);
    h.click("create");
    h.frame(vec![]);
    // a click on the backdrop leaves it open, as PoB's popup
    let pos = Pos2::new(5.0, 5.0);
    for pressed in [true, false] {
        h.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]);
    }
    h.frame(vec![]);
    assert!(
        h.ui.display.text_window.is_some(),
        "a click outside closed it"
    );
    // Escape closes it, even with the text box focused
    h.click("text_edit");
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(vec![]);
    assert!(h.ui.display.text_window.is_none(), "Escape left it open");
}

fn stacked_view() -> DisplayItemView {
    let line = |label: &str, slider: bool, mutable: bool| RangeLineView {
        label: label.into(),
        range: slider.then_some(0.5),
        slider,
        mutable,
        mutated: false,
    };
    DisplayItemView {
        range_lines: vec![
            line("+(10-20) to Strength", true, false),
            line("Foulborn line", false, true),
            line("+(5-9)% to all Resistances", true, true),
        ],
        stacked: true,
        ..display_view()
    }
}

#[test]
fn stacked_lines_have_a_slider_and_box_each() {
    let mut h = Harness::new(sample());
    h.show_display(stacked_view());
    h.settle();
    // PoB's rows: a slider where the line has a range, a box where it can mutate
    for key in [
        "stacked_slider_0",
        "stacked_mutated_1",
        "stacked_slider_2",
        "stacked_mutated_2",
    ] {
        assert!(h.ui.has_rect(key), "{key} not drawn");
    }
    assert!(!h.ui.has_rect("stacked_slider_1"));
    assert!(!h.ui.has_rect("stacked_mutated_0"));
    // the line dropdown is not there
    assert!(!h.ui.has_rect("range_lines"));
    let sent = h.drag_slider("stacked_slider_2", 0.9);
    assert!(
        matches!(sent.as_slice(), [Command::SetRangeLine { line: 2, value }] if *value > 0.6),
        "{sent:?}"
    );
    assert_eq!(
        debug(&h.click("stacked_mutated_1").commands),
        ["SetRangeLineMutated { line: 1, on: true }"]
    );
}

#[test]
fn items_tree_dropdown_selects_and_manages() {
    let mut view = sample();
    view.specs = Some(DropList {
        labels: vec!["Main".into(), "[3.27] Old".into()],
        selected: 0,
        enabled: true,
    });
    let mut h = Harness::new(view);
    h.settle();
    h.click("spec_combo");
    assert_eq!(
        debug(&h.click("spec_1").commands),
        ["SelectSpec { index: 1 }"]
    );
    let out = h.click("spec_manage");
    assert!(out.manage_trees && out.commands.is_empty());
}

/// The Config tab's "Add Mod" on a custom modifier group opens PoB's Mod Browser for it (R28).
/// Here with the Items gestures: it is the Items part's window.
#[test]
fn add_mod_opens_the_mod_browser() {
    use crate::engine::views::ConfigView;
    use crate::ui::config::ConfigUi;
    let view: ConfigView = serde_json::from_value(serde_json::json!({
        "active_set": 1,
        "custom_mods": [
            { "title": "Default", "enabled": true, "text": "" },
            { "title": "Second", "enabled": true, "text": "+1 to Strength" },
        ],
    }))
    .unwrap();
    let ctx = egui::Context::default();
    let mut config = ConfigUi::default();
    let mut run = |events: Vec<Event>| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut sent = Vec::new();
        ctx.run_ui(input, |ui| sent.extend(config.show(ui, &view, false)))
            .drop_without_applying_deltas();
        (sent, config.rects.get("mod_add:1").copied())
    };
    run(vec![]);
    let (_, rect) = run(vec![]);
    let pos = rect.expect("no Add Mod button").center();
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
    ] {
        sent.extend(run(events).0);
    }
    assert_eq!(debug(&sent), ["OpenPopup(ModBrowser { group: 1 })"]);
}

// ---- Shared items (NF-2) ----

fn shared_sample() -> ItemsView {
    use crate::engine::views::{SharedItemEntry, SharedSetEntry};
    let entry = |name: &str| SharedItemEntry {
        name: name.into(),
        rarity: "RARE".into(),
        label: format!("^xFFFF77{name}"),
    };
    ItemsView {
        shared_items: vec![entry("Shared Loop"), entry("Shared Band")],
        shared_sets: vec![SharedSetEntry {
            title: "Shared Set".into(),
            lines: vec!["^7Ring 1: ^xFFFF77Set Loop".into()],
            raw_title: Some("Shared Set".into()),
        }],
        shared_shown: true,
        ..sample()
    }
}

fn drag(h: &mut Harness, from: &str, to: &str) -> Vec<String> {
    let from = h.ui.rect(from).center();
    let to = h.ui.rect(to).center();
    debug(&h.drag_between(from, to))
}

#[test]
fn shared_rows_drag_onto_slots_and_the_list() {
    let mut h = Harness::new(shared_sample());
    h.settle();
    assert_eq!(
        drag(&mut h, "shared_row_0", "slot_Ring 1"),
        ["DropSharedItem { index: 0, slot: Some(\"Ring 1\"), at: None }"]
    );
    // on the item list, at the row under the cursor (ItemListControl.lua:279)
    assert_eq!(
        drag(&mut h, "shared_row_1", "row_2"),
        ["DropSharedItem { index: 1, slot: None, at: Some(2) }"]
    );
    // a list PoB doesn't let reorder (a loadout filter) appends
    let mut view = shared_sample();
    view.can_reorder = false;
    let mut h = Harness::new(view);
    h.settle();
    assert_eq!(
        drag(&mut h, "shared_row_1", "row_2"),
        ["DropSharedItem { index: 1, slot: None, at: None }"]
    );
}

#[test]
fn ctrl_c_on_a_shared_row_copies_it() {
    let mut h = Harness::new(shared_sample());
    h.settle();
    h.click("shared_row_1");
    let o = h.frame(vec![Event::Copy]);
    assert!(
        debug(&o.queries).contains(&"CopySharedItem { index: 1 }".to_owned()),
        "{:?}",
        o.queries
    );
    let reply = |id: u32, shared: bool, text: &str| {
        Snapshot::ItemText(Arc::new(ItemText {
            id,
            text: text.into(),
            shared,
        }))
    };
    // a build item's text with the same number is not the shared one
    h.ui.on_snapshot(&reply(1, false, "build item"));
    h.frame(vec![]);
    assert!(h.copied.is_empty(), "{:?}", h.copied);
    h.ui.on_snapshot(&reply(1, true, "Rarity: RARE\r\nShared Band\r\n"));
    h.frame(vec![]);
    assert_eq!(h.copied, ["Rarity: RARE\r\nShared Band\r\n"]);
}

#[test]
fn build_items_and_db_rows_drag_onto_the_shared_list() {
    let mut h = Harness::new(shared_sample());
    h.open_db();
    // a drop on a row's lower half lands after it, as PoB rounds the cursor's row
    // (ListControl.lua:160)
    assert_eq!(
        drag(&mut h, "row_1", "shared_row_1"),
        ["AddSharedItem { source: Item { id: 1 }, index: 2 }"]
    );
    assert_eq!(
        drag(&mut h, "db_row_1", "shared_row_0"),
        ["AddSharedItem { source: Db { db: Uniques, name: \"Tabula Rasa\" }, index: 1 }"]
    );
}

#[test]
fn shared_delete_asks_and_double_click_views() {
    let mut h = Harness::new(shared_sample());
    h.settle();
    h.click("shared_row_1");
    assert!(
        h.click("shared_delete").commands.is_empty(),
        "PoB asks first"
    );
    assert_eq!(
        debug(&h.click("confirm_shared_delete").commands),
        ["DeleteSharedItem { index: 1 }"]
    );
    assert_eq!(
        debug(&h.double_click("shared_row_0")),
        ["ViewSharedItem { index: 0 }"]
    );
}

#[test]
fn shared_hover_asks_the_shared_tooltip() {
    let mut h = Harness::new(shared_sample());
    h.settle();
    let pos = h.ui.rect("shared_row_1").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let asked = h.frame(vec![]).queries;
    assert_eq!(
        debug(&asked),
        [
            "ItemTooltip(ItemTooltipQuery { id: 0, slot: None, theirs: false, shift: false, shared: Some(1) })"
        ]
    );
}

#[test]
fn the_shared_list_hides_under_a_displayed_item() {
    let mut view = shared_sample();
    view.shared_shown = false;
    let mut h = Harness::new(view);
    h.settle();
    assert!(!h.ui.has_rect("shared_list"));
}

#[test]
fn shared_sets_drag_rename_and_delete() {
    let mut h = Harness::new(shared_sample());
    h.click("manage");
    h.settle();
    assert_eq!(
        drag(&mut h, "sets_row_0", "shared_sets_row_0"),
        ["AddSharedItemSet { id: 1 }"]
    );
    assert_eq!(
        drag(&mut h, "shared_sets_row_0", "sets_row_1"),
        ["ImportSharedItemSet { index: 0 }"]
    );
    h.click("shared_sets_row_0");
    // F2 on the focused row renames, as PoB's
    h.key(Key::F2, Modifiers::NONE);
    h.settle();
    // Save waits for an edit (SharedItemSetListControl.lua:31-38)
    assert!(h.click("shared_sets_save").commands.is_empty());
    h.type_into("shared_sets_name", "Renamed");
    assert_eq!(
        debug(&h.click("shared_sets_save").commands),
        ["RenameSharedItemSet { index: 0, title: \"Renamed\" }"]
    );
    h.click("shared_sets_row_0");
    h.click("shared_sets_delete");
    assert_eq!(
        debug(&h.click("shared_sets_confirm_delete").commands),
        ["DeleteSharedItemSet { index: 0 }"]
    );
}

#[test]
fn renaming_an_untitled_shared_set_starts_empty() {
    use crate::engine::views::SharedSetEntry;
    let mut view = shared_sample();
    // PoB draws an untitled set as "Default" but names it nothing
    view.shared_sets = vec![SharedSetEntry {
        title: "Default".into(),
        lines: Vec::new(),
        raw_title: None,
    }];
    let mut h = Harness::new(view);
    h.click("manage");
    h.settle();
    h.click("shared_sets_row_0");
    h.click("shared_sets_rename");
    h.settle();
    assert_eq!(
        h.ui.sets.shared_naming_text(),
        Some(""),
        "the box starts from the set's own title"
    );
    // unchanged, Save is off: an untitled set isn't saved as "Default"
    assert!(h.click("shared_sets_save").commands.is_empty());
}

/// The shared list takes PoB's list keys while one of its rows has the keyboard: the arrows
/// pick a row and Backspace asks to delete it (ListControl.lua:375-392); the build's item
/// list keeps its own
#[test]
fn shared_list_keys_move_the_selection_and_delete_asks() {
    let mut h = Harness::new(shared_sample());
    h.settle();
    h.click("shared_row_0");
    h.key(Key::ArrowDown, Modifiers::NONE);
    h.settle();
    let o = h.key(Key::Backspace, Modifiers::NONE);
    assert!(o.commands.is_empty(), "PoB asks first: {:?}", o.commands);
    h.settle();
    assert_eq!(
        debug(&h.click("confirm_shared_delete").commands),
        ["DeleteSharedItem { index: 1 }"]
    );
}

/// In the item set window Delete on a focused shared set asks about that set, not the build's
/// selected one
#[test]
fn delete_on_a_shared_set_row_asks_about_the_shared_set() {
    let mut h = Harness::new(shared_sample());
    h.click("manage");
    h.settle();
    h.click("shared_sets_row_0");
    h.key(Key::Delete, Modifiers::NONE);
    h.settle();
    assert!(
        !h.ui.has_rect("sets_confirm_delete"),
        "the build's set was asked about"
    );
    assert_eq!(
        debug(&h.click("shared_sets_confirm_delete").commands),
        ["DeleteSharedItemSet { index: 0 }"]
    );
}
