use super::popup::*;
use crate::engine::views::{
    ControlTooltipView, ItemTooltip, PopupControl, PopupControlKind, PopupRow, PopupView,
};
use crate::engine::{Command, ControlTooltipQuery, PopupInput, PopupKind, Snapshot};
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use std::sync::Arc;

fn control(name: &str, kind: PopupControlKind, pos: [f32; 2], size: [f32; 2]) -> PopupControl {
    PopupControl {
        name: name.into(),
        kind,
        label: name.into(),
        pos,
        size,
        shown: true,
        enabled: true,
        choices: vec![],
        selected: 0,
        value: 0.0,
        divisions: None,
        checked: false,
        text: String::new(),
        tooltip_text: None,
        hover_tooltip: false,
        rows: vec![],
        row_selected: None,
        row_highlight: None,
        note: None,
    }
}

fn button(name: &str, x: f32, y: f32) -> PopupControl {
    control(name, PopupControlKind::Button, [x, y], [100.0, 20.0])
}

fn dropdown(name: &str, choices: &[&str], selected: usize) -> PopupControl {
    PopupControl {
        choices: choices.iter().map(|c| (*c).to_owned()).collect(),
        selected,
        ..control(name, PopupControlKind::Dropdown, [0.0, 0.0], [200.0, 20.0])
    }
}

fn popup(controls: Vec<PopupControl>) -> PopupView {
    PopupView {
        kind: PopupKind::Corrupt,
        title: "Corrupt Item".into(),
        controls,
    }
}

fn set(name: &str, value: PopupInput) -> String {
    format!(
        "{:?}",
        Command::SetPopupControl {
            name: name.into(),
            value
        }
    )
}

fn press(name: &str) -> String {
    format!("{:?}", Command::PressPopupButton { name: name.into() })
}

fn debug(commands: &[Command]) -> Vec<String> {
    commands.iter().map(|c| format!("{c:?}")).collect()
}

struct Harness {
    popup: PopupWindow,
    ctx: egui::Context,
    time: f64,
    step: f64,
}

