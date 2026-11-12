//! The app around the Options window: what it writes, what it sends and when.

use super::*;
use crate::engine::fake::Fake;
use crate::engine::{Backend, SettingEffect, SettingsView};
use crate::ui::import::{clear_marks, marked};
use crate::ui::options::testing::{options_view, row, settings_view};
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn temp_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("auto-pob-appopts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("auto-pob.toml")
}

fn app(settings: AppSettings, config: &std::path::Path) -> (AutoPobApp, egui::Context) {
    let app = AutoPobApp::new(Box::new(|| {
        Engine::spawn_with(|_| Ok(Box::new(Fake::new().0) as Box<dyn Backend>), || {})
    }))
    .with_settings(settings, config.to_path_buf());
    (app, egui::Context::default())
}

fn frame(app: &mut AutoPobApp, ctx: &egui::Context, events: Vec<Event>) {
    clear_marks();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
        events,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| app.show(ui))
        .drop_without_applying_deltas();
}

fn until_ready(app: &mut AutoPobApp, ctx: &egui::Context) {
    let start = Instant::now();
    while !app.ready {
        assert!(start.elapsed() < Duration::from_secs(10), "no Ready");
        std::thread::sleep(Duration::from_millis(5));
        frame(app, ctx, vec![]);
    }
    frame(app, ctx, vec![]);
}

fn click(app: &mut AutoPobApp, ctx: &egui::Context, pos: Pos2) {
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

/// Opens the window over PoB's Options controls, as if the engine's session had answered.
fn open_options(app: &mut AutoPobApp, ctx: &egui::Context) {
    app.options.begin(&app.settings);
    show_controls(app, ctx);
}

/// Waits for the engine's answers, which replace the window's view with the fake's empty one,
/// then puts the Options controls back.
fn show_controls(app: &mut AutoPobApp, ctx: &egui::Context) {
    let start = Instant::now();
    while app.behind() {
        assert!(start.elapsed() < Duration::from_secs(10), "engine behind");
        std::thread::sleep(Duration::from_millis(5));
        frame(app, ctx, vec![]);
    }
    // and the fake's own settings view, which has no rows
    app.options.on_settings(&settings_view(&[]), 0);
    app.on_popup_snapshot(
        &Snapshot::Popup(Some(Arc::new(options_view()))),
        app.last_sent,
    );
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![]);
}

fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

use std::collections::BTreeMap;

#[test]
fn overrides_sent_after_ready() {
    let config = temp_config("ready");
    let settings = AppSettings {
        overrides: overrides(&[("showFlavourText", "false")]),
        ..AppSettings::default()
    };
    let (mut app, ctx) = app(settings, &config);
    assert!(app.sent_log.is_empty(), "nothing goes before Ready");
    until_ready(&mut app, &ctx);
    assert_eq!(app.sent_log, ["SetOverrides"]);
    assert!(
        app.sent_full[0].contains("showFlavourText"),
        "{:?}",
        app.sent_full
    );
    // reading the file is not writing it
    assert!(!config.exists());

    // with nothing to override there is nothing to send
    let (mut app, ctx) = self::app(AppSettings::default(), &config);
    until_ready(&mut app, &ctx);
    assert!(app.sent_log.is_empty());
}

#[test]
fn restart_keeps_the_settings() {
    let config = temp_config("restart");
    let settings = AppSettings {
        overrides: overrides(&[("a", "1")]),
        ..AppSettings::default()
    };
    let (mut app, ctx) = app(settings.clone(), &config);
    until_ready(&mut app, &ctx);
    app.restart();
    assert_eq!(app.settings, settings);
    assert_eq!(app.config_path.as_deref(), Some(config.as_path()));
    until_ready(&mut app, &ctx);
    assert_eq!(app.sent_log, ["SetOverrides"]);
}

#[test]
fn options_save_writes_overrides_then_sends_them() {
    let config = temp_config("save");
    let (mut app, ctx) = app(AppSettings::default(), &config);
    until_ready(&mut app, &ctx);
    // the window is open over a snapshot of the engine's Options session
    open_options(&mut app, &ctx);
    let pos = marked("override_showFlavourText")
        .expect("the box")
        .center();
    click(&mut app, &ctx, pos);
    assert_eq!(
        app.sent_log.last().map(String::as_str),
        Some("SetOverrides")
    );
    show_controls(&mut app, &ctx);
    app.options.draft_mut().update_check = true;
    let pos = app.options.window.rects.map["btn_save"].center();
    click(&mut app, &ctx, pos);
    assert!(
        app.sent_full
            .iter()
            .any(|c| c.contains("PressPopupButton") && c.contains("save")),
        "{:?}",
        app.sent_full
    );
    // PoB's Save has not answered yet: nothing is written
    assert!(!config.exists());
    let before = app.sent_log.len();
    let seq = app.last_sent;
    app.on_settings(
        &SettingsView {
            saved_overrides: overrides(&[("showAnimations", "true")]),
            ..settings_view(&[])
        },
        seq,
    );
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("update_check = true"), "{text}");
    assert!(text.contains("showAnimations = \"true\""), "{text}");
    assert!(text.contains("showFlavourText = \"true\""), "{text}");
    // the whole map goes to the engine after the write
    assert_eq!(app.sent_log.len(), before + 1);
    assert_eq!(
        app.sent_log.last().map(String::as_str),
        Some("SetOverrides")
    );
    let last = app.sent_full.last().unwrap();
    assert!(
        last.contains("showAnimations") && last.contains("showFlavourText"),
        "{last}"
    );
    assert_eq!(app.settings.overrides.len(), 2);
}

