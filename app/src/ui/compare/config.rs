//! The Config view: PoB's rows (per section, the options the two builds set differently, then
//! the ones they set alike that PoB lists), PoB's search box over their labels, "Show All
//! Configurations", our own config set, and "Copy Config from Compare Build" (R8).

use super::{CompareOutput, CompareUi};
use crate::engine::views::{CompareConfigRow, CompareView};
use crate::engine::{Command, PopupInput};
use crate::widgets::pob_text;
use eframe::egui;

/// PoB's `configSearchEdit` filter (CompareTab.lua:2091-2097): with any non-space character
/// typed, the lowercased label must match the lowercased text as `string.matchOrPattern` does
/// once every pattern character but `(`, `)` and `|` is escaped. So the text is a plain
/// substring, except that `|` separates alternatives and a `(...)` group holds alternatives
/// of its own. The text is not trimmed.
pub(super) fn matches(row: &CompareConfigRow, filter: &str) -> bool {
    if filter.chars().all(is_lua_space) {
        return true;
    }
    // PoB lowers the raw label, its colour codes included (CompareTab.lua:2094)
    match_or_pattern(&lua_lower(&row.label), &lua_lower(filter))
}

/// Lua's `%s` in the C locale
fn is_lua_space(c: char) -> bool {
    c == ' ' || ('\t'..='\r').contains(&c)
}

/// Lua's `string.lower`: ASCII letters only
fn lua_lower(text: &str) -> String {
    text.to_ascii_lowercase()
}

