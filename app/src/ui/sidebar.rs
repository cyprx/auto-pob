//! The stats sidebar: the player's rows, then the minion's, coloured like PoB.

use crate::engine::views::{StatRow, StatsView};
use crate::theme;
use crate::widgets::pob_text;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, stats: Option<&StatsView>, behind: bool) {
    ui.horizontal(|ui| {
        theme::heading(ui, "Stats");
        if behind {
            ui.spinner();
        }
    });
    let Some(stats) = stats else {
        ui.label("Calculating…");
        return;
    };
    egui::ScrollArea::vertical().show(ui, |ui| {
        rows(ui, "player_stats", &stats.stats);
        if !stats.minion.is_empty() {
            ui.add_space(6.0);
            ui.strong("Minion");
            rows(ui, "minion_stats", &stats.minion);
        }
    });
}

fn rows(ui: &mut egui::Ui, id: &str, rows: &[StatRow]) {
    let default = ui.visuals().text_color();
    egui::Grid::new(id).striped(true).show(ui, |ui| {
        for row in rows {
            let label = format!("{}{}", row.color.as_deref().unwrap_or(""), row.label);
            ui.label(pob_text(&label, 14.0, default));
            ui.label(pob_text(&row.text, 14.0, default));
            ui.end_row();
        }
    });
}
