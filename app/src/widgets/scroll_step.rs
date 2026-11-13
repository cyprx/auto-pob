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
    /// A window is open that PoB would hold as a popup: nothing behind it gets the wheel or
    /// a key (Main.lua:365-367)
    pub modal: bool,
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

/// A text field had the keyboard, or Build's list was dropped, as the frame began: PoB's
/// selected control takes the frame's presses (ControlHost.lua:4-8, Build.lua:1230)
pub fn control_selected_at_start(ctx: &egui::Context) -> bool {
    let scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
    scope.typing
}

/// A box of the top bar or the sidebar has the keyboard: Build's host takes every key before
/// the tab sees it (Build.lua:1230)
pub fn build_box_typing(ctx: &egui::Context) -> bool {
    let scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
    scope.typing && scope.build_box
}

/// Under one of the app's windows, a control of a panel behind it gets nothing: panels are
/// egui's background layer, and the windows' own controls are drawn above it
fn behind_window(ctx: &egui::Context, response: &egui::Response) -> bool {
    let scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
    scope.modal && response.layer_id.order == egui::Order::Background
}

fn build_searches_id() -> egui::Id {
    egui::Id::NULL.with("pob_build_searches")
}

/// A dropped list of Build's own dropdown notes itself: it is Build's selected control
/// (Build.lua:1230), though it sits in a popup outside the top bar and the sidebar
/// A tab's own dropped list notes itself too, so a tab can tell its selected control from a
/// popup that isn't its own
pub fn note_search(ctx: &egui::Context, search: egui::Id) {
    let scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
    let pass = ctx.cumulative_pass_nr();
    if scope.build_host {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<(u64, egui::Id)>>(build_searches_id())
                .push((pass, search))
        });
    } else {
        note_tab_popup(ctx);
    }
}

fn tab_popup_id() -> egui::Id {
    egui::Id::NULL.with("pob_tab_popup")
}

/// A popup of the tab being drawn is open (its contents call this while shown)
pub fn note_tab_popup(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(tab_popup_id(), pass));
}

/// The open popup is the tab's own: one was noted this frame or the last. A dropped list of
/// Build's, or a menu of another host, is not the tab's selected control (ItemSlotControl.lua
/// :51 reads only the tab's selControl).
pub fn tab_popup_open(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_pass_nr();
    egui::Popup::is_any_open(ctx)
        && ctx
            .data(|d| d.get_temp::<u64>(tab_popup_id()))
            .is_some_and(|pass| pass + 1 >= now)
}

/// Whether one of Build's dropdowns was dropped last frame; forgets the notes
pub fn take_build_dropped(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.remove_temp::<Vec<(u64, egui::Id)>>(build_searches_id()))
        .unwrap_or_default()
        .iter()
        .any(|(pass, _)| pass + 1 == now)
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
    take_wheel(ui);
    ui.input_mut(|i| {
        i.events.retain(|e| match e {
            Event::Key { key, .. } => !arrows.iter().any(|(k, _)| k == key),
            _ => true,
        });
    });
}

fn wheel_taken_id() -> egui::Id {
    egui::Id::NULL.with("pob_wheel_taken")
}

/// Takes the wheel out of this frame, and its remainder out of the frames after it: egui doles
/// a notch out to the scroll areas over several frames (its smoothing), so a notch a control
/// stepped on would still scroll the page behind it. [`end_taken_wheel`] ends that.
fn take_wheel(ui: &egui::Ui) {
    let had = ui.input_mut(|i| {
        let had = i
            .events
            .iter()
            .any(|e| matches!(e, Event::MouseWheel { .. }));
        i.smooth_scroll_delta = egui::Vec2::ZERO;
        i.events.retain(|e| !matches!(e, Event::MouseWheel { .. }));
        had
    });
    if had {
        ui.ctx().data_mut(|d| d.insert_temp(wheel_taken_id(), true));
    }
}

