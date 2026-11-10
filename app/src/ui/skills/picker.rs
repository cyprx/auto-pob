//! The gem picker: a popup with a search box and PoB's gem list (spec 5.4). It sends searches
//! (debounced), shows the streamed list with PoB's DPS marks, and picks with `SetGem`.

use crate::engine::views::{GemRow, GemSearchView, GemTooltipView};
use crate::engine::{Command, DpsMark, GemFilter, GemSearchQuery, GemTarget, GemTooltipQuery};
use crate::ui::tree::Rects;
use crate::widgets::{pob_colour, tooltip_ui};
use eframe::egui;
use std::sync::Arc;

/// How long typing must pause before the search is sent (spec 5.4).
const DEBOUNCE: f64 = 0.1;
/// A row must be hovered this long before its tooltip is asked for.
const HOVER_DELAY: f64 = 0.1;
/// Float slack for comparing times that are sums of tenths.
const EPS: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKey {
    Up,
    Down,
    Enter,
    Escape,
}

#[derive(Debug)]
pub enum PickerAction {
    None,
    Pick(Command),
    Close,
}

#[derive(Debug, Default)]
pub struct PickerOutput {
    pub commands: Vec<Command>,
    pub queries: Vec<Command>,
    pub close: bool,
}

pub struct GemPicker {
    group: usize,
    target: GemTarget,
    text: String,
    filter: GemFilter,
    /// The highlighted entry; the imbued popup's "None" row is entry 0
    selected: Option<usize>,
    last_edit: f64,
    /// The last search sent; replies to anything else are stale
    sent: Option<GemSearchQuery>,
    view: Option<Arc<GemSearchView>>,
    failed: bool,
    percent: Option<u32>,
    ended: bool,
    hover: Option<(String, f64)>,
    tooltip_asked: Option<GemTooltipQuery>,
    tooltip: Option<Arc<GemTooltipView>>,
    focus_pending: bool,
    scroll_to_selected: bool,
    pub(crate) rects: Rects,
}

impl GemPicker {
    pub fn open(group: usize, target: GemTarget, now: f64) -> GemPicker {
        GemPicker {
            group,
            target,
            text: String::new(),
            filter: GemFilter::All,
            selected: None,
            last_edit: now,
            sent: None,
            view: None,
            failed: false,
            percent: None,
            ended: false,
            hover: None,
            tooltip_asked: None,
            tooltip: None,
            focus_pending: true,
            scroll_to_selected: false,
            rects: Rects::default(),
        }
    }

    pub fn group(&self) -> usize {
        self.group
    }

    pub fn target(&self) -> GemTarget {
        self.target
    }

    fn query(&self) -> GemSearchQuery {
        GemSearchQuery {
            group: self.group,
            target: self.target,
            text: self.text.clone(),
            filter: self.filter,
        }
    }

    /// Types into the search box.
    pub fn set_text(&mut self, text: &str, now: f64) {
        if self.text != text {
            self.text = text.to_owned();
            self.selected = None;
            self.last_edit = now;
        }
    }

    fn set_filter(&mut self, filter: GemFilter) {
        if self.filter != filter {
            self.filter = filter;
            self.selected = None;
            // a button press isn't typing: no debounce
            self.last_edit = f64::NEG_INFINITY;
        }
    }

    /// A streamed list. A reply to a search that isn't the last one sent is ignored.
    pub fn on_search(&mut self, view: Arc<GemSearchView>) {
        if self.sent.as_ref() != Some(&view.query) {
            return;
        }
        self.ended |= view.ended;
        self.percent = view.percent;
        self.view = Some(view);
        let n = self.entries();
        if self.selected.is_some_and(|s| s >= n) {
            self.selected = n.checked_sub(1);
        }
    }

    /// The DPS step reported progress.
    pub fn on_progress(&mut self, percent: u32) {
        if self.view.as_ref().is_some_and(|v| v.sorting) {
            self.percent = Some(percent);
        }
    }

    /// The DPS sort failed; the list stays as it is.
    pub fn on_failed(&mut self) {
        self.failed = true;
    }

    pub fn on_tooltip(&mut self, view: Arc<GemTooltipView>) {
        if self.tooltip_asked.as_ref() == Some(&view.query) {
            self.tooltip = Some(view);
        }
    }

    /// The list answers what is typed now, not an older search.
    fn current_view(&self) -> Option<&Arc<GemSearchView>> {
        self.view.as_ref().filter(|v| v.query == self.query())
    }

