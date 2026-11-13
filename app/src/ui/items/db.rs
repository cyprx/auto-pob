//! The item databases panel: PoB's uniques and rare templates with their filters, search and
//! stat sort (R13-R16). It keeps the query the user built, sends it (the search after a
//! pause), and draws the rows the engine answers with.

use super::ItemsOutput;
use crate::engine::views::{DropList, ItemDbView};
use crate::engine::{Command, ControlTooltipQuery, ItemDb, ItemDbQuery};
use crate::ui::popup::HoverTip;
use crate::ui::tree::Rects;
use crate::widgets::{EditBox, EditBoxOptions, pob_text};
use eframe::egui;
use std::sync::Arc;

/// The search is sent once typing has paused this long.
const DEBOUNCE: f64 = 0.3;
/// Float slack for comparing times that are sums of tenths.
const EPS: f64 = 1e-6;
const ROW_HEIGHT: f32 = 18.0;
/// PoB's search box holds 100 bytes (ItemDBControl.lua:51)
const SEARCH_LIMIT: usize = 100;
/// Filter dropdowns per line, so they fit a narrow column.
const PER_ROW: usize = 3;

/// The payload of a database row being dragged: onto a slot to add and equip it (R23)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DbDrag {
    pub db: ItemDb,
    pub name: String,
}

pub struct DbState {
    /// The panel is open; the engine has a database open while this is so and the tab shows
    pub expanded: bool,
    /// What the user chose (the search is the one last sent; the box holds what is typed)
    query: ItemDbQuery,
    /// The engine has a database open for us
    open_sent: bool,
    pub(super) search: EditBox,
    /// The box's text when it was last seen changing, and when
    seen: (String, f64),
    percent: Option<u32>,
    failed: Option<String>,
    pub(super) selected: Option<String>,
    tip: HoverTip,
    pub(crate) rects: Rects,
    /// The search box's id, and Ctrl+F's ask that it take the keyboard when next drawn
    search_id: Option<egui::Id>,
    focus_search: bool,
    /// The wiki page of the row under the pointer: a unique's name, a rare's base
    /// (ItemDBControl.lua:375-382, ItemTools.lua:395-399)
    pub(super) hovered_wiki: Option<String>,
    /// The rows are the selected control: the list's keys reach them (ListControl.lua:375-392)
    pub(super) list_focus: bool,
    /// The row whose copy text was asked for (Ctrl+C, ItemDBControl.lua:368-370)
    pub(super) copy_asked: Option<String>,
    /// The rows' scroll offset and view height last frame, and the row a key moved the
    /// selection to, to scroll into view (DropDownControl/ListControl ScrollSelIntoView)
    rows_view: (f32, f32),
    scroll_to: Option<usize>,
    /// A dialog of the tab's or the app's is open: the rows' keys are left alone, as PoB's
    /// popups are modal
    pub(super) keys_blocked: bool,
}

fn default_query(db: ItemDb) -> ItemDbQuery {
    ItemDbQuery {
        db,
        slot: 0,
        kind: 0,
        league: 0,
        requirement: 0,
        obtainable: 0,
        sort: 0,
        search: String::new(),
        search_mode: 0,
    }
}

impl Default for DbState {
    fn default() -> Self {
        Self {
            expanded: false,
            query: default_query(ItemDb::Uniques),
            open_sent: false,
            search: EditBox::default(),
            seen: (String::new(), 0.0),
            percent: None,
            failed: None,
            selected: None,
            tip: HoverTip::default(),
            rects: Rects::default(),
            search_id: None,
            focus_search: false,
            hovered_wiki: None,
            list_focus: false,
            copy_asked: None,
            rows_view: (0.0, 0.0),
            scroll_to: None,
            keys_blocked: false,
        }
    }
}

