// PoB's shortcuts (R25) and the Help window, through the app: one test per key. Included in
// app.rs's tests module, whose helpers it uses.

use super::*;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2};

fn release(key: Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn plain(key: Key) -> Vec<Event> {
    key_with(key, Modifiers::NONE)
}

fn mouse4() -> Vec<Event> {
    vec![Event::PointerButton {
        pos: Pos2::new(400.0, 400.0),
        button: PointerButton::Extra1,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]
}

/// The URLs egui was asked to open over `events`.
fn opened(app: &mut AutoPobApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<String> {
    let input = egui::RawInput {
        events,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| app.show(ui));
    output
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

/// The text egui was asked to copy over `events`.
fn copied(app: &mut AutoPobApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<String> {
    let input = egui::RawInput {
        events,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| app.show(ui));
    output
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_digit_switches_tabs() {
    let (mut app, ctx) = tree_app();
    for (key, tab) in [
        (Key::Num3, BuildTab::Items),
        (Key::Num2, BuildTab::Skills),
        (Key::Num4, BuildTab::Calcs),
        (Key::Num5, BuildTab::Config),
        (Key::Num6, BuildTab::Notes),
        (Key::Num1, BuildTab::Tree),
    ] {
        frame(&mut app, &ctx, ctrl(key));
        assert_eq!(app.tab, tab, "Ctrl+{key:?}");
    }
    // Ctrl+7 is PoB's Party tab, which is out (R25)
    frame(&mut app, &ctx, ctrl(Key::Num7));
    assert_eq!(app.tab, BuildTab::Tree);
}

#[test]
fn ctrl_i_opens_import() {
    let (mut app, ctx) = tree_app();
    frame(&mut app, &ctx, ctrl(Key::I));
    assert_eq!(app.tab, BuildTab::Import);
    // the code box has the keyboard (Build.lua:1163)
    frame(&mut app, &ctx, vec![]);
    assert!(ctx.text_edit_focused(), "the code box isn't selected");
}

#[test]
fn ctrl_w_closes_with_unsaved_prompt() {
    let (mut app, ctx) = open_new_build();
    // a saved build closes at once
    frame(&mut app, &ctx, ctrl(Key::W));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.screen, Screen::List);

    let (mut app, ctx) = open_new_build();
    app.send(Command::SetConfig {
        var: "x".into(),
        value: ConfigValue::Bool(true),
    });
    pump_until(&mut app, &ctx, "unsaved", |a| {
        a.build.as_ref().is_some_and(|b| b.unsaved)
    });
    frame(&mut app, &ctx, ctrl(Key::W));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.prompt, Some(Pending::BackToList));
    assert_eq!(app.screen, Screen::Build);
}

#[test]
fn ctrl_f_focuses_the_tab_search() {
    let (mut app, ctx) = tree_app();
    assert!(!ctx.text_edit_focused());
    frame(&mut app, &ctx, ctrl(Key::F));
    frame(&mut app, &ctx, vec![]);
    let search = ctx.memory(|m| m.focused());
    assert!(ctx.text_edit_focused(), "the tree search isn't selected");
    let field = rect_of(&app, "search");
    assert!(
        ctx.read_response(search.unwrap())
            .is_some_and(|r| r.rect == field)
    );

    // the Config tab's search too
    let (mut app, ctx) = config_app(sample_config());
    assert!(!ctx.text_edit_focused());
    frame(&mut app, &ctx, ctrl(Key::F));
    frame(&mut app, &ctx, vec![]);
    assert!(ctx.text_edit_focused(), "the config search isn't selected");

    // the Items tab: the database panel opens on the uniques with its search selected; from
    // there Ctrl+F goes to the rares (ItemsTab.lua:1471-1480)
    let (mut app, ctx) = items_app();
    let before = app.sent_full.len();
    frame(&mut app, &ctx, ctrl(Key::F));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(
        ctx.text_edit_focused(),
        "the database search isn't selected"
    );
    let sent = sent_since(&app, before).join("\n");
    assert!(
        sent.contains("SetItemDb") && sent.contains("Uniques"),
        "{sent}"
    );
    let before = app.sent_full.len();
    frame(&mut app, &ctx, ctrl(Key::F));
    frame(&mut app, &ctx, vec![]);
    let sent = sent_since(&app, before).join("\n");
    assert!(
        sent.contains("SetItemDb") && sent.contains("Rares"),
        "{sent}"
    );
    assert!(ctx.text_edit_focused());
}

#[test]
fn ctrl_m_opens_spec_manager() {
    let (mut app, ctx) = tree_app();
    assert!(!egui::Popup::is_any_open(&ctx));
    frame(&mut app, &ctx, ctrl(Key::M));
    frame(&mut app, &ctx, vec![]);
    assert!(egui::Popup::is_any_open(&ctx), "the tree list didn't open");
    // its entries are drawn: the trees to pick
    assert!(app.tree.rects.map.contains_key("spec:1"));
}

#[test]
fn up_down_switch_specs() {
    let (mut app, ctx) = tree_app();
    // the first of two trees: Up has nowhere to go
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::ArrowUp));
    assert!(sent_since(&app, before).is_empty());
    frame(&mut app, &ctx, plain(Key::ArrowDown));
    assert_eq!(sent_since(&app, before), ["SelectSpec { index: 1 }"]);

    // with the pointer on the tree selector the keys leave the trees alone
    let combo = rect_of(&app, "spec_combo").center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(combo)]);
    frame(&mut app, &ctx, vec![]);
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::ArrowDown));
    assert!(sent_since(&app, before).is_empty());
}

