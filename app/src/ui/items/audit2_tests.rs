//! Parity audit round 2 (Lane I): the Items rows checked against PoB 2.67.2. A child of
//! `tests`, for its harness.

use super::*;

fn wheel(delta: f32) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::Vec2::new(0.0, delta),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    }
}

/// Every slider steps on the wheel and the arrows (SliderControl.lua:164-180), the stacked
/// range rows' too (ItemsTab.lua:1142)
#[test]
fn the_wheel_steps_a_stacked_range_slider() {
    let mut h = Harness::new(sample());
    h.show_display(parity_tests::stacked_view());
    h.settle();
    let at = h.ui.rect("stacked_slider_0").center();
    h.frame(vec![Event::PointerMoved(at)]);
    let sent = h.frame(vec![wheel(1.0)]).commands;
    assert!(
        matches!(sent.as_slice(), [Command::SetRangeLine { line: 0, value }] if (*value - 0.55).abs() < 1e-9),
        "{sent:?}"
    );
}

/// Hovers `key` until its control tooltip question goes out; the questions asked
fn hover_queries(h: &mut Harness, pos: Pos2) -> Vec<String> {
    h.step = 0.06;
    let mut queries = h.frame(vec![Event::PointerMoved(pos)]).queries;
    for _ in 0..3 {
        queries.extend(h.frame(vec![]).queries);
    }
    h.step = 0.2;
    debug(&queries)
}

/// The closed affix dropdown shows its selected entry's tooltip (mode "BODY"), and the slider
/// its own, for the value under the pointer (ItemsTab.lua:792-938, SliderControl.lua:117-125)
#[test]
fn an_affix_body_and_its_slider_ask_their_own_tooltips() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    let body = h.ui.rect("affix_1").center();
    assert_eq!(
        hover_queries(&mut h, body),
        ["ControlTooltip(Affix { slot: 1, index: Some(1) })"]
    );
    let slider = h.ui.rect("affix_range_0");
    let at = Pos2::new(slider.left() + slider.width() * 0.75, slider.center().y);
    let asked = hover_queries(&mut h, at);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].starts_with("ControlTooltip(AffixSlider { slot: 0, at: 7"),
        "{asked:?}"
    );
    // its label is PoB's, grey while the dropdown is on "None"
    assert!(h.ui.display.crafting.rects.map.contains_key("affix_roll_0"));
}

/// The database rows are a ListControl: Up, Down, Home and End move the selection, and Ctrl+C
/// copies the selected row as PoB's OnSelCopy (ItemDBControl.lua:368-370)
#[test]
fn db_rows_take_the_list_keys_and_ctrl_c() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.click("db_row_0");
    h.key(Key::ArrowDown, Modifiers::NONE);
    let o = h.frame(vec![Event::Copy]);
    assert_eq!(
        debug(&o.queries),
        ["CopyDbItem { db: Uniques, name: \"Tabula Rasa\" }"]
    );
    // only the reply for the row asked about reaches the clipboard
    let reply = |name: &str| {
        Snapshot::ItemText(Arc::new(ItemText {
            id: 0,
            text: format!("{name}\r\n"),
            shared: false,
            db_item: Some(name.into()),
        }))
    };
    h.ui.on_snapshot(&reply("Goldrim"));
    h.frame(vec![]);
    assert!(h.copied.is_empty(), "{:?}", h.copied);
    h.ui.on_snapshot(&reply("Tabula Rasa"));
    h.frame(vec![]);
    assert_eq!(h.copied, ["Tabula Rasa\r\n"]);
    // End picks the last row; a click elsewhere leaves the list, and its keys with it
    h.key(Key::End, Modifiers::NONE);
    let o = h.frame(vec![Event::Copy]);
    assert_eq!(
        debug(&o.queries),
        ["CopyDbItem { db: Uniques, name: \"Kaom's Heart\" }"]
    );
    h.click("row_1");
    let o = h.frame(vec![Event::Copy]);
    assert!(
        !debug(&o.queries)
            .iter()
            .any(|q| q.starts_with("CopyDbItem")),
        "{:?}",
        o.queries
    );
}

/// The search box is PoB's: `%c`, 100 bytes, and an "x" that empties it and searches at once
/// (ItemDBControl.lua:51, EditControl.lua:73-77)
#[test]
fn the_db_search_box_clears_on_its_x_and_holds_100_bytes() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.type_into("db_search", &"a".repeat(120));
    assert_eq!(h.ui.db.search.text.len(), 100);
    h.frame(vec![Event::Key {
        key: Key::Enter,
        physical_key: Some(Key::Enter),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    h.settle();
    let o = h.click("db_search_clear");
    let sent = debug(&o.commands);
    assert!(
        sent.iter()
            .any(|c| c.contains("SetItemDb") && c.contains("search: \"\"")),
        "{sent:?}"
    );
    assert!(h.ui.db.search.text.is_empty());
}
