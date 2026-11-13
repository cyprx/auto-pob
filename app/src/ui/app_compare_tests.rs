//! The app around the Compare tab: following PoB's tab switch, the overlay's on and off, and
//! what leaving the tab sends.

use super::*;
use crate::engine::Backend;
use crate::engine::fake::Fake;
use crate::engine::views::CompareView;
use crate::ui::compare::tests::sample;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use std::time::{Duration, Instant};

fn frame(app: &mut AutoPobApp, ctx: &egui::Context, events: Vec<Event>) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
        events,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| app.show(ui))
        .drop_without_applying_deltas();
}

/// Runs frames until the engine has answered everything sent.
fn settle(app: &mut AutoPobApp, ctx: &egui::Context) {
    let start = Instant::now();
    loop {
        frame(app, ctx, vec![]);
        if app.ready && !app.behind() && !app.compare_waiting() && app.entering.is_none() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "engine behind");
        std::thread::sleep(Duration::from_millis(5));
    }
    // late snapshots of the last command
    std::thread::sleep(Duration::from_millis(20));
    frame(app, ctx, vec![]);
}

/// A new build on the build screen, everything answered. The engine answers the Compare
/// view with `sample()`.
fn open_build() -> (AutoPobApp, egui::Context) {
    let mut app = AutoPobApp::new(Box::new(|| {
        Engine::spawn_with(
            |_| {
                let mut fake = Fake::new().0;
                fake.compare = Some(sample());
                Ok(Box::new(fake) as Box<dyn Backend>)
            },
            || {},
        )
    }));
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);
    app.enter_build(Command::NewBuild);
    settle(&mut app, &ctx);
    assert_eq!(app.screen, Screen::Build);
    (app, ctx)
}

fn inject(app: &mut AutoPobApp, view: Option<CompareView>) {
    app.on_event(Event_::Snapshot {
        seq: app.last_sent,
        snapshot: Snapshot::Compare(view.map(Arc::new)),
    });
}

use crate::engine::Event as Event_;

/// The Compare tab entered and its answer in (`sample()`)
fn show_compare(app: &mut AutoPobApp, ctx: &egui::Context) {
    app.tab = BuildTab::Compare;
    settle(app, ctx);
    frame(app, ctx, vec![]);
}

fn click(app: &mut AutoPobApp, ctx: &egui::Context, key: &str) {
    let pos = app.compare.rect(key).center();
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    for events in [
        vec![Event::PointerMoved(pos)],
        vec![button(true)],
        vec![button(false)],
        vec![],
    ] {
        frame(app, ctx, events);
    }
}

/// The overlay commands sent since `before`.
fn overlays(app: &AutoPobApp, before: usize) -> Vec<&str> {
    app.sent_full[before..]
        .iter()
        .map(String::as_str)
        .filter(|c| c.starts_with("SetCompareOverlay"))
        .collect()
}

#[test]
fn tab_order_puts_compare_before_import_and_it_has_no_undo() {
    let (mut app, ctx) = open_build();
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    assert!(!BuildTab::Compare.has_undo());
    let before = app.sent_full.len();
    frame(
        &mut app,
        &ctx,
        vec![Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
    );
    app.undo_redo(false);
    assert!(
        app.sent_full[before..].is_empty(),
        "{:?}",
        &app.sent_full[before..]
    );
}

#[test]
fn pobs_switch_to_its_compare_tab_is_followed() {
    let (mut app, ctx) = open_build();
    app.tab = BuildTab::Import;
    let mut view = sample();
    view.switched = true;
    inject(&mut app, Some(view));
    assert_eq!(app.tab, BuildTab::Compare);
    // only once: the user may leave again
    app.tab = BuildTab::Items;
    inject(&mut app, Some(sample()));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.tab, BuildTab::Items);
}

#[test]
fn a_comparison_landing_under_a_window_keeps_its_tab() {
    // PoB switches at the user's Import click; a link's comparison lands later, and the
    // switch would end the window the user has open by then
    let (mut app, ctx) = open_build();
    app.tab = BuildTab::Items;
    frame(&mut app, &ctx, vec![]);
    app.items.open_text_window_for_test();
    frame(&mut app, &ctx, vec![]);
    let mut view = sample();
    view.switched = true;
    inject(&mut app, Some(view));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.tab, BuildTab::Items);
    assert!(app.items.text_window_open(), "the text window was dropped");
    assert!(
        app.notices.iter().any(|n| n.contains("Compare tab")),
        "{:?}",
        app.notices
    );
}

