//! The Summary view: PoB's `DrawStatList` rows as the engine recorded them (R4), with the
//! difference coloured as PoB colours it.

use crate::engine::views::{CompareRow, CompareView};
use crate::theme;
use crate::widgets::pob_text;
use eframe::egui;

/// The difference's colour: PoB's positive colour when the comparison is better for us, its
/// negative one when worse, the plain text colour otherwise.
pub(super) fn diff_colour(row: &CompareRow, default: egui::Color32) -> egui::Color32 {
    match row.better {
        Some(true) => theme::POSITIVE,
        Some(false) => theme::NEGATIVE,
        None => default,
    }
}

pub(super) fn show(ui: &mut egui::Ui, view: &CompareView) {
    let default = ui.visuals().text_color();
    egui::ScrollArea::both()
        .id_salt("compare_summary")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("compare_summary_grid")
                .striped(true)
                .spacing([24.0, 2.0])
                .show(ui, |ui| {
                    ui.label(pob_text("^7Stat", 14.0, default));
                    ui.label(pob_text(&view.headers[0], 14.0, default));
                    ui.label(pob_text(&view.headers[1], 14.0, default));
                    ui.label(pob_text("^7Difference", 14.0, default));
                    ui.end_row();
                    for row in &view.summary {
                        ui.label(pob_text(&row.label, 14.0, default));
                        ui.label(pob_text(&row.ours, 14.0, default));
                        ui.label(pob_text(&row.theirs, 14.0, default));
                        // the colour comes from `better`, not from the diff's own code
                        let colour = diff_colour(row, default);
                        let text = super::plain(&row.diff);
                        ui.label(egui::RichText::new(text).color(colour));
                        ui.end_row();
                    }
                });
        });
}
