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
/// Filter dropdowns per line, so they fit a narrow column.
const PER_ROW: usize = 3;

pub struct DbState {
    /// The panel is open; the engine has a database open while this is so and the tab shows
    pub expanded: bool,
    /// What the user chose (the search is the one last sent; the box holds what is typed)
    query: ItemDbQuery,
    /// The engine has a database open for us
    open_sent: bool,
    search: EditBox,
    /// The box's text when it was last seen changing, and when
    seen: (String, f64),
    percent: Option<u32>,
    failed: Option<String>,
    selected: Option<String>,
    tip: HoverTip,
    pub(crate) rects: Rects,
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
        }
    }
}

impl DbState {
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
        egui::ComboBox::from_id_salt(("items_db", key))
            .width(100.0)
            .truncate()
            .selected_text(pob_text(current, 14.0, default))
            .show_ui(ui, |ui| {
                for (i, label) in list.labels.iter().enumerate() {
                    let item = ui.selectable_label(i == selected, pob_text(label, 14.0, default));
                    rects.note(&item, || format!("db_{key}_{i}"));
                    if item.clicked() && i != selected {
                        picked = Some(i);
                    }
                }
            })
    });
    rects.note(&combo.inner.response, || format!("db_{key}"));
    picked
}

pub fn show(
    ui: &mut egui::Ui,
    view: Option<&ItemDbView>,
    state: &mut DbState,
    now: f64,
) -> ItemsOutput {
    let mut out = ItemsOutput::default();
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
            (ItemDb::Rares, "Rares", "db_rares"),
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

    let opts = EditBoxOptions {
        width: 240.0,
        hint: Some("Search"),
        ..EditBoxOptions::default()
    };
    ui.horizontal(|ui| {
        let id = ui.scope_id().with("items_db_search");
        let (response, send) = state
            .search
            .show(ui, id, &state.query.search.clone(), false, &opts);
        state.rects.note(&response, || "db_search".into());
        if let Some(text) = send {
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
    egui::ScrollArea::vertical()
        .id_salt("items_db_rows")
        .auto_shrink(false)
        .show_rows(ui, ROW_HEIGHT, view.rows.len(), |ui, range| {
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
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| ui.weak(format!("{v:.1}")),
                            );
                        }
                    })
                    .response
                    .interact(egui::Sense::click());
                if selected {
                    ui.painter().rect_filled(
                        response.rect,
                        2.0,
                        ui.visuals().selection.bg_fill.gamma_multiply(0.4),
                    );
                }
                state.rects.note(&response, || format!("db_row_{i}"));
                let ctrl = ui.input(|inp| inp.modifiers.ctrl || inp.modifiers.command);
                if response.double_clicked() && !ctrl {
                    // PoB's double click: shows the item on the display panel (R15)
                    state.selected = Some(row.name.clone());
                    out.commands.push(Command::ViewDbItem {
                        db,
                        name: row.name.clone(),
                    });
                } else if response.clicked() {
                    state.selected = Some(row.name.clone());
                    // PoB's Ctrl+click adds the item and equips it (R15)
                    // (the second click of a double click is the same gesture: once)
                    if ctrl && !response.double_clicked() {
                        out.commands.push(Command::AddDbItem {
                            db,
                            name: row.name.clone(),
                        });
                    }
                }
                state.tip.note(
                    &response,
                    ControlTooltipQuery::DbItem {
                        db,
                        name: row.name.clone(),
                    },
                );
            }
        });
}