#[test]
fn p_toggles_heatmap() {
    let (mut app, ctx) = tree_app();
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::P));
    assert_eq!(sent_since(&app, before), ["ToggleHeatmap"]);
    // only on the Tree tab
    app.tab = BuildTab::Calcs;
    frame(&mut app, &ctx, vec![]);
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::P));
    assert!(sent_since(&app, before).is_empty());
}

#[test]
fn ctrl_d_toggles_tree_stat_differences() {
    let (mut app, ctx) = tree_app();
    let before = app.sent_full.len();
    frame(&mut app, &ctx, ctrl(Key::D));
    // the fake's tree has them off
    assert_eq!(
        sent_since(&app, before),
        ["SetTreeStatDifferences { on: true }"]
    );
}

/// A tree whose node 1 has stat lines and node 2 is a jewel socket
fn tree_with_text() -> (AutoPobApp, egui::Context) {
    let (mut app, ctx) = tree_app();
    let node = |id, kind: &str, stats: &[&str]| crate::engine::TreeNode {
        id,
        pos: [id as f32 * 100.0, 0.0],
        kind: kind.into(),
        name: format!("Node {id}"),
        stats: stats.iter().map(|s| s.to_string()).collect(),
        orbit: None,
        group_center: None,
        art: crate::engine::NodeArt::default(),
        ascendancy: None,
        replaceable: false,
        mastery_effects: false,
    };
    app.on_event(Event_::Snapshot {
        seq: app.last_sent,
        snapshot: Snapshot::TreeLayout(Arc::new(crate::engine::TreeLayout {
            version: "3_28".into(),
            nodes: vec![
                node(
                    1,
                    "Notable",
                    &["+10 to Strength", "5% increased maximum Life"],
                ),
                node(2, "Socket", &[]),
            ],
            edges: vec![],
            sheets: vec![],
            key: "text".into(),
        })),
    });
    frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

use crate::engine::Event as Event_;

#[test]
fn ctrl_c_copies_hovered_node_text() {
    let (mut app, ctx) = tree_with_text();
    app.tree.hover_for_test(Some(1));
    // egui-winit sends Ctrl+C as a copy event
    assert_eq!(
        copied(&mut app, &ctx, vec![Event::Copy]),
        ["# Node 1\n+10 to Strength\n5% increased maximum Life\n"]
    );
    // the key itself does too
    app.tree.hover_for_test(Some(1));
    assert_eq!(copied(&mut app, &ctx, ctrl(Key::C)).len(), 1);
    // nothing over a jewel socket or over no node
    app.tree.hover_for_test(Some(2));
    assert!(copied(&mut app, &ctx, vec![Event::Copy]).is_empty());
    app.tree.hover_for_test(None);
    assert!(copied(&mut app, &ctx, vec![Event::Copy]).is_empty());
}

#[test]
fn page_keys_zoom() {
    let (mut app, ctx) = tree_app();
    let start = app.tree.zoom_for_test();
    // Shift: three steps (PassiveTreeView.lua:214-217)
    frame(&mut app, &ctx, key_with(Key::PageDown, Modifiers::SHIFT));
    let down = app.tree.zoom_for_test();
    assert!(
        (start / down - 1.2f32.powi(3)).abs() < 1e-3,
        "{start} -> {down}"
    );
    frame(&mut app, &ctx, plain(Key::PageUp));
    let up = app.tree.zoom_for_test();
    assert!((up / down - 1.2).abs() < 1e-3, "{down} -> {up}");
}

#[test]
fn e_edits_hovered_slot_item() {
    let (mut app, ctx) = items_app();
    // over an empty slot nothing happens
    let empty = app.items.rect("slot_combo_Helmet").center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(empty)]);
    frame(&mut app, &ctx, vec![]);
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::E));
    assert!(sent_since(&app, before).is_empty());
    // over the body armour's slot, its item opens for editing
    let body = app.items.rect("slot_combo_Body Armour").center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(body)]);
    frame(&mut app, &ctx, vec![]);
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::E));
    assert_eq!(sent_since(&app, before), ["EditItem { id: 2 }"]);
}