#[test]
fn options_cancel_writes_nothing() {
    let config = temp_config("cancel");
    let (mut app, ctx) = app(AppSettings::default(), &config);
    until_ready(&mut app, &ctx);
    open_options(&mut app, &ctx);
    let pos = marked("override_showFlavourText").unwrap().center();
    click(&mut app, &ctx, pos);
    show_controls(&mut app, &ctx);
    show_controls(&mut app, &ctx);
    app.options.draft_mut().update_check = true;
    let pos = app.options.window.rects.map["btn_cancel"].center();
    click(&mut app, &ctx, pos);
    let log = app.sent_full.join("\n");
    assert!(log.contains("cancel"), "{log}");
    assert!(!log.contains("\"save\""), "{log}");
    // the engine's answers to the cancel write nothing either
    app.on_settings(
        &settings_view(&[("showAnimations", "true")]),
        app.last_sent + 1,
    );
    assert!(!config.exists());
    assert!(app.settings.overrides.is_empty());
    assert!(!app.settings.update_check);
}

#[test]
fn zoom_follows_dpi_override() {
    let config = temp_config("zoom");
    let (mut app, ctx) = app(AppSettings::default(), &config);
    until_ready(&mut app, &ctx);
    assert!((ctx.zoom_factor() - 1.0).abs() < 1e-6);
    let view = |dpi: &str, title: &str| SettingsView {
        rows: vec![
            row(
                "dpiScaleOverridePercent",
                "dpiScaleOverride",
                dpi,
                SettingEffect::Ui,
            ),
            row("showTitlebarName", "titlebarName", title, SettingEffect::Ui),
        ],
        saved_overrides: BTreeMap::new(),
    };
    app.on_settings(&view("150", "false"), 0);
    // the factor becomes active as the next pass starts
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(
        (ctx.zoom_factor() - 1.5).abs() < 1e-6,
        "{}",
        ctx.zoom_factor()
    );
    app.on_settings(&view("0", "false"), 0);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!((ctx.zoom_factor() - 1.0).abs() < 1e-6);
    // the title follows showTitlebarName once a build is open
    assert_eq!(app.applied_title, "auto-pob");
    app.screen = Screen::Build;
    app.build = Some(Arc::new(BuildView {
        name: "Witch".into(),
        path: None,
        class: "Witch".into(),
        ascendancy: None,
        level: 1,
        unsaved: false,
        can_undo: false,
        can_redo: false,
    }));
    app.on_settings(&view("0", "true"), 0);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.applied_title, "auto-pob - Witch");
    app.on_settings(&view("0", "false"), 0);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.applied_title, "auto-pob");
}

#[test]
fn options_button_opens_the_window_from_the_list() {
    let config = temp_config("button");
    let (mut app, ctx) = app(AppSettings::default(), &config);
    until_ready(&mut app, &ctx);
    app.list = Some(Arc::new(BuildListView {
        folder: String::new(),
        entries: vec![],
        sort: Default::default(),
        filter: String::new(),
        ..Default::default()
    }));
    frame(&mut app, &ctx, vec![]);
    assert!(!app.options.window.is_blocking());
    app.open_options();
    assert!(app.sent_log.contains(&"OpenPopup".to_owned()));
    assert!(
        app.options.window.is_blocking(),
        "nothing behind may take input"
    );
    assert!(!app.popup.is_blocking(), "Options is not the item popup's");
}

#[test]
fn a_failed_write_does_not_look_saved() {
    use std::os::unix::fs::PermissionsExt as _;
    let config = temp_config("readonly");
    let dir = config.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(dir.join("probe"), "").is_ok() {
        // running as root: a read-only folder can't be made
        let _ = std::fs::remove_file(dir.join("probe"));
        return;
    }
    let before = AppSettings {
        trade_league: "Old".into(),
        ..AppSettings::default()
    };
    let (mut app, ctx) = app(before.clone(), &config);
    until_ready(&mut app, &ctx);
    open_options(&mut app, &ctx);
    app.options.draft_mut().trade_league = "Edited".into();
    app.options.save_sent(app.last_sent + 1);
    let opens = app.sent_log.iter().filter(|c| *c == "OpenPopup").count();
    app.on_settings(
        &SettingsView {
            saved_overrides: overrides(&[("showAnimations", "true")]),
            ..settings_view(&[])
        },
        app.last_sent + 1,
    );
    // the old settings are still the ones in effect, and nothing was written
    assert_eq!(app.settings, before);
    assert!(!config.exists());
    // the window comes back with the draft, so Save can be pressed again
    assert_eq!(
        app.sent_log.iter().filter(|c| *c == "OpenPopup").count(),
        opens + 1
    );
    assert_eq!(app.options.draft_mut().trade_league, "Edited");
    assert_eq!(app.options.draft_mut().overrides["showAnimations"], "true");
    show_controls(&mut app, &ctx);
    assert!(
        marked("options_save_error").is_some(),
        "the error shows inline"
    );
    assert_eq!(app.options.draft_mut().trade_league, "Edited");
    // once the folder can be written, the same Save works
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    app.options.save_sent(app.last_sent + 1);
    app.on_settings(&settings_view(&[]), app.last_sent + 1);
    assert_eq!(app.settings.trade_league, "Edited");
    assert!(config.exists());
}

#[test]
fn a_locked_settings_file_is_never_written() {
    let config = temp_config("locked");
    let (app, ctx) = app(AppSettings::default(), &config);
    let mut app = app.with_locked_settings(Some("permission denied".into()));
    until_ready(&mut app, &ctx);
    assert!(app.write_settings(&AppSettings::default()).is_err());
    assert!(!config.exists());
}
