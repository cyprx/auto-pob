//! The Calcs tab: PoB's skill and mode selector above its grid of calculation sections. It
//! takes engine snapshots and returns commands; it knows nothing else about PoB.

pub mod breakdown;
pub mod grid;

use crate::engine::views::{CalcsSection, CalcsSelector, CalcsView, DropList, TooltipLine};
use crate::engine::{CalcsControl, CalcsCount, CellRef, Command, Snapshot};
use crate::ui::tree::Rects;
use crate::widgets::{pob_text, tooltip_ui};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::{collections::HashMap, sync::Arc};

/// PoB's `colorCodes.HIGHLIGHT`
const HIGHLIGHT: &str = "^xFF0000";
/// The text colour of `^7`
const TEXT: Color32 = Color32::from_rgb(255, 255, 255);
/// A row label's column, and where the cells start
const LABEL_WIDTH: f32 = 130.0;
const CELL_START: f32 = 134.0;
const ROW_HEIGHT: f32 = 18.0;

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct CalcsOutput {
    pub commands: Vec<Command>,
    /// Questions about the hovered cell; the app may send only the latest of them.
    pub queries: Vec<Command>,
    /// A passive node the user clicked in a breakdown; the app shows it on the tree.
    pub focus_node: Option<u32>,
}

pub struct CalcsUi {
    view: Option<Arc<CalcsView>>,
    /// The cell the pointer is over, if it has a breakdown
    pub hovered: Option<CellRef>,
    /// Text being typed, by field, while the field has focus or the engine is behind
    edits: HashMap<&'static str, String>,
    /// What each count field last sent
    sent: HashMap<&'static str, String>,
    focus_search: bool,
    bd: breakdown::State,
    /// A cell clicked this frame, with its row's label
    clicked: Option<(CellRef, String)>,
    /// The text fields' ids, which focus is looked up by
    stages_id: egui::Id,
    mines_id: egui::Id,
    search_id: egui::Id,
    pub(crate) rects: Rects,
}

impl Default for CalcsUi {
    fn default() -> Self {
        Self {
            view: None,
            hovered: None,
            edits: HashMap::new(),
            sent: HashMap::new(),
            focus_search: false,
            bd: breakdown::State::default(),
            clicked: None,
            stages_id: egui::Id::unique("calcs_stages"),
            mines_id: egui::Id::unique("calcs_mines"),
            search_id: egui::Id::unique("calcs_search"),
            rects: Rects::default(),
        }
    }
}

/// Whether two views hold the same sections and selector, ignoring the search's match flags.
fn same_numbers(a: &CalcsView, b: &CalcsView) -> bool {
    let plain = |v: &CalcsView| {
        let mut v = v.clone();
        for section in &mut v.sections {
            for sub in &mut section.subs {
                sub.search_match = false;
                for row in &mut sub.rows {
                    row.search_match = false;
                }
            }
        }
        (v.sections, v.selector)
    };
    plain(a) == plain(b)
}

impl CalcsUi {
    /// Forgets the build's state, as for another build.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The undo and redo buttons' enablement.
    pub fn history(&self) -> (bool, bool) {
        self.view
            .as_ref()
            .map_or((false, false), |v| (v.can_undo, v.can_redo))
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        match snapshot {
            Snapshot::Calcs(view) => {
                // typing in the search box changes only the match flags, which no breakdown shows
                let changed = self
                    .view
                    .as_ref()
                    .is_none_or(|old| !same_numbers(old, view));
                self.view = Some(view.clone());
                if changed {
                    self.bd.calcs_changed();
                }
            }
            Snapshot::Breakdown(view) => self.bd.on_view(view),
            _ => {}
        }
    }

    #[cfg(test)]
    pub fn breakdown_unavailable(&self) -> bool {
        self.bd.failed()
    }

