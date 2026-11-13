//! The popup window: any PoB popup drawn from a `PopupView` (R5). Its controls come in PoB's
//! order and are grouped into rows by position; every change goes to the engine as a
//! `SetPopupControl` or `PressPopupButton`, and the engine's next view is what the window shows.
//! The window is modal: nothing behind it takes input, because any other command would end
//! the engine's session (R4).

use super::tree::Rects;
use crate::engine::views::{ControlTooltipView, PopupControl, PopupControlKind, PopupView};
use crate::engine::{Command, ControlTooltipQuery, PopupInput, PopupKind, Snapshot};
use crate::widgets::{EditBoxOptions, EditBoxes, item_tooltip_ui, pob_text, search_combo_ui};
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;

/// A control must be hovered this long before its tooltip is asked for.
pub(crate) const HOVER_DELAY: f64 = 0.1;
/// Controls whose vertical centres are this close are on one row (R5).
const ROW_TOLERANCE: f32 = 6.0;
/// The widest a control is drawn, in points.
const MAX_CONTROL_WIDTH: f32 = 560.0;

/// What the window asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct PopupOutput {
    pub commands: Vec<Command>,
    /// Questions (tooltips); the app sends them without waiting for the answer.
    pub queries: Vec<Command>,
}

/// Where PoB's trade windows link to (R20); the engine keeps only these, and so does the UI
pub const TRADE_SEARCH: &str = "https://www.pathofexile.com/trade/search/";

/// The trade link of the user's own press (R20): a popup button press may answer with one, and
/// only the answer to that press, by its seq, is opened in the browser and copied, once.
#[derive(Debug, Default)]
pub struct TradeLink {
    /// The seq of the last button press, while it is unanswered
    pressed: Option<u64>,
    /// A link to open and copy on the next frame
    ready: Option<String>,
}

impl TradeLink {
    /// A popup button press went out as `seq`.
    pub fn pressed(&mut self, seq: u64) {
        self.pressed = Some(seq);
    }

    /// The engine's answer: a link for `seq`
    pub fn on_link(&mut self, seq: u64, url: &str) {
        if self.pressed != Some(seq) {
            return;
        }
        self.pressed = None;
        if url.starts_with(TRADE_SEARCH) {
            self.ready = Some(url.to_owned());
        }
    }

    /// The link to open and copy now, once
    pub fn take(&mut self) -> Option<String> {
        self.ready.take()
    }
}

/// Extras a window adds around the engine's controls (the Options window's "auto-pob only"
/// boxes and fields). They draw only; what they change is theirs to send.
pub trait Decorate {
    /// Drawn above a row, before its controls
    fn before_row(&mut self, _ui: &mut egui::Ui, _view: &PopupView, _row: &[usize]) {}
    /// Drawn right after a control, in its row
    fn after_control(&mut self, _ui: &mut egui::Ui, _control: &PopupControl) {}
}

struct Plain;

impl Decorate for Plain {}

/// The shown controls of `view`, grouped by vertical centre and sorted by x within a row (R5).
pub fn rows(view: &PopupView) -> Vec<Vec<usize>> {
    let centre = |i: usize| {
        let c = &view.controls[i];
        c.pos[1] + c.size[1] / 2.0
    };
    let mut shown: Vec<usize> = (0..view.controls.len())
        .filter(|&i| view.controls[i].shown)
        .collect();
    shown.sort_by(|&a, &b| centre(a).total_cmp(&centre(b)));
    let mut grouped: Vec<(f32, Vec<usize>)> = Vec::new();
    for i in shown {
        match grouped.last_mut() {
            Some((y, row)) if (centre(i) - *y).abs() <= ROW_TOLERANCE => row.push(i),
            _ => grouped.push((centre(i), vec![i])),
        }
    }
    grouped
        .into_iter()
        .map(|(_, mut row)| {
            row.sort_by(|&a, &b| view.controls[a].pos[0].total_cmp(&view.controls[b].pos[0]));
            row
        })
        .collect()
}

/// Where a row dragged from `from` lands when dropped at height `y`: its 0-based place after
/// the drop, counted over the other rows whose middle is above `y`.
pub(crate) fn drop_place(rects: &[egui::Rect], from: usize, y: f32) -> usize {
    rects
        .iter()
        .enumerate()
        .filter(|&(i, r)| i != from && r.center().y < y)
        .count()
}