impl DbState {
    /// Ctrl+F (ItemsTab.lua:1471-1480): from the uniques' search, or anywhere while the rares
    /// show, the rares' search; else the uniques'. Ours is one box over both, so the panel
    /// opens on that database and its box takes the keyboard. Returns the query to resend
    /// when the database changed under an open panel.
    pub fn focus_search(&mut self, ctx: &egui::Context) -> Option<Command> {
        let in_box = self
            .search_id
            .is_some_and(|id| ctx.memory(|m| m.has_focus(id)));
        let db = match (self.query.db, in_box) {
            (ItemDb::Uniques, true) | (ItemDb::Rares, false) => ItemDb::Rares,
            _ => ItemDb::Uniques,
        };
        let changed = db != self.query.db;
        self.toggle_db(db);
        self.expanded = true;
        self.focus_search = true;
        (changed && self.open_sent).then(|| Command::SetItemDb(self.query.clone()))
    }

    /// The engine's answer (or its end). A view of another database than the toggles say is
    /// stale and ignored by the panel.
    pub fn on_view(&mut self, view: Option<&ItemDbView>) {
        // the rows may have changed under the pointer
        self.tip.reset();
        match view {
            Some(v) if v.db != self.query.db => {}
            Some(v) => {
                self.failed = None;
                if !v.sorting {
                    self.percent = None;
                }
            }
            None => {
                self.percent = None;
                // a failure and then a closed database: the engine refused the query, so
                // sending it again (Retry) would only be refused again. The filters go back
                // to their defaults, the typed search stays.
                if self.failed.is_some() {
                    self.query = ItemDbQuery {
                        search: self.query.search.clone(),
                        ..default_query(self.query.db)
                    };
                }
            }
        }
    }

    pub fn on_tooltip(&mut self, view: &Arc<crate::engine::views::ControlTooltipView>) {
        self.tip.on_reply(view);
    }

    pub fn on_progress(&mut self, percent: u32) {
        if self.open_sent {
            self.failed = None;
            self.percent = Some(percent.min(100));
        }
    }

    pub fn on_failed(&mut self, message: String) {
        if self.open_sent {
            self.percent = None;
            self.failed = Some(message);
        }
    }

    /// The typed search, once, as the query to send. For the moment before a command that
    /// would otherwise overtake the debounce (R4).
    pub fn flush(&mut self) -> Option<Command> {
        if !self.open_sent {
            return None;
        }
        let text = self.search.take_pending()?;
        self.query.search = text;
        Some(Command::SetItemDb(self.query.clone()))
    }

    /// Leaving the tab closes the engine's database; the panel stays as it is for the return.
    pub fn leave(&mut self) -> Option<Command> {
        self.list_focus = false;
        if !std::mem::take(&mut self.open_sent) {
            return None;
        }
        self.percent = None;
        self.failed = None;
        self.tip.reset();
        Some(Command::CloseItemDb)
    }

    /// The query the shown database tooltip answers.
    #[cfg(test)]
    pub(crate) fn tip_query(&self) -> Option<&ControlTooltipQuery> {
        self.tip.held()
    }

    /// The panel's query as the controls would send it now.
    #[cfg(test)]
    pub(crate) fn query(&self) -> &ItemDbQuery {
        &self.query
    }

    fn toggle_db(&mut self, db: ItemDb) {
        if self.query.db == db {
            return;
        }
        // the other database's lists don't mean the same indices, and the rares have only
        // slot, type, search and search mode (R13)
        self.query = ItemDbQuery {
            search: self.query.search.clone(),
            search_mode: self.query.search_mode,
            ..default_query(db)
        };
        self.percent = None;
        self.failed = None;
        self.selected = None;
        self.tip.reset();
    }
}

