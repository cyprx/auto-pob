//! Parity audit round 2 (Lane I): the Items rows checked against PoB 2.67.2. A child of
//! `tests`, for its harness.

use super::*;
use crate::engine::ControlTooltipQuery;

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

/// PoB's popups are modal: while one is open, the selected DB list's keys do nothing
#[test]
fn db_row_keys_wait_for_a_dialog() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.click("db_row_0");
    let first = h.ui.db.selected.clone();
    let quiet = |h: &mut Harness| {
        h.key(Key::ArrowDown, Modifiers::NONE);
        let o = h.frame(vec![Event::Copy]);
        assert!(o.queries.is_empty(), "{:?}", o.queries);
        assert_eq!(h.ui.db.selected, first);
    };
    h.ui.set_modal(true);
    quiet(&mut h);
    h.ui.set_modal(false);
    h.ui.confirm_delete_all = true;
    quiet(&mut h);
    h.ui.confirm_delete_all = false;
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert_ne!(h.ui.db.selected, first);
}

/// PoB's questions are modal: Del Unused, Del All and Delete do nothing under one
#[test]
fn the_delete_buttons_wait_for_an_open_question() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    for ask in [true, false] {
        if ask {
            h.ui.confirm_delete_all = true;
        } else {
            h.ui.confirm_delete = Some((2, "In use".into()));
        }
        h.frame(vec![]);
        for key in ["delete_unused", "delete_all", "delete"] {
            let o = h.click(key);
            assert!(o.commands.is_empty(), "{key}: {:?}", o.commands);
        }
        assert_eq!(h.ui.confirm_delete_all, ask);
        assert_eq!(h.ui.confirm_delete.is_some(), !ask);
        h.ui.confirm_delete_all = false;
        h.ui.confirm_delete = None;
    }
}

