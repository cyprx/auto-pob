//! The Config view: PoB's rows (per section, the options the two builds set differently, then
//! the ones they set alike that PoB lists), PoB's search box over their labels, "Show All
//! Configurations", our own config set, and "Copy Config from Compare Build" (R8).

use super::{CompareOutput, CompareUi};
use crate::engine::views::{CompareConfigRow, CompareView};
use crate::engine::{Command, PopupInput};
use crate::widgets::pob_text;
use eframe::egui;

/// PoB's `configSearchEdit` filter: the label contains the text, ignoring case.
pub(super) fn matches(row: &CompareConfigRow, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty() || super::plain(&row.label).to_lowercase().contains(&filter)
}

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
        let search = ui.add(
            egui::TextEdit::singleline(&mut tab.config_filter)
                .hint_text("Search")
                .desired_width(240.0),
        );
        tab.rects.note(&search, || "config_search".into());
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
                let combo = egui::ComboBox::from_id_salt("compare_config_set")
                    .selected_text(active)
                    .show_ui(ui, |ui| {
                        for set in &config.sets {
                            let item = ui.selectable_label(set.id == config.active_set, &set.title);
                            rects.note(&item, || format!("config_set:{}", set.id));
                            if item.clicked() && set.id != config.active_set {
                                out.commands.push(Command::SelectConfigSet { id: set.id });
                            }
                        }
                    });
                rects.note(&combo.response, || "config_set".into());
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
                ui.label("No option to show.");
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
                    ui.strong("This build");
                    ui.strong(super::plain(&view.headers[1]));
                    ui.end_row();
                    for row in rows {
                        // the options set alike follow the differing ones, dimmed
                        let colour = if row.common { weak } else { default };
                        let r = ui.label(pob_text(&row.label, 14.0, colour));
                        tab.rects.note(&r, || format!("config_row:{}", row.var));
                        ui.label(pob_text(&row.ours, 14.0, default));
                        ui.label(pob_text(&row.theirs, 14.0, default));
                        ui.end_row();
                    }
                });
        });
}
