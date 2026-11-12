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
    // dropped anywhere else, nothing is sent
    let to = h.ui.rect("row_1").center();
    assert!(h.drag_between(from, to).is_empty());
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
        ["ItemTooltip(ItemTooltipQuery { id: 1, slot: None, theirs: false, shift: false })"]
    );
    // holding Shift is a new question, asked once after the delay
    h.modifiers = Modifiers::SHIFT;
    let mut asked = h.frame(vec![]).queries;
    asked.extend(h.frame(vec![]).queries);
    asked.extend(h.frame(vec![]).queries);
    assert_eq!(
        debug(&asked),
        ["ItemTooltip(ItemTooltipQuery { id: 1, slot: None, theirs: false, shift: true })"]
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
