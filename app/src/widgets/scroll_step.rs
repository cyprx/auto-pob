//! PoB's keys on a hovered dropdown or slider (DropDownControl.lua:476-516,
//! SliderControl.lua:164-180): the wheel, PageUp/PageDown and the arrow keys step it, unless
//! PoB's "Disable control scroll interaction" option stops the wheel on a closed dropdown.
//! One step per wheel notch; the wheel is then taken from the frame so a list around the
//! control doesn't scroll too.

use eframe::egui::{self, Event, Key};

/// PoB's two options for this, kept in egui's memory by the app
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollOptions {
    /// `disableScrollControlInteraction`
    pub disable: bool,
    /// `invertSliderScrollDirection`
    pub invert_slider: bool,
}

/// Where a text field has the keyboard, as PoB's ControlHosts see it (ControlHost.lua:56-71)
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FocusScope {
    /// A text field has the keyboard
    pub typing: bool,
    /// ... and it is Build's own: the top bar's or the sidebar's
    pub build_box: bool,
    /// The controls being drawn are Build's own (the top bar and the sidebar)
    pub build_host: bool,
}

fn scope_id() -> egui::Id {
    egui::Id::NULL.with("pob_focus_scope")
}

/// The app says where the keyboard is, once a frame before drawing
pub fn set_scope(ctx: &egui::Context, scope: FocusScope) {
    ctx.data_mut(|d| d.insert_temp(scope_id(), scope));
}

/// The app says whose controls it draws next: Build's (true) or a tab's or window's
pub fn set_build_host(ctx: &egui::Context, build: bool) {
    let mut scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
    scope.build_host = build;
    set_scope(ctx, scope);
}

/// A text field of this control's host has the keyboard: PoB gives that host's keys to it,
/// so a hovered dropdown or slider of the host gets none (ControlHost.lua:56-71). Build's
/// host is processed before the tab's (Build.lua:1230), so a tab's box holds only the tab's
/// controls, and a Build box holds every control.
fn held(ctx: &egui::Context) -> bool {
    let scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
    scope.typing && (scope.build_box || !scope.build_host)
}

fn options_id() -> egui::Id {
    egui::Id::NULL.with("pob_scroll_options")
}

pub fn set_options(ctx: &egui::Context, options: ScrollOptions) {
    ctx.data_mut(|d| d.insert_temp(options_id(), options));
}

fn options(ctx: &egui::Context) -> ScrollOptions {
    ctx.data(|d| d.get_temp(options_id())).unwrap_or_default()
}

/// The notches and keys of this frame: (steps down, the wheel's share, Shift, Ctrl), down
/// being the wheel down, PageDown, Down or Right
fn steps(input: &egui::InputState, arrows: &[(Key, i32)]) -> (i32, i32, bool, bool) {
    let mut wheel = 0;
    let mut keys = 0;
    let (mut shift, mut ctrl) = (false, false);
    for event in &input.events {
        match event {
            Event::MouseWheel {
                delta, modifiers, ..
            } if delta.y != 0.0 => {
                // the content moves down: the wheel went up
                wheel += if delta.y > 0.0 { -1 } else { 1 };
                shift |= modifiers.shift;
                ctrl |= modifiers.command;
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if let Some((_, step)) = arrows.iter().find(|(k, _)| k == key) {
                    keys += step;
                    shift |= modifiers.shift;
                    ctrl |= modifiers.command;
                }
            }
            _ => {}
        }
    }
    (wheel + keys, wheel, shift, ctrl)
}

/// Takes the wheel and the keys out of this frame
fn take(ui: &egui::Ui, arrows: &[(Key, i32)]) {
    ui.input_mut(|i| {
        i.smooth_scroll_delta = egui::Vec2::ZERO;
        i.events.retain(|e| match e {
            Event::MouseWheel { .. } => false,
            Event::Key { key, .. } => !arrows.iter().any(|(k, _)| k == key),
            _ => true,
        });
    });
}

const DROPDOWN_KEYS: [(Key, i32); 4] = [
    (Key::PageUp, -1),
    (Key::PageDown, 1),
    (Key::ArrowUp, -1),
    (Key::ArrowDown, 1),
];

/// A dropdown of `len` entries, `selected`, drawn as `response`: the entry the wheel or keys
/// step it to while it is pointed at, if another. `open`: its list is dropped, where the wheel
/// scrolls a list too long to show whole instead, and only the arrows step it.
pub fn dropdown_step(
    ui: &egui::Ui,
    response: &egui::Response,
    len: usize,
    selected: usize,
    open: bool,
) -> Option<usize> {
    if len == 0 || !response.enabled() || !(response.hovered() || open) {
        return None;
    }
    // a dropped list is the selected control itself: its own search box has the keyboard
    if !open && held(ui.ctx()) {
        return None;
    }
    // a dropped list scrolls on the wheel and PageUp/PageDown only when it is too long to
    // show whole; a short one steps (DropDownControl.lua:488-511)
    let spacing = ui.spacing();
    let rows = spacing.combo_height / (spacing.interact_size.y + spacing.item_spacing.y);
    // the search box takes a row
    let scrolls = open && len as f32 > rows - 1.0;
    let arrows: &[(Key, i32)] = if scrolls {
        &DROPDOWN_KEYS[2..]
    } else {
        &DROPDOWN_KEYS
    };
    let (mut step, wheel, _, _) = ui.input(|i| steps(i, arrows));
    if scrolls || options(ui.ctx()).disable {
        step -= wheel;
    }
    if step == 0 {
        return None;
    }
    take(ui, if scrolls { arrows } else { &DROPDOWN_KEYS });
    // SetSel clamps to the list
    let to = (selected as i64 + i64::from(step)).clamp(0, len as i64 - 1) as usize;
    (to != selected).then_some(to)
}