    fn imbued(&self) -> bool {
        self.target == GemTarget::Imbued
    }

    /// The rows of the list, plus the imbued popup's "None".
    fn entries(&self) -> usize {
        self.view
            .as_ref()
            .map_or(0, |v| v.rows.len() + usize::from(self.imbued()))
    }

    fn pick(&self, entry: usize) -> Option<Command> {
        let view = self.current_view()?;
        let imbued = self.imbued();
        if imbued && entry == 0 {
            return Some(Command::SetImbuedSupport {
                group: self.group,
                gem_id: None,
            });
        }
        let row = view.rows.get(entry - usize::from(imbued))?;
        Some(self.pick_row(row))
    }

    fn pick_row(&self, row: &GemRow) -> Command {
        match self.target {
            GemTarget::Slot(slot) => Command::SetGem {
                group: self.group,
                slot,
                gem_id: row.gem_id.clone(),
            },
            GemTarget::Imbued => Command::SetImbuedSupport {
                group: self.group,
                gem_id: Some(row.gem_id.clone()),
            },
        }
    }

    pub fn key(&mut self, key: PickerKey) -> PickerAction {
        let n = self.entries();
        match key {
            PickerKey::Escape => PickerAction::Close,
            PickerKey::Up | PickerKey::Down if n == 0 => PickerAction::None,
            PickerKey::Up => {
                self.selected = Some(self.selected.map_or(0, |s| s.saturating_sub(1)));
                self.scroll_to_selected = true;
                PickerAction::None
            }
            PickerKey::Down => {
                self.selected = Some(self.selected.map_or(0, |s| (s + 1).min(n - 1)));
                self.scroll_to_selected = true;
                PickerAction::None
            }
            PickerKey::Enter => match self.pick(self.selected.unwrap_or(0)) {
                Some(command) => PickerAction::Pick(command),
                None => PickerAction::None,
            },
        }
    }

    /// The search to send now, once typing has paused (the first one goes at once).
    pub fn tick(&mut self, now: f64) -> Option<Command> {
        let want = self.query();
        if self.sent.as_ref() == Some(&want) {
            return None;
        }
        if self.sent.is_some() && now - self.last_edit < DEBOUNCE - EPS {
            return None;
        }
        self.failed = false;
        self.percent = None;
        self.sent = Some(want.clone());
        Some(Command::SearchGems(want))
    }

