//! The app around PoB's trade windows (R10, R20): where "Buy similar" is offered, and the trade
//! link a press answers, opened in the browser and copied for that press only.

use super::*;
use crate::engine::fake::{FAKE_TRADE_URL, Fake, Log};
use crate::engine::views::{DisplayItemView, ExternalUrl};
use crate::engine::{Backend, BuySource};
use crate::ui::compare::tests::sample as compare_sample;
use crate::ui::items::tests::sample as items_sample;
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One frame; returns the links the app opened and the text it copied.
fn frame(
    app: &mut AutoPobApp,
    ctx: &egui::Context,
    events: Vec<Event>,
) -> (Vec<String>, Vec<String>) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
        events,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| app.show(ui));
    let (mut opened, mut copied) = (Vec::new(), Vec::new());
    for command in &output.platform_output.commands {
        match command {
            egui::OutputCommand::OpenUrl(open) => opened.push(open.url.clone()),
            egui::OutputCommand::CopyText(text) => copied.push(text.clone()),
            _ => {}
        }
    }
    output.drop_without_applying_deltas();
    (opened, copied)
}

/// Runs frames until the engine has answered everything sent; returns what they opened and
/// copied.
fn settle(app: &mut AutoPobApp, ctx: &egui::Context) -> (Vec<String>, Vec<String>) {
    let start = Instant::now();
    let (mut opened, mut copied) = (Vec::new(), Vec::new());
    let mut take = |(o, c): (Vec<String>, Vec<String>)| {
        opened.extend(o);
        copied.extend(c);
    };
    loop {
        take(frame(app, ctx, vec![]));
        if app.ready && !app.behind() && !app.compare_waiting() && app.entering.is_none() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "engine behind: sent {} got {}",
            app.last_sent,
            app.last_snapshot
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // late snapshots of the last command
    std::thread::sleep(Duration::from_millis(20));
    take(frame(app, ctx, vec![]));
    (opened, copied)
}

/// A new build on the build screen, everything answered; the engine's log
fn open_build() -> (AutoPobApp, egui::Context, Arc<Mutex<Log>>) {
    let log = Arc::new(Mutex::new(None));
    let keep = log.clone();
    let mut app = AutoPobApp::new(Box::new(move || {
        let keep = keep.clone();
        Engine::spawn_with(
            move |_| {
                let (mut fake, log) = Fake::new();
                // the Compare tab's answer
                fake.compare = Some(compare_sample());
                *keep.lock().unwrap() = Some(log);
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
    let log = log.lock().unwrap().clone().unwrap();
    (app, ctx, log)
}

fn link(app: &mut AutoPobApp, seq: u64, url: &str) {
    app.on_event(crate::engine::Event::Snapshot {
        seq,
        snapshot: Snapshot::ExternalUrl(Arc::new(ExternalUrl { url: url.into() })),
    });
}

/// The `SetTradeDefaults` the engine applied, in order
fn trade_defaults(log: &Arc<Mutex<Log>>) -> Vec<String> {
    log.lock()
        .unwrap()
        .requests
        .iter()
        .filter(|r| r.contains("SetTradeDefaults"))
        .cloned()
        .collect()
}

#[test]
fn external_url_opens_and_copies_once_for_its_seq() {
    let (mut app, ctx, log) = open_build();
    // the app's realm and league went to the engine once it was ready
    assert_eq!(
        trade_defaults(&log),
        ["apply SetTradeDefaults { realm: \"PC\", league: \"Standard\" }"]
    );

    // the user's press: the engine answers the link with the press's seq, and it is opened
    // and copied, once
    let seq = app.send(Command::PressPopupButton {
        name: "search".into(),
    });
    let (opened, copied) = settle(&mut app, &ctx);
    assert_eq!(opened, [FAKE_TRADE_URL]);
    assert_eq!(copied, [FAKE_TRADE_URL]);
    // the same answer again opens nothing
    link(&mut app, seq, FAKE_TRADE_URL);
    assert_eq!(frame(&mut app, &ctx, vec![]), (vec![], vec![]));

    // a link answering an older press is not the user's click now
    let older = seq;
    let seq = app.send(Command::PressPopupButton {
        name: "searchTradeButton".into(),
    });
    settle(&mut app, &ctx);
    // another window's button between the press and its link doesn't drop the link
    app.send(Command::PressPopupButton {
        name: "save".into(),
    });
    settle(&mut app, &ctx);
    let late = "https://www.pathofexile.com/trade/search/Standard?q=late";
    link(&mut app, older, late);
    assert_eq!(frame(&mut app, &ctx, vec![]), (vec![], vec![]));
    // the current press's own answer still opens
    let own = "https://www.pathofexile.com/trade/search/Standard?q=own";
    link(&mut app, seq, own);
    assert_eq!(
        frame(&mut app, &ctx, vec![]),
        (vec![own.to_owned()], vec![own.to_owned()])
    );
}

#[test]
fn only_trade_search_urls_are_opened() {
    let (mut app, ctx, _log) = open_build();
    for url in [
        "https://evil.test/trade/search/Standard",
        "http://www.pathofexile.com/trade/search/Standard?q=1",
        "https://www.pathofexile.com/oauth/authorize?x=1",
    ] {
        let seq = app.send(Command::PressPopupButton {
            name: "searchTradeButton".into(),
        });
        settle(&mut app, &ctx);
        link(&mut app, seq, url);
        assert_eq!(frame(&mut app, &ctx, vec![]), (vec![], vec![]), "{url}");
    }
}

#[test]
fn trade_defaults_follow_an_options_save() {
    let (mut app, _ctx, log) = open_build();
    let dir = std::env::temp_dir().join(format!("auto-pob-trade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.config_path = Some(dir.join("auto-pob.toml"));
    // the Options window's Save with a new realm and league, answered by the engine
    app.options.begin(&app.settings);
    app.options.draft_mut().trade_realm = "PS4".into();
    app.options.draft_mut().trade_league = "Settlers".into();
    let seq = app.last_sent + 1;
    app.options.save_sent(seq);
    app.on_settings(&crate::ui::options::testing::settings_view(&[]), seq);
    assert_eq!(app.settings.trade_realm, "PS4");
    let start = Instant::now();
    while trade_defaults(&log).len() < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no SetTradeDefaults after Save"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        trade_defaults(&log)[1],
        "apply SetTradeDefaults { realm: \"PS4\", league: \"Settlers\" }"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn click_at(app: &mut AutoPobApp, ctx: &egui::Context, pos: Pos2, button: PointerButton) {
    let press = |pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    };
    for events in [
        vec![Event::PointerMoved(pos)],
        vec![press(true)],
        vec![press(false)],
        vec![],
    ] {
        frame(app, ctx, events);
    }
}

/// The `OpenPopup`s sent since `before`
fn opened_popups(app: &AutoPobApp, before: usize) -> Vec<&str> {
    app.sent_full[before..]
        .iter()
        .map(String::as_str)
        .filter(|c| c.starts_with("OpenPopup"))
        .collect()
}

#[test]
fn buy_similar_offered_on_display_slot_menu_and_compare() {
    let (mut app, ctx, _log) = open_build();
    let seq = app.last_sent;
    app.on_event(crate::engine::Event::Snapshot {
        seq,
        snapshot: Snapshot::Items(Arc::new(items_sample())),
    });
    app.on_event(crate::engine::Event::Snapshot {
        seq,
        snapshot: Snapshot::DisplayItem(Some(Arc::new(DisplayItemView {
            id: Some(2),
            name: "Kaom's Heart".into(),
            save_label: "Save".into(),
            ..DisplayItemView::default()
        }))),
    });
    app.tab = BuildTab::Items;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);

    // the display panel's button
    let before = app.sent_full.len();
    let pos = app.items.rect("buy_similar").center();
    click_at(&mut app, &ctx, pos, PointerButton::Primary);
    let display = format!(
        "{:?}",
        Command::OpenPopup(PopupKind::BuySimilar {
            source: BuySource::DisplayItem
        })
    );
    assert_eq!(opened_popups(&app, before), [display.as_str()]);
    app.popup.reset();
    settle(&mut app, &ctx);

    // a filled slot's context menu; an empty one offers none
    assert!(app.items.has_rect("slot_label_Helmet"));
    let pos = app.items.rect("slot_label_Helmet").center();
    click_at(&mut app, &ctx, pos, PointerButton::Secondary);
    assert!(
        !app.items.has_rect("slot_buy_Helmet"),
        "an empty slot offers Buy similar"
    );
    frame(
        &mut app,
        &ctx,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    let pos = app.items.rect("slot_label_Body Armour").center();
    click_at(&mut app, &ctx, pos, PointerButton::Secondary);
    let before = app.sent_full.len();
    let pos = app.items.rect("slot_buy_Body Armour").center();
    click_at(&mut app, &ctx, pos, PointerButton::Primary);
    let slot = format!(
        "{:?}",
        Command::OpenPopup(PopupKind::BuySimilar {
            source: BuySource::Slot("Body Armour".into())
        })
    );
    assert_eq!(opened_popups(&app, before), [slot.as_str()]);
    app.popup.reset();
    settle(&mut app, &ctx);

    // the Compare tab's Items rows: Buy where the comparison has an item
    // entering it asks the engine for the view (`compare_sample()`); leaving Items also
    // closes the popup session, which the fake doesn't answer, so wait for the view alone
    app.tab = BuildTab::Compare;
    frame(&mut app, &ctx, vec![]);
    let start = Instant::now();
    while app.compare_waiting() {
        assert!(start.elapsed() < Duration::from_secs(10), "no Compare view");
        std::thread::sleep(Duration::from_millis(5));
        frame(&mut app, &ctx, vec![]);
    }
    frame(&mut app, &ctx, vec![]);
    let pos = app.compare.rect("sub:items").center();
    click_at(&mut app, &ctx, pos, PointerButton::Primary);
    assert!(
        !app.compare.has_rect("buy:Ring 1"),
        "a row without their item offers Buy"
    );
    let before = app.sent_full.len();
    let pos = app.compare.rect("buy:Ring 3").center();
    click_at(&mut app, &ctx, pos, PointerButton::Primary);
    let compare = format!(
        "{:?}",
        Command::OpenPopup(PopupKind::BuySimilar {
            source: BuySource::CompareSlot("Ring 3".into())
        })
    );
    assert_eq!(opened_popups(&app, before), [compare.as_str()]);
}

#[test]
fn timeless_progress_reaches_the_window() {
    let (mut app, ctx, _log) = open_build();
    app.tab = BuildTab::Tree;
    app.send(Command::OpenPopup(PopupKind::TimelessJewel));
    app.on_event(crate::engine::Event::Snapshot {
        seq: app.last_sent,
        snapshot: Snapshot::Popup(Some(Arc::new(crate::engine::PopupView {
            kind: PopupKind::TimelessJewel,
            title: "Find a Timeless Jewel".into(),
            enter: None,
            controls: vec![],
        }))),
    });
    frame(&mut app, &ctx, vec![]);
    app.on_event(crate::engine::Event::Progress {
        task: crate::engine::BackgroundTask::Timeless,
        percent: 40,
    });
    assert_eq!(app.popup.search_state(), (Some(40), None));
    app.on_event(crate::engine::Event::BackgroundFailed {
        task: crate::engine::BackgroundTask::Timeless,
        message: "boom".into(),
    });
    assert_eq!(app.popup.search_state(), (None, Some("boom")));
    // the tree's node power is not told
    assert!(app.notices.is_empty(), "{:?}", app.notices);
}

/// The first time the Items tab shows, PoB's item database load starts in the background,
/// once per engine; it is no edit, so the app waits for no answer.
#[test]
fn the_items_tab_starts_the_item_database_load_once() {
    let (mut app, ctx, log) = open_build();
    let preloads = |log: &Arc<Mutex<Log>>| {
        log.lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| *r == "apply PreloadItemDbs")
            .count()
    };
    settle(&mut app, &ctx);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(preloads(&log), 0, "sent before the tab showed");
    for tab in [BuildTab::Items, BuildTab::Tree, BuildTab::Items] {
        app.tab = tab;
        settle(&mut app, &ctx);
        assert!(!app.behind());
    }
    let start = Instant::now();
    while preloads(&log) == 0 && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(preloads(&log), 1);
}