/// The alternatives `generateOrPatterns` makes of `pattern` (Common.lua:976-1008): each
/// balanced `(...)` group is replaced by each of its own alternatives, then the results are split
/// on `|`, empty pieces dropped
fn or_patterns(pattern: &str) -> Vec<String> {
    // the balanced groups, left to right, as `%b()` finds them
    let bytes = pattern.as_bytes();
    let mut groups = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'(' {
            let mut depth = 0;
            let mut end = None;
            for (j, &b) in bytes.iter().enumerate().skip(at) {
                match b {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match end {
                Some(end) => {
                    groups.push((at, end));
                    at = end + 1;
                }
                // `%b()` finds no group at an unbalanced "(": it tries the next one
                None => at += 1,
            }
        } else {
            at += 1;
        }
    }
    let first = groups.first().map_or(pattern.len(), |g| g.0);
    let mut generated = vec![pattern[..first].to_owned()];
    for (i, &(open, close)) in groups.iter().enumerate() {
        let next = groups.get(i + 1).map_or(pattern.len(), |g| g.0);
        let after = &pattern[close + 1..next];
        let inner = or_patterns(&pattern[open + 1..close]);
        let mut out = Vec::new();
        for start in &generated {
            for alt in &inner {
                out.push(format!("{start}{alt}{after}"));
            }
        }
        generated = out;
    }
    let mut out: Vec<String> = Vec::new();
    for g in generated {
        for piece in g.split('|').filter(|p| !p.is_empty()) {
            if !out.iter().any(|o| o == piece) {
                out.push(piece.to_owned());
            }
        }
    }
    out
}

/// `string.matchOrPattern` on escaped text: any alternative found in `text`. A leftover `(` or
/// `)` makes Lua's `match` raise, which PoB's `PCall` turns into no match.
fn match_or_pattern(text: &str, pattern: &str) -> bool {
    let alternatives = or_patterns(pattern);
    if alternatives.iter().any(|a| a.contains(['(', ')'])) {
        return false;
    }
    alternatives.iter().any(|a| text.contains(a.as_str()))
}

/// PoB's box takes no control characters (`%c`) and 100 bytes (CompareTab.lua:936)
const SEARCH_LIMIT: usize = 100;

pub(super) fn show(
    ui: &mut egui::Ui,
    tab: &mut CompareUi,
    view: &CompareView,
    out: &mut CompareOutput,
) {
    let default = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    ui.horizontal_wrapped(|ui| {
        let copy = ui
            .add_enabled(
                !tab.busy,
                egui::Button::new("Copy Config from Compare Build"),
            )
            .on_hover_text(
                "Sets this build's options to the comparison's (one Undo on the Config tab)",
            );
        tab.rects.note(&copy, || "copy_config".into());
        if copy.clicked() {
            out.commands.push(Command::CopyCompareConfig);
        }
        // PoB's toggle, labelled by its state; the engine presses it to the state asked for
        if let Some(toggle) = super::control(view, "configToggleBtn").filter(|c| c.shown) {
            let r = ui.add_enabled(
                toggle.enabled && !tab.busy,
                egui::Button::new(super::plain(&toggle.label)),
            );
            tab.rects.note(&r, || "config_toggle".into());
            if r.clicked() {
                out.commands.push(Command::SetCompareControl {
                    name: toggle.name.clone(),
                    value: PopupInput::Check(!toggle.checked),
                });
            }
        }
        let before = tab.config_filter.clone();
        let pasted = crate::ui::name_box::pasted(ui);
        let search = ui.add(
            egui::TextEdit::singleline(&mut tab.config_filter)
                .hint_text("Search")
                .desired_width(240.0),
        );
        if search.changed() {
            let raw = std::mem::take(&mut tab.config_filter);
            tab.config_filter = crate::ui::name_box::spliced_by(
                &before,
                &raw,
                |c| !c.is_ascii_control(),
                SEARCH_LIMIT,
                pasted,
            );
            crate::ui::name_box::place_caret(
                ui.ctx(),
                search.id,
                &before,
                &raw,
                &tab.config_filter,
            );
        }
        crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &search, &tab.config_filter);
        tab.rects.note(&search, || "config_search".into());
        // its "x" empties it (CompareTab's configSearchEdit is clearable)
        if let Some(x) = crate::widgets::edit_box::clear_button(ui, &mut tab.config_filter) {
            tab.rects.note(&x, || "config_search_clear".into());
        }
        if std::mem::take(&mut tab.focus_search) {
            search.request_focus();
        }
        if let Some(config) = tab.config.clone() {
            ui.label("Config set:");
            let active = config
                .sets
                .iter()
                .find(|s| s.id == config.active_set)
                .map_or("", |s| s.title.as_str());
            let rects = &mut tab.rects;
            ui.add_enabled_ui(config.sets.len() > 1, |ui| {
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt("compare_config_set")
                    .selected_text(active)
                    .show_ui(ui, |ui| {
                        for set in &config.sets {
                            let item = crate::widgets::dropdown::item(
                                ui,
                                set.id == config.active_set,
                                &set.title,
                            );
                            rects.note(&item, || format!("config_set:{}", set.id));
                            if item.clicked() && set.id != config.active_set {
                                out.commands.push(Command::SelectConfigSet { id: set.id });
                            }
                        }
                    });
                rects.note(&combo.response, || "config_set".into());
                // the wheel and the arrows step it (DropDownControl.lua:476-516)
                let at = config
                    .sets
                    .iter()
                    .position(|s| s.id == config.active_set)
                    .unwrap_or(0);
                if let Some(i) = crate::widgets::scroll_step::combo_step(
                    ui,
                    &combo.response,
                    config.sets.len(),
                    at,
                ) {
                    out.commands.push(Command::SelectConfigSet {
                        id: config.sets[i].id,
                    });
                }
            });
        }
    });
    let rows: Vec<_> = view
        .config
        .iter()
        .filter(|r| matches(r, &tab.config_filter))
        .collect();
    egui::ScrollArea::both()
        .id_salt("compare_config")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if view.config.is_empty() {
                // PoB's line for no section to draw (CompareTab.lua:4993-4996)
                ui.label(pob_text(
                    "^x33FF77No configuration options to display.",
                    16.0,
                    default,
                ));
                return;
            }
            if rows.is_empty() {
                ui.label("No option matches the search.");
                return;
            }
            egui::Grid::new("compare_config_grid")
                .striped(true)
                .spacing([24.0, 2.0])
                .show(ui, |ui| {
                    ui.strong("Option");
                    // PoB's column headers: our short build name in its POSITIVE colour, the
                    // comparison's label in WARNING (CompareTab.lua:1810-1813, 4370-4371, 4922-4925)
                    ui.label(super::header(&view.headers[0], default));
                    ui.label(super::header(&view.headers[1], default));
                    ui.end_row();
                    for row in rows {
                        // the options set alike follow the differing ones, dimmed
                        let colour = if row.common { weak } else { default };
                        let r = ui.label(pob_text(&row.label, 14.0, colour));
                        tab.rects.note(&r, || format!("config_row:{}", row.var));
                        // PoB's boxes for each build's value (CompareTab.lua:1127-1223),
                        // its formatted value where there is none
                        for (control, value) in [
                            (&row.ours_control, &row.ours),
                            (&row.theirs_control, &row.theirs),
                        ] {
                            match control {
                                Some(control) => {
                                    let commands =
                                        tab.renderer.inline_controls(ui, &[control], tab.busy);
                                    out.commands
                                        .extend(commands.into_iter().map(super::readdress));
                                }
                                None => {
                                    ui.label(pob_text(value, 14.0, default));
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}