    /// `busy`: the engine hasn't caught up with what was sent, so typed text stays.
    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool) -> CalcsOutput {
        let mut out = CalcsOutput::default();
        let Some(view) = self.view.clone() else {
            ui.spinner();
            return out;
        };
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.focus_search = true;
        }
        self.selector(ui, &view, busy, &mut out.commands);
        ui.separator();
        self.hovered = None;
        self.clicked = None;
        let now = ui.input(|i| i.time);
        if self.bd.visible() {
            egui::Panel::right("calcs_breakdown")
                .resizable(true)
                .default_size(380.0)
                .show(ui, |ui| {
                    let (focus, unpin) = self.bd.show(ui, &mut self.rects);
                    out.focus_node = focus;
                    if unpin {
                        self.bd.pinned = None;
                    }
                });
        }
        egui::ScrollArea::both()
            .auto_shrink(false)
            .show(ui, |ui| self.sections(ui, &view, &mut out.commands));
        if let Some((cell, label)) = self.clicked.take() {
            self.bd.click(cell, label);
        }
        self.bd
            .drive(ui.ctx(), now, self.hovered.as_ref(), &mut out.queries);
        out
    }

    // ---- the selector row ----------------------------------------------------------------

    fn selector(
        &mut self,
        ui: &mut egui::Ui,
        view: &CalcsView,
        busy: bool,
        out: &mut Vec<Command>,
    ) {
        let sel: &CalcsSelector = &view.selector;
        ui.horizontal_wrapped(|ui| {
            self.dropdown(
                ui,
                "group",
                "Socket group",
                CalcsControl::Group,
                Some(&sel.group),
                out,
            );
            self.dropdown(
                ui,
                "skill",
                "Skill",
                CalcsControl::Skill,
                sel.skill.as_ref(),
                out,
            );
            self.dropdown(
                ui,
                "part",
                "Part",
                CalcsControl::Part,
                sel.part.as_ref(),
                out,
            );
            if let Some(text) = &sel.stages {
                self.count(ui, "stages", "Stages", CalcsCount::Stages, text, busy, out);
            }
            if let Some(text) = &sel.mines {
                self.count(ui, "mines", "Mines", CalcsCount::Mines, text, busy, out);
            }
            if let Some(show) = sel.show_minion {
                let mut on = show;
                let response = ui.checkbox(&mut on, "Show minion");
                self.rects.note(&response, || "show_minion".into());
                if response.changed() {
                    out.push(Command::SetShowMinion { show: on });
                }
            }
            self.dropdown(
                ui,
                "minion",
                "Minion",
                CalcsControl::Minion,
                sel.minion.as_ref(),
                out,
            );
            self.dropdown(
                ui,
                "minion_skill",
                "Minion skill",
                CalcsControl::MinionSkill,
                sel.minion_skill.as_ref(),
                out,
            );
            let mode = self.dropdown(ui, "mode", "Mode", CalcsControl::Mode, Some(&sel.mode), out);
            if let Some(response) = mode
                && !sel.mode_tooltip.is_empty()
            {
                response.on_hover_ui(|ui| {
                    let lines: Vec<TooltipLine> = sel
                        .mode_tooltip
                        .lines()
                        .map(|text| TooltipLine::Line {
                            size: 14,
                            text: text.to_owned(),
                        })
                        .collect();
                    tooltip_ui(ui, &lines);
                });
            }
            self.search(ui, view, busy, out);
        });
    }

    /// A labelled dropdown, unless PoB hides it. Returns the combo box's response.
    fn dropdown(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        label: &str,
        control: CalcsControl,
        list: Option<&DropList>,
        out: &mut Vec<Command>,
    ) -> Option<egui::Response> {
        let list = list?;
        ui.label(format!("{label}:"));
        let default = ui.visuals().text_color();
        let current = list.labels.get(list.selected).map_or("", String::as_str);
        let mut response = None;
        ui.add_enabled_ui(list.enabled, |ui| {
            let combo = egui::ComboBox::from_id_salt(("calcs_select", key))
                .selected_text(pob_text(current, 14.0, default))
                .show_ui(ui, |ui| {
                    for (i, text) in list.labels.iter().enumerate() {
                        let item =
                            ui.selectable_label(i == list.selected, pob_text(text, 14.0, default));
                        self.rects.note(&item, || format!("{key}_item_{i}"));
                        if item.clicked() && i != list.selected {
                            out.push(Command::SetCalcsSelect { control, index: i });
                        }
                    }
                });
            self.rects.note(&combo.response, || key.to_owned());
            response = Some(combo.response);
        });
        response
    }

    /// The stage or mine count: sent when the field loses focus, which Enter also does.
    #[allow(clippy::too_many_arguments)]
    fn count(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        label: &str,
        control: CalcsCount,
        text: &str,
        busy: bool,
        out: &mut Vec<Command>,
    ) {
        ui.label(format!("{label}:"));
        let buf = self.edits.entry(key).or_default();
        let id = if key == "stages" {
            self.stages_id
        } else {
            self.mines_id
        };
        let focused = ui.memory(|m| m.has_focus(id));
        if !focused && !busy {
            buf.clear();
            buf.push_str(text);
            self.sent.insert(key, text.to_owned());
        }
        let response = ui.add(egui::TextEdit::singleline(buf).id(id).desired_width(50.0));
        self.rects.note(&response, || key.to_owned());
        if response.lost_focus() && self.sent.get(key).is_none_or(|s| s != buf) {
            self.sent.insert(key, buf.clone());
            out.push(Command::SetCalcsCount {
                control,
                text: buf.clone(),
            });
        }
    }

    /// The search box: sent on change; Ctrl+F focuses it.
    fn search(&mut self, ui: &mut egui::Ui, view: &CalcsView, busy: bool, out: &mut Vec<Command>) {
        let id = self.search_id;
        let buf = self.edits.entry("search").or_default();
        if !ui.memory(|m| m.has_focus(id)) && !busy {
            buf.clone_from(&view.search);
        }
        let response = ui.add(
            egui::TextEdit::singleline(buf)
                .id(id)
                .hint_text("Search")
                .desired_width(160.0),
        );
        self.rects.note(&response, || "search".into());
        if std::mem::take(&mut self.focus_search) {
            response.request_focus();
        }
        if response.changed() {
            out.push(Command::SetCalcsSearch { text: buf.clone() });
        }
    }

    // ---- the grid ------------------------------------------------------------------------

    fn sections(&mut self, ui: &mut egui::Ui, view: &CalcsView, out: &mut Vec<Command>) {
        // PoB lays out and shows only sections that are enabled
        let shown: Vec<&CalcsSection> = view.sections.iter().filter(|s| s.enabled).collect();
        let items: Vec<(u32, u32, f32)> = shown
            .iter()
            .map(|s| (s.group, s.width_cols, grid::section_height(s)))
            .collect();
        let columns = grid::max_cols(ui.available_width());
        let placed = grid::arrange_in(&items, columns, ui.available_height() - grid::TOP);
        let mut size = Vec2::ZERO;
        for (section, spot) in shown.iter().zip(&placed) {
            if let Some((col, y)) = spot {
                let right = *col as f32 * (grid::COL_WIDTH + grid::GAP)
                    + grid::section_width(section.width_cols);
                size.x = size.x.max(right);
                size.y = size.y.max(y + grid::section_height(section));
            }
        }
        let (area, _) = ui.allocate_exact_size(size + Vec2::new(8.0, 8.0), Sense::hover());
        for (section, spot) in shown.iter().zip(&placed) {
            let Some((col, y)) = *spot else { continue };
            let min = area.min + Vec2::new(4.0 + col as f32 * (grid::COL_WIDTH + grid::GAP), y);
            let rect = Rect::from_min_size(
                min,
                Vec2::new(
                    grid::section_width(section.width_cols),
                    grid::section_height(section),
                ),
            );
            if ui.is_rect_visible(rect) {
                self.section(ui, section, rect, out);
            }
        }
    }

    fn section(
        &mut self,
        ui: &mut egui::Ui,
        section: &CalcsSection,
        rect: Rect,
        out: &mut Vec<Command>,
    ) {
        let colour = parse_colour(&section.colour).unwrap_or(Color32::GRAY);
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect_filled(rect, 0.0, colour);
        painter.rect_filled(rect.shrink(2.0), 0.0, Color32::from_gray(26));
        let (offsets, _) = grid::sub_offsets(section);
        let first_collapsed = section.subs.first().is_some_and(|s| s.collapsed);
        for (si, (sub, offset)) in section.subs.iter().zip(offsets).enumerate() {
            let top = rect.min.y + offset;
            let line = |y: f32| {
                Rect::from_min_size(
                    Pos2::new(rect.min.x + 2.0, y),
                    Vec2::new(rect.width() - 4.0, 2.0),
                )
            };
            painter.rect_filled(line(top), 0.0, colour);
            painter.rect_filled(line(top + 20.0), 0.0, colour);

            let text_colour = if sub.search_match { HIGHLIGHT } else { "^7" };
            let label = pob_text(&format!("{text_colour}{}:", sub.label), 16.0, TEXT);
            let galley = painter.layout_job(label);
            let label_width = galley.size().x;
            painter.galley(Pos2::new(rect.min.x + 3.0, top + 3.0), galley, TEXT);
            if let Some(extra) = &sub.extra {
                let galley = painter.layout_job(pob_text(&format!("^7{extra}"), 16.0, TEXT));
                painter.galley(
                    Pos2::new(rect.min.x + 3.0 + label_width + 10.0, top + 3.0),
                    galley,
                    TEXT,
                );
            }

            // the first sub-section's toggle always shows; the others only while it is open
            if si == 0 || !first_collapsed {
                let toggle = Rect::from_min_size(
                    Pos2::new(rect.max.x - 3.0 - 16.0, top + 3.0),
                    Vec2::splat(16.0),
                );
                let response = ui.interact(
                    toggle,
                    ui.scope_id().with(("calcs_toggle", &section.id, si)),
                    Sense::click(),
                );
                self.rects
                    .note(&response, || format!("toggle_{}_{si}", section.id));
                let fill = if response.hovered() {
                    Color32::from_gray(70)
                } else {
                    Color32::from_gray(45)
                };
                painter.rect_filled(toggle, 2.0, fill);
                painter.rect_stroke(
                    toggle,
                    2.0,
                    Stroke::new(1.0, Color32::GRAY),
                    StrokeKind::Inside,
                );
                let galley =
                    painter.layout_job(pob_text(if sub.collapsed { "+" } else { "-" }, 14.0, TEXT));
                let at = toggle.center() - galley.size() / 2.0;
                painter.galley(at, galley, TEXT);
                if response.clicked() {
                    out.push(Command::SetCalcsCollapsed {
                        section: section.id.clone(),
                        sub: si,
                        collapsed: !sub.collapsed,
                    });
                }
            }

            if sub.collapsed {
                continue;
            }
            for (ri, row) in sub.rows.iter().enumerate() {
                let y = top + 22.0 + ROW_HEIGHT * ri as f32;
                if let Some(label) = &row.label {
                    let text_colour = if row.search_match {
                        HIGHLIGHT
                    } else {
                        row.colour.as_deref().unwrap_or("^7")
                    };
                    let galley = painter.layout_job(pob_text(
                        &format!("{text_colour}{label}^7:"),
                        16.0,
                        TEXT,
                    ));
                    let x = rect.min.x + LABEL_WIDTH + 2.0 - galley.size().x;
                    painter.galley(Pos2::new(x, y), galley, TEXT);
                }
                let count = row.cells.len().max(1) as f32;
                let width = (rect.width() - CELL_START - 2.0) / count;
                for (ci, cell) in row.cells.iter().enumerate() {
                    let cell_rect = Rect::from_min_size(
                        Pos2::new(rect.min.x + CELL_START + width * ci as f32, y),
                        Vec2::new(width, ROW_HEIGHT),
                    );
                    painter.rect_filled(
                        Rect::from_min_size(
                            cell_rect.min + Vec2::new(0.0, 2.0),
                            Vec2::new(2.0, ROW_HEIGHT - 2.0),
                        ),
                        0.0,
                        colour,
                    );
                    let id =
                        ui.scope_id()
                            .with(("calcs_cell", &section.id, si, row.index, cell.index));
                    let sense = if cell.breakdown {
                        Sense::click()
                    } else {
                        Sense::hover()
                    };
                    let response = ui.interact(cell_rect, id, sense);
                    self.rects.note(&response, || {
                        format!("cell_{}_{si}_{}_{}", section.id, row.index, cell.index)
                    });
                    if cell.breakdown && response.hovered() {
                        painter.rect_stroke(
                            cell_rect.shrink2(Vec2::new(2.0, 0.0)),
                            0.0,
                            Stroke::new(1.0, Color32::from_rgb(64, 255, 64)),
                            StrokeKind::Inside,
                        );
                        let at = CellRef {
                            section: section.id.clone(),
                            sub: si,
                            row: row.index,
                            cell: cell.index,
                        };
                        if response.clicked() {
                            self.clicked =
                                Some((at.clone(), row.label.clone().unwrap_or_default()));
                        }
                        self.hovered = Some(at);
                    }
                    let clipped = painter.with_clip_rect(cell_rect.intersect(painter.clip_rect()));
                    let galley =
                        clipped.layout_job(pob_text(&cell.text, row.text_size as f32, TEXT));
                    let at = Pos2::new(
                        cell_rect.min.x + 3.0,
                        cell_rect.center().y - galley.size().y / 2.0,
                    );
                    clipped.galley(at, galley, TEXT);
                }
            }
        }
    }
}

