//! A text box that sends its text when an edit completes: on Enter or when it loses focus,
//! and only when the text changed. Esc puts back what was last sent.
//!
//! Every "send on Enter or blur" field of the tabs is one of these, so they all behave alike:
//! - an incoming view replaces the text only while the box is unfocused and the engine isn't
//!   busy (`busy`), so typed text isn't lost to a view that predates it;
//! - an edit still being typed can be flushed (`take_pending`), once; the blur that follows
//!   sends nothing;
//! - a box that wasn't drawn while its edit was pending is flushed by `EditBoxes::end_frame`.

use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// How a box looks and what it accepts.
pub struct EditBoxOptions {
    pub width: f32,
    /// The most bytes the box holds, as PoB's `EditControl` counts its limit (EditControl.lua:148)
    pub byte_limit: Option<usize>,
    /// Characters the box accepts, for typed and pasted text. With a filter or a limit the box
    /// takes an edit as PoB's does (`name_box::spliced_by`): a paste turns non-ASCII bytes into
    /// "?" and is refused whole past the limit, typed keys past it are refused one by one.
    pub filter: Option<fn(char) -> bool>,
    pub hint: Option<&'static str>,
    /// PoB's integer box (filter `%D`, EditControl.lua:64-75): + and - buttons, and Up, Down
    /// and the wheel step its number by one, sent at once (EditControl.lua:680-700)
    pub numeric: bool,
    /// A `^%-%d` box: its step goes below 0
    pub signed: bool,
}

impl Default for EditBoxOptions {
    fn default() -> Self {
        Self {
            width: 50.0,
            byte_limit: None,
            filter: None,
            hint: None,
            numeric: false,
            signed: false,
        }
    }
}

impl EditBoxOptions {
    /// PoB's `%D` box: digits only, at most `limit` of them, with its + and - buttons
    pub const fn number(width: f32, limit: Option<usize>) -> Self {
        Self {
            width,
            byte_limit: limit,
            filter: Some(|c| c.is_ascii_digit()),
            hint: None,
            numeric: true,
            signed: false,
        }
    }
}

/// `command` as a step's when `stepped` (`Command::Step`), else as it is
pub fn as_step(command: crate::engine::Command, stepped: bool) -> crate::engine::Command {
    if stepped {
        crate::engine::Command::Step(Box::new(command))
    } else {
        command
    }
}

/// A clearable box's "x" (EditControl.lua:73-77): shown while the box holds text; a click
/// empties it. True when it was clicked.
pub fn clear_button(ui: &mut egui::Ui, text: &mut String) -> Option<egui::Response> {
    if text.is_empty() {
        return None;
    }
    let x = ui.small_button("x");
    if x.clicked() {
        text.clear();
    }
    Some(x)
}

/// PoB's step of a `%D` box (EditControl.lua:680-700): up adds one to its number, or makes it
/// 1; down takes one from a number above 0 (any number in a `signed` `^%-%d` box), or makes it 0
pub fn stepped(text: &str, up: bool, signed: bool) -> String {
    let current = text.trim().parse::<i64>().ok();
    match (current, up) {
        (Some(n), true) => n.saturating_add(1).to_string(),
        (None, true) => "1".to_owned(),
        (Some(n), false) if n > 0 || signed => n.saturating_sub(1).to_string(),
        (_, false) => "0".to_owned(),
    }
}

#[derive(Default)]
pub struct EditBox {
    pub(crate) text: String,
    /// What the engine last had from this box (or showed it)
    sent: String,
    /// Typed, still focused and not sent
    pending: bool,
    /// The box has been drawn once, so `text` was seeded from the view
    seeded: bool,
    /// The last frame's edit was refused whole by the filter or the limit: PoB made no edit
    /// and called no change function
    pub(crate) refused: bool,
    /// What `show` returned last came from a step, to be sent as `Command::Step`
    pub(crate) stepped: bool,
}