const SLIDER_KEYS: [(Key, i32); 4] = [
    (Key::ArrowDown, -1),
    (Key::ArrowLeft, -1),
    (Key::ArrowUp, 1),
    (Key::ArrowRight, 1),
];

/// A slider at `value` (0 to 1) drawn as `response`: the value the wheel or arrows move it to
/// while it is pointed at: 0.05 a step, 0.25 with Shift, 0.01 with Ctrl, the wheel reversed
/// by PoB's option.
pub fn slider_step(ui: &egui::Ui, response: &egui::Response, value: f64) -> Option<f64> {
    if !response.enabled() || !response.hovered() || held(ui.ctx()) {
        return None;
    }
    // a focused slider takes the arrows itself (egui's own step)
    let focused = response.has_focus();
    let arrows: &[(Key, i32)] = if focused { &[] } else { &SLIDER_KEYS };
    let (total, wheel, shift, ctrl) = ui.input(|i| steps(i, arrows));
    let keys = total - wheel;
    // the wheel down lowers the value (SliderControl.lua:164)
    let wheel = if options(ui.ctx()).invert_slider {
        wheel
    } else {
        -wheel
    };
    let step = keys + wheel;
    if step == 0 {
        return None;
    }
    take(ui, arrows);
    let size = if shift {
        0.25
    } else if ctrl {
        0.01
    } else {
        0.05
    };
    let to = (value + f64::from(step) * size).clamp(0.0, 1.0);
    (to != value).then_some(to)
}

/// `dropdown_step` for an egui ComboBox, `response` being its button: whether its list is
/// dropped is read from egui.
pub fn combo_step(
    ui: &egui::Ui,
    response: &egui::Response,
    len: usize,
    selected: usize,
) -> Option<usize> {
    let open = egui::Popup::is_id_open(ui.ctx(), response.id.with("popup"));
    dropdown_step(ui, response, len, selected, open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Modifiers, Pos2, RawInput, Rect, Vec2};

    fn wheel(y: f32, modifiers: Modifiers) -> Event {
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, y),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers,
        }
    }

    /// One frame over a slider at `value` with the pointer on it; returns the step's value
    fn slide(ctx: &egui::Context, value: f64, events: Vec<Event>) -> Option<f64> {
        let mut out = None;
        let mut all = vec![Event::PointerMoved(Pos2::new(60.0, 20.0))];
        all.extend(events);
        for events in [vec![Event::PointerMoved(Pos2::new(60.0, 20.0))], all] {
            ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut v = value;
                    let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                    out = slider_step(ui, &r, value);
                },
            )
            .drop_without_applying_deltas();
        }
        out
    }

    #[test]
    fn the_wheel_moves_a_pointed_at_slider_by_pobs_steps() {
        let ctx = egui::Context::default();
        let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
        // the wheel down lowers it 0.05 (SliderControl.lua:164-171)
        assert!(close(
            slide(&ctx, 0.5, vec![wheel(-1.0, Modifiers::NONE)]),
            0.45
        ));
        assert!(close(
            slide(&ctx, 0.5, vec![wheel(1.0, Modifiers::SHIFT)]),
            0.75
        ));
        assert!(close(
            slide(&ctx, 0.5, vec![wheel(1.0, Modifiers::COMMAND)]),
            0.51
        ));
        // clamped at its ends
        assert!(slide(&ctx, 1.0, vec![wheel(1.0, Modifiers::NONE)]).is_none());
        // PoB's invert option reverses the wheel
        set_options(
            &ctx,
            ScrollOptions {
                disable: false,
                invert_slider: true,
            },
        );
        assert!(close(
            slide(&ctx, 0.5, vec![wheel(-1.0, Modifiers::NONE)]),
            0.55
        ));
    }

    /// `dropdown_step` on a dropped list of `len` entries, selected 0, with this frame's events
    fn dropped(ctx: &egui::Context, len: usize, events: Vec<Event>) -> Option<usize> {
        let mut out = None;
        ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let r = ui.button("list");
                out = dropdown_step(ui, &r, len, 0, true);
            },
        )
        .drop_without_applying_deltas();
        out
    }

    #[test]
    fn a_short_dropped_list_steps_on_the_wheel_and_a_long_one_scrolls() {
        let ctx = egui::Context::default();
        // no scroll bar: the wheel steps (DropDownControl.lua:488-497)
        assert_eq!(
            dropped(&ctx, 3, vec![wheel(-1.0, Modifiers::NONE)]),
            Some(1)
        );
        // a scroll bar: the wheel scrolls the list, the arrows still step
        assert_eq!(dropped(&ctx, 60, vec![wheel(-1.0, Modifiers::NONE)]), None);
        let down = Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(dropped(&ctx, 60, vec![down]), Some(1));
    }

    #[test]
    fn a_focused_slider_keeps_its_own_arrows() {
        let ctx = egui::Context::default();
        let up = Event::Key {
            key: Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut seen = Vec::new();
        for (focus, events) in [
            (true, vec![]),
            (false, vec![Event::PointerMoved(Pos2::new(60.0, 20.0))]),
            (false, vec![up.clone()]),
        ] {
            ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut v = 0.5;
                    let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                    if focus {
                        r.request_focus();
                    }
                    seen.push(slider_step(ui, &r, 0.5));
                },
            )
            .drop_without_applying_deltas();
        }
        // egui's own step moves a focused slider; PoB's isn't added on top (no stray command)
        assert_eq!(seen.last().copied().flatten(), None);
        // unfocused and pointed at, the arrows are PoB's
        let unfocused = egui::Context::default();
        assert!(slide(&unfocused, 0.5, vec![up]).is_some());
    }
}