fn drop_list(
    ui: &mut egui::Ui,
    rects: &mut Rects,
    key: &str,
    list: &DropList,
    selected: usize,
) -> Option<usize> {
    let default = ui.visuals().text_color();
    let current = list.labels.get(selected).map_or("", String::as_str);
    let mut picked = None;
    let combo = ui.add_enabled_ui(list.enabled, |ui| {
        crate::widgets::dropdown::Dropdown::from_id_salt(("items_db", key))
            .width(100.0)
            .truncate()
            .selected_text(pob_text(current, 14.0, default))
            .show_ui(ui, |ui| {
                for (i, label) in list.labels.iter().enumerate() {
                    let item = crate::widgets::dropdown::item(
                        ui,
                        i == selected,
                        pob_text(label, 14.0, default),
                    );
                    rects.note(&item, || format!("db_{key}_{i}"));
                    if item.clicked() && i != selected {
                        picked = Some(i);
                    }
                }
            })
    });
    rects.note(&combo.inner.response, || format!("db_{key}"));
    // the wheel and the arrows step it (DropDownControl.lua:476-516)
    picked.or_else(|| {
        crate::widgets::scroll_step::combo_step(
            ui,
            &combo.inner.response,
            list.labels.len(),
            selected,
        )
    })
}

pub fn show(
    ui: &mut egui::Ui,
    view: Option<&ItemDbView>,
    state: &mut DbState,
    now: f64,
) -> ItemsOutput {
    let mut out = ItemsOutput::default();
    state.hovered_wiki = None;
    let header = egui::CollapsingHeader::new("Item databases")
        .id_salt("items_db_header")
        .open(Some(state.expanded))
        .show(ui, |ui| body(ui, view, state, now, &mut out));
    state
        .rects
        .note(&header.header_response, || "db_header".into());
    if header.header_response.clicked() {
        state.expanded = !state.expanded;
        if !state.expanded && std::mem::take(&mut state.open_sent) {
            state.percent = None;
            state.failed = None;
            out.commands.push(Command::CloseItemDb);
        }
    }
    if state.expanded && !state.open_sent {
        state.open_sent = true;
        out.commands.push(Command::SetItemDb(state.query.clone()));
    }
    out
}