/// A PoB `^xRRGGBB` colour code.
fn parse_colour(code: &str) -> Option<Color32> {
    let hex = code
        .strip_prefix("^x")
        .or_else(|| code.strip_prefix("^X"))?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::views::{BreakdownView, CalcsCell, CalcsRow, CalcsSub};
    use eframe::egui::{Event, PointerButton, RawInput};

    fn list(labels: &[&str]) -> DropList {
        DropList {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            selected: 0,
            enabled: true,
        }
    }

    fn sample(part: Option<DropList>) -> CalcsView {
        CalcsView {
            sections: vec![CalcsSection {
                id: "sec".into(),
                group: 1,
                width_cols: 1,
                colour: "^xFF8800".into(),
                enabled: true,
                subs: vec![CalcsSub {
                    label: "Sub".into(),
                    extra: None,
                    collapsed: false,
                    search_match: false,
                    rows: vec![CalcsRow {
                        index: 2,
                        label: Some("Label".into()),
                        colour: None,
                        text_size: 14,
                        search_match: false,
                        cells: vec![CalcsCell {
                            index: 0,
                            text: "^x33FF77123".into(),
                            breakdown: true,
                        }],
                    }],
                }],
            }],
            selector: CalcsSelector {
                group: list(&["Group"]),
                mode: list(&["Effective DPS"]),
                part,
                ..CalcsSelector::default()
            },
            ..CalcsView::default()
        }
    }

    struct Harness {
        ui: CalcsUi,
        ctx: egui::Context,
        time: f64,
    }

    impl Harness {
        fn new(view: CalcsView) -> Self {
            let mut ui = CalcsUi::default();
            ui.on_snapshot(&Snapshot::Calcs(Arc::new(view)));
            let mut h = Self {
                ui,
                ctx: egui::Context::default(),
                time: 1.0,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<Command> {
            self.time += 0.1;
            self.frame_out(self.time, events).commands
        }

        /// One frame at the given time, with everything the tab asked for.
        fn frame_out(&mut self, time: f64, events: Vec<Event>) -> CalcsOutput {
            self.time = time;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = CalcsOutput::default();
            self.ctx
                .run_ui(input, |ui| {
                    out = self.ui.show(ui, false);
                })
                .drop_without_applying_deltas();
            out
        }

        /// A click on the widget under `key`, in frames from now; returns all their output.
        fn click_out(&mut self, key: &str) -> Vec<CalcsOutput> {
            let pos = self.ui.rects.map[key].center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut all = Vec::new();
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                all.push(self.frame_out(self.time + 0.001, events));
            }
            all
        }

        /// Presses and releases the middle of the widget drawn under `key`.
        fn click(&mut self, key: &str) -> Vec<Command> {
            let pos = self.ui.rects.map[key].center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
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
    }

    fn debug(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|c| format!("{c:?}")).collect()
    }

    #[test]
    fn selector_sends_set_calcs_select() {
        let mut h = Harness::new(sample(Some(list(&["One", "Two"]))));
        let mut sent = h.click("part");
        for _ in 0..3 {
            sent.extend(h.frame(vec![]));
        }
        assert!(
            h.ui.rects.map.contains_key("part_item_1"),
            "the list opened"
        );
        sent.extend(h.click("part_item_1"));
        assert_eq!(debug(&sent), ["SetCalcsSelect { control: Part, index: 1 }"]);
    }

    #[test]
    fn collapse_toggle_sends_command() {
        let mut h = Harness::new(sample(None));
        let sent = h.click("toggle_sec_0");
        assert_eq!(
            debug(&sent),
            ["SetCalcsCollapsed { section: \"sec\", sub: 0, collapsed: true }"]
        );
    }

    #[test]
    fn hidden_lists_are_not_drawn() {
        let h = Harness::new(sample(None));
        assert!(!h.ui.rects.map.contains_key("part"));
        assert!(h.ui.rects.map.contains_key("group"));
        assert!(h.ui.rects.map.contains_key("mode"));
        assert!(!h.ui.rects.map.contains_key("stages"));
    }

    #[test]
    fn hovering_a_cell_with_a_breakdown_is_recorded() {
        let mut h = Harness::new(sample(None));
        let toggle = h.ui.rects.map["toggle_sec_0"];
        // the cell is on the row below the sub-section header, at the section's right
        let pos = Pos2::new(
            toggle.min.x - 40.0,
            toggle.min.y + 3.0 + 22.0 + 9.0 - 3.0 + 3.0,
        );
        h.frame(vec![Event::PointerMoved(pos)]);
        h.frame(vec![]);
        assert_eq!(
            h.ui.hovered,
            Some(CellRef {
                section: "sec".into(),
                sub: 0,
                row: 2,
                cell: 0
            })
        );
    }

    #[test]
    fn count_is_sent_on_enter_and_the_search_on_change() {
        let mut view = sample(None);
        view.selector.stages = Some("1".into());
        let mut h = Harness::new(view);
        h.click("stages");
        h.frame(vec![Event::Text("2".into())]);
        let sent = h.frame(vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(
            debug(&sent),
            ["SetCalcsCount { control: Stages, text: \"12\" }"]
        );
        h.click("search");
        let sent = h.frame(vec![Event::Text("x".into())]);
        assert_eq!(debug(&sent), ["SetCalcsSearch { text: \"x\" }"]);
    }

    const CELL: &str = "cell_sec_0_2_0";

    fn target() -> CellRef {
        CellRef {
            section: "sec".into(),
            sub: 0,
            row: 2,
            cell: 0,
        }
    }

    fn table(rows: &[(&str, Option<u32>, bool)]) -> BreakdownView {
        use crate::engine::views::{BreakdownColumn, BreakdownRow, BreakdownSection};
        BreakdownView {
            cell: target(),
            error: None,
            sections: vec![BreakdownSection::Table {
                label: None,
                footer: None,
                columns: vec![BreakdownColumn {
                    label: "Name".into(),
                    key: "name".into(),
                    right: false,
                }],
                rows: rows
                    .iter()
                    .map(|(text, node, tip)| BreakdownRow {
                        cells: [("name".to_owned(), text.to_string())].into(),
                        tooltips: if *tip {
                            [(
                                "name".to_owned(),
                                vec![TooltipLine::Line {
                                    size: 14,
                                    text: "tip".into(),
                                }],
                            )]
                            .into()
                        } else {
                            Default::default()
                        },
                        nodes: node.map(|n| ("name".to_owned(), n)).into_iter().collect(),
                    })
                    .collect(),
            }],
        }
    }

    fn failure() -> BreakdownView {
        BreakdownView {
            cell: target(),
            sections: vec![],
            error: Some("boom".into()),
        }
    }

    fn answer(h: &mut Harness, view: BreakdownView) {
        h.ui.on_snapshot(&Snapshot::Breakdown(Arc::new(view)));
        h.frame(vec![]);
    }

    fn breakdowns(out: &[CalcsOutput]) -> usize {
        out.iter().map(|o| o.queries.len()).sum()
    }

    #[test]
    fn hover_waits_100ms_before_querying() {
        let mut h = Harness::new(sample(None));
        let t0 = h.time;
        let pos = h.ui.rects.map[CELL].center();
        let at = |h: &mut Harness, dt: f64, events| h.frame_out(t0 + dt, events);
        let a = at(&mut h, 0.0, vec![Event::PointerMoved(pos)]);
        let b = at(&mut h, 0.05, vec![]);
        assert_eq!(breakdowns(&[a, b]), 0, "50 ms is too soon");
        let c = at(&mut h, 0.12, vec![]);
        assert_eq!(c.queries.len(), 1);
        assert_eq!(
            debug(&c.queries),
            [format!("{:?}", Command::Breakdown(target()))]
        );
        let d = at(&mut h, 0.2, vec![]);
        assert!(d.queries.is_empty(), "asked once");
    }

    #[test]
    fn click_pins_and_second_click_unpins() {
        let mut view = sample(None);
        let cells = &mut view.sections[0].subs[0].rows[0].cells;
        cells.push(CalcsCell {
            index: 1,
            text: "7".into(),
            breakdown: true,
        });
        let mut h = Harness::new(view);
        let out = h.click_out(CELL);
        assert_eq!(breakdowns(&out), 1, "pinning asks at once");
        assert_eq!(h.ui.bd.pinned.as_ref().map(|p| p.0.clone()), Some(target()));
        assert_eq!(h.ui.bd.pinned.as_ref().unwrap().1, "Label");
        answer(&mut h, table(&[("a", None, false)]));
        // hovering another breakdown cell while pinned asks nothing
        let other = h.ui.rects.map["cell_sec_0_2_1"].center();
        let t = h.time;
        h.frame_out(t + 0.01, vec![Event::PointerMoved(other)]);
        let out = h.frame_out(t + 0.5, vec![]);
        assert!(out.queries.is_empty());
        assert_eq!(h.ui.hovered.as_ref().map(|c| c.cell), Some(1));
        h.click_out(CELL);
        assert_eq!(h.ui.bd.pinned, None);
    }

    #[test]
    fn pinned_breakdown_requeried_on_new_calcs_view() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        answer(&mut h, table(&[("a", None, false)]));
        let mut changed = sample(None);
        changed.selector.mode.selected += 1;
        h.ui.on_snapshot(&Snapshot::Calcs(Arc::new(changed)));
        let out = h.frame_out(h.time + 0.01, vec![]);
        assert_eq!(
            debug(&out.queries),
            [format!("{:?}", Command::Breakdown(target()))]
        );
        let out = h.frame_out(h.time + 0.01, vec![]);
        assert!(out.queries.is_empty());
    }

    #[test]
    fn unpinned_breakdown_refreshes_on_new_calcs_view() {
        let mut h = Harness::new(sample(None));
        let t = h.time;
        let pos = h.ui.rects.map[CELL].center();
        h.frame_out(t, vec![Event::PointerMoved(pos)]);
        let asked = h.frame_out(t + 0.2, vec![]);
        assert_eq!(asked.queries.len(), 1);
        answer(&mut h, table(&[("old", None, false)]));
        assert!(h.ui.bd.shown.is_some());
        let mut changed = sample(None);
        changed.selector.mode.selected += 1;
        h.ui.on_snapshot(&Snapshot::Calcs(Arc::new(changed)));
        let out = h.frame_out(h.time + 0.01, vec![]);
        assert_eq!(
            debug(&out.queries),
            [format!("{:?}", Command::Breakdown(target()))],
            "the cell on show is asked about again"
        );
        answer(&mut h, table(&[("new", None, false)]));
        let out = h.frame_out(h.time + 0.01, vec![]);
        assert!(out.queries.is_empty());
        assert!(h.ui.bd.shown.is_some());
    }

    #[test]
    fn search_does_not_requery_pinned_breakdown() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        answer(&mut h, table(&[("a", None, false)]));
        let mut searched = sample(None);
        searched.search = "li".into();
        searched.sections[0].subs[0].search_match = !searched.sections[0].subs[0].search_match;
        searched.sections[0].subs[0].rows[0].search_match =
            !searched.sections[0].subs[0].rows[0].search_match;
        h.ui.on_snapshot(&Snapshot::Calcs(Arc::new(searched)));
        let out = h.frame_out(h.time + 0.01, vec![]);
        assert!(out.queries.is_empty(), "only match flags changed");
        let mut changed = sample(None);
        changed.selector.mode.selected += 1;
        h.ui.on_snapshot(&Snapshot::Calcs(Arc::new(changed)));
        let out = h.frame_out(h.time + 0.02, vec![]);
        assert_eq!(out.queries.len(), 1);
    }

    #[test]
    fn pinned_breakdown_unpins_when_empty() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        answer(&mut h, table(&[("a", None, false)]));
        answer(
            &mut h,
            BreakdownView {
                cell: target(),
                sections: vec![],
                error: None,
            },
        );
        assert_eq!(h.ui.bd.pinned, None);
        assert!(h.ui.bd.shown.is_none());
    }

    #[test]
    fn breakdowns_for_other_cells_are_ignored() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        let mut other = table(&[("a", None, false)]);
        other.cell.row = 9;
        answer(&mut h, other.clone());
        assert!(h.ui.bd.shown.is_none());
        // unpinned: only the cell last asked about counts
        let mut h = Harness::new(sample(None));
        answer(&mut h, table(&[("a", None, false)]));
        assert!(h.ui.bd.shown.is_none(), "nothing was asked");
        let t = h.time;
        let pos = h.ui.rects.map[CELL].center();
        h.frame_out(t, vec![Event::PointerMoved(pos)]);
        h.frame_out(t + 0.2, vec![]);
        answer(&mut h, other);
        assert!(h.ui.bd.shown.is_none());
        answer(&mut h, table(&[("a", None, false)]));
        assert!(h.ui.bd.shown.is_some());
    }

    #[test]
    fn table_sort_cycles_and_sorts_numbers_numerically() {
        use super::breakdown::{Dir, Sort, next_sort, row_order};
        let view = table(&[
            ("1,200", None, false),
            ("300", None, false),
            ("^x00FF0010", None, false),
        ]);
        let crate::engine::views::BreakdownSection::Table { rows, .. } = &view.sections[0] else {
            unreachable!()
        };
        let asc = next_sort(None, "name");
        assert_eq!(
            asc,
            Some(Sort {
                key: "name".into(),
                dir: Dir::Asc
            })
        );
        assert_eq!(row_order(rows, asc.as_ref()), [2, 1, 0]);
        let desc = next_sort(asc.as_ref(), "name");
        assert_eq!(desc.as_ref().map(|s| s.dir), Some(Dir::Desc));
        assert_eq!(row_order(rows, desc.as_ref()), [0, 1, 2]);
        assert_eq!(next_sort(desc.as_ref(), "name"), None);
        assert_eq!(row_order(rows, None), [0, 1, 2]);
    }

    #[test]
    fn mixed_column_sort_does_not_panic() {
        use super::breakdown::{Dir, Sort, row_order};
        let texts = ["10", "9", "1a", "nan", "", "inf", "-3", "b", "1,000"];
        let view = table(&texts.map(|t| (t, None, false)));
        let crate::engine::views::BreakdownSection::Table { rows, .. } = &view.sections[0] else {
            unreachable!()
        };
        for dir in [Dir::Asc, Dir::Desc] {
            let sort = Sort {
                key: "name".into(),
                dir,
            };
            let order = row_order(rows, Some(&sort));
            let mut seen = order.clone();
            seen.sort();
            assert_eq!(seen, (0..texts.len()).collect::<Vec<_>>());
            let shown: Vec<&str> = order.iter().map(|&i| texts[i]).collect();
            if dir == Dir::Asc {
                assert_eq!(&shown[..4], ["-3", "9", "10", "1,000"], "numbers first");
            }
        }
    }

    #[test]
    fn header_click_sorts_the_drawn_table() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        answer(
            &mut h,
            table(&[("1,200", None, false), ("300", None, false)]),
        );
        h.frame(vec![]);
        h.click_out("bd_head_0_name");
        h.frame(vec![]);
        let first = h.ui.rects.map["bd_cell_0_1_name"].min.y;
        let second = h.ui.rects.map["bd_cell_0_0_name"].min.y;
        assert!(first < second, "300 is drawn above 1,200");
    }

    #[test]
    fn node_cell_click_focuses_tree() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        answer(
            &mut h,
            table(&[("a", Some(42), false), ("b", Some(7), true)]),
        );
        h.frame(vec![]);
        let out = h.click_out("bd_cell_0_0_name");
        assert_eq!(
            out.iter().filter_map(|o| o.focus_node).collect::<Vec<_>>(),
            [42]
        );
        let out = h.click_out("bd_cell_0_1_name");
        assert!(
            out.iter().all(|o| o.focus_node.is_none()),
            "a cell with a tooltip is not a node link"
        );
    }

    #[test]
    fn failed_query_says_so() {
        let mut h = Harness::new(sample(None));
        h.click_out(CELL);
        answer(&mut h, failure());
        assert!(h.ui.bd.visible());
        assert!(h.ui.breakdown_unavailable());
        h.frame(vec![]);
        // a pinned cell is asked again by the next recalculation, and hovering retries
        let mut h = Harness::new(sample(None));
        let t = h.time;
        let pos = h.ui.rects.map[CELL].center();
        h.frame_out(t, vec![Event::PointerMoved(pos)]);
        let asked = h.frame_out(t + 0.2, vec![]);
        assert_eq!(asked.queries.len(), 1);
        answer(&mut h, failure());
        let again = h.frame_out(t + 0.4, vec![]);
        assert!(
            again.queries.is_empty(),
            "no retry loop while the pointer stays"
        );
        h.frame_out(t + 0.5, vec![Event::PointerGone]);
        h.frame_out(t + 0.55, vec![]);
        h.frame_out(t + 0.6, vec![Event::PointerMoved(pos)]);
        h.frame_out(t + 0.65, vec![]);
        let retry = h.frame_out(t + 0.8, vec![]);
        assert_eq!(retry.queries.len(), 1, "hovering again retries");
    }
}