impl EditBox {
    /// Draws the box. Returns the text to send when an edit completes (Enter or blur, changed).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        current: &str,
        busy: bool,
        opts: &EditBoxOptions,
    ) -> (egui::Response, Option<String>) {
        self.stepped = false;
        if !opts.numeric {
            return self.show_box(ui, id, current, busy, opts);
        }
        ui.horizontal(|ui| {
            let (response, mut send) = self.show_box(ui, id, current, busy, opts);
            let mut step = 0;
            ui.add_enabled_ui(response.enabled(), |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                if ui.small_button("+").clicked() {
                    step += 1;
                }
                if ui.small_button("-").clicked() {
                    step -= 1;
                }
            });
            step += crate::widgets::scroll_step::number_step(ui, &response);
            if step != 0 {
                // SetText(..., true): the change function runs at once (EditControl.lua:96-103)
                for _ in 0..step.unsigned_abs() {
                    self.text = stepped(&self.text, step > 0, opts.signed);
                }
                self.sent.clone_from(&self.text);
                self.pending = false;
                self.stepped = true;
                send = Some(self.text.clone());
            }
            (response, send)
        })
        .inner
    }

    fn show_box(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        current: &str,
        busy: bool,
        opts: &EditBoxOptions,
    ) -> (egui::Response, Option<String>) {
        let focused = ui.memory(|m| m.has_focus(id));
        // a box that lost focus this frame still holds its edit: the view must not replace it
        if !focused && (!busy || !self.seeded) && !self.pending {
            self.text.clear();
            self.text.push_str(current);
            self.sent.clear();
            self.sent.push_str(current);
        }
        self.seeded = true;
        let guarded = opts.filter.is_some() || opts.byte_limit.is_some();
        let before = guarded.then(|| self.text.clone());
        let pasted = guarded && focused && crate::ui::name_box::pasted(ui);
        let mut edit = egui::TextEdit::singleline(&mut self.text)
            .id(id)
            .desired_width(opts.width);
        if let Some(hint) = opts.hint {
            edit = edit.hint_text(hint);
        }
        let response = ui.add(edit);
        crate::widgets::url_click::open_on_ctrl_click(ui, &response, &self.text);
        self.refused = false;
        if let Some(before) = before
            && response.changed()
        {
            let keeps = opts.filter.unwrap_or(|_| true);
            let limit = opts.byte_limit.unwrap_or(usize::MAX);
            let raw = std::mem::take(&mut self.text);
            self.text = crate::ui::name_box::spliced_by(&before, &raw, keeps, limit, pasted);
            // typing over the text with itself is an edit; one the box drops is none
            self.refused = raw != before && self.text == before;
        }
        let mut send = None;
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.text.clone_from(&self.sent);
            self.pending = false;
        } else if response.lost_focus() || (self.pending && !response.has_focus()) {
            if self.text != self.sent {
                self.sent.clone_from(&self.text);
                send = Some(self.text.clone());
            }
            self.pending = false;
        } else {
            self.pending = response.has_focus() && self.text != self.sent;
        }
        (response, send)
    }

    /// The edit still being typed, once: a blur after it sends nothing.
    pub fn take_pending(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.pending) {
            return None;
        }
        self.sent.clone_from(&self.text);
        Some(self.text.clone())
    }

    /// Drops what is being typed (the user chose not to save, or the item is going).
    pub fn discard(&mut self) {
        self.pending = false;
        self.sent.clone_from(&self.text);
    }

    /// An edit is being typed and not sent.
    pub fn has_pending(&self) -> bool {
        self.pending
    }
}

static NEXT_SALT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Boxes by key, for owners with many or changing fields.
pub struct EditBoxes<K: Hash + Eq + Clone> {
    boxes: HashMap<K, EditBox>,
    /// The keys drawn since the last `end_frame`
    drawn: HashSet<K>,
    /// Keeps the boxes' ids apart from other owners' boxes with the same keys
    salt: egui::Id,
}

impl<K: Hash + Eq + Clone> Default for EditBoxes<K> {
    fn default() -> Self {
        Self {
            boxes: HashMap::new(),
            drawn: HashSet::new(),
            // `Id::unique` hashes its source, so each owner needs its own number
            salt: egui::Id::unique((
                "edit_boxes",
                NEXT_SALT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            )),
        }
    }
}