fn body(
    ui: &mut egui::Ui,
    view: Option<&ItemDbView>,
    state: &mut DbState,
    now: f64,
    out: &mut ItemsOutput,
) {
    // a view of the other database is not what the toggles say
    let view = view.filter(|v| v.db == state.query.db);
    let mut changed = false;

    ui.horizontal(|ui| {
        for (db, label, key) in [
            (ItemDb::Uniques, "Uniques", "db_uniques"),
            // PoB's names (ItemsTab.lua:274)
            (ItemDb::Rares, "Rare Templates", "db_rares"),
        ] {
            let on = state.query.db == db;
            let toggle = ui.selectable_label(on, label);
            state.rects.note(&toggle, || key.into());
            if toggle.clicked() && !on {
                state.toggle_db(db);
                changed = true;
            }
        }
    });

    if let Some(view) = view {
        let q = &state.query;
        // the uniques' own filters and sort; PoB has none of them on the rares
        let controls: Vec<(&str, &DropList, usize)> = [
            Some(("slot", &view.slot, q.slot)),
            Some(("kind", &view.kind, q.kind)),
            view.league.as_ref().map(|l| ("league", l, q.league)),
            view.requirement
                .as_ref()
                .map(|l| ("requirement", l, q.requirement)),
            view.obtainable
                .as_ref()
                .map(|l| ("obtainable", l, q.obtainable)),
            Some(("search_mode", &view.search_mode, q.search_mode)),
            view.sort.as_ref().map(|l| ("sort", l, q.sort)),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut picked = None;
        for row in controls.chunks(PER_ROW) {
            ui.horizontal(|ui| {
                for &(key, list, selected) in row {
                    if let Some(i) = drop_list(ui, &mut state.rects, key, list, selected) {
                        picked = Some((key, i));
                    }
                }
            });
        }
        if let Some((key, i)) = picked {
            let q = &mut state.query;
            match key {
                "slot" => q.slot = i,
                "kind" => q.kind = i,
                "league" => q.league = i,
                "requirement" => q.requirement = i,
                "obtainable" => q.obtainable = i,
                "search_mode" => q.search_mode = i,
                _ => q.sort = i,
            }
            changed = true;
        }
    }

    // PoB's box: filter `%c`, 100 bytes, clearable (ItemDBControl.lua:51)
    let opts = EditBoxOptions {
        width: 240.0,
        hint: Some("Search"),
        byte_limit: Some(SEARCH_LIMIT),
        filter: Some(|c| !c.is_control()),
        // Esc leaves the box and keeps what was typed (EditControl.lua:505)
        keep_on_escape: true,
        ..EditBoxOptions::default()
    };
    ui.horizontal(|ui| {
        let id = ui.scope_id().with("items_db_search");
        let (response, send) = state
            .search
            .show(ui, id, &state.query.search.clone(), false, &opts);
        state.rects.note(&response, || "db_search".into());
        state.search_id = Some(id);
        if std::mem::take(&mut state.focus_search) {
            response.request_focus();
        }
        // its "x" empties it and searches at once (EditControl.lua:73-77, SetText(.., true))
        let cleared = crate::widgets::edit_box::clear_button(ui, id, &mut state.search.text)
            .is_some_and(|x| {
                state.rects.note(&x, || "db_search_clear".into());
                x.clicked()
            });
        if cleared {
            state.search.discard();
            state.query.search.clear();
            changed = true;
        } else if let Some(text) = send {
            state.query.search = text;
            changed = true;
        } else if state.search.has_pending() {
            // typing: send once it pauses
            if state.seen.0 != state.search.text {
                state.seen = (state.search.text.clone(), now);
            }
            if now - state.seen.1 >= DEBOUNCE - EPS {
                if let Some(text) = state.search.take_pending() {
                    state.query.search = text;
                    changed = true;
                }
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        let equip = ui.add_enabled(state.selected.is_some(), egui::Button::new("Add and equip"));
        state.rects.note(&equip, || "db_add".into());
        if equip.clicked()
            && let Some(name) = state.selected.clone()
        {
            out.commands.push(Command::AddDbItem {
                db: state.query.db,
                name,
                second_slot: false,
            });
        }
    });

    if let Some(message) = state.failed.clone() {
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, message);
            let retry = ui.button("Retry");
            state.rects.note(&retry, || "db_retry".into());
            if retry.clicked() {
                state.failed = None;
                // the last query, as it was sent
                out.commands.push(Command::SetItemDb(state.query.clone()));
            }
        });
    }

    if changed && state.open_sent {
        state.failed = None;
        state.percent = None;
        out.commands.push(Command::SetItemDb(state.query.clone()));
    }

    let Some(view) = view else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Loading...");
        });
        return;
    };
    if view.loading || view.sorting {
        ui.horizontal(|ui| {
            let percent = state.percent.or(view.percent);
            match percent {
                Some(p) => {
                    let bar = ui.add(
                        egui::ProgressBar::new(p as f32 / 100.0)
                            .desired_width(200.0)
                            .text(format!("{p}%")),
                    );
                    state.rects.note(&bar, || "db_progress".into());
                }
                None => {
                    ui.spinner();
                }
            }
            ui.label(if view.loading {
                "Loading..."
            } else {
                "Sorting..."
            });
        });
    }
    if view.rows.is_empty() {
        if !view.loading && !view.sorting {
            ui.weak(&view.note);
        }
        return;
    }
    rows(ui, view, state, out);
    if let Some(command) = state.tip.end_frame(ui.ctx(), now) {
        out.queries.push(command);
    }
}