#[test]
fn the_tree_view_turns_the_overlay_on_and_off() {
    let (mut app, ctx) = open_build();
    show_compare(&mut app, &ctx);
    let before = app.sent_full.len();
    click(&mut app, &ctx, "sub:tree");
    assert_eq!(overlays(&app, before), ["SetCompareOverlay { on: true }"]);
    // our tree is drawn under the Compare header
    assert!(app.tree.rects.map.contains_key("spec_combo"));
    // leaving the tab turns it off, coming back on
    let before = app.sent_full.len();
    app.tab = BuildTab::Items;
    frame(&mut app, &ctx, vec![]);
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        overlays(&app, before),
        [
            "SetCompareOverlay { on: false }",
            "SetCompareOverlay { on: true }"
        ]
    );
    // another sub-view turns it off
    let before = app.sent_full.len();
    click(&mut app, &ctx, "sub:items");
    assert_eq!(overlays(&app, before), ["SetCompareOverlay { on: false }"]);
    // the last comparison removed while the Tree view shows
    click(&mut app, &ctx, "sub:tree");
    let before = app.sent_full.len();
    inject(&mut app, None);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(overlays(&app, before), ["SetCompareOverlay { on: false }"]);
    // a build change drops the overlay in the engine: nothing is sent for it
    inject(&mut app, Some(sample()));
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "sub:tree");
    assert!(app.compare_overlay);
    let before = app.sent_full.len();
    app.enter_build(Command::NewBuild);
    settle(&mut app, &ctx);
    assert!(!app.compare_overlay);
    assert!(
        overlays(&app, before).is_empty(),
        "{:?}",
        overlays(&app, before)
    );
}

#[test]
fn leaving_the_tab_sends_a_typed_count_and_closes_the_load_window() {
    let (mut app, ctx) = open_build();
    show_compare(&mut app, &ctx);
    click(&mut app, &ctx, "load");
    click(&mut app, &ctx, "load:code");
    assert!(app.compare.has_dialog());
    click(&mut app, &ctx, "edit_cmpStageCount");
    frame(&mut app, &ctx, vec![Event::Text("4".into())]);
    assert!(app.compare.has_pending());
    let before = app.sent_full.len();
    app.tab = BuildTab::Notes;
    frame(&mut app, &ctx, vec![]);
    assert_eq!(
        &app.sent_full[before..],
        ["SetCompareControl { name: \"cmpStageCount\", value: Text(\"4\") }"]
    );
    assert!(!app.compare.has_dialog());
}

#[test]
fn a_comparison_import_never_replaces_the_build_screen() {
    let (mut app, ctx) = open_build();
    app.tab = BuildTab::Compare;
    app.send(Command::ImportFromUrl {
        url: "https://pobb.in/abc".into(),
        mode: ImportMode::Comparison,
    });
    assert_eq!(app.import_seq, None);
    settle(&mut app, &ctx);
    assert_eq!(app.tab, BuildTab::Compare);
}

#[test]
fn undo_on_the_compare_tree_view_is_the_tree_tabs() {
    let (mut app, ctx) = open_build();
    show_compare(&mut app, &ctx);
    // on the Summary there is nothing to undo
    app.undo_redo(false);
    click(&mut app, &ctx, "sub:tree");
    let before = app.sent_full.len();
    let ctrl_z = Event::Key {
        key: Key::Z,
        physical_key: Some(Key::Z),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    frame(&mut app, &ctx, vec![ctrl_z]);
    app.undo_redo(true);
    assert_eq!(&app.sent_full[before..], ["Undo(Tree)", "Redo(Tree)"]);
}

#[test]
fn a_comparison_pob_refuses_shows_pobs_text() {
    // CompareTab.lua:1507-1514: a bad code leaves PoB's popup open with its red text
    let (mut app, ctx) = open_build();
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "load");
    click(&mut app, &ctx, "load:code");
    click(&mut app, &ctx, "dialog_text");
    frame(&mut app, &ctx, vec![Event::Text("bad".into())]);
    click(&mut app, &ctx, "dialog_load");
    settle(&mut app, &ctx);
    assert!(app.compare.has_dialog(), "the popup closed");
    assert!(
        app.compare.rects.map.contains_key("dialog_error"),
        "no error in the popup"
    );
    assert!(app.notices.is_empty(), "{:?}", app.notices);
    // a good one closes it
    click(&mut app, &ctx, "dialog_text");
    frame(&mut app, &ctx, vec![Event::Text("x".into())]);
    click(&mut app, &ctx, "dialog_load");
    settle(&mut app, &ctx);
    assert!(!app.compare.has_dialog(), "the popup stayed");
    assert!(app.notices.is_empty(), "{:?}", app.notices);
}