impl<K: Hash + Eq + Clone> EditBoxes<K> {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        key: K,
        current: &str,
        busy: bool,
        opts: &EditBoxOptions,
    ) -> (egui::Response, Option<String>) {
        let mut hasher = std::hash::DefaultHasher::new();
        key.hash(&mut hasher);
        let id = self.salt.with(hasher.finish());
        self.drawn.insert(key.clone());
        self.boxes
            .entry(key)
            .or_default()
            .show(ui, id, current, busy, opts)
    }

    /// What the box `key` sent last came from a step: send it as `Command::Step`
    pub fn stepped(&self, key: &K) -> bool {
        self.boxes.get(key).is_some_and(|b| b.stepped)
    }

    /// The pending edit of the box with focus (at most one has), once.
    pub fn take_pending(&mut self) -> Option<(K, String)> {
        self.boxes
            .iter_mut()
            .find_map(|(k, b)| b.take_pending().map(|text| (k.clone(), text)))
    }

    /// Pending edits of boxes not drawn since the last call (R25). Call it once per frame,
    /// after drawing.
    pub fn end_frame(&mut self) -> Vec<(K, String)> {
        let drawn = std::mem::take(&mut self.drawn);
        self.boxes
            .iter_mut()
            .filter(|(k, _)| !drawn.contains(*k))
            .filter_map(|(k, b)| b.take_pending().map(|text| (k.clone(), text)))
            .collect()
    }

    /// Drops the edits of boxes not drawn since the last `end_frame` or `discard_undrawn`: what
    /// they were for (a skill, an item) is gone, so sending would hit another target.
    pub fn discard_undrawn(&mut self) {
        let drawn = std::mem::take(&mut self.drawn);
        for (_, b) in self.boxes.iter_mut().filter(|(k, _)| !drawn.contains(*k)) {
            b.discard();
        }
    }

    /// The id salt, which keeps this owner's boxes apart from other owners' boxes.
    #[cfg(test)]
    pub(crate) fn salt(&self) -> egui::Id {
        self.salt
    }

    pub fn has_pending(&self) -> bool {
        self.boxes.values().any(EditBox::has_pending)
    }

    /// The keys of the boxes with an edit pending.
    pub fn pending_keys(&self) -> impl Iterator<Item = &K> {
        self.boxes.iter().filter(|(_, b)| b.pending).map(|(k, _)| k)
    }

    /// The text typed into a box and not sent, if any.
    pub fn pending_text(&self, key: &K) -> Option<&str> {
        self.boxes
            .get(key)
            .filter(|b| b.pending)
            .map(|b| b.text.as_str())
    }

    pub fn discard(&mut self) {
        for b in self.boxes.values_mut() {
            b.discard();
        }
    }

    /// The text a box holds.
    #[cfg(test)]
    pub fn text(&self, key: &K) -> Option<&str> {
        self.boxes.get(key).map(|b| b.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    struct Harness {
        ctx: egui::Context,
        time: f64,
        current: String,
        busy: bool,
        opts: EditBoxOptions,
        a: EditBox,
        /// a second, unrelated widget, to click away to
        other: Rect,
        a_rect: Rect,
        id: egui::Id,
        draw: bool,
    }

    impl Harness {
        fn new(current: &str) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                time: 1.0,
                current: current.to_owned(),
                busy: false,
                opts: EditBoxOptions::default(),
                a: EditBox::default(),
                other: Rect::NOTHING,
                a_rect: Rect::NOTHING,
                id: egui::Id::unique("box"),
                draw: true,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<String> {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut sent = Vec::new();
            let mut a_rect = self.a_rect;
            let mut other = self.other;
            self.ctx
                .run_ui(input, |ui| {
                    if self.draw {
                        let (r, s) = self
                            .a
                            .show(ui, self.id, &self.current, self.busy, &self.opts);
                        a_rect = r.rect;
                        sent.extend(s);
                    }
                    other = ui.button("elsewhere").rect;
                })
                .drop_without_applying_deltas();
            self.a_rect = a_rect;
            self.other = other;
            sent
        }

        fn click(&mut self, rect: Rect) -> Vec<String> {
            let pos = rect.center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = Vec::new();
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                all.extend(self.frame(events));
            }
            all
        }

        fn focus(&mut self) {
            self.click(self.a_rect);
        }

        fn click_away(&mut self) -> Vec<String> {
            self.click(self.other)
        }

        fn key(&mut self, key: Key) -> Vec<String> {
            let mut all = Vec::new();
            for pressed in [true, false] {
                all.extend(self.frame(vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]));
            }
            all
        }

        fn type_text(&mut self, text: &str) -> Vec<String> {
            self.frame(vec![Event::Text(text.to_owned())])
        }
    }

    #[test]
    fn enter_sends_once_and_blur_after_enter_sends_nothing() {
        let mut h = Harness::new("1");
        h.focus();
        let mut sent = h.type_text("2");
        sent.extend(h.key(Key::Enter));
        assert_eq!(sent, ["12"]);
        let later = h.click_away();
        assert!(later.is_empty(), "{later:?}");
    }

    #[test]
    fn blur_sends_a_changed_text_and_an_unchanged_one_nothing() {
        let mut h = Harness::new("1");
        h.focus();
        h.type_text("3");
        assert_eq!(h.click_away(), ["13"]);
        let mut h = Harness::new("1");
        h.focus();
        assert!(h.click_away().is_empty());
    }

    #[test]
    fn escape_reverts_and_sends_nothing() {
        let mut h = Harness::new("1");
        h.focus();
        h.type_text("9");
        let mut sent = h.key(Key::Escape);
        sent.extend(h.frame(vec![]));
        sent.extend(h.click_away());
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(h.a.text, "1");
        assert!(!h.a.has_pending());
    }

    #[test]
    fn unfocused_box_follows_the_view_unless_busy() {
        let mut h = Harness::new("1");
        h.focus();
        h.type_text("5");
        h.busy = true;
        let sent = h.click_away();
        assert_eq!(sent, ["15"]);
        // the engine is behind: its old view doesn't replace what was typed
        h.current = "1".into();
        h.frame(vec![]);
        assert_eq!(h.a.text, "15");
        // caught up: the view wins
        h.busy = false;
        h.current = "7".into();
        h.frame(vec![]);
        assert_eq!(h.a.text, "7");
    }

    #[test]
    fn flush_takes_the_pending_text_once() {
        let mut h = Harness::new("1");
        h.focus();
        h.type_text("4");
        assert!(h.a.has_pending());
        assert_eq!(h.a.take_pending().as_deref(), Some("14"));
        assert_eq!(h.a.take_pending(), None);
        let sent = h.click_away();
        assert!(sent.is_empty(), "a blur after a flush resends: {sent:?}");
    }

    #[test]
    fn discard_drops_the_pending_edit_and_its_blur() {
        let mut h = Harness::new("1");
        h.focus();
        h.type_text("4");
        h.a.discard();
        assert!(!h.a.has_pending());
        assert!(h.click_away().is_empty());
    }

    #[test]
    fn filter_and_limit_apply_to_typed_and_pasted_text() {
        let mut h = Harness::new("");
        h.opts.filter = Some(|c| c.is_ascii_digit());
        h.opts.byte_limit = Some(2);
        h.focus();
        // PoB's Insert drops the rejected characters, then refuses a text past the limit whole
        h.frame(vec![Event::Paste("a1b2".into())]);
        assert_eq!(h.a.text, "12");
        h.a.text.clear();
        h.frame(vec![Event::Paste("a1b2c3".into())]);
        assert_eq!(h.a.text, "");
        h.type_text("x7y");
        assert_eq!(h.a.text, "7");
        h.type_text("89");
        assert_eq!(h.a.text, "78");
    }

    /// The shared box opens the first http(s) link of its text on a Ctrl+click (url_click)
    #[test]
    fn a_ctrl_click_on_the_box_opens_its_link() {
        let ctx = egui::Context::default();
        let mut b = EditBox::default();
        let id = egui::Id::unique("url_box");
        let mut rect = Rect::NOTHING;
        let mut opened = Vec::new();
        for step in 0..2 {
            let events = if step == 0 {
                vec![]
            } else {
                vec![
                    Event::ModifiersChanged(Modifiers::COMMAND),
                    Event::PointerMoved(rect.center()),
                    Event::PointerButton {
                        pos: rect.center(),
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::COMMAND,
                    },
                ]
            };
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let opts = EditBoxOptions {
                        width: 300.0,
                        ..EditBoxOptions::default()
                    };
                    rect = b.show(ui, id, "see https://pobb.in/x", false, &opts).0.rect;
                },
            );
            for command in &out.platform_output.commands {
                if let egui::OutputCommand::OpenUrl(url) = command {
                    opened.push(url.url.clone());
                }
            }
            out.drop_without_applying_deltas();
        }
        assert_eq!(opened, ["https://pobb.in/x"]);
    }

    #[test]
    fn a_number_steps_as_pobs() {
        assert_eq!(stepped("41", true, false), "42");
        assert_eq!(stepped("", true, false), "1");
        assert_eq!(stepped("x", true, false), "1");
        assert_eq!(stepped("3", false, false), "2");
        // a `%D` box never goes below 0
        assert_eq!(stepped("0", false, false), "0");
        assert_eq!(stepped("", false, false), "0");
        // a `^%-%d` box goes below 0
        assert_eq!(stepped("0", false, true), "-1");
    }

    /// PoB's `%D` box: its + button, Up and Down while it has the keyboard, and the wheel over
    /// it step the number and send it at once (EditControl.lua:64-75, 680-700)
    #[test]
    fn a_number_box_steps_on_its_button_the_arrows_and_the_wheel() {
        let mut h = Harness::new("5");
        h.opts = EditBoxOptions::number(40.0, Some(2));
        h.frame(vec![]);
        let plus = egui::Rect::from_center_size(
            egui::pos2(h.a_rect.right() + 8.0, h.a_rect.center().y),
            egui::vec2(2.0, 2.0),
        );
        assert_eq!(h.click(plus), ["6"]);
        h.current = "6".into();
        h.focus();
        assert_eq!(h.key(Key::ArrowUp), ["7"]);
        assert_eq!(h.key(Key::ArrowDown), ["6"]);
        h.click_away();
        h.current = "6".into();
        h.frame(vec![Event::PointerMoved(h.a_rect.center())]);
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(h.frame(vec![wheel]), ["5"]);
        // the filter is `%D`
        h.focus();
        h.frame(vec![Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]);
        h.type_text("a12");
        assert_eq!(h.a.text, "12");
    }

    /// PoB's limit counts bytes (EditControl.lua:148), so a 3-byte character takes three
    #[test]
    fn the_limit_counts_bytes_not_characters() {
        let mut h = Harness::new("");
        h.opts.byte_limit = Some(4);
        h.focus();
        h.type_text("\u{65e5}\u{65e5}x");
        assert_eq!(h.a.text, "\u{65e5}x");
        // a paste turns each non-ASCII byte into "?" (EditControl.lua:533)
        let mut h = Harness::new("");
        h.opts.byte_limit = Some(50);
        h.focus();
        h.frame(vec![Event::Paste("a\u{e9}".into())]);
        assert_eq!(h.a.text, "a??");
    }

    #[test]
    fn a_box_seeds_from_the_view_on_its_first_draw_even_when_busy() {
        let mut h = Harness::new("5");
        assert_eq!(h.a.text, "5");
        let mut fresh = Harness::new("1");
        fresh.a = EditBox::default();
        fresh.busy = true;
        fresh.current = "8".into();
        fresh.frame(vec![]);
        assert_eq!(fresh.a.text, "8");
        h.busy = true;
        h.current = "6".into();
        h.frame(vec![]);
        assert_eq!(
            h.a.text, "5",
            "later, a busy engine's view doesn't replace it"
        );
    }

    #[test]
    fn two_owners_boxes_with_the_same_key_have_their_own_ids() {
        let a = EditBoxes::<&'static str>::default();
        let b = EditBoxes::<&'static str>::default();
        assert_ne!(a.salt(), b.salt());
    }

    #[test]
    fn discard_undrawn_drops_only_boxes_not_drawn() {
        let mut boxes = EditBoxes::<&'static str>::default();
        boxes.boxes.entry("gone").or_default().pending = true;
        boxes.boxes.entry("here").or_default().pending = true;
        boxes.drawn.insert("here");
        boxes.discard_undrawn();
        assert_eq!(boxes.pending_keys().collect::<Vec<_>>(), [&"here"]);
    }

    #[test]
    fn undrawn_pending_box_is_returned_by_end_frame() {
        let ctx = egui::Context::default();
        let mut boxes = EditBoxes::<&'static str>::default();
        let opts = EditBoxOptions::default();
        let mut time = 1.0;
        let mut frame = |boxes: &mut EditBoxes<&'static str>,
                         events: Vec<Event>,
                         draw: bool|
         -> (Rect, Vec<(&'static str, String)>) {
            time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut rect = Rect::NOTHING;
            let mut ended = Vec::new();
            ctx.run_ui(input, |ui| {
                if draw {
                    rect = boxes.show(ui, "k", "1", false, &opts).0.rect;
                }
                ended = boxes.end_frame();
            })
            .drop_without_applying_deltas();
            (rect, ended)
        };
        let (rect, _) = frame(&mut boxes, vec![], true);
        let pos = rect.center();
        for pressed in [true, false] {
            frame(
                &mut boxes,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
                true,
            );
        }
        frame(&mut boxes, vec![], true);
        let (_, ended) = frame(&mut boxes, vec![Event::Text("2".into())], true);
        assert!(ended.is_empty(), "a drawn box isn't returned: {ended:?}");
        assert!(boxes.has_pending());
        let (_, ended) = frame(&mut boxes, vec![], false);
        assert_eq!(ended, [("k", "12".to_owned())]);
        assert!(!boxes.has_pending());
        let (_, ended) = frame(&mut boxes, vec![], false);
        assert!(ended.is_empty(), "returned once: {ended:?}");
    }
}