/// For the app's `raw_input_hook`, before egui reads the next frame's input: after a frame
/// whose wheel a control took, the wheel's scroll action is cancelled, so the part of the notch
/// egui's smoothing still holds doesn't reach a scroll area (egui keeps it in its private
/// wheel state; a `Cancel` phase is its public way to drop it). PoB steps the control and
/// nothing else on that notch.
pub fn end_taken_wheel(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if ctx
        .data_mut(|d| d.remove_temp::<bool>(wheel_taken_id()))
        .unwrap_or(false)
    {
        raw.events.insert(
            0,
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::Vec2::ZERO,
                phase: egui::TouchPhase::Cancel,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        );
    }
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
    dropdown_step_in(ui, response, len, selected, open, None)
}

/// `dropdown_step` over the entries a search kept (`visible`, one flag an entry): PoB steps
/// through the matches only, in its drop list (DropDownControl.lua:57-107, 156-165)
pub fn dropdown_step_in(
    ui: &egui::Ui,
    response: &egui::Response,
    len: usize,
    selected: usize,
    open: bool,
    visible: Option<&[bool]>,
) -> Option<usize> {
    // the drop list: the matches, or every entry with no search
    let drop: Vec<usize> = match visible.filter(|v| v.len() == len) {
        Some(v) => (0..len).filter(|&i| v[i]).collect(),
        None => (0..len).collect(),
    };
    if drop.is_empty()
        || !response.enabled()
        || !(response.hovered() || open)
        || behind_window(ui.ctx(), response)
    {
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
    let scrolls = open && drop.len() as f32 > rows - 1.0;
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
    // ListIndexToDropIndex: a selection the search hides is before the first match
    let at = drop.iter().position(|&i| i == selected);
    let from = at.map_or(-1, |a| a as i64);
    // SetSel clamps to the drop list
    let to = drop[(from + i64::from(step)).clamp(0, drop.len() as i64 - 1) as usize];
    (to != selected).then_some(to)
}

const NUMBER_KEYS: [(Key, i32); 2] = [(Key::ArrowUp, -1), (Key::ArrowDown, 1)];

/// A `%D` box drawn as `response`: the steps up (down if negative) the wheel and Up/Down give
/// it. PoB sends these keys up to the selected control, or with none to the one pointed at
/// (ControlHost.lua:56-71); the wheel is off with "Disable control scroll interaction"
/// (EditControl.lua:680-700).
pub fn number_step(ui: &egui::Ui, response: &egui::Response) -> i32 {
    let ctx = ui.ctx();
    if !response.enabled() || behind_window(ctx, response) {
        return 0;
    }
    let focused = response.has_focus();
    if !focused {
        if !response.hovered() || held(ctx) {
            return 0;
        }
        // the hovered box gets the key only when its host has no selected control
        // (ControlHost.lua:68-73): a tab's box, while another widget of the tab has the
        // keyboard, gets nothing. Build's host is apart from the tab's, so its box still steps.
        let scope: FocusScope = ctx.data(|d| d.get_temp(scope_id())).unwrap_or_default();
        if !scope.build_host && ctx.memory(|m| m.focused()).is_some() {
            return 0;
        }
    }
    let (total, wheel, _, _) = ui.input(|i| steps(i, &NUMBER_KEYS));
    let keys = total - wheel;
    let wheel = if options(ctx).disable { 0 } else { wheel };
    if keys != 0 && focused {
        // the selected box takes its keys (ControlHost.lua:57-66); a hovered one leaves them to
        // whatever has the keyboard, as its OnKeyUp returns nil (EditControl.lua:720)
        ui.input_mut(|i| {
            i.events.retain(|e| {
                !matches!(e, Event::Key { key, .. }
                    if NUMBER_KEYS.iter().any(|(k, _)| k == key))
            });
        });
    }
    if wheel != 0 {
        take_wheel(ui);
    }
    -(keys + wheel)
}

const SLIDER_KEYS: [(Key, i32); 4] = [
    (Key::ArrowDown, -1),
    (Key::ArrowLeft, -1),
    (Key::ArrowUp, 1),
    (Key::ArrowRight, 1),
];

fn focused_slider_id() -> egui::Id {
    egui::Id::NULL.with("pob_focused_slider")
}

fn slider_keys_id() -> egui::Id {
    egui::Id::NULL.with("pob_slider_keys")
}

/// The arrows a focused slider takes this frame: (its id, the pass, steps up, Shift, Ctrl)
type SliderKeys = (egui::Id, u64, i32, bool, bool);

/// Call before drawing a slider. The slider with the keyboard (egui's focus, PoB's selected
/// control) steps on all four arrows by PoB's steps (SliderControl.lua:164-180), so its arrows
/// are taken out of the frame before egui's own slider sees them: egui would step it by its
/// own amount, on Left and Right only. [`slider_step`] spends them.
pub fn slider_keys(ui: &egui::Ui) {
    let ctx = ui.ctx();
    let Some(id) = ctx.data(|d| d.get_temp::<egui::Id>(focused_slider_id())) else {
        return;
    };
    if ctx.memory(|m| m.focused()) != Some(id) {
        return;
    }
    let (total, wheel, shift, ctrl) = ui.input(|i| steps(i, &SLIDER_KEYS));
    let keys = total - wheel;
    if keys == 0 {
        return;
    }
    ui.input_mut(|i| {
        i.events.retain(
            |e| !matches!(e, Event::Key { key, .. } if SLIDER_KEYS.iter().any(|(k, _)| k == key)),
        );
    });
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp::<SliderKeys>(slider_keys_id(), (id, pass, keys, shift, ctrl)));
}