/// The line that shows where a dragged row will land: across the row it goes before, or under
/// the last one
fn drop_line(rects: &[egui::Rect], from: usize, to: usize) -> Option<(egui::Rangef, f32)> {
    let others: Vec<&egui::Rect> = rects
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != from)
        .map(|(_, r)| r)
        .collect();
    let y = match others.get(to) {
        Some(r) => r.top(),
        None => others.last()?.bottom(),
    };
    let first = rects.first()?;
    Some((first.x_range(), y))
}

/// The height of a popup list row: a selectable button around one line of 14 px text, at
/// least the interact height (the button's own minimum)
pub(crate) fn popup_row_height(ui: &egui::Ui) -> f32 {
    let text = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(14.0)));
    let padding = ui.spacing().button_padding.y;
    (text + 2.0 * padding).max(ui.spacing().interact_size.y)
}

/// Asks for a control's tooltip once the pointer has rested on it, and keeps the answer to the
/// latest question only.
#[derive(Default)]
pub(crate) struct HoverTip {
    /// What the pointer rests on, and since when
    hover: Option<(ControlTooltipQuery, f64)>,
    /// The latest query sent; only its reply is kept
    asked: Option<ControlTooltipQuery>,
    tooltip: Option<Arc<ControlTooltipView>>,
    /// What the pointer is over this frame
    now: Option<ControlTooltipQuery>,
}

impl HoverTip {
    /// Notes what the pointer is over, and draws the tooltip once its reply is here.
    pub(crate) fn note(&mut self, response: &egui::Response, query: ControlTooltipQuery) {
        if response.hovered() || (!response.enabled() && response.contains_pointer()) {
            self.now = Some(query.clone());
        }
        if let Some(t) = &self.tooltip
            && t.query == query
        {
            let t = t.clone();
            response.clone().on_hover_ui(|ui| {
                item_tooltip_ui(ui, &t.tooltip);
            });
        }
    }

    /// Call once per frame after drawing. Returns the question to send, if the pointer has
    /// rested long enough.
    pub(crate) fn end_frame(&mut self, ctx: &egui::Context, now: f64) -> Option<Command> {
        match (self.now.take(), &self.hover) {
            (Some(q), Some((p, _))) if q == *p => {}
            (Some(q), _) => {
                self.hover = Some((q, now));
                self.asked = None;
                self.tooltip = None;
            }
            (None, _) => {
                self.hover = None;
                self.asked = None;
                self.tooltip = None;
            }
        }
        let (query, since) = self.hover.as_ref()?;
        if self.asked.as_ref() == Some(query) {
            return None;
        }
        if now - since >= HOVER_DELAY {
            self.asked = Some(query.clone());
            Some(Command::ControlTooltip(query.clone()))
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            None
        }
    }

    /// A reply; one to a question that is no longer the latest is ignored.
    pub(crate) fn on_reply(&mut self, view: &Arc<ControlTooltipView>) {
        if self.asked.as_ref() == Some(&view.query) {
            self.tooltip = Some(view.clone());
        }
    }