#[test]
fn a_link_import_waits_for_its_download_in_the_popup() {
    // CompareTab.lua:1488-1503: "Downloading..." until the build lands; a failure stays in
    // the popup in red, not as a notice
    let (mut app, ctx) = open_build();
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, "load");
    click(&mut app, &ctx, "load:code");
    click(&mut app, &ctx, "dialog_text");
    frame(
        &mut app,
        &ctx,
        vec![Event::Text("https://pobb.in/abc".into())],
    );
    click(&mut app, &ctx, "dialog_load");
    settle(&mut app, &ctx);
    assert!(app.compare.has_dialog(), "closed before the download ended");
    assert!(app.compare.rects.map.contains_key("dialog_downloading"));
    let finished = |error: Option<&str>| crate::engine::Event::Snapshot {
        seq: 0,
        snapshot: crate::engine::Snapshot::Requests(std::sync::Arc::new(
            crate::engine::views::RequestsView {
                pending: vec![],
                finished: vec![crate::engine::views::FinishedRequest {
                    id: 1,
                    label: "Fetching comparison build".into(),
                    error: error.map(str::to_owned),
                    cancelled: false,
                    new_build: false,
                }],
            },
        )),
    };
    app.on_event(finished(Some("Response code: 500")));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(app.compare.has_dialog());
    assert!(app.compare.rects.map.contains_key("dialog_error"));
    assert!(app.notices.is_empty(), "{:?}", app.notices);
    // pressed again, the download lands: it closes
    click(&mut app, &ctx, "dialog_load");
    settle(&mut app, &ctx);
    app.on_event(finished(None));
    frame(&mut app, &ctx, vec![]);
    assert!(!app.compare.has_dialog());
}