fn list_selected_id() -> egui::Id {
    egui::Id::NULL.with("pob_list_selected")
}

/// A tab notes, before it draws, that one of its lists that holds its selection apart from
/// egui's focus (the item databases) is its selected control this frame
pub fn note_list_selected(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(list_selected_id(), pass));
}

/// Another control of the host `response` is in is PoB's selected control: a widget of its
/// layer with the keyboard (a list row, a box), a dropped list, or a database list the tab
/// noted. PoB then gives the frame's keys and wheel to that control, and none to the control
/// pointed at (ControlHost.lua:56-73).
fn other_selected(ctx: &egui::Context, response: &egui::Response) -> bool {
    let focused_elsewhere = ctx
        .memory(|m| m.focused())
        .filter(|id| *id != response.id)
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| r.layer_id == response.layer_id);
    let list = response.layer_id.order == egui::Order::Background
        && ctx.data(|d| d.get_temp::<u64>(list_selected_id())) == Some(ctx.cumulative_pass_nr());
    focused_elsewhere || list || egui::Popup::is_any_open(ctx)
}

fn slider_size(shift: bool, ctrl: bool) -> f64 {
    if shift {
        0.25
    } else if ctrl {
        0.01
    } else {
        0.05
    }
}

/// A slider at `value` (0 to 1) drawn as `response`: the value the wheel or arrows move it to
/// while it is pointed at, or the arrows while it has the keyboard ([`slider_keys`] took them
/// before it was drawn): 0.05 a step, 0.25 with Shift, 0.01 with Ctrl, the wheel reversed by
/// PoB's option.
pub fn slider_step(ui: &egui::Ui, response: &egui::Response, value: f64) -> Option<f64> {
    let ctx = ui.ctx();
    if response.has_focus() {
        // the arrows stay on it: egui would move the focus on with Up and Down
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        ctx.data_mut(|d| d.insert_temp(focused_slider_id(), response.id));
        let pass = ctx.cumulative_pass_nr();
        let taken = ctx.data_mut(|d| {
            let taken = d.get_temp::<SliderKeys>(slider_keys_id());
            d.remove::<SliderKeys>(slider_keys_id());
            taken
        });
        if let Some((id, at, keys, shift, ctrl)) = taken
            && id == response.id
            && at == pass
            && response.enabled()
        {
            let to = (value + f64::from(keys) * slider_size(shift, ctrl)).clamp(0.0, 1.0);
            return (to != value).then_some(to);
        }
    }
    if !response.enabled()
        || !response.hovered()
        || held(ctx)
        || behind_window(ctx, response)
        || (!response.has_focus() && other_selected(ctx, response))
    {
        return None;
    }
    // a focused slider's arrows were taken before it was drawn: the wheel is left
    let arrows: &[(Key, i32)] = if response.has_focus() {
        &[]
    } else {
        &SLIDER_KEYS
    };
    let (total, wheel, shift, ctrl) = ui.input(|i| steps(i, arrows));
    let keys = total - wheel;
    // the wheel down lowers the value (SliderControl.lua:164)
    let wheel = if options(ctx).invert_slider {
        wheel
    } else {
        -wheel
    };
    let step = keys + wheel;
    if step == 0 {
        return None;
    }
    take(ui, arrows);
    let to = (value + f64::from(step) * slider_size(shift, ctrl)).clamp(0.0, 1.0);
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
    // a dropped list's search keeps some entries (`Dropdown` notes them)
    let visible = open
        .then(|| ui.data(|d| d.get_temp::<Vec<bool>>(response.id.with("visible"))))
        .flatten();
    dropdown_step_in(ui, response, len, selected, open, visible.as_deref())
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

    /// A notch a slider took scrolls nothing behind it, in its own frame or the frames after it,
    /// where egui's smoothing would dole the rest of the notch out to the scroll area
    #[test]
    fn a_taken_notch_leaves_no_remainder_to_scroll_the_page() {
        let run = |hook: bool| {
            let ctx = egui::Context::default();
            let mut offset = 0.0;
            let mut stepped = None;
            let mut time = 1.0;
            for frame in 0..30 {
                let mut events = vec![Event::PointerMoved(Pos2::new(60.0, 20.0))];
                if frame == 2 {
                    events.push(wheel(-1.0, Modifiers::NONE));
                }
                time += 1.0 / 60.0;
                let mut raw = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                };
                if hook {
                    end_taken_wheel(&ctx, &mut raw);
                }
                ctx.run_ui(raw, |ui| {
                    let out = egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut v = 0.5;
                        let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                        if let Some(to) = slider_step(ui, &r, 0.5) {
                            stepped = Some(to);
                        }
                        ui.add_space(2000.0);
                    });
                    offset = out.state.clamped_offset().y;
                })
                .drop_without_applying_deltas();
            }
            (stepped, offset)
        };
        // the leak this guards against: without the hook the page moves after the step
        let (stepped, offset) = run(false);
        assert!(stepped.is_some());
        assert!(offset > 0.0, "egui's smoothing no longer leaks: {offset}");
        let (stepped, offset) = run(true);
        assert!(stepped.is_some(), "the slider didn't step");
        assert_eq!(offset, 0.0, "the page scrolled behind the slider");
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
    fn under_a_window_only_the_windows_own_dropdowns_step() {
        let down = || Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        // a dropped list in a panel, and one in a window, each stepped by Down
        let run = |in_window: bool| {
            let ctx = egui::Context::default();
            let mut out = None;
            // a window's first frame only sizes it
            for events in [vec![], vec![down()]] {
                ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        set_scope(
                            ui.ctx(),
                            FocusScope {
                                modal: true,
                                ..Default::default()
                            },
                        );
                        if in_window {
                            egui::Window::new("w").show(ui.ctx(), |ui| {
                                let r = ui.button("list");
                                out = dropdown_step(ui, &r, 3, 0, true);
                            });
                        } else {
                            let r = ui.button("list");
                            out = dropdown_step(ui, &r, 3, 0, true);
                        }
                    },
                )
                .drop_without_applying_deltas();
            }
            out
        };
        assert_eq!(
            run(false),
            None,
            "a panel's dropdown stepped under a window"
        );
        assert_eq!(run(true), Some(1), "the window's own dropdown didn't step");
    }

    /// The slider with the keyboard steps by PoB's steps on all four arrows, pointed at or not,
    /// and egui's own step doesn't move it (SliderControl.lua:164-180)
    #[test]
    fn a_focused_slider_steps_by_pobs_steps_on_every_arrow() {
        let ctx = egui::Context::default();
        let key = |key: Key, modifiers: Modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // (value after egui drew it, PoB's step) for each frame
        let mut seen = Vec::new();
        let frames = [
            (true, vec![]),
            (false, vec![]),
            (false, vec![key(Key::ArrowUp, Modifiers::NONE)]),
            (false, vec![key(Key::ArrowDown, Modifiers::NONE)]),
            (false, vec![key(Key::ArrowLeft, Modifiers::NONE)]),
            (false, vec![key(Key::ArrowRight, Modifiers::NONE)]),
            (false, vec![key(Key::ArrowUp, Modifiers::SHIFT)]),
            (false, vec![key(Key::ArrowDown, Modifiers::COMMAND)]),
        ];
        for (focus, events) in frames {
            ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    // a button first, so Up would have somewhere to move the focus to
                    let _ = ui.button("above");
                    let mut v = 0.5;
                    slider_keys(ui);
                    let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                    if focus {
                        r.request_focus();
                    }
                    seen.push((v, slider_step(ui, &r, 0.5)));
                },
            )
            .drop_without_applying_deltas();
        }
        let steps: Vec<Option<f64>> = seen[2..]
            .iter()
            .map(|(_, s)| s.map(|v| (v * 100.0).round() / 100.0))
            .collect();
        assert_eq!(
            steps,
            [
                Some(0.55),
                Some(0.45),
                Some(0.45),
                Some(0.55),
                Some(0.75),
                Some(0.49)
            ]
        );
        // egui's own step never moved it
        assert!(seen.iter().all(|(v, _)| *v == 0.5), "{seen:?}");
        // unfocused and not pointed at, the arrows do nothing
        let unfocused = egui::Context::default();
        let mut out = Some(0.0);
        unfocused
            .run_ui(
                RawInput {
                    events: vec![key(Key::ArrowUp, Modifiers::NONE)],
                    ..Default::default()
                },
                |ui| {
                    let mut v = 0.5;
                    slider_keys(ui);
                    let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
                    out = slider_step(ui, &r, 0.5);
                },
            )
            .drop_without_applying_deltas();
        assert_eq!(out, None);
    }

    /// A hovered top bar box steps on Down while a tab's box has the keyboard, as PoB's
    /// ControlHost gives it the KeyUp (ControlHost.lua:68-73), but leaves the key to the focused
    /// box, whose caret PoB moves on the KeyDown
    #[test]
    fn a_hovered_build_box_steps_but_leaves_the_arrows_to_the_focused_box() {
        let ctx = egui::Context::default();
        let down = Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut result = (0, false);
        let mut rect = Rect::NOTHING;
        for (step, events) in [
            (0, vec![]),
            (1, vec![Event::PointerMoved(Pos2::new(30.0, 10.0))]),
            (2, vec![down]),
        ] {
            ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let typing = step > 0;
                    set_scope(
                        ui.ctx(),
                        FocusScope {
                            typing,
                            build_box: false,
                            build_host: true,
                            modal: false,
                        },
                    );
                    let mut level = "5".to_owned();
                    let r = ui.add(egui::TextEdit::singleline(&mut level).desired_width(60.0));
                    rect = r.rect;
                    let n = number_step(ui, &r);
                    let left = ui.input(|i| {
                        i.events.iter().any(|e| {
                            matches!(
                                e,
                                Event::Key {
                                    key: Key::ArrowDown,
                                    ..
                                }
                            )
                        })
                    });
                    set_build_host(ui.ctx(), false);
                    let mut notes = "a\nb".to_owned();
                    let notes = ui.add(egui::TextEdit::multiline(&mut notes));
                    if step == 0 {
                        notes.request_focus();
                    }
                    if step == 2 {
                        result = (n, left);
                    }
                },
            )
            .drop_without_applying_deltas();
        }
        assert!(rect.contains(Pos2::new(30.0, 10.0)), "{rect:?}");
        assert_eq!(result, (-1, true));
    }
}