/// PoB copies on C while Ctrl is down, Shift held or not (ListControl.lua:384); egui-winit
/// sends Ctrl+Shift+C as the key, not as `Event::Copy`
#[test]
fn ctrl_shift_c_copies_the_selected_row() {
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut h = Harness::new(sample());
    h.open_db();
    h.click("db_row_0");
    let o = h.key(Key::C, ctrl_shift);
    assert_eq!(
        debug(&o.queries),
        ["CopyDbItem { db: Uniques, name: \"Goldrim\" }"]
    );
    h.click("row_1");
    let o = h.key(Key::C, ctrl_shift);
    let copies: Vec<_> = debug(&o.queries)
        .into_iter()
        .filter(|q| q.starts_with("Copy"))
        .collect();
    assert_eq!(copies, ["CopyItem { id: 1 }"]);
    // the shared list's too (SharedItemListControl.lua:62-64, a ListControl)
    let mut h = Harness::new(super::parity_tests::shared_sample());
    h.settle();
    h.click("shared_row_1");
    let o = h.key(Key::C, ctrl_shift);
    assert!(
        debug(&o.queries).contains(&"CopySharedItem { index: 1 }".to_owned()),
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

fn with_set_lines() -> ItemsView {
    let mut view = sample();
    view.sets[0].row = "Default  ^9(Current)".into();
    view.sets[0].lines = vec!["^7Body Armour: ^xAF6025Kaom's Heart".into()];
    view.sets[1].row = "Alt".into();
    view
}

/// The set dropdown's entries and the manager's rows show PoB's set tooltip, and the active
/// row reads "(Current)" (ItemsTab.lua:106-111, ItemSetListControl.lua:79-90)
#[test]
fn item_sets_show_pobs_tooltip_and_current_marker() {
    let mut h = Harness::new(with_set_lines());
    h.click("set");
    h.frame(vec![]);
    let at = h.ui.rect("set_item_0").center();
    h.frame(vec![Event::PointerMoved(at)]);
    h.settle();
    assert!(
        shown(&h, "Body Armour: Kaom's Heart"),
        "no tooltip on the dropdown entry"
    );
    h.key(Key::Escape, Modifiers::NONE);
    h.click("manage");
    h.frame(vec![]);
    let at = h.ui.rect("sets_row_0").center();
    h.frame(vec![Event::PointerMoved(at)]);
    h.settle();
    assert!(
        shown(&h, "Body Armour: Kaom's Heart"),
        "no tooltip on the row"
    );
    assert!(shown(&h, "Default  (Current)"), "{:?}", h.drawn);
    // "Done" closes the window
    h.click("sets_done");
    assert!(!h.ui.sets.is_open());
}

/// Whether the last frame drew a text holding `text`
fn shown(h: &Harness, text: &str) -> bool {
    h.drawn.iter().any(|t| t.contains(text))
}

/// The list's Delete button acts on the selected row, as the Delete key does, and is disabled
/// without one (ItemListControl.lua:64-69); the empty list shows PoB's text (line 13)
#[test]
fn the_list_has_pobs_delete_button_and_empty_text() {
    let mut h = Harness::new(sample());
    assert!(h.click("delete").commands.is_empty(), "nothing selected");
    h.click("row_1");
    assert_eq!(
        debug(&h.click("delete").commands),
        ["DeleteItem { id: 1, confirmed: false }"]
    );
    // an item in use asks first
    h.click("row_2");
    assert!(h.click("delete").commands.is_empty());
    assert!(h.ui.has_dialog());
    let mut view = sample();
    view.listed.clear();
    let h = Harness::new(view);
    assert!(shown(
        &h,
        "This is the list of items that have been added to this build."
    ));
}

/// A flask's box has PoB's tooltip, and the slot's label PoB's colon (ItemSlotControl.lua:41,
/// 139)
#[test]
fn flask_box_tooltip_and_slot_label_colon() {
    let mut h = Harness::new(sample());
    h.settle();
    assert!(shown(&h, "Flask 1:"), "{:?}", h.drawn);
    let at = h.ui.rect("slot_flask_Flask 1").center();
    h.frame(vec![Event::PointerMoved(at)]);
    h.settle();
    assert!(shown(&h, "Activate this flask."), "{:?}", h.drawn);
}

/// The display panel's sections in PoB's order (ItemsTab.lua:352-1206): the enchant buttons
/// right after the sockets, then influence, quality, catalyst, the cluster jewel, the sorting,
/// the affixes, the custom section's buttons and rows, the range line, the tooltip
#[test]
fn display_sections_follow_pobs_order() {
    let mut h = Harness::new(sample());
    let view = DisplayItemView {
        sockets: rich_view().sockets,
        influences: rich_view().influences,
        catalyst: rich_view().catalyst,
        ..crafted_view()
    };
    h.show_display(view);
    h.settle();
    let y = |h: &Harness, key: &str| h.ui.rect(key).center().y;
    let order = [
        "socket_0",
        "open_Change Enchantment...",
        "influence_0",
        "quality",
        "catalyst",
        "cluster_skill",
        "crafting_sort",
        "affix_0",
        "open_Add modifier...",
        "remove_0",
        "range_lines",
        "tip_line_0",
    ];
    for pair in order.windows(2) {
        assert!(
            y(&h, pair[0]) < y(&h, pair[1]),
            "{} is not above {}",
            pair[0],
            pair[1]
        );
    }
    // the influence dropdowns have PoB's tooltip
    let at = h.ui.rect("influence_0").center();
    h.frame(vec![Event::PointerMoved(at)]);
    h.settle();
    assert!(shown(&h, "Selecting an influence here"), "{:?}", h.drawn);
}

/// The Foulborn box comes before the slider (ItemsTab.lua:1033-1074), and the line dropdown
/// goes back to the first line whenever PoB rebuilds it (UpdateDisplayItemRangeLines)
#[test]
fn the_range_row_is_pobs_and_resets_with_pob() {
    let mut h = Harness::new(sample());
    let mut view = display_view();
    view.range_lines[1].slider = true;
    h.show_display(view.clone());
    h.click("range_lines");
    h.click("range_line_1");
    h.frame(vec![]);
    assert!(h.ui.rect("range_mutated").center().x < h.ui.rect("range_slider").center().x);
    // a view after an edit that keeps PoB's list leaves the pick alone
    h.show_display(view.clone());
    assert_eq!(h.ui.display.range_line, 1);
    // one after PoB rebuilt the list (a Foulborn toggle, a variant) puts it back on line 0
    view.range_resets += 1;
    h.show_display(view);
    assert_eq!(h.ui.display.range_line, 0);
}

/// The empty panel's buttons are "Craft item..." then "Create custom...", with PoB's hint
/// (ItemsTab.lua:301-322); the tree dropdown sits between the flasks and the jewels
/// (ItemsTab.lua:193-216)
#[test]
fn empty_panel_and_slot_column_order() {
    let mut view = sample();
    view.specs = Some(DropList {
        labels: vec!["Main".into()],
        selected: 0,
        enabled: false,
    });
    let mut h = Harness::new(view);
    h.settle();
    assert!(h.ui.rect("craft").center().x < h.ui.rect("create").center().x);
    assert!(shown(&h, "Double-click an item from one of the lists,"));
    let y = |key: &str| h.ui.rect(key).center().y;
    assert!(y("slot_Flask 1") < y("spec_combo"));
    assert!(y("spec_combo") < y("slot_Jewel 5"));
}

/// The raw-text window is PoB's: its titles, "Create" for a new item, and a Save enabled only
/// while PoB reads the text as an item, with the item's tooltip or PoB's hint
/// (ItemsTab.lua:2606-2658). The text is only checked (a parse, PoB's `save.enabled`) after a
/// pause; the item's tooltip, which calculates, is asked for only while Save is hovered.
#[test]
fn the_text_window_asks_pob_whether_save_is_enabled() {
    let mut h = Harness::new(sample());
    h.settle();
    h.click("create");
    let asked = h.frame(vec![]).queries;
    h.frame(vec![]);
    assert!(shown(&h, "Create Custom Item from Text"), "{:?}", h.drawn);
    assert!(shown(&h, "Create"));
    let check = ControlTooltipQuery::ItemTextCheck {
        rarity: 2,
        text: String::new(),
    };
    let full = ControlTooltipQuery::ItemText {
        rarity: 2,
        text: String::new(),
    };
    // the first text of a window is checked at once
    assert_eq!(h.ui.display.text_asked.as_ref(), Some(&check), "{asked:?}");
    assert_eq!(
        h.ui.display.text_tip_asked, None,
        "no tooltip before Save is hovered"
    );
    let reply = |query: ControlTooltipQuery, first: &str| {
        Snapshot::ControlTooltip(Arc::new(crate::engine::views::ControlTooltipView {
            query,
            tooltip: ItemTooltip {
                lines: if first.is_empty() {
                    Vec::new()
                } else {
                    vec![tip_line(first, None)]
                },
                ..ItemTooltip::default()
            },
        }))
    };
    h.ui.on_snapshot(&reply(check.clone(), "The item is invalid."));
    h.frame(vec![]);
    let sent = h.click("text_save");
    assert!(sent.commands.is_empty(), "Save is disabled: {sent:?}");
    assert!(
        !debug(&sent.queries)
            .iter()
            .any(|q| q.contains("ItemText {")),
        "the hint came with the check: {:?}",
        sent.queries
    );
    // a readable text: hovering Save asks for the item's tooltip, once
    h.frame(vec![Event::PointerMoved(Pos2::new(5.0, 5.0))]);
    h.ui.on_snapshot(&reply(check, ""));
    assert!(h.frame(vec![]).queries.is_empty(), "Save isn't hovered");
    let pos = h.ui.rect("text_save").center();
    let mut queries = h.frame(vec![Event::PointerMoved(pos)]).queries;
    queries.extend(h.frame(vec![]).queries);
    queries.extend(h.frame(vec![]).queries);
    assert_eq!(debug(&queries), [format!("ControlTooltip({full:?})")]);
    h.ui.on_snapshot(&reply(full, "Doom Loop"));
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(shown(&h, "Doom Loop"), "{:?}", h.drawn);
    assert_eq!(
        debug(&h.click("text_save").commands),
        ["SetDisplayItemText { rarity: 2, text: \"\" }"]
    );
}

/// The engine answers only the latest control tooltip question. Save hovered while the text
/// is new asks for the item's tooltip, whose answer carries the verdict too: the check isn't
/// asked over it, so that answer is not lost.
#[test]
fn a_text_hovered_on_save_is_asked_once() {
    let mut h = Harness::new(sample());
    h.settle();
    h.click("create");
    h.settle();
    let save = h.ui.rect("text_save").center();
    h.click("text_edit");
    h.frame(vec![Event::PointerMoved(save)]);
    let mut queries = h.frame(vec![Event::Text("hello".into())]).queries;
    for _ in 0..4 {
        queries.extend(h.frame(vec![]).queries);
    }
    let full = ControlTooltipQuery::ItemText {
        rarity: 2,
        text: "hello".into(),
    };
    let hello: Vec<_> = debug(&queries)
        .into_iter()
        .filter(|q| q.contains("hello"))
        .collect();
    assert_eq!(hello, [format!("ControlTooltip({full:?})")]);
    // only that answer arrives: it disables Save
    h.ui.on_snapshot(&Snapshot::ControlTooltip(Arc::new(
        crate::engine::views::ControlTooltipView {
            query: full,
            tooltip: ItemTooltip {
                lines: vec![tip_line("The item is invalid.", None)],
                ..ItemTooltip::default()
            },
        },
    )));
    h.frame(vec![]);
    let sent = h.click("text_save").commands;
    assert!(sent.is_empty(), "Save is disabled: {sent:?}");
}

/// The set row and the slot column carry PoB's labels: "Item set:", "Equipped items:" and
/// "Weapon Set:" with its I and II (ItemsTab.lua:112, 217-254)
#[test]
fn the_set_row_and_slot_header_have_pobs_labels() {
    let mut h = Harness::new(sample());
    h.settle();
    for text in ["Item set:", "Equipped items:", "Weapon Set:"] {
        assert!(shown(&h, text), "{text}: {:?}", h.drawn);
    }
    assert!(h.ui.rect("weapons_I").center().x < h.ui.rect("weapons_II").center().x);
    assert_eq!(
        debug(&h.click("weapons_II").commands),
        ["SelectWeaponSet { second: true }"]
    );
}

/// The socket dropdowns' entries in PoB's colours: STRENGTH, DEXTERITY, INTELLIGENCE, SCION
/// (ItemsTab.lua:28-33, Global.lua:33-35, 32, 66-68)
#[test]
fn socket_entries_have_pobs_colours() {
    use super::display::socket_label;
    assert_eq!(
        ["R", "G", "B", "W"].map(socket_label),
        ["^xE05030R", "^x70FF70G", "^x7070FFB", "^xFFF0F0W"]
    );
}

/// PoB's node count slider keeps its unrounded value: wheel notches of 0.05 add up until the
/// rounded count moves (SliderControl.lua:45-53, 164-180; ItemsTab.lua:711-716). Ours snapped
/// each notch back, so the wheel never moved it.
#[test]
fn wheel_notches_add_up_on_the_cluster_count_slider() {
    let mut h = Harness::new(sample());
    h.show_display(crafted_view());
    h.settle();
    let at = h.ui.rect("cluster_count").center();
    h.frame(vec![Event::PointerMoved(at)]);
    let mut sent = Vec::new();
    for _ in 0..3 {
        sent.extend(h.frame(vec![wheel(1.0)]).commands);
    }
    // 8..=12: three notches are 0.15 of the travel, 0.6 of a node, rounded to one
    assert_eq!(debug(&sent), ["SetClusterNodeCount { count: 9 }"]);
}

/// A jewel socket's slot shows PoB's view of the socket's place in the tree while it is
/// pointed at (ItemSlotControl.lua:142-153, ItemSlotHelper.DrawViewer); a gear slot shows none
#[test]
fn a_jewel_slot_shows_the_socket_viewer() {
    use crate::engine::views::{NodeArt, TreeLayout, TreeNode};
    let node = |id: u32, x: f32| TreeNode {
        id,
        pos: [x, 0.0],
        kind: "Socket".into(),
        name: String::new(),
        stats: Vec::new(),
        orbit: None,
        group_center: None,
        art: NodeArt {
            icon: [None, None],
            frame: [None, None],
        },
        ascendancy: None,
        replaceable: false,
        mastery_effects: false,
    };
    let mut view = sample();
    view.slots.last_mut().unwrap().node = Some(5);
    let mut h = Harness::new(view);
    h.ui.set_socket_scene(Some(crate::ui::tree::SocketScene {
        layout: Arc::new(TreeLayout {
            version: "3_29".into(),
            nodes: vec![node(5, 0.0), node(6, 300.0), node(7, 9000.0)],
            edges: vec![(0, 1)],
            sheets: Vec::new(),
            key: String::new(),
        }),
        allocated: Arc::new([5].into_iter().collect()),
    }));
    h.settle();
    let at = h.ui.rect("slot_Ring 1").center();
    h.frame(vec![Event::PointerMoved(at)]);
    assert!(!h.ui.has_rect("socket_viewer"), "a gear slot has no viewer");
    let at = h.ui.rect("slot_combo_Jewel 5").center();
    h.frame(vec![Event::PointerMoved(at)]);
    h.frame(vec![]);
    assert!(h.ui.has_rect("socket_viewer"));
    let viewer = h.ui.rect("socket_viewer");
    assert_eq!(viewer.size(), egui::vec2(308.0, 280.0));
    // dropped, the viewer stays clear of the slot's list
    h.click("slot_combo_Jewel 5");
    h.frame(vec![]);
    let button = h.ui.rect("slot_combo_Jewel 5");
    let list = h.ui.rect("slot_Jewel 5_item_4");
    let viewer = h.ui.rect("socket_viewer");
    assert!(!viewer.intersects(list), "{viewer:?} covers {list:?}");
    assert!(!viewer.intersects(button), "{viewer:?} covers {button:?}");
}

/// Enter in the set name box presses an enabled Save, as PoB's popup opens with "save" as its
/// enter control (ItemSetListControl.lua:70)
#[test]
fn enter_saves_a_set_name() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    h.key(Key::F2, Modifiers::NONE);
    h.frame(vec![]);
    h.type_into("sets_name", "Alt2");
    let o = h.key(Key::Enter, Modifiers::NONE);
    let mut sent = debug(&o.commands);
    sent.extend(debug(&h.frame(vec![]).commands));
    assert_eq!(sent, ["RenameItemSet { id: 2, title: \"Alt2\" }"]);
}

/// Esc leaves the database search box with its text (EditControl.lua:505): what was typed is
/// searched, not dropped
#[test]
fn escape_keeps_the_db_search() {
    let mut h = Harness::new(sample());
    h.open_db();
    h.step = 0.05;
    h.type_into("db_search", "kaom");
    let o = h.key(Key::Escape, Modifiers::NONE);
    let mut sent = debug(&o.commands);
    sent.extend(debug(&h.frame(vec![]).commands));
    assert!(
        sent.iter().any(|c| c.contains("search: \"kaom\"")),
        "{sent:?}"
    );
    assert_eq!(h.ui.db.search.text, "kaom");
}

/// A row the keys select is scrolled into view, as PoB's ScrollSelIntoView
#[test]
fn the_db_keys_scroll_the_selection_into_view() {
    let mut view = db_view(ItemDb::Uniques);
    view.rows = (0..200).map(|i| db_row(&format!("Item {i:03}"))).collect();
    let mut h = Harness::new(sample());
    h.settle();
    h.click("db_header");
    h.show_db(view);
    h.settle();
    h.click("db_row_0");
    h.key(Key::End, Modifiers::NONE);
    h.settle();
    assert!(
        h.ui.db.rects.map.contains_key("db_row_199"),
        "the last row is not drawn"
    );
}