#[test]
fn mouse4_goes_back() {
    let (mut app, ctx) = open_new_build();
    frame(&mut app, &ctx, mouse4());
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.screen, Screen::List);
}

#[test]
fn f1_opens_help() {
    // on the list: the first section ("Build List tab" matches none)
    let (mut app, ctx) = started();
    assert!(!app.help.is_open());
    frame(&mut app, &ctx, release(Key::F1));
    assert!(app.help.is_open());
    pump_until(&mut app, &ctx, "help", |a| a.help.target().is_some());
    assert_eq!(app.help.target(), Some((false, 0)));

    // on the Skills tab: PoB's "Skills Tab" section
    let (mut app, ctx) = skills_app();
    frame(&mut app, &ctx, release(Key::F1));
    pump_until(&mut app, &ctx, "help", |a| a.help.target().is_some());
    assert_eq!(app.help.target(), Some((false, 8)));
    // asked once: the window keeps PoB's text
    let asked = app.sent_log.iter().filter(|c| *c == "Help").count();
    assert_eq!(asked, 1);
    app.help.close();
    frame(&mut app, &ctx, release(Key::F1));
    assert_eq!(app.sent_log.iter().filter(|c| *c == "Help").count(), 1);

    // the loadouts' "Help >>" asks for the seventh section, Loadouts (Build.lua:312)
    app.open_help(HelpStart::Section(7));
    assert_eq!(app.help.target(), Some((false, 6)));
}

#[test]
fn f1_over_a_node_opens_its_wiki_page() {
    let (mut app, ctx) = tree_with_text();
    app.tree.hover_for_test(Some(1));
    assert_eq!(
        opened(&mut app, &ctx, release(Key::F1)),
        ["https://www.poewiki.net/wiki/Node_1"]
    );
    assert!(!app.help.is_open());
}

#[test]
fn shortcuts_ignored_while_typing() {
    let (mut app, ctx) = tree_app();
    click(&mut app, &ctx, "search");
    assert!(ctx.text_edit_focused());
    let before = app.sent_full.len();
    // the tree view's keys and F1 go to the selected search box, as in PoB
    for events in [plain(Key::P), ctrl(Key::D), release(Key::F1)] {
        frame(&mut app, &ctx, events);
    }
    assert!(!app.help.is_open(), "F1 opened Help while typing");
    assert!(
        !sent_since(&app, before)
            .iter()
            .any(|c| c.starts_with("ToggleHeatmap") || c.starts_with("SetTreeStat")),
        "{:?}",
        sent_since(&app, before)
    );
    // ... and the box still gets the typed letter
    assert!(ctx.text_edit_focused());
    // TreeTab's own Down comes before its controls (TreeTab.lua:365-377)
    frame(&mut app, &ctx, plain(Key::ArrowDown));
    assert!(sent_since(&app, before).contains(&"SelectSpec { index: 1 }"));
    // Build's Ctrl+2 too (Build.lua:1175)
    frame(&mut app, &ctx, ctrl(Key::Num2));
    assert_eq!(app.tab, BuildTab::Skills);
    // and the mouse's back button
    frame(&mut app, &ctx, mouse4());
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.screen, Screen::List);
}

#[test]
fn shortcuts_wait_under_a_modal() {
    let (mut app, ctx) = tree_app();
    app.prompt = Some(Pending::BackToList);
    let before = app.sent_full.len();
    frame(&mut app, &ctx, plain(Key::P));
    frame(&mut app, &ctx, ctrl(Key::Num3));
    assert_eq!(app.tab, BuildTab::Tree);
    assert!(sent_since(&app, before).is_empty());
}

#[test]
fn the_about_buttons_open_the_version_history() {
    let (mut app, ctx) = started();
    app.on_list_action(ListScreenAction::About);
    pump_until(&mut app, &ctx, "help", |a| a.help.target().is_some());
    assert_eq!(app.help.target(), Some((true, 0)));
}

#[test]
fn f1_over_a_socket_group_opens_its_first_gems_page() {
    let (mut app, ctx) = skills_app();
    let row = app.skills.rect("row_0").center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(row)]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        opened(&mut app, &ctx, release(Key::F1)),
        ["https://www.poewiki.net/wiki/Boneshatter"]
    );
    assert!(!app.help.is_open());
}

