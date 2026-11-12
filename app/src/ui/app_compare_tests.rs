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
        if app.ready && !app.behind() && app.entering.is_none() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "engine behind");
        std::thread::sleep(Duration::from_millis(5));
    }
    // late snapshots of the last command
    std::thread::sleep(Duration::from_millis(20));
    frame(app, ctx, vec![]);
}

/// A new build on the build screen, everything answered.
fn open_build() -> (AutoPobApp, egui::Context) {
    let mut app = AutoPobApp::new(Box::new(|| {
        Engine::spawn_with(|_| Ok(Box::new(Fake::new().0) as Box<dyn Backend>), || {})
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
    inject(&mut app, Some(sample()));
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
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
    inject(&mut app, Some(sample()));
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
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
    inject(&mut app, Some(sample()));
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
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
    assert!(
        app.notices
            .iter()
            .any(|n| n == "Failed to import build for comparison."),
        "{:?}",
        app.notices
    );
}