#[test]
fn the_engine_knows_whether_the_compare_tab_shows() {
    let (fake, log) = Fake::new();
    let fake = std::sync::Mutex::new(Some(fake));
    let mut app = AutoPobApp::new(Box::new(move || {
        let fake = fake.lock().unwrap().take().expect("one engine");
        Engine::spawn_with(move |_| Ok(Box::new(fake) as Box<dyn Backend>), || {})
    }));
    let ctx = egui::Context::default();
    let shown = |log: &std::sync::Mutex<crate::engine::fake::Log>| -> Vec<String> {
        log.lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.contains("SetCompareShown"))
            .cloned()
            .collect()
    };
    settle(&mut app, &ctx);
    // the build list has no edits to hold the view back from
    assert!(shown(&log).is_empty());
    // a build opens on its Tree tab
    app.enter_build(Command::NewBuild);
    settle(&mut app, &ctx);
    assert_eq!(shown(&log), ["apply SetCompareShown { on: false }"]);
    app.tab = BuildTab::Compare;
    settle(&mut app, &ctx);
    assert_eq!(
        shown(&log)[1..],
        ["apply SetCompareShown { on: true }".to_owned()]
    );
    app.tab = BuildTab::Config;
    settle(&mut app, &ctx);
    assert_eq!(
        shown(&log)[2..],
        ["apply SetCompareShown { on: false }".to_owned()]
    );
    // PoB's switch to its Compare tab, as an Import-tab comparison lands
    app.tab = BuildTab::Import;
    settle(&mut app, &ctx);
    let mut view = sample();
    view.switched = true;
    inject(&mut app, Some(view));
    assert_eq!(app.tab, BuildTab::Compare);
    settle(&mut app, &ctx);
    assert_eq!(
        shown(&log)[3..],
        ["apply SetCompareShown { on: true }".to_owned()]
    );
    // a tab key (Ctrl+5) leaves it in the frame it is pressed
    frame(
        &mut app,
        &ctx,
        vec![Event::Key {
            key: Key::Num5,
            physical_key: Some(Key::Num5),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
    );
    assert_eq!(app.tab, BuildTab::Config);
    settle(&mut app, &ctx);
    assert_eq!(
        shown(&log)[4..],
        ["apply SetCompareShown { on: false }".to_owned()]
    );
    // back to the list from the Compare tab: the list has no edits, so nothing is said there;
    // the next build opens on its Tree tab, which hides it
    app.tab = BuildTab::Compare;
    settle(&mut app, &ctx);
    app.back_to_list();
    // (the fake leaves the listing unanswered, so no settle here)
    for _ in 0..10 {
        frame(&mut app, &ctx, vec![]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.screen, Screen::List);
    assert_eq!(
        shown(&log)[5..],
        ["apply SetCompareShown { on: true }".to_owned()]
    );
    app.enter_build(Command::NewBuild);
    settle(&mut app, &ctx);
    assert_eq!(app.tab, BuildTab::Tree);
    assert_eq!(
        shown(&log)[6..],
        ["apply SetCompareShown { on: false }".to_owned()]
    );
}

#[test]
fn entering_the_tab_after_a_hidden_edit_waits_for_its_view() {
    // an engine slow to answer the Compare view
    let mut app = AutoPobApp::new(Box::new(|| {
        Engine::spawn_with(
            |_| {
                let mut fake = Fake::new().0;
                fake.compare = Some(sample());
                fake.compare_delay = Duration::from_millis(500);
                Ok(Box::new(fake) as Box<dyn Backend>)
            },
            || {},
        )
    }));
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);
    app.enter_build(Command::NewBuild);
    settle(&mut app, &ctx);
    // an edit on the Tree tab: the engine leaves the Compare view out
    app.send(Command::SetConfig {
        var: "conditionFullLife".into(),
        value: crate::engine::ConfigValue::Bool(true),
    });
    settle(&mut app, &ctx);
    assert!(!app.compare_waiting());
    // the tab's first frame already waits for the fresh view: what it holds is dimmed and
    // its buttons are off
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    assert!(app.compare_waiting());
    let before = app.sent_full.len();
    click(&mut app, &ctx, "remove");
    assert!(app.compare_waiting(), "the view came too soon for the test");
    assert!(
        !app.sent_full[before..]
            .iter()
            .any(|c| c.starts_with("RemoveCompare")),
        "{:?}",
        &app.sent_full[before..]
    );
    // once it is in, they work
    settle(&mut app, &ctx);
    assert!(!app.compare_waiting());
    let before = app.sent_full.len();
    click(&mut app, &ctx, "remove");
    assert_eq!(
        app.sent_full[before..],
        ["RemoveCompare { index: 0 }".to_owned()]
    );
}

/// Entering the tab asks for a fresh view. When that view fails, the tab keeps the one it
/// holds and its controls work again: nothing else is coming for that seq.
#[test]
fn a_failed_view_on_entering_the_tab_leaves_its_controls_on() {
    let fails: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let flag = fails.clone();
    let mut app = AutoPobApp::new(Box::new(move || {
        let flag = flag.clone();
        Engine::spawn_with(
            move |_| {
                let mut fake = Fake::new().0;
                fake.compare = Some(sample());
                fake.compare_fails = flag;
                Ok(Box::new(fake) as Box<dyn Backend>)
            },
            || {},
        )
    }));
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);
    app.enter_build(Command::NewBuild);
    settle(&mut app, &ctx);
    show_compare(&mut app, &ctx);
    app.tab = BuildTab::Config;
    settle(&mut app, &ctx);
    // the next view fails
    fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let errors = app.notices.len();
    app.tab = BuildTab::Compare;
    let start = Instant::now();
    while app.notices.len() == errors {
        assert!(start.elapsed() < Duration::from_secs(10), "no error came");
        frame(&mut app, &ctx, vec![]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.compare_waiting());
    let before = app.sent_full.len();
    click(&mut app, &ctx, "remove");
    assert_eq!(
        app.sent_full[before..],
        ["RemoveCompare { index: 0 }".to_owned()],
        "the tab's controls stayed off"
    );
    // a command of the tab whose Compare view fails too: its tree view answers the batch
    // (`compare_changed`), so the controls come back as well
    // (the same error counts on the notice already up)
    let start = Instant::now();
    while app.behind() {
        assert!(start.elapsed() < Duration::from_secs(10), "no answer came");
        frame(&mut app, &ctx, vec![]);
        std::thread::sleep(Duration::from_millis(5));
    }
    let before = app.sent_full.len();
    click(&mut app, &ctx, "remove");
    assert_eq!(
        app.sent_full[before..],
        ["RemoveCompare { index: 0 }".to_owned()],
        "the controls stayed off after a failed view of the tab's own command"
    );
}

/// A snapshot of the seq the tab waits for, or a later one, ends the wait: the engine has
/// answered the showing, whatever came of its view
#[test]
fn a_later_snapshot_ends_the_wait() {
    let (mut app, _ctx) = open_build();
    let seq = app.last_sent.max(app.last_snapshot) + 1;
    app.compare_wait = Some(seq);
    assert!(app.compare_waiting());
    app.on_event(Event_::Snapshot {
        seq,
        snapshot: Snapshot::Compare(Some(Arc::new(sample()))),
    });
    assert!(!app.compare_waiting());
}

/// A count typed into a Compare selector box goes out before Save, as every other box's
/// edit does (`flush_edits`): PoB's box applied it key by key, so a Save after it has it
#[test]
fn a_typed_count_goes_out_before_save() {
    let (mut app, ctx) = open_build();
    show_compare(&mut app, &ctx);
    click(&mut app, &ctx, "edit_cmpStageCount");
    frame(&mut app, &ctx, vec![Event::Text("4".into())]);
    assert!(app.compare.has_pending());
    let before = app.sent_full.len();
    frame(
        &mut app,
        &ctx,
        vec![Event::Key {
            key: Key::S,
            physical_key: Some(Key::S),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
    );
    assert_eq!(
        app.sent_full[before..].first().map(String::as_str),
        Some("SetCompareControl { name: \"cmpStageCount\", value: Text(\"4\") }"),
        "{:?}",
        &app.sent_full[before..]
    );
}