    pub fn show(&mut self, ui: &mut egui::Ui, now: f64) -> PickerOutput {
        let mut out = PickerOutput::default();
        let keys = ui.input_mut(|i| {
            let mut keys = Vec::new();
            for (egui_key, key) in [
                (egui::Key::ArrowUp, PickerKey::Up),
                (egui::Key::ArrowDown, PickerKey::Down),
                (egui::Key::Enter, PickerKey::Enter),
                (egui::Key::Escape, PickerKey::Escape),
            ] {
                if i.consume_key(egui::Modifiers::NONE, egui_key) {
                    keys.push(key);
                }
            }
            keys
        });
        for key in keys {
            match self.key(key) {
                PickerAction::None => {}
                PickerAction::Pick(command) => {
                    out.commands.push(command);
                    out.close = true;
                }
                PickerAction::Close => out.close = true,
            }
        }
        if out.close {
            return out;
        }

        let ctx = ui.ctx().clone();
        let mut open = true;
        egui::Window::new(match self.target {
            GemTarget::Imbued => "Imbued support",
            GemTarget::Slot(_) => "Choose a gem",
        })
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(&ctx, |ui| self.body(ui, now, &mut out));
        if !open || self.ended {
            out.close = true;
        }
        if out.close {
            return out;
        }

        if let Some(query) = self.tick(now) {
            out.queries.push(query);
        } else if self.sent.as_ref() != Some(&self.query()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        out
    }

    fn body(&mut self, ui: &mut egui::Ui, now: f64, out: &mut PickerOutput) {
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            let mut text = self.text.clone();
            let edit = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("Search gems")
                    .desired_width(300.0),
            );
            self.rects.note(&edit, || "picker_text".into());
            if std::mem::take(&mut self.focus_pending) {
                edit.request_focus();
            }
            if edit.changed() {
                self.set_text(&text, now);
            }
            for (label, filter) in [("S", GemFilter::Support), ("A", GemFilter::Active)] {
                let on = self.filter == filter;
                let button = ui.selectable_label(on, label).on_hover_text(match filter {
                    GemFilter::Support => "Support gems first",
                    _ => "Active skill gems first",
                });
                self.rects.note(&button, || format!("picker_{label}"));
                if button.clicked() {
                    self.set_filter(if on { GemFilter::All } else { filter });
                }
            }
        });

        let sorting = self.view.as_ref().is_some_and(|v| v.sorting);
        if self.failed {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 120), "DPS sort failed");
                let retry = ui.button("Retry");
                self.rects.note(&retry, || "picker_retry".into());
                if retry.clicked() {
                    self.failed = false;
                    if let Some(query) = self.sent.clone() {
                        out.queries.push(Command::SearchGems(query));
                    }
                }
            });
        } else if sorting {
            let fraction = f32::from(self.percent.unwrap_or(0).min(100) as u16) / 100.0;
            ui.add(egui::ProgressBar::new(fraction).desired_height(6.0));
        }
        ui.separator();

        let Some(view) = self.view.clone() else {
            ui.spinner();
            return;
        };
        let imbued = self.imbued();
        let mut hovered = None;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if imbued {
                    let row = self.entry_frame(ui, 0, |ui| {
                        ui.label("None");
                    });
                    self.rects.note(&row, || "picker_none".into());
                    if row.clicked() {
                        out.commands.push(Command::SetImbuedSupport {
                            group: self.group,
                            gem_id: None,
                        });
                        out.close = true;
                    }
                }
                for (i, gem) in view.rows.iter().enumerate() {
                    let entry = i + usize::from(imbued);
                    let response = self.entry_frame(ui, entry, |ui| {
                        gem_row(ui, gem, view.base_dps);
                    });
                    self.rects.note(&response, || format!("picker_row_{i}"));
                    if response.hovered() {
                        hovered = Some(gem.gem_id.clone());
                    }
                    if let Some(t) = &self.tooltip
                        && t.query.gem_id.as_deref() == Some(&gem.gem_id)
                        && !t.lines.is_empty()
                    {
                        response.clone().on_hover_ui(|ui| tooltip_ui(ui, &t.lines));
                    }
                    if response.clicked() {
                        out.commands.push(self.pick_row(gem));
                        out.close = true;
                    }
                }
                if view.rows.is_empty() {
                    ui.weak("No gems match.");
                }
            });
        self.scroll_to_selected = false;
        self.ask_tooltip(ui, hovered, now, out);
    }

    /// One clickable list entry, highlighted when it is the keyboard's.
    fn entry_frame(
        &mut self,
        ui: &mut egui::Ui,
        entry: usize,
        add: impl FnOnce(&mut egui::Ui),
    ) -> egui::Response {
        let selected = self.selected == Some(entry);
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let frame = egui::Frame::new().fill(fill).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(add);
        });
        let response = ui.interact(
            frame.response.rect,
            ui.scope_id().with(("picker_entry", entry)),
            egui::Sense::click(),
        );
        if selected && self.scroll_to_selected {
            response.scroll_to_me(None);
        }
        response
    }

    /// A row hovered long enough asks for its tooltip.
    fn ask_tooltip(
        &mut self,
        ui: &egui::Ui,
        hovered: Option<String>,
        now: f64,
        out: &mut PickerOutput,
    ) {
        let Some(id) = hovered else {
            self.hover = None;
            return;
        };
        let since = match &self.hover {
            Some((h, since)) if *h == id => *since,
            _ => {
                self.hover = Some((id.clone(), now));
                now
            }
        };
        let query = GemTooltipQuery {
            group: self.group,
            target: self.target,
            gem_id: Some(id),
        };
        if self.tooltip_asked.as_ref() == Some(&query) {
            return;
        }
        if now - since >= HOVER_DELAY - EPS {
            self.tooltip_asked = Some(query.clone());
            out.queries.push(Command::GemTooltip(query));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

/// The change against the DPS without the gem.
fn dps_change(dps: f64, base: f64) -> String {
    if base > 0.0 {
        format!("{:+.1}%", (dps - base) / base * 100.0)
    } else {
        format!("{dps:.0}")
    }
}

fn gem_row(ui: &mut egui::Ui, gem: &GemRow, base_dps: f64) {
    let default = ui.visuals().text_color();
    let dps_colour = gem
        .dps_colour
        .as_deref()
        .map_or(default, |c| pob_colour(c, default));
    ui.label(match gem.mark {
        Some(DpsMark::Check) => egui::RichText::new("\u{2713}").color(dps_colour),
        Some(DpsMark::Plus) => egui::RichText::new("+").color(dps_colour),
        None => egui::RichText::new(" "),
    });
    let name_colour = pob_colour(&gem.colour, default);
    ui.label(egui::RichText::new(&gem.name).color(name_colour));
    ui.weak(&gem.tags);
    if let Some(dps) = gem.dps {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(dps_change(dps, base_dps)).color(dps_colour));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    fn gem_row(id: &str) -> GemRow {
        GemRow {
            gem_id: id.into(),
            name: id.into(),
            colour: "^x33FF77".into(),
            tags: "Spell".into(),
            support: false,
            can_support: false,
            dps: None,
            dps_colour: None,
            mark: None,
        }
    }

    fn search_view(q: GemSearchQuery, ids: &[&str]) -> Arc<GemSearchView> {
        Arc::new(GemSearchView {
            query: q,
            rows: ids.iter().map(|i| gem_row(i)).collect(),
            base_dps: 100.0,
            sorting: false,
            percent: None,
            ended: false,
        })
    }

    fn debug(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|c| format!("{c:?}")).collect()
    }

    /// A picker whose first (empty) search was sent and answered with `ids`.
    fn answered(target: GemTarget, ids: &[&str]) -> GemPicker {
        let mut p = GemPicker::open(0, target, 0.0);
        let Some(Command::SearchGems(q)) = p.tick(0.0) else {
            panic!("the first search goes at once");
        };
        p.on_search(search_view(q, ids));
        p
    }

    #[test]
    fn picker_debounces_search_100ms() {
        let mut p = GemPicker::open(0, GemTarget::Slot(1), 0.0);
        assert!(matches!(p.tick(0.0), Some(Command::SearchGems(_))));
        p.set_text("f", 0.0);
        p.set_text("fi", 0.05);
        assert!(p.tick(0.05).is_none());
        assert!(p.tick(0.1).is_none(), "only 50 ms since the last key");
        let sent = p.tick(0.15);
        assert_eq!(
            debug(&sent.into_iter().collect::<Vec<_>>()),
            ["SearchGems(GemSearchQuery { group: 0, target: Slot(1), text: \"fi\", filter: All })"]
        );
        assert!(p.tick(0.5).is_none(), "one search for the text");
    }

    #[test]
    fn up_down_enter_picks() {
        let mut p = answered(GemTarget::Slot(2), &["a", "b", "c"]);
        assert!(matches!(p.key(PickerKey::Down), PickerAction::None));
        assert!(matches!(p.key(PickerKey::Down), PickerAction::None));
        let PickerAction::Pick(c) = p.key(PickerKey::Enter) else {
            panic!("Enter picks");
        };
        assert_eq!(
            format!("{c:?}"),
            "SetGem { group: 0, slot: 2, gem_id: \"b\" }"
        );
    }

    #[test]
    fn enter_through_show_picks_then_closes() {
        let mut h = Harness::new(GemTarget::Slot(0), &["a", "b"]);
        h.key(Key::ArrowDown);
        h.key(Key::ArrowDown);
        let o = h.key(Key::Enter);
        assert_eq!(
            debug(&o.commands),
            ["SetGem { group: 0, slot: 0, gem_id: \"b\" }"]
        );
        assert!(o.close);
    }

    #[test]
    fn escape_closes_without_picking() {
        let mut p = answered(GemTarget::Slot(0), &["a"]);
        assert!(matches!(p.key(PickerKey::Escape), PickerAction::Close));
        let mut h = Harness::new(GemTarget::Slot(0), &["a"]);
        let o = h.key(Key::Escape);
        assert!(o.commands.is_empty() && o.close);
    }

    #[test]
    fn stale_search_reply_ignored() {
        let mut p = GemPicker::open(0, GemTarget::Slot(0), 0.0);
        let Some(Command::SearchGems(first)) = p.tick(0.0) else {
            panic!()
        };
        p.set_text("fire", 1.0);
        let Some(Command::SearchGems(second)) = p.tick(1.2) else {
            panic!()
        };
        // the reply to the first search arrives after the second was sent
        p.on_search(search_view(first.clone(), &["old"]));
        assert!(p.view.is_none(), "a reply to an older text was shown");
        // so does a reply for another group or target
        let mut other = second.clone();
        other.group = 3;
        p.on_search(search_view(other, &["elsewhere"]));
        assert!(p.view.is_none());
        p.on_search(search_view(second, &["new"]));
        assert_eq!(p.view.as_ref().unwrap().rows[0].gem_id, "new");
    }

    #[test]
    fn failed_dps_shows_retry_and_retry_resends() {
        let mut h = Harness::new(GemTarget::Slot(0), &["a"]);
        h.p.on_failed();
        h.frame(vec![]);
        let o = h.click("picker_retry");
        assert_eq!(
            debug(&o.queries),
            ["SearchGems(GemSearchQuery { group: 0, target: Slot(0), text: \"\", filter: All })"]
        );
        assert!(!h.p.failed);
    }

    #[test]
    fn imbued_none_row_clears() {
        let mut h = Harness::new(GemTarget::Imbued, &["a"]);
        let o = h.click("picker_none");
        assert_eq!(
            debug(&o.commands),
            ["SetImbuedSupport { group: 0, gem_id: None }"]
        );
        assert!(o.close);
        // a row picks the imbued gem
        let mut h = Harness::new(GemTarget::Imbued, &["a"]);
        let o = h.click("picker_row_0");
        assert_eq!(
            debug(&o.commands),
            ["SetImbuedSupport { group: 0, gem_id: Some(\"a\") }"]
        );
    }

    #[test]
    fn ended_search_closes_picker() {
        let mut h = Harness::new(GemTarget::Slot(0), &["a"]);
        let mut v = (*search_view(h.p.query(), &[])).clone();
        v.ended = true;
        h.p.on_search(Arc::new(v));
        assert!(h.frame(vec![]).close);
    }

    #[test]
    fn filter_buttons_toggle_and_search_at_once() {
        let mut h = Harness::new(GemTarget::Slot(0), &["a"]);
        let o = h.click("picker_S");
        assert_eq!(
            debug(&o.queries),
            [
                "SearchGems(GemSearchQuery { group: 0, target: Slot(0), text: \"\", filter: Support })"
            ]
        );
        let o = h.click("picker_S");
        assert!(debug(&o.queries)[0].contains("filter: All"));
    }

    #[test]
    fn hovering_a_row_asks_for_its_tooltip_once() {
        let mut h = Harness::new(GemTarget::Slot(0), &["a", "b"]);
        for _ in 0..3 {
            h.frame(vec![]);
        }
        let pos = h.p.rects.map["picker_row_1"].center();
        let mut asked = Vec::new();
        for _ in 0..4 {
            asked.extend(h.frame(vec![Event::PointerMoved(pos)]).queries);
        }
        assert_eq!(
            debug(&asked),
            ["GemTooltip(GemTooltipQuery { group: 0, target: Slot(0), gem_id: Some(\"b\") })"]
        );
    }

    #[test]
    fn typing_in_the_box_searches_after_the_pause() {
        let mut h = Harness::new(GemTarget::Slot(0), &["a"]);
        let o = h.frame(vec![Event::Text("ice".into())]);
        assert!(o.queries.is_empty());
        let mut sent = Vec::new();
        for _ in 0..3 {
            sent.extend(h.frame(vec![]).queries);
        }
        assert!(debug(&sent)[0].contains("text: \"ice\""), "{sent:?}");
    }

    struct Harness {
        p: GemPicker,
        ctx: egui::Context,
        time: f64,
    }

    impl Harness {
        /// A picker whose first search was sent and answered.
        fn new(target: GemTarget, ids: &[&str]) -> Self {
            let mut h = Harness {
                p: GemPicker::open(0, target, 1.0),
                ctx: egui::Context::default(),
                time: 1.0,
            };
            let o = h.frame(vec![]);
            let Some(Command::SearchGems(q)) = o.queries.into_iter().next() else {
                panic!("the first frame sends the search");
            };
            h.p.on_search(search_view(q, ids));
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> PickerOutput {
            self.time += 0.2;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = PickerOutput::default();
            let time = self.time;
            let output = self.ctx.run_ui(input, |ui| {
                let pass = self.p.show(ui, time);
                out.commands.extend(pass.commands);
                out.queries.extend(pass.queries);
                out.close |= pass.close;
            });
            output.drop_without_applying_deltas();
            out
        }

        fn key(&mut self, key: Key) -> PickerOutput {
            self.frame(vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }])
        }

        fn click(&mut self, name: &str) -> PickerOutput {
            // the window settles over a few frames
            for _ in 0..3 {
                self.frame(vec![]);
            }
            let pos = self.p.rects.map[name].center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = PickerOutput::default();
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                let o = self.frame(events);
                all.commands.extend(o.commands);
                all.queries.extend(o.queries);
                all.close |= o.close;
            }
            all
        }
    }
}