    /// The question the kept tooltip answers.
    #[cfg(test)]
    pub(crate) fn held(&self) -> Option<&ControlTooltipQuery> {
        self.tooltip.as_ref().map(|t| &t.query)
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The Anoint list's stat sort, which runs in the background
#[derive(Default)]
struct SortState {
    percent: Option<u32>,
    failed: Option<String>,
}

/// The timeless window's search, which runs in the background (R17)
#[derive(Default)]
struct SearchState {
    /// Running, this far along
    percent: Option<u32>,
    /// The last search failed with PoB's message
    failed: Option<String>,
}

#[derive(Default)]
pub struct PopupWindow {
    view: Option<Arc<PopupView>>,
    boxes: EditBoxes<String>,
    /// A slider's value shown until the engine's view catches up
    sliders: HashMap<String, f64>,
    tip: HoverTip,
    sort: SortState,
    search: SearchState,
    /// The seq of the last button press, so only its refusal ends the search line
    press_seq: Option<u64>,
    /// A `ClosePopup` went out as this command: a view from before it is stale
    close_seq: Option<u64>,
    /// A text box had the focus when the last frame ended
    typing: bool,
    /// `OpenPopup` went out as this command and nothing has answered it yet
    opening: Option<u64>,
    pub(crate) rects: Rects,
    /// The height the rows needed in the last frame
    content_height: f32,
}

impl PopupWindow {
    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    /// The window is up, or an `OpenPopup` is on its way: nothing behind may take input, or
    /// a command could end the session before it shows.
    pub fn is_blocking(&self) -> bool {
        self.view.is_some() || self.opening.is_some()
    }

    /// `OpenPopup` went out as command `seq`.
    pub fn open_sent(&mut self, seq: u64) {
        self.opening = Some(seq);
    }

    pub fn kind(&self) -> Option<&PopupKind> {
        self.view.as_ref().map(|v| &v.kind)
    }

    #[cfg(test)]
    pub(crate) fn box_text(&self, name: &str) -> Option<&str> {
        self.boxes.text(&name.to_owned())
    }

    #[cfg(test)]
    pub(crate) fn forget_typing(&mut self) {
        self.typing = false;
    }

    #[cfg(test)]
    pub(crate) fn tooltip_held(&self) -> bool {
        self.tip.tooltip.is_some()
    }

    #[cfg(test)]
    pub(crate) fn sort_line(&self) -> (Option<u32>, Option<&str>) {
        (self.sort.percent, self.sort.failed.as_deref())
    }

    /// Forgets everything, as for another build or a restarted engine.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A snapshot with no seq of its own (tests): it counts as the latest.
    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        self.on_snapshot_at(snapshot, u64::MAX);
    }

    pub fn on_snapshot_at(&mut self, snapshot: &Snapshot, seq: u64) {
        if matches!(snapshot, Snapshot::Popup(_)) && self.opening.is_some_and(|o| seq >= o) {
            self.opening = None;
        }
        match snapshot {
            Snapshot::Popup(Some(view)) => {
                // a view the engine made before it heard of the close must not reopen the window
                if self.close_seq.is_some_and(|c| seq <= c) {
                    return;
                }
                self.close_seq = None;
                let new_session = self.view.as_ref().is_none_or(|old| old.kind != view.kind);
                if new_session {
                    self.forget();
                }
                self.sliders.clear();
                // a rebuilt list may put another entry under the pointer
                self.tip.reset();
                // the engine's next view is the end of a sort that was running
                self.sort.percent = None;
                self.view = Some(view.clone());
            }
            Snapshot::Popup(None) => {
                self.forget();
                self.view = None;
            }
            Snapshot::ControlTooltip(view) => self.tip.on_reply(view),
            _ => {}
        }
    }

    fn forget(&mut self) {
        self.boxes = EditBoxes::default();
        self.sliders.clear();
        self.tip.reset();
        self.sort = SortState::default();
        self.search = SearchState::default();
    }

    /// A button press of the window went out as `seq`.
    pub fn press_sent(&mut self, seq: u64) {
        self.press_seq = Some(seq);
    }

    /// The command that closed the window went out as `seq`.
    pub fn close_sent(&mut self, seq: u64) {
        self.close_seq = Some(seq);
    }

    /// The engine refused something: a value shown ahead of its view is not what PoB has.
    pub fn on_error(&mut self, seq: u64) {
        self.sliders.clear();
        // a refused Search started nothing
        if self.press_seq == Some(seq) {
            self.search.percent = None;
        }
        if self.opening.is_some_and(|o| seq >= o) {
            self.opening = None;
        }
    }

    /// The Anoint stat sort is at `percent`.
    pub fn on_progress(&mut self, percent: u32) {
        if self.view.is_some() {
            self.sort.failed = None;
            self.sort.percent = Some(percent);
        }
    }

    /// The timeless search is at `percent`; at 100 it is over (R17).
    pub fn on_search_progress(&mut self, percent: u32) {
        if self.kind() == Some(&PopupKind::TimelessJewel) {
            self.search.failed = None;
            self.search.percent = (percent < 100).then_some(percent);
        }
    }

    /// The timeless search failed with PoB's message, which the window shows (R17).
    pub fn on_search_failed(&mut self, message: String) {
        if self.kind() == Some(&PopupKind::TimelessJewel) {
            self.search.percent = None;
            self.search.failed = Some(message);
        }
    }

    #[cfg(test)]
    pub(crate) fn search_state(&self) -> (Option<u32>, Option<&str>) {
        (self.search.percent, self.search.failed.as_deref())
    }

    /// The Anoint stat sort failed; the list shows the error with Retry.
    pub fn on_failed(&mut self, message: String) {
        if self.view.is_some() {
            self.sort.percent = None;
            self.sort.failed = Some(message);
        }
    }

    /// Typed text waiting for Enter or blur, to go out before whatever comes next.
    pub fn flush(&mut self) -> Option<Command> {
        self.view.as_ref()?;
        let (name, text) = self.boxes.take_pending()?;
        Some(Command::SetPopupControl {
            name,
            value: PopupInput::Text(text),
        })
    }

    /// Closes the window and returns the command that ends the session. The window goes at once;
    /// views of the session still on their way are ignored (`close_sent`).
    pub fn close(&mut self) -> Option<Command> {
        // an open still on its way is closed too: the close follows it, and its view is stale
        let pending = self.opening.take().is_some();
        if self.view.take().is_none() && !pending {
            return None;
        }
        self.forget();
        Some(Command::ClosePopup)
    }

    /// Draws `controls` with the window's renderers, outside any window (the Compare tab's
    /// selectors). What they change comes back as `SetPopupControl`s for the caller to
    /// readdress; a box still being typed in goes first. No tooltip is ever asked for: there is
    /// no popup session to ask. Lists are not drawn.
    pub(crate) fn inline_controls(
        &mut self,
        ui: &mut egui::Ui,
        controls: &[&PopupControl],
        busy: bool,
    ) -> Vec<Command> {
        let view = PopupView {
            kind: PopupKind::Craft,
            title: String::new(),
            enter: None,
            controls: Vec::new(),
        };
        let mut out = PopupOutput::default();
        for c in controls.iter().filter(|c| c.kind != PopupControlKind::List) {
            let first = out.commands.len();
            self.draw_control(ui, &view, c, busy, &mut out);
            if c.kind != PopupControlKind::Edit
                && out.commands.len() > first
                && let Some((name, text)) = self.boxes.take_pending()
            {
                out.commands.insert(
                    first,
                    Command::SetPopupControl {
                        name,
                        value: PopupInput::Text(text),
                    },
                );
            }
        }
        out.commands
    }

    /// For `inline_controls`: the text typed into a box and not sent yet, once.
    pub(crate) fn take_typed(&mut self) -> Option<(String, String)> {
        self.boxes.take_pending()
    }

    /// For `inline_controls`, once per frame after drawing: the typed text of boxes that were
    /// not drawn this frame, to send now.
    pub(crate) fn end_inline_frame(&mut self) -> Vec<(String, String)> {
        self.tip.reset();
        self.boxes.end_frame()
    }

    /// For `inline_controls`: whether a box holds text not sent yet.
    pub(crate) fn has_typed(&self) -> bool {
        self.boxes.has_pending()
    }

    /// Draws the window. `busy`: the engine hasn't caught up with what was sent.
    pub fn show(&mut self, ctx: &egui::Context, now: f64, busy: bool) -> PopupOutput {
        self.show_with(ctx, now, busy, &mut Plain)
    }

    /// `show`, with `deco` drawing around the controls.
    pub fn show_with(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        busy: bool,
        deco: &mut dyn Decorate,
    ) -> PopupOutput {
        let mut out = PopupOutput::default();
        let Some(view) = self.view.clone() else {
            if self.opening.is_some() {
                // nothing to show yet, but nothing behind may be touched either
                egui::Modal::new(egui::Id::unique("auto_pob_popup_pending"))
                    .backdrop_color(egui::Color32::TRANSPARENT)
                    .show(ctx, |ui| ui.spinner());
            }
            return out;
        };
        // Enter clicks PoB's `enterControl` when no control has the keyboard
        // (PopupDialog.lua:79-83; ControlHost.lua:39-42 gives a selected control the key)
        let enter = view.enter.clone().filter(|_| {
            !self.typing
                && !egui::Popup::is_any_open(ctx)
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        });
        // Esc closes the window, unless it is closing a dropdown or reverting a box
        // (egui drops the focus as the frame begins, so what the last frame ended with counts)
        let escape = !std::mem::take(&mut self.typing)
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut close = escape;

        let width = window_width(&view);
        let mut content_height = self.content_height;
        egui::Modal::new(egui::Id::unique("auto_pob_popup_window")).show(ctx, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width.max(MAX_CONTROL_WIDTH));
            ui.horizontal(|ui| {
                ui.heading(pob_text_plain(&view.title));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let x = ui.button("\u{d7}");
                    self.rects.note(&x, || "popup_close".into());
                    close |= x.clicked();
                });
            });
            ui.separator();
            // the timeless search runs in the background: its progress, and a way to stop it
            if view.kind == PopupKind::TimelessJewel {
                self.search_line(ui, &mut out);
            }
            let searching = self.search.percent.is_some();
            let max_height = (ctx.content_rect().height() - 140.0).max(200.0);
            // The window's size is the last frame's, so a scroll area left to shrink would
            // stay as small as the first view it held; it is given the height it needed last.
            let shown = egui::ScrollArea::vertical()
                .id_salt("popup_scroll")
                .max_height(max_height)
                .min_scrolled_height(content_height.min(max_height))
                .auto_shrink([true, true])
                .show(ui, |ui| {
                    // nothing but Cancel is offered while PoB's search runs (the engine refuses)
                    ui.add_enabled_ui(!searching, |ui| {
                        for row in rows(&view) {
                            self.row(ui, &view, &row, busy, &mut out, deco);
                        }
                    });
                });
            content_height = shown.content_size.y;
        });
        self.content_height = content_height;

        // a box not drawn this frame (its control was hidden) drops what was typed: the
        // engine refuses a control PoB doesn't show (R25)
        self.boxes.discard_undrawn();
        if let Some(query) = self.tip.end_frame(ctx, now) {
            out.queries.push(query);
        }
        self.typing = ctx.text_edit_focused();
        if let Some(name) = enter
            && !close
            && self.search.percent.is_none()
            && view.controls.iter().any(|c| {
                c.name == name && c.kind == PopupControlKind::Button && c.shown && c.enabled
            })
        {
            out.commands.extend(self.flush());
            out.commands.push(Command::PressPopupButton { name });
        }
        if close {
            out.commands.extend(self.flush());
            out.commands.extend(self.close());
        }
        out
    }

    /// "Searching… n%" with Cancel while the timeless search runs; its failure after
    fn search_line(&mut self, ui: &mut egui::Ui, out: &mut PopupOutput) {
        if let Some(percent) = self.search.percent {
            ui.horizontal(|ui| {
                ui.spinner();
                let line = ui.label(format!("Searching\u{2026} {percent}%"));
                self.rects.note(&line, || "timeless_progress".into());
                let cancel = ui.button("Cancel search");
                self.rects.note(&cancel, || "timeless_cancel".into());
                if cancel.clicked() {
                    out.commands.push(Command::CancelTimelessSearch);
                    self.search.percent = None;
                }
            });
        } else if let Some(message) = &self.search.failed {
            let line = ui.colored_label(
                ui.visuals().error_fg_color,
                format!("The search failed: {message}"),
            );
            self.rects.note(&line, || "timeless_failed".into());
        }
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        view: &PopupView,
        row: &[usize],
        busy: bool,
        out: &mut PopupOutput,
        deco: &mut dyn Decorate,
    ) {
        deco.before_row(ui, view, row);
        let has_list = row
            .iter()
            .any(|&i| view.controls[i].kind == PopupControlKind::List);
        let mut draw = |ui: &mut egui::Ui| {
            for &i in row {
                self.control(ui, view, &view.controls[i], busy, out);
                deco.after_control(ui, &view.controls[i]);
            }
        };
        if has_list {
            ui.vertical(draw);
        } else {
            ui.horizontal_wrapped(&mut draw);
        }
    }

    fn control(
        &mut self,
        ui: &mut egui::Ui,
        view: &PopupView,
        c: &PopupControl,
        busy: bool,
        out: &mut PopupOutput,
    ) {
        let first = out.commands.len();
        self.draw_control(ui, view, c, busy, out);
        // a box still being typed in goes first, whichever control comes next (Review Focus 4)
        if c.kind != PopupControlKind::Edit
            && out.commands.len() > first
            && let Some(flush) = self.flush()
        {
            out.commands.insert(first, flush);
        }
    }

    fn draw_control(
        &mut self,
        ui: &mut egui::Ui,
        view: &PopupView,
        c: &PopupControl,
        busy: bool,
        out: &mut PopupOutput,
    ) {
        let default = ui.visuals().text_color();
        let name = c.name.as_str();
        let width = c.size[0].clamp(0.0, MAX_CONTROL_WIDTH);
        match c.kind {
            PopupControlKind::Label => {
                ui.label(pob_text(&c.label, 14.0, default));
            }
            PopupControlKind::Button => {
                let button = egui::Button::new(pob_text(&c.label, 14.0, default))
                    .min_size(egui::vec2(width.min(240.0), 0.0));
                let response = ui.add_enabled(c.enabled, button);
                self.rects.note(&response, || format!("btn_{name}"));
                self.hint(&response, c, None);
                if response.clicked() {
                    // the timeless search starts in the background: its line shows at once
                    if view.kind == PopupKind::TimelessJewel && name == "searchButton" {
                        self.search = SearchState {
                            percent: Some(0),
                            failed: None,
                        };
                    }
                    out.commands.push(Command::PressPopupButton {
                        name: name.to_owned(),
                    });
                }
            }
            PopupControlKind::Dropdown => {
                self.caption(ui, c);
                let id = ui.make_persistent_id(("popup_dropdown", name));
                let (rects, tip) = (&mut self.rects, &mut self.tip);
                let hover = c.hover_tooltip;
                let mut response = None;
                let mut picked = None;
                ui.add_enabled_ui(c.enabled, |ui| {
                    let (r, pick) = search_combo_ui(
                        ui,
                        id,
                        &c.choices,
                        c.selected,
                        Some(width.max(120.0)),
                        &mut |i, entry| {
                            rects.note(entry, || format!("{name}_{i}"));
                            if hover {
                                tip.note(
                                    entry,
                                    ControlTooltipQuery::Popup {
                                        control: name.to_owned(),
                                        index: Some(i),
                                    },
                                );
                            }
                        },
                    );
                    response = Some(r);
                    picked = pick;
                });
                if let Some(response) = response {
                    self.rects.note(&response, || name.to_owned());
                    if let Some(text) = &c.tooltip_text {
                        response.on_hover_text(text);
                    }
                }
                if let Some(index) = picked {
                    out.commands.push(Command::SetPopupControl {
                        name: name.to_owned(),
                        value: PopupInput::Choice {
                            index,
                            label: c.choices[index].clone(),
                        },
                    });
                }
            }
            PopupControlKind::Slider => {
                self.caption(ui, c);
                let mut value = self.sliders.get(name).copied().unwrap_or(c.value);
                ui.spacing_mut().slider_width = width.max(80.0);
                let mut widget = egui::Slider::new(&mut value, 0.0..=1.0).show_value(false);
                if let Some(d) = c.divisions.filter(|d| *d > 0) {
                    widget = widget.step_by(1.0 / f64::from(d));
                }
                let slider = ui.add_enabled(c.enabled, widget);
                self.rects.note(&slider, || name.to_owned());
                // the wheel and the arrows move it, to its divisions (SliderControl.lua:164-180)
                if let Some(to) = crate::widgets::scroll_step::slider_step(ui, &slider, value) {
                    let to = match c.divisions.filter(|d| *d > 0) {
                        Some(d) => (to * f64::from(d)).round() / f64::from(d),
                        None => to,
                    };
                    self.sliders.insert(name.to_owned(), to);
                    out.commands.push(Command::SetPopupControl {
                        name: name.to_owned(),
                        value: PopupInput::Number(to),
                    });
                }
                self.hint(&slider, c, None);
                if slider.dragged() || slider.changed() {
                    self.sliders.insert(name.to_owned(), value);
                }
                // it sends on release, not on every step of the drag
                if slider.drag_stopped()
                    || (slider.changed() && !slider.is_pointer_button_down_on())
                {
                    out.commands.push(Command::SetPopupControl {
                        name: name.to_owned(),
                        value: PopupInput::Number(value),
                    });
                }
            }
            PopupControlKind::Check => {
                let mut on = c.checked;
                let check = ui.add_enabled(
                    c.enabled,
                    egui::Checkbox::new(&mut on, pob_text(&c.label, 14.0, default)),
                );
                self.rects.note(&check, || name.to_owned());
                self.hint(&check, c, None);
                if check.changed() {
                    out.commands.push(Command::SetPopupControl {
                        name: name.to_owned(),
                        value: PopupInput::Check(on),
                    });
                }
            }
            PopupControlKind::Edit => {
                self.caption(ui, c);
                // the box's own filter and limit, as PoB's EditControl holds them
                let opts = EditBoxOptions {
                    width: width.max(60.0),
                    char_limit: c.limit,
                    filter: c
                        .ascii_only
                        .then_some(|ch: char| crate::ui::name_box::Filter::Plain.keeps(ch)),
                    ..EditBoxOptions::default()
                };
                let enabled = c.enabled;
                let boxes = &mut self.boxes;
                let inner = ui.add_enabled_ui(enabled, |ui| {
                    boxes.show(ui, name.to_owned(), &c.text, busy, &opts)
                });
                let (response, sent) = inner.inner;
                self.rects.note(&response, || format!("edit_{name}"));
                if let Some(text) = sent {
                    out.commands.push(Command::SetPopupControl {
                        name: name.to_owned(),
                        value: PopupInput::Text(text),
                    });
                }
            }
            PopupControlKind::List => self.list(ui, view, c, out),
        }
    }

    /// A caption before a dropdown, slider or box; PoB puts these in labels of their own, so
    /// this is only the control's own `label` when it has one.
    fn caption(&self, ui: &mut egui::Ui, c: &PopupControl) {
        if !c.label.is_empty() {
            let default = ui.visuals().text_color();
            ui.label(pob_text(&c.label, 14.0, default));
        }
    }

    /// The static tooltip, or the engine's for a control that builds one.
    fn hint(&mut self, response: &egui::Response, c: &PopupControl, index: Option<usize>) {
        if let Some(text) = &c.tooltip_text
            && !c.hover_tooltip
        {
            response.clone().on_hover_text(text);
        }
        if c.hover_tooltip {
            self.tip.note(
                response,
                ControlTooltipQuery::Popup {
                    control: c.name.clone(),
                    index,
                },
            );
        }
    }

    fn list(
        &mut self,
        ui: &mut egui::Ui,
        view: &PopupView,
        c: &PopupControl,
        out: &mut PopupOutput,
    ) {
        let default = ui.visuals().text_color();
        let name = c.name.as_str();
        ui.add_enabled_ui(c.enabled, |ui| {
            if let Some(message) = self.sort.failed.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 120, 120),
                        format!("The stat sort failed: {message}"),
                    );
                    let retry = ui.button("Retry");
                    self.rects.note(&retry, || format!("{name}_retry"));
                    if retry.clicked() {
                        self.sort.failed = None;
                        out.commands.extend(self.retry_sort(view, name));
                    }
                });
            } else if let Some(percent) = self.sort.percent {
                ui.horizontal(|ui| {
                    let label = ui.label(format!("Sorting\u{2026} {percent}%"));
                    self.rects.note(&label, || format!("{name}_progress"));
                    let fraction = f32::from(percent.min(100) as u16) / 100.0;
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_height(6.0)
                            .desired_width(160.0),
                    );
                });
            }
            let height = c.size[1].clamp(80.0, 320.0);
            // one row's height, measured: rows are laid out only where the list is scrolled to
            // (the Mod Browser has thousands, R28), so every row must be this tall. A row is
            // one line of 14 px text (truncated, never wrapped) in a button's padding.
            let row_height = popup_row_height(ui);
            // a Shift+click range (the timeless results) shows selected end to end
            let at = |key: &Option<String>| {
                key.as_deref()
                    .and_then(|k| c.rows.iter().position(|r| r.key == k))
            };
            let range = match (at(&c.row_selected), at(&c.row_highlight)) {
                (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
                _ => None,
            };
            egui::ScrollArea::vertical()
                .id_salt(("popup_list", name))
                .max_height(height)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, c.rows.len(), |ui, shown| {
                    ui.set_min_width(c.size[0].clamp(120.0, MAX_CONTROL_WIDTH));
                    // a mutable list's rows are dragged to a new place, as in PoB; only the
                    // rows laid out have rects, from `first` on
                    let first = shown.start;
                    let mut rects = Vec::with_capacity(shown.len());
                    let mut dragged = None;
                    for i in shown {
                        let row = &c.rows[i];
                        let selected = c.row_selected.as_deref() == Some(row.key.as_str())
                            || range.is_some_and(|(a, b)| a <= i && i <= b);
                        let mut response = ui.add(
                            egui::Button::selectable(selected, pob_text(&row.label, 14.0, default))
                                .truncate(),
                        );
                        if c.reorder {
                            response = response.interact(egui::Sense::click_and_drag());
                            rects.push(response.rect);
                            if response.dragged() || response.drag_stopped() {
                                dragged = Some((i, response.drag_stopped()));
                            }
                        }
                        self.rects.note(&response, || format!("{name}_row_{i}"));
                        self.tip.note(
                            &response,
                            ControlTooltipQuery::Popup {
                                control: name.to_owned(),
                                index: Some(i),
                            },
                        );
                        // PoB's list takes the second click of a double click, and a click with
                        // Shift held (ListControl.lua:338-372)
                        let key = row.key.clone();
                        let value = if response.double_clicked() {
                            Some(PopupInput::RowDoubleClick(key))
                        } else if response.clicked() {
                            Some(if ui.input(|i| i.modifiers.shift) {
                                PopupInput::RowShiftClick(key)
                            } else {
                                PopupInput::Row(key)
                            })
                        } else {
                            None
                        };
                        if let Some(value) = value {
                            out.commands.push(Command::SetPopupControl {
                                name: name.to_owned(),
                                value,
                            });
                        }
                    }
                    if let Some((from, dropped)) = dragged
                        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
                    {
                        // the rows above the laid-out ones all stay above the drop
                        let local = from - first;
                        let to_local = drop_place(&rects, local, pos.y);
                        let to = first + to_local;
                        if dropped {
                            if to != from {
                                out.commands.push(Command::SetPopupControl {
                                    name: name.to_owned(),
                                    value: PopupInput::RowMove {
                                        key: c.rows[from].key.clone(),
                                        to,
                                    },
                                });
                            }
                        } else if let Some(line) = drop_line(&rects, local, to_local) {
                            let stroke = ui.visuals().selection.stroke;
                            ui.painter().hline(line.0, line.1, stroke);
                        }
                    }
                });
            if c.rows.is_empty()
                && let Some(note) = &c.note
            {
                ui.label(pob_text(note, 14.0, ui.visuals().weak_text_color()));
            }
        });
    }

    /// Sets the list's sort dropdown to what it shows, which starts the sort again.
    fn retry_sort(&self, view: &PopupView, list: &str) -> Option<Command> {
        let sort = view
            .controls
            .iter()
            .find(|c| c.kind == PopupControlKind::Dropdown && c.name == format!("{list}.sort"))?;
        let label = sort.choices.get(sort.selected)?.clone();
        Some(Command::SetPopupControl {
            name: sort.name.clone(),
            value: PopupInput::Choice {
                index: sort.selected,
                label,
            },
        })
    }
}

/// A title as plain text: PoB's colour codes are dropped.
fn pob_text_plain(title: &str) -> String {
    pob_text(title, 14.0, egui::Color32::WHITE).text
}

/// The window is as wide as its widest shown control, within sensible limits.
fn window_width(view: &PopupView) -> f32 {
    view.controls
        .iter()
        .filter(|c| c.shown)
        .map(|c| c.pos[0] + c.size[0])
        .fold(0.0_f32, f32::max)
        .clamp(320.0, MAX_CONTROL_WIDTH + 40.0)
}