fn rows(ui: &mut egui::Ui, view: &ItemDbView, state: &mut DbState, out: &mut ItemsOutput) {
    let default = ui.visuals().text_color();
    let db = state.query.db;
    let mut row_clicked = false;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("items_db_rows")
        .auto_shrink(false);
    // a row the keys selected comes into view, scrolling as little as it takes
    if let Some(i) = state.scroll_to.take() {
        let step = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let (offset, height) = state.rows_view;
        let (top, bottom) = (i as f32 * step, (i + 1) as f32 * step);
        if top < offset {
            area = area.vertical_scroll_offset(top);
        } else if bottom > offset + height {
            area = area.vertical_scroll_offset(bottom - height);
        }
    }
    let shown = area.show_rows(ui, ROW_HEIGHT, view.rows.len(), |ui, range| {
        for i in range {
            let row = &view.rows[i];
            let selected = state.selected.as_deref() == Some(&row.name);
            let response = ui
                .horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    ui.style_mut().interaction.selectable_labels = false;
                    ui.label(pob_text(&row.label, 14.0, default));
                    ui.weak(&row.base);
                    if let Some(v) = row.sort_value {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(format!("{v:.1}"))
                        });
                    }
                })
                .response
                .interact(egui::Sense::click_and_drag());
            // PoB's drag of a row onto a slot (R23)
            response.dnd_set_drag_payload(DbDrag {
                db,
                name: row.name.clone(),
            });
            if selected {
                ui.painter().rect_filled(
                    response.rect,
                    2.0,
                    ui.visuals().selection.bg_fill.gamma_multiply(0.4),
                );
            }
            state.rects.note(&response, || format!("db_row_{i}"));
            let ctrl = ui.input(|inp| inp.modifiers.ctrl || inp.modifiers.command);
            let shift = ui.input(|inp| inp.modifiers.shift);
            if response.double_clicked() && !ctrl {
                // PoB's double click: shows the item on the display panel (R15)
                state.selected = Some(row.name.clone());
                out.commands.push(Command::ViewDbItem {
                    db,
                    name: row.name.clone(),
                });
            } else if response.clicked() {
                row_clicked = true;
                state.selected = Some(row.name.clone());
                // PoB's Ctrl+click adds the item and equips it (R15), Ctrl+Shift+click
                // into the second slot (R23)
                // (the second click of a double click is the same gesture: once)
                if ctrl && !response.double_clicked() {
                    out.commands.push(Command::AddDbItem {
                        db,
                        name: row.name.clone(),
                        second_slot: shift,
                    });
                }
            }
            if response.hovered() {
                state.hovered_wiki = Some(match db {
                    ItemDb::Uniques => row.name.clone(),
                    ItemDb::Rares => row.base.clone(),
                });
            }
            state.tip.note(
                &response,
                ControlTooltipQuery::DbItem {
                    db,
                    name: row.name.clone(),
                    // Shift held asks for PoB's child tooltips (R34)
                    shift,
                },
            );
        }
    });
    state.rows_view = (shown.state.clamped_offset().y, shown.inner_rect.height());
    // a click on a row makes the list the selected control; a click elsewhere, or a text box
    // taking the keyboard, ends that
    if row_clicked {
        state.list_focus = true;
    } else if ui.input(|i| i.pointer.any_click()) || ui.ctx().text_edit_focused() {
        state.list_focus = false;
    }
    if !state.list_focus || state.keys_blocked {
        return;
    }
    // Up, Down, Home and End move the selection (ListControl.lua:375-382)
    let at = state
        .selected
        .as_ref()
        .and_then(|name| view.rows.iter().position(|r| &r.name == name));
    if let Some(i) = ui.input_mut(|inp| crate::ui::shortcuts::list_key(inp, view.rows.len(), at)) {
        state.selected = Some(view.rows[i].name.clone());
        state.scroll_to = Some(i);
        ui.ctx().request_repaint();
    }
    // Ctrl+C copies the selected row's text, as PoB's OnSelCopy (ItemDBControl.lua:368-370)
    if let Some(name) = state.selected.clone()
        && view.rows.iter().any(|r| r.name == name)
        && ui.input(crate::ui::shortcuts::list_copy)
    {
        state.copy_asked = Some(name.clone());
        out.queries.push(Command::CopyDbItem { db, name });
    }
}
