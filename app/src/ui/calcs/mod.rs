//! The Calcs tab: PoB's skill and mode selector above its grid of calculation sections. It
//! takes engine snapshots and returns commands; it knows nothing else about PoB.

pub mod breakdown;
pub mod grid;

use crate::engine::views::{CalcsSection, CalcsSelector, CalcsView, DropList, TooltipLine};
use crate::engine::{CalcsControl, CalcsCount, CellRef, Command, PopupKind, Snapshot};
use crate::ui::tree::Rects;
use crate::widgets::{EditBoxOptions, EditBoxes, pob_text, tooltip_ui};
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
    /// The stage and mine counts, which send on Enter or blur
    counts: EditBoxes<&'static str>,
    focus_search: bool,
    bd: breakdown::State,
    /// A cell clicked this frame, with its row's label
    clicked: Option<(CellRef, String)>,
    /// The text fields' ids, which focus is looked up by
    search_id: egui::Id,
    pub(crate) rects: Rects,
    /// The sections popped out with "^" into their own windows, by id; UI state PoB doesn't
    /// save either (R24)
    pub(crate) popped: Vec<String>,
    /// The socket group dropdown's tooltip (CalcsTab.lua:46-51)
    pub group_hover: crate::ui::sidebar::GroupHover,
}

impl Default for CalcsUi {
    fn default() -> Self {
        Self {
            view: None,
            hovered: None,
            edits: HashMap::new(),
            counts: EditBoxes::default(),
            focus_search: false,
            bd: breakdown::State::default(),
            clicked: None,
            search_id: egui::Id::unique("calcs_search"),
            rects: Rects::default(),
            popped: Vec::new(),
            group_hover: Default::default(),
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

/// PoB's count boxes are `%D` boxes with no limit (CalcsTab.lua:70, 79)
const COUNT_OPTIONS: EditBoxOptions = EditBoxOptions::number(50.0, None);

fn count_command(key: &str, text: String) -> Command {
    Command::SetCalcsCount {
        control: if key == "stages" {
            CalcsCount::Stages
        } else {
            CalcsCount::Mines
        },
        text,
    }
}

impl CalcsUi {
    #[cfg(test)]
    pub(crate) fn count_salt(&self) -> egui::Id {
        self.counts.salt()
    }

    /// A count typed and not sent yet, which goes out before whatever command comes next.
    pub fn flush(&mut self) -> Option<Command> {
        let (key, text) = self.counts.take_pending()?;
        Some(count_command(key, text))
    }

    /// A count is being typed: PoB's box sets the build changed on each keystroke
    pub fn has_pending(&self) -> bool {
        self.counts.has_pending()
    }

    /// Drops the count being typed (the user chose not to save).
    pub fn discard(&mut self) {
        self.counts.discard();
    }

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
        self.popout_windows(ui.ctx(), &view, &mut out.commands);
        if let Some((cell, label)) = self.clicked.take() {
            self.bd.click(cell, label);
        }
        self.bd
            .drive(ui.ctx(), now, self.hovered.as_ref(), &mut out.queries);
        // a count whose field PoB hid belongs to a skill that is gone: not sent
        self.counts.discard_undrawn();
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
                self.count(ui, "stages", "Stages", text, busy, out);
            }
            if let Some(text) = &sel.mines {
                self.count(ui, "mines", "Mines", text, busy, out);
            }
            if let Some(show) = sel.show_minion {
                let mut on = show;
                let response = ui
                    .checkbox(&mut on, "Show minion")
                    .on_hover_text("Show stats for the minion instead of the player.");
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
            // PoB's "Spectre Library" row, for spectre skills (R18)
            if sel.spectre_library {
                let button = ui.button("Manage Spectres…");
                self.rects.note(&button, || "spectre_library".into());
                if button.clicked() {
                    out.push(Command::OpenPopup(PopupKind::SpectreLibrary));
                }
            }
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
        let mut items = Vec::new();
        ui.add_enabled_ui(list.enabled, |ui| {
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt(("calcs_select", key))
                .selected_text(pob_text(current, 14.0, default))
                .show_ui(ui, |ui| {
                    for (i, text) in list.labels.iter().enumerate() {
                        let item = crate::widgets::dropdown::item(
                            ui,
                            i == list.selected,
                            pob_text(text, 14.0, default),
                        );
                        self.rects.note(&item, || format!("{key}_item_{i}"));
                        if item.clicked() && i != list.selected {
                            out.push(Command::SetCalcsSelect { control, index: i });
                        }
                        items.push((i, item));
                    }
                });
            self.rects.note(&combo.response, || key.to_owned());
            if control == CalcsControl::Group {
                crate::ui::sidebar::track_group_hover(
                    &mut self.group_hover,
                    ui,
                    &combo.response,
                    &items,
                    list.selected,
                );
            }
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            if let Some(index) = crate::widgets::scroll_step::combo_step(
                ui,
                &combo.response,
                list.labels.len(),
                list.selected,
            ) {
                out.push(Command::SetCalcsSelect { control, index });
            }
            response = Some(combo.response);
        });
        response
    }

    /// The stage or mine count: sent on Enter or when the field loses focus; Esc reverts.
    #[allow(clippy::too_many_arguments)]
    fn count(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        label: &str,
        text: &str,
        busy: bool,
        out: &mut Vec<Command>,
    ) {
        ui.label(format!("{label}:"));
        let (response, sent) = self.counts.show(ui, key, text, busy, &COUNT_OPTIONS);
        self.rects.note(&response, || key.to_owned());
        out.extend(sent.map(|text| count_command(key, text)));
    }

    /// The search box: sent on change; Ctrl+F focuses it.
    fn search(&mut self, ui: &mut egui::Ui, view: &CalcsView, busy: bool, out: &mut Vec<Command>) {
        let id = self.search_id;
        let buf = self.edits.entry("search").or_default();
        if !ui.memory(|m| m.has_focus(id)) && !busy {
            buf.clone_from(&view.search);
        }
        let before = buf.clone();
        let pasted = crate::ui::name_box::pasted(ui);
        let sel = crate::ui::name_box::selection(ui.ctx(), id);
        let response = ui.add(
            egui::TextEdit::singleline(buf)
                .id(id)
                .hint_text("Search")
                .desired_width(160.0),
        );
        // PoB's box drops control characters and takes 100 bytes (CalcsTab.lua:35)
        if response.changed() {
            let raw = std::mem::take(buf);
            *buf = crate::ui::name_box::spliced_at(
                &before,
                &raw,
                sel,
                |c| !c.is_control(),
                100,
                pasted,
            );
            crate::ui::name_box::place_caret(ui.ctx(), id, &before, &raw, buf, sel);
        }
        crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &response, buf);
        self.rects.note(&response, || "search".into());
        if std::mem::take(&mut self.focus_search) {
            response.request_focus();
        }
        // its "x" empties it (CalcsTab.lua:35, clearable)
        let cleared = crate::widgets::edit_box::clear_button(ui, buf);
        if let Some(x) = &cleared {
            self.rects.note(x, || "search_clear".into());
        }
        if response.changed() || cleared.is_some_and(|x| x.clicked()) {
            out.push(Command::SetCalcsSearch { text: buf.clone() });
        }
    }

    // ---- the grid ------------------------------------------------------------------------

    /// The popped-out sections over any tab (PoB draws its overlay panes on top of every tab).
    /// Off the Calcs tab their cells show no breakdown: the breakdown panel lives on the tab.
    pub fn show_popouts(&mut self, ctx: &egui::Context) -> Vec<Command> {
        let mut out = Vec::new();
        if let Some(view) = self.view.clone() {
            self.popout_windows(ctx, &view, &mut out);
        }
        self.hovered = None;
        self.clicked = None;
        out
    }

    /// One floating window per popped-out section, toggled by "^" and closed by its x
    /// (CalcSectionControl.lua:56-62, 223-240, 280-294). The window drags as PoB's pane does.
    fn popout_windows(&mut self, ctx: &egui::Context, view: &CalcsView, out: &mut Vec<Command>) {
        let popped = self.popped.clone();
        for id in popped {
            // a section PoB disables isn't drawn (its `shown` needs `enabled`); it stays popped
            // and comes back when PoB enables it again
            let Some(section) = view.sections.iter().find(|s| s.id == id && s.enabled) else {
                continue;
            };
            let mut open = true;
            let title = section
                .subs
                .first()
                .map_or(id.as_str(), |s| s.label.as_str());
            let size = Vec2::new(
                grid::section_width(section.width_cols),
                grid::section_height(section),
            );
            egui::Window::new(title)
                .id(egui::Id::unique(("calcs_popout", &id)))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    self.section(ui, section, rect, false, out);
                });
            if !open {
                self.popped.retain(|p| *p != id);
            }
        }
    }

    fn sections(&mut self, ui: &mut egui::Ui, view: &CalcsView, out: &mut Vec<Command>) {
        // PoB lays out and shows only sections that are enabled and not popped out
        let shown: Vec<&CalcsSection> = view
            .sections
            .iter()
            .filter(|s| s.enabled && !self.popped.contains(&s.id))
            .collect();
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
                self.section(ui, section, rect, section.pop_out, out);
            }
        }
    }

    fn section(
        &mut self,
        ui: &mut egui::Ui,
        section: &CalcsSection,
        rect: Rect,
        pop_out: bool,
        out: &mut Vec<Command>,
    ) {
        let colour = parse_colour(&section.colour).unwrap_or(Color32::GRAY);
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect_filled(rect, 0.0, colour);
        painter.rect_filled(rect.shrink(2.0), 0.0, Color32::from_gray(26));
        // PoB's "^" button, left of the first toggle: pops the section out (R24)
        if pop_out {
            let button = Rect::from_min_size(
                Pos2::new(rect.max.x - 22.0 - 16.0, rect.min.y + 3.0),
                Vec2::splat(16.0),
            );
            let response = ui.interact(
                button,
                ui.scope_id().with(("calcs_popout_button", &section.id)),
                Sense::click(),
            );
            self.rects
                .note(&response, || format!("popout_button_{}", section.id));
            let fill = if response.hovered() {
                Color32::from_gray(70)
            } else {
                Color32::from_gray(45)
            };
            painter.rect_filled(button, 2.0, fill);
            painter.rect_stroke(
                button,
                2.0,
                Stroke::new(1.0, Color32::GRAY),
                StrokeKind::Inside,
            );
            let galley = painter.layout_job(pob_text("^", 14.0, TEXT));
            painter.galley(button.center() - galley.size() / 2.0, galley, TEXT);
            if response.clicked() && !self.popped.contains(&section.id) {
                self.popped.push(section.id.clone());
            }
        }
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
                // PoB's widths (R24): every cell of the sub-section is `colWidth` wide, or the
                // section less the label column, at its place in the row (UpdateSize)
                let width = section.col_widths.get(si).copied();
                for (ci, cell) in row.cells.iter().enumerate() {
                    let (x, width) = match width {
                        Some(w) => (CELL_START + w * cell.index as f32, w),
                        None => {
                            let w = (rect.width() - CELL_START - 2.0) / row.cells.len() as f32;
                            (CELL_START + w * ci as f32, w)
                        }
                    };
                    let cell_rect = Rect::from_min_size(
                        Pos2::new(rect.min.x + x, y),
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
                col_widths: vec![95.0],
                pop_out: true,
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

    /// The Calcs selectors are PoB's searchable dropdowns (SearchHost)
    #[test]
    fn a_selector_filters_as_typed() {
        let mut h = Harness::new(sample(Some(list(&["One", "Two"]))));
        h.click("part");
        for _ in 0..3 {
            h.frame(vec![]);
        }
        h.frame(vec![egui::Event::Text("tw".into())]);
        h.frame(vec![]);
        assert_eq!(
            h.ui.rects.map["part_item_0"].height(),
            0.0,
            "One wasn't filtered out"
        );
        let sent = h.click("part_item_1");
        assert_eq!(debug(&sent), ["SetCalcsSelect { control: Part, index: 1 }"]);
    }

    #[test]
    fn calcs_spectre_button_only_when_pob_shows_it() {
        let h = Harness::new(sample(None));
        assert!(!h.ui.rects.map.contains_key("spectre_library"));
        let mut view = sample(None);
        view.selector.spectre_library = true;
        let mut h = Harness::new(view);
        let sent = h.click("spectre_library");
        assert_eq!(debug(&sent), ["OpenPopup(SpectreLibrary)"]);
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
        // PoB's box drops control characters (CalcsTab.lua:35, "%c")
        let sent = h.frame(vec![Event::Text("\u{7}x".into())]);
        assert_eq!(debug(&sent), ["SetCalcsSearch { text: \"x\" }"]);
        // the box is clearable: its "x" empties it (CalcsTab.lua:35)
        h.frame(vec![]);
        let sent = h.click("search_clear");
        assert_eq!(debug(&sent), ["SetCalcsSearch { text: \"\" }"]);
    }

    #[test]
    fn a_count_typed_and_then_hidden_is_not_flushed() {
        let mut view = sample(None);
        view.selector.stages = Some("1".into());
        let mut h = Harness::new(view);
        h.click("stages");
        h.frame(vec![Event::Text("3".into())]);
        assert!(h.ui.counts.has_pending());
        // the main skill changed and PoB hides the control
        h.ui.on_snapshot(&Snapshot::Calcs(Arc::new(sample(None))));
        let sent = h.frame(vec![]);
        assert!(sent.is_empty(), "{sent:?}");
        assert!(h.ui.flush().is_none());
    }

    #[test]
    fn escape_reverts_a_count_and_sends_nothing() {
        let mut view = sample(None);
        view.selector.stages = Some("1".into());
        let mut h = Harness::new(view);
        h.click("stages");
        h.frame(vec![Event::Text("9".into())]);
        let mut sent = h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        sent.extend(h.frame(vec![]));
        sent.extend(h.click("search"));
        assert!(sent.is_empty(), "{sent:?}");
        assert!(h.ui.flush().is_none());
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

    // ---- Task 13: pop-out windows and PoB's column widths (R24) ----

    fn two_cells() -> CalcsView {
        let mut view = sample(None);
        let row = &mut view.sections[0].subs[0].rows[0];
        row.cells.push(CalcsCell {
            index: 2,
            text: "456".into(),
            breakdown: true,
        });
        view
    }

    #[test]
    fn cells_use_column_widths() {
        let mut h = Harness::new(two_cells());
        h.frame(vec![]);
        let first = h.ui.rects.map["cell_sec_0_2_0"];
        let third = h.ui.rects.map["cell_sec_0_2_2"];
        assert_eq!(first.width(), 95.0);
        assert_eq!(third.width(), 95.0);
        // the cell sits at its own place in PoB's row, two columns along
        assert_eq!(third.min.x - first.min.x, 190.0);
        // without PoB's widths the cells share the row as before
        let mut view = two_cells();
        view.sections[0].col_widths.clear();
        let mut h = Harness::new(view);
        h.frame(vec![]);
        let first = h.ui.rects.map["cell_sec_0_2_0"];
        assert!((first.width() - (grid::COL_WIDTH - CELL_START - 2.0) / 2.0).abs() < 0.01);
    }

    #[test]
    fn popout_window_toggles() {
        let mut h = Harness::new(sample(None));
        h.frame(vec![]);
        assert!(h.ui.rects.map.contains_key("popout_button_sec"));
        h.click("popout_button_sec");
        assert_eq!(h.ui.popped, ["sec"]);
        // the window draws the section without its "^"; the cells work there too
        h.ui.rects.map.clear();
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(!h.ui.rects.map.contains_key("popout_button_sec"));
        assert!(h.ui.rects.map.contains_key("cell_sec_0_2_0"));
        // a new view keeps it popped; another build forgets it
        h.ui.on_snapshot(&Snapshot::Calcs(Arc::new(sample(None))));
        h.frame(vec![]);
        assert_eq!(h.ui.popped, ["sec"]);
        // the window's close button puts the section back
        let area = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(("calcs_popout", &"sec".to_string()))))
            .expect("the window");
        let close = Pos2::new(area.max.x - 14.0, area.min.y + 12.0);
        let button = |pressed| Event::PointerButton {
            pos: close,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![Event::PointerMoved(close)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            h.frame(events);
        }
        assert!(h.ui.popped.is_empty(), "the x didn't close it");
        h.ui.rects.map.clear();
        h.frame(vec![]);
        assert!(h.ui.rects.map.contains_key("popout_button_sec"));
        // a section with controls offers no pop-out
        let mut view = sample(None);
        view.sections[0].pop_out = false;
        let mut h = Harness::new(view);
        h.frame(vec![]);
        assert!(!h.ui.rects.map.contains_key("popout_button_sec"));
    }
}