impl Harness {
    fn new(view: PopupView) -> Self {
        let mut popup = PopupWindow::default();
        popup.on_snapshot(&Snapshot::Popup(Some(Arc::new(view))));
        let mut h = Self {
            popup,
            ctx: egui::Context::default(),
            time: 1.0,
            step: 0.2,
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn view(&mut self, view: PopupView) {
        self.popup
            .on_snapshot(&Snapshot::Popup(Some(Arc::new(view))));
    }

    fn frame(&mut self, events: Vec<Event>) -> PopupOutput {
        self.time += self.step;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut out = PopupOutput::default();
        let time = self.time;
        let output = self.ctx.run_ui(input, |ui| {
            // egui may run a frame twice; each pass's output counts
            let pass = self.popup.show(ui.ctx(), time, false);
            out.commands.extend(pass.commands);
            out.queries.extend(pass.queries);
        });
        output.drop_without_applying_deltas();
        out
    }

    fn rect(&self, key: &str) -> Rect {
        *self
            .popup
            .rects
            .map
            .get(key)
            .unwrap_or_else(|| panic!("no rect {key}; have {:?}", self.popup.rects.map.keys()))
    }

    fn has(&self, key: &str) -> bool {
        self.popup.rects.map.contains_key(key)
    }

    fn click(&mut self, key: &str) -> PopupOutput {
        let pos = self.rect(key).center();
        self.click_at(pos)
    }

    fn click_at(&mut self, pos: Pos2) -> PopupOutput {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut all = PopupOutput::default();
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

    fn key(&mut self, key: Key) -> PopupOutput {
        self.frame(vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }])
    }

    fn hover(&mut self, key: &str) {
        let pos = self.rect(key).center();
        self.frame(vec![Event::PointerMoved(pos)]);
    }
}

#[test]
fn popup_controls_render_in_rows_by_position() {
    // names run against the x order, and the second row's centre is a pixel off the first's
    let view = popup(vec![
        button("alpha", 200.0, 10.0),
        control("mid", PopupControlKind::Slider, [100.0, 7.0], [80.0, 18.0]),
        button("zeta", 0.0, 10.0),
        button("below", 0.0, 60.0),
    ]);
    let rows: Vec<Vec<&str>> = rows(&view)
        .iter()
        .map(|r| r.iter().map(|&i| view.controls[i].name.as_str()).collect())
        .collect();
    assert_eq!(rows, [vec!["zeta", "mid", "alpha"], vec!["below"]]);

    let h = Harness::new(view);
    let (a, m, z) = (h.rect("btn_zeta"), h.rect("mid"), h.rect("btn_alpha"));
    assert!(a.min.x < m.min.x && m.min.x < z.min.x, "{a:?} {m:?} {z:?}");
    assert!(
        (a.center().y - z.center().y).abs() < 6.0,
        "one row: {a:?} {z:?}"
    );
    assert!(h.rect("btn_below").min.y > a.max.y);
}

#[test]
fn rows_split_when_centres_are_further_apart_than_six_pixels() {
    let view = popup(vec![
        control("a", PopupControlKind::Label, [0.0, 0.0], [50.0, 20.0]),
        control("b", PopupControlKind::Label, [60.0, 6.0], [50.0, 20.0]),
        control("c", PopupControlKind::Label, [120.0, 7.0], [50.0, 20.0]),
    ]);
    // centres 10, 16 and 17: c is 7 away from a, so it starts the next row
    let grouped = rows(&view);
    assert_eq!(grouped, [vec![0, 1], vec![2]]);
}

#[test]
fn rows_group_by_centre_not_by_top() {
    // tops 14 apart but centres equal: one row
    let tall = control("tall", PopupControlKind::Label, [0.0, 0.0], [50.0, 40.0]);
    let short = control("short", PopupControlKind::Label, [60.0, 14.0], [50.0, 12.0]);
    assert_eq!(rows(&popup(vec![tall, short])), [vec![0, 1]]);
    // tops equal but centres 10 apart: two rows
    let a = control("a", PopupControlKind::Label, [0.0, 0.0], [50.0, 20.0]);
    let b = control("b", PopupControlKind::Label, [60.0, 0.0], [50.0, 40.0]);
    assert_eq!(rows(&popup(vec![a, b])), [vec![0], vec![1]]);
}

#[test]
fn hidden_popup_controls_are_not_drawn() {
    let mut hidden = button("secret", 0.0, 40.0);
    hidden.shown = false;
    let view = popup(vec![button("visible", 0.0, 0.0), hidden]);
    assert_eq!(rows(&view), [vec![0]]);
    let h = Harness::new(view);
    assert!(h.has("btn_visible"));
    assert!(!h.has("btn_secret"));
}

#[test]
fn dropdown_pick_sends_index_and_label() {
    let mut h = Harness::new(popup(vec![dropdown(
        "kind",
        &["^7Alpha", "Beta", "Gamma"],
        0,
    )]));
    h.click("kind");
    h.frame(vec![]);
    // narrowed to Gamma: its position in the shown list is 0, its index is 2
    h.frame(vec![Event::Text("gam".into())]);
    h.frame(vec![]);
    let out = h.click("kind_2");
    assert_eq!(
        debug(&out.commands),
        [set(
            "kind",
            PopupInput::Choice {
                index: 2,
                label: "Gamma".into()
            }
        )]
    );
}

#[test]
fn dropdown_label_is_the_one_shown_with_its_colour_codes() {
    let mut h = Harness::new(popup(vec![dropdown(
        "kind",
        &["^7Alpha", "^xFF0000Beta"],
        0,
    )]));
    h.click("kind");
    h.frame(vec![]);
    let out = h.click("kind_1");
    assert_eq!(
        debug(&out.commands),
        [set(
            "kind",
            PopupInput::Choice {
                index: 1,
                label: "^xFF0000Beta".into()
            }
        )]
    );
}

#[test]
fn selected_past_choices_does_not_panic() {
    let mut h = Harness::new(popup(vec![dropdown("legacy", &["One", "Two"], 7)]));
    h.click("legacy");
    h.frame(vec![]);
    assert!(h.has("legacy_0") && h.has("legacy_1"));
    let out = h.click("legacy_0");
    assert_eq!(
        debug(&out.commands),
        [set(
            "legacy",
            PopupInput::Choice {
                index: 0,
                label: "One".into()
            }
        )]
    );
}

#[test]
fn slider_sends_on_release_only() {
    let slider = PopupControl {
        value: 0.2,
        ..control(
            "amount",
            PopupControlKind::Slider,
            [0.0, 0.0],
            [200.0, 18.0],
        )
    };
    let mut h = Harness::new(popup(vec![slider]));
    let rect = h.rect("amount");
    let y = rect.center().y;
    let from = Pos2::new(rect.min.x + rect.width() * 0.2, y);
    let to = Pos2::new(rect.min.x + rect.width() * 0.8, y);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut sent = Vec::new();
    sent.extend(h.frame(vec![Event::PointerMoved(from)]).commands);
    sent.extend(h.frame(vec![button(from, true)]).commands);
    for t in [0.4, 0.6, 0.8] {
        let pos = Pos2::new(rect.min.x + rect.width() * t, y);
        sent.extend(h.frame(vec![Event::PointerMoved(pos)]).commands);
    }
    assert!(sent.is_empty(), "nothing is sent while dragging: {sent:?}");
    sent.extend(h.frame(vec![button(to, false)]).commands);
    sent.extend(h.frame(vec![]).commands);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let Command::SetPopupControl {
        name,
        value: PopupInput::Number(v),
    } = &sent[0]
    else {
        panic!("{sent:?}");
    };
    assert_eq!(name, "amount");
    assert!(*v > 0.6, "dragged well past the start: {v}");
}

#[test]
fn popup_button_sends_press() {
    let mut h = Harness::new(popup(vec![button("save", 0.0, 0.0)]));
    let out = h.click("btn_save");
    assert_eq!(debug(&out.commands), [press("save")]);
}

#[test]
fn disabled_button_sends_nothing() {
    let mut off = button("save", 0.0, 0.0);
    off.enabled = false;
    let mut h = Harness::new(popup(vec![off]));
    let out = h.click("btn_save");
    assert!(out.commands.is_empty(), "{:?}", out.commands);
}

#[test]
fn escape_or_close_sends_close_popup() {
    let mut h = Harness::new(popup(vec![button("save", 0.0, 0.0)]));
    let out = h.key(Key::Escape);
    assert_eq!(debug(&out.commands), [format!("{:?}", Command::ClosePopup)]);
    assert!(!h.popup.is_open(), "the window goes at once");

    let mut h = Harness::new(popup(vec![button("save", 0.0, 0.0)]));
    let out = h.click("popup_close");
    assert_eq!(debug(&out.commands), [format!("{:?}", Command::ClosePopup)]);
    assert!(!h.popup.is_open());
}

#[test]
fn a_view_from_before_the_close_does_not_reopen_the_window() {
    let mut h = Harness::new(popup(vec![button("save", 0.0, 0.0)]));
    h.key(Key::Escape);
    h.popup.close_sent(7);
    let stale = Snapshot::Popup(Some(Arc::new(popup(vec![]))));
    h.popup.on_snapshot_at(&stale, 6);
    assert!(!h.popup.is_open());
    // a later session opens normally
    h.popup.on_snapshot_at(&stale, 9);
    assert!(h.popup.is_open());
}

#[test]
fn escape_in_a_text_box_reverts_it_and_keeps_the_window() {
    let edit = control("custom", PopupControlKind::Edit, [0.0, 0.0], [200.0, 20.0]);
    let mut h = Harness::new(popup(vec![edit]));
    h.click("edit_custom");
    h.frame(vec![Event::Text("abc".into())]);
    let out = h.key(Key::Escape);
    assert!(out.commands.is_empty(), "{:?}", out.commands);
    assert!(h.popup.is_open());
    h.frame(vec![]);
    assert_eq!(
        h.popup.box_text("custom"),
        Some(""),
        "the typed text is reverted"
    );
    // nothing pending is left to send, and the next Esc closes
    assert!(h.popup.flush().is_none());
    let out = h.key(Key::Escape);
    assert_eq!(debug(&out.commands), [format!("{:?}", Command::ClosePopup)]);
}

#[test]
fn escape_does_not_close_while_a_dropdown_is_open() {
    // the dropdown's own filter box is not what stops the window: the guard is
    let mut h = Harness::new(popup(vec![dropdown("kind", &["One", "Two"], 0)]));
    h.click("kind");
    h.frame(vec![]);
    h.popup.forget_typing();
    let out = h.key(Key::Escape);
    assert!(out.commands.is_empty(), "{:?}", out.commands);
    assert!(h.popup.is_open());
}

#[test]
fn escape_closes_an_open_dropdown_first() {
    let mut h = Harness::new(popup(vec![dropdown("kind", &["One", "Two"], 0)]));
    h.click("kind");
    h.frame(vec![]);
    let out = h.key(Key::Escape);
    assert!(out.commands.is_empty(), "{:?}", out.commands);
    assert!(h.popup.is_open());
}

#[test]
fn list_row_click_sends_row_key() {
    let list = PopupControl {
        rows: vec![
            PopupRow {
                key: "a".into(),
                label: "First".into(),
            },
            PopupRow {
                key: "42".into(),
                label: "^xFF0000Second".into(),
            },
        ],
        row_selected: Some("a".into()),
        ..control("notes", PopupControlKind::List, [0.0, 0.0], [300.0, 200.0])
    };
    let mut h = Harness::new(popup(vec![list]));
    let out = h.click("notes_row_1");
    assert_eq!(
        debug(&out.commands),
        [set("notes", PopupInput::Row("42".into()))]
    );
}

fn tip_for(control: &str, index: Option<usize>) -> Arc<ControlTooltipView> {
    Arc::new(ControlTooltipView {
        query: ControlTooltipQuery::Popup {
            control: control.into(),
            index,
        },
        tooltip: ItemTooltip::default(),
    })
}

#[test]
fn hovering_an_entry_asks_for_its_tooltip_after_100ms() {
    let mut drop = dropdown("kind", &["One", "Two"], 0);
    drop.hover_tooltip = true;
    let mut h = Harness::new(popup(vec![drop]));
    h.step = 0.06;
    h.click("kind");
    h.frame(vec![]);
    h.hover("kind_1");
    let first = h.frame(vec![]);
    assert!(first.queries.is_empty(), "too early: {:?}", first.queries);
    let mut queries = Vec::new();
    for _ in 0..3 {
        queries.extend(h.frame(vec![]).queries);
    }
    assert_eq!(
        debug(&queries),
        [format!(
            "{:?}",
            Command::ControlTooltip(ControlTooltipQuery::Popup {
                control: "kind".into(),
                index: Some(1)
            })
        )]
    );
}

#[test]
fn a_button_with_a_hover_tooltip_asks_and_one_without_does_not() {
    let mut with = button("with", 0.0, 0.0);
    with.hover_tooltip = true;
    let without = button("without", 0.0, 40.0);
    let mut h = Harness::new(popup(vec![with, without]));
    h.step = 0.06;
    h.hover("btn_without");
    let mut queries = Vec::new();
    for _ in 0..4 {
        queries.extend(h.frame(vec![]).queries);
    }
    assert!(queries.is_empty(), "{queries:?}");
    h.hover("btn_with");
    for _ in 0..4 {
        queries.extend(h.frame(vec![]).queries);
    }
    assert_eq!(
        debug(&queries),
        [format!(
            "{:?}",
            Command::ControlTooltip(ControlTooltipQuery::Popup {
                control: "with".into(),
                index: None
            })
        )]
    );
}

#[test]
fn stale_control_tooltip_ignored() {
    let mut drop = dropdown("kind", &["One", "Two"], 0);
    drop.hover_tooltip = true;
    let mut h = Harness::new(popup(vec![drop]));
    h.click("kind");
    h.frame(vec![]);
    h.hover("kind_0");
    let mut asked = Vec::new();
    for _ in 0..2 {
        asked.extend(h.frame(vec![]).queries);
    }
    assert_eq!(asked.len(), 1);
    // the pointer moves on to another entry before the first answer arrives
    h.hover("kind_1");
    for _ in 0..2 {
        asked.extend(h.frame(vec![]).queries);
    }
    assert_eq!(asked.len(), 2, "{asked:?}");
    h.popup
        .on_snapshot(&Snapshot::ControlTooltip(tip_for("kind", Some(0))));
    assert!(!h.popup.tooltip_held(), "the reply is to an older question");
    h.popup
        .on_snapshot(&Snapshot::ControlTooltip(tip_for("kind", Some(1))));
    assert!(h.popup.tooltip_held());
}

/// Types into the box, then presses and releases the button in one frame, so the box has not
/// blurred when the button reports its click.
fn typed_then_pressed(button_above: bool) -> Vec<String> {
    let (edit_y, button_y) = if button_above {
        (60.0, 0.0)
    } else {
        (0.0, 60.0)
    };
    let edit = control(
        "custom",
        PopupControlKind::Edit,
        [0.0, edit_y],
        [200.0, 20.0],
    );
    let add = button("save", 0.0, button_y);
    let mut h = Harness::new(popup(vec![edit, add]));
    h.click("edit_custom");
    h.frame(vec![Event::Text("+10 to Strength".into())]);
    let pos = h.rect("btn_save").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut sent = h.frame(vec![button(true), button(false)]).commands;
    sent.extend(h.frame(vec![]).commands);
    debug(&sent)
}

#[test]
fn popup_edit_box_flushes_before_press() {
    let expected = [
        set("custom", PopupInput::Text("+10 to Strength".into())),
        press("save"),
    ];
    assert_eq!(typed_then_pressed(false), expected, "button below the box");
    assert_eq!(typed_then_pressed(true), expected, "button above the box");
}

#[test]
fn a_check_flushes_a_pending_box_first() {
    let check = control("legacy", PopupControlKind::Check, [0.0, 0.0], [100.0, 20.0]);
    let edit = control("custom", PopupControlKind::Edit, [0.0, 60.0], [200.0, 20.0]);
    let mut h = Harness::new(popup(vec![check, edit]));
    h.click("edit_custom");
    h.frame(vec![Event::Text("abc".into())]);
    let pos = h.rect("legacy").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let b = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut sent = h.frame(vec![b(true), b(false)]).commands;
    sent.extend(h.frame(vec![]).commands);
    assert_eq!(
        debug(&sent),
        [
            set("custom", PopupInput::Text("abc".into())),
            set("legacy", PopupInput::Check(true))
        ]
    );
}

#[test]
fn a_box_typed_into_sends_on_enter() {
    let edit = control("custom", PopupControlKind::Edit, [0.0, 0.0], [200.0, 20.0]);
    let mut h = Harness::new(popup(vec![edit]));
    h.click("edit_custom");
    h.frame(vec![Event::Text("abc".into())]);
    let out = h.key(Key::Enter);
    assert_eq!(
        debug(&out.commands),
        [set("custom", PopupInput::Text("abc".into()))]
    );
}

#[test]
fn check_and_label_controls_render() {
    let check = PopupControl {
        checked: false,
        ..control("legacy", PopupControlKind::Check, [0.0, 0.0], [100.0, 20.0])
    };
    let label = control("note", PopupControlKind::Label, [0.0, 40.0], [100.0, 20.0]);
    let mut h = Harness::new(popup(vec![check, label]));
    assert!(h.has("legacy"));
    let out = h.click("legacy");
    assert_eq!(
        debug(&out.commands),
        [set("legacy", PopupInput::Check(true))]
    );
    // a label has nothing to click, and sends nothing wherever the pointer goes
    let out = h.click_at(Pos2::new(5.0, 5.0));
    assert!(out.commands.is_empty());
}

fn anoint_view(rows: usize) -> PopupView {
    let list = PopupControl {
        rows: (0..rows)
            .map(|i| PopupRow {
                key: i.to_string(),
                label: format!("Notable {i}"),
            })
            .collect(),
        note: Some("No notables".into()),
        ..control(
            "notableDB",
            PopupControlKind::List,
            [0.0, 60.0],
            [300.0, 200.0],
        )
    };
    PopupView {
        kind: PopupKind::Anoint { slot: 0 },
        title: "Anoint Item".into(),
        controls: vec![
            dropdown("notableDB.sort", &["Sort by Dps", "Sort by Life"], 1),
            list,
        ],
    }
}

#[test]
fn anoint_progress_and_failure_shown_in_list() {
    let mut h = Harness::new(anoint_view(3));
    assert_eq!(h.popup.sort_line(), (None, None));
    h.popup.rects.map.clear();
    assert!(!h.has("notableDB_progress"));
    h.popup.on_progress(40);
    h.frame(vec![]);
    assert_eq!(h.popup.sort_line(), (Some(40), None));
    assert!(h.has("notableDB_progress"), "the progress line is drawn");

    // the failure replaces the progress and offers Retry
    h.popup.on_failed("calc broke".into());
    h.frame(vec![]);
    assert_eq!(h.popup.sort_line(), (None, Some("calc broke")));
    assert!(h.has("notableDB_retry"));
    // Retry sets the sort dropdown to what it shows, which starts the sort again
    let out = h.click("notableDB_retry");
    assert_eq!(
        debug(&out.commands),
        [set(
            "notableDB.sort",
            PopupInput::Choice {
                index: 1,
                label: "Sort by Life".into()
            }
        )]
    );
    assert_eq!(h.popup.sort_line(), (None, None));

    // the view that ends the sort clears the progress
    h.popup.on_progress(90);
    h.view(anoint_view(3));
    assert_eq!(h.popup.sort_line(), (None, None));
    h.popup.rects.map.clear();
    h.frame(vec![]);
    assert!(!h.has("notableDB_progress"));
}

#[test]
fn progress_with_no_window_is_dropped() {
    let mut popup = PopupWindow::default();
    popup.on_progress(10);
    popup.on_failed("late".into());
    assert_eq!(popup.sort_line(), (None, None));
}

#[test]
fn a_dead_session_closes_the_window() {
    let mut h = Harness::new(popup(vec![button("save", 0.0, 0.0)]));
    h.popup.on_snapshot(&Snapshot::Popup(None));
    assert!(!h.popup.is_open());
    assert!(h.frame(vec![]).commands.is_empty());
}

#[test]
fn a_hidden_box_with_a_pending_edit_is_discarded_not_sent() {
    // the engine refuses a control PoB doesn't show, so sending would only raise a notice
    let edit = control("custom", PopupControlKind::Edit, [0.0, 0.0], [200.0, 20.0]);
    let mut h = Harness::new(popup(vec![edit.clone(), button("save", 0.0, 60.0)]));
    h.click("edit_custom");
    h.frame(vec![Event::Text("x".into())]);
    let mut gone = edit;
    gone.shown = false;
    h.view(popup(vec![gone, button("save", 0.0, 60.0)]));
    let out = h.frame(vec![]);
    assert!(out.commands.is_empty(), "{:?}", out.commands);
    assert!(h.popup.flush().is_none(), "nothing stays pending");
}

#[test]
fn progress_line_and_cancel() {
    let mut h = Harness::new(PopupView {
        kind: PopupKind::TimelessJewel,
        title: "Find a Timeless Jewel".into(),
        controls: vec![
            button("searchButton", 0.0, 0.0),
            button("closeButton", 120.0, 0.0),
        ],
    });
    // Search starts the job: its line shows at once
    let out = h.click("btn_searchButton");
    assert_eq!(debug(&out.commands), [press("searchButton")]);
    assert_eq!(h.popup.search_state(), (Some(0), None));
    h.popup.on_search_progress(40);
    h.frame(vec![]);
    assert_eq!(h.popup.search_state(), (Some(40), None));
    assert!(h.has("timeless_progress"));
    // nothing else is offered while it runs
    let out = h.click("btn_searchButton");
    assert!(out.commands.is_empty(), "{:?}", out.commands);
    // Cancel stops it
    let out = h.click("timeless_cancel");
    assert_eq!(debug(&out.commands), ["CancelTimelessSearch"]);
    assert_eq!(h.popup.search_state(), (None, None));
    // a step that ran before the cancel reached the engine reports late; the engine's answer
    // to the cancel (100) ends the line
    h.popup.on_search_progress(40);
    h.popup.on_search_progress(100);
    assert_eq!(h.popup.search_state(), (None, None));
    // a search that finishes takes its line away
    h.click("btn_searchButton");
    h.popup.on_search_progress(100);
    assert_eq!(h.popup.search_state(), (None, None));
    // a failure shows PoB's message
    h.click("btn_searchButton");
    h.popup.on_search_failed("boom".into());
    h.frame(vec![]);
    assert_eq!(h.popup.search_state(), (None, Some("boom")));
    assert!(h.has("timeless_failed"));
    // another window has no search line
    let mut other = Harness::new(popup(vec![button("save", 0.0, 0.0)]));
    other.popup.on_search_progress(40);
    assert_eq!(other.popup.search_state(), (None, None));
}

#[test]
fn list_double_and_shift_clicks_and_the_range() {
    let rows: Vec<PopupRow> = (1..=4)
        .map(|i| PopupRow {
            key: i.to_string(),
            label: format!("seed {i}"),
        })
        .collect();
    let list = PopupControl {
        rows,
        row_selected: Some("1".into()),
        row_highlight: Some("3".into()),
        ..control(
            "searchResults",
            PopupControlKind::List,
            [0.0, 0.0],
            [300.0, 200.0],
        )
    };
    let mut h = Harness::new(PopupView {
        kind: PopupKind::TimelessJewel,
        ..popup(vec![list])
    });
    // a double click: PoB's list takes the first click, then the second as a double click
    let pos = h.rect("searchResults_row_1").center();
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    h.step = 0.05;
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
    assert_eq!(
        debug(&sent),
        [
            set("searchResults", PopupInput::Row("2".into())),
            set("searchResults", PopupInput::RowDoubleClick("2".into())),
        ]
    );
    // Shift held: the other end of a range
    h.step = 1.0;
    h.frame(vec![]);
    h.step = 0.2;
    let pos = h.rect("searchResults_row_3").center();
    let shift = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::SHIFT,
    };
    let mut sent = Vec::new();
    for events in [
        vec![
            Event::ModifiersChanged(Modifiers::SHIFT),
            Event::PointerMoved(pos),
        ],
        vec![Event::ModifiersChanged(Modifiers::SHIFT), shift(true)],
        vec![Event::ModifiersChanged(Modifiers::SHIFT), shift(false)],
        vec![Event::ModifiersChanged(Modifiers::NONE)],
    ] {
        sent.extend(h.frame(events).commands);
    }
    assert_eq!(
        debug(&sent),
        [set("searchResults", PopupInput::RowShiftClick("4".into()))]
    );
}

#[test]
fn only_the_searchs_own_refusal_ends_its_line() {
    let mut h = Harness::new(PopupView {
        kind: PopupKind::TimelessJewel,
        title: "Find a Timeless Jewel".into(),
        controls: vec![button("searchButton", 0.0, 0.0)],
    });
    h.click("btn_searchButton");
    h.popup.press_sent(7);
    // another command's refusal leaves the running search's line
    h.popup.on_error(6);
    assert_eq!(h.popup.search_state(), (Some(0), None));
    // the Search's own refusal started nothing
    h.popup.on_error(7);
    assert_eq!(h.popup.search_state(), (None, None));
}