#[test]
fn f1_over_an_item_opens_its_page() {
    let (mut app, ctx) = items_app();
    // a rare's page is its base's (ItemTools.lua:395-399)
    let body = app.items.rect("slot_combo_Body Armour").center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(body)]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        opened(&mut app, &ctx, release(Key::F1)),
        ["https://www.poewiki.net/wiki/Base"]
    );
}

#[test]
fn f2_renames_the_active_tree_and_config_set() {
    let (mut app, ctx) = tree_app();
    assert!(!app.tree.has_dialog());
    frame(&mut app, &ctx, plain(Key::F2));
    assert!(app.tree.has_dialog(), "no Rename box");

    let (mut app, ctx) = config_app(sample_config());
    frame(&mut app, &ctx, plain(Key::F2));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    // the set's Rename box has the keyboard (config.rs set_edit)
    assert!(ctx.text_edit_focused(), "no Rename box");
}

#[test]
fn the_help_windows_button_fetches_missing_files_only_on_its_click() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (app, ctx) = started();
    let mut app = app.with_help_download(Box::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok(())).unwrap();
        rx
    }));
    // what the engine answers on an install without the files
    app.help.open(HelpStart::Section(1));
    app.help.on_view(Arc::new(crate::engine::HelpView {
        missing: vec!["help.txt".into(), "changelog.txt".into()],
        ..Default::default()
    }));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "fetched without a click");
    let pos = app.help.download_rect.expect("no download button").center();
    let press = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(pos)]);
    frame(&mut app, &ctx, vec![press(true)]);
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // the files arrived: PoB's help is read again
    pump_until(&mut app, &ctx, "help read again", |a| {
        a.help.view_for_test().is_some_and(|v| v.missing.is_empty())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn the_help_window_is_modal() {
    let (mut app, ctx) = tree_app();
    frame(&mut app, &ctx, release(Key::F1));
    assert!(app.help.is_open());
    // PoB's About is a popup: nothing behind it gets a key (Main.lua:365-367)
    let before = app.sent_full.len();
    frame(&mut app, &ctx, ctrl(Key::Num3));
    frame(&mut app, &ctx, plain(Key::P));
    assert_eq!(app.tab, BuildTab::Tree);
    assert!(sent_since(&app, before).is_empty());
    frame(&mut app, &ctx, plain(Key::Escape));
    frame(&mut app, &ctx, vec![]);
    assert!(!app.help.is_open(), "Esc closes it");
}

#[test]
fn a_calcs_count_being_typed_asks_before_mouse4_leaves() {
    let (mut app, ctx) = notes_app();
    let mut view = crate::engine::views::CalcsView::default();
    view.selector.stages = Some("1".into());
    app.calcs.on_snapshot(&Snapshot::Calcs(Arc::new(view)));
    app.tab = BuildTab::Calcs;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(!app.current_unsaved());
    let pos = app.calcs.rects.map["stages"].center();
    let press = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    for events in [
        vec![Event::PointerMoved(pos)],
        vec![press(true)],
        vec![press(false)],
        vec![Event::Text("3".into())],
    ] {
        frame(&mut app, &ctx, events);
    }
    // PoB's count box marks the build changed as it is typed in
    assert!(app.current_unsaved());
    frame(&mut app, &ctx, mouse4());
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.prompt, Some(Pending::BackToList));
    assert_eq!(app.screen, Screen::Build);
}

#[test]
fn pobs_fork_and_version_labels_show_on_every_screen() {
    let (mut app, ctx) = started();
    frame(&mut app, &ctx, vec![]);
    let want = ["PoB Community Fork", "Version: 2.67.2"];
    assert_eq!(app.label_texts, want);
    let (mut app, ctx) = tree_app();
    app.label_texts.clear();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.label_texts, want);
}

#[test]
fn a_right_click_into_a_text_box_pastes_the_clipboard() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let (app, ctx) = tree_app();
    let mut app = app.with_clipboard_reader(Box::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        Some("life".into())
    }));
    // a right-click away from any box reads nothing
    let away = rect_of(&app, "reset").center();
    let right = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(&mut app, &ctx, vec![Event::PointerMoved(away)]);
    frame(&mut app, &ctx, vec![right(away, true)]);
    frame(&mut app, &ctx, vec![right(away, false)]);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    // on the tree search it selects the box and pastes (EditControl.lua:527)
    let search = rect_of(&app, "search").center();
    frame(&mut app, &ctx, vec![Event::PointerMoved(search)]);
    frame(&mut app, &ctx, vec![right(search, true)]);
    frame(&mut app, &ctx, vec![right(search, false)]);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert!(ctx.text_edit_focused());
    pump_until(&mut app, &ctx, "the pasted search", |a| {
        a.sent_full
            .iter()
            .any(|c| c == "SetTreeSearch { text: \"life\" }")
    });
}
