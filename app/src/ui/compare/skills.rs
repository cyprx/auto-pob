//! The Skills view: PoB's socket-group pairs, as PoB matches and draws them (its "+ " and "- "
//! marks and colour codes kept), each side under its own group header.

use crate::engine::views::{CompareSkillRow, CompareView};
use crate::widgets::pob_text;
use eframe::egui;

/// The two sides' group headers: `label` is ours, or theirs when this build has no group in
/// the pair
fn headers(row: &CompareSkillRow) -> [Option<&str>; 2] {
    if row.ours.is_empty() {
        [None, row.theirs_label.as_deref().or(Some(&row.label))]
    } else {
        [Some(&row.label), row.theirs_label.as_deref()]
    }
}

pub(super) fn show(ui: &mut egui::Ui, view: &CompareView) {
    let default = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    egui::ScrollArea::both()
        .id_salt("compare_skills")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if view.skills.is_empty() {
                ui.label("Neither build has a socket group.");
                return;
            }
            egui::Grid::new("compare_skills_grid")
                .striped(true)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("This build");
                    ui.strong(super::plain(&view.headers[1]));
                    ui.end_row();
                    for row in &view.skills {
                        for (header, lines) in
                            headers(row).into_iter().zip([&row.ours, &row.theirs])
                        {
                            ui.vertical(|ui| {
                                if let Some(h) = header {
                                    ui.label(pob_text(h, 16.0, default));
                                }
                                if lines.is_empty() {
                                    ui.label(egui::RichText::new("(no group)").color(weak));
                                }
                                for line in lines {
                                    ui.label(pob_text(line, 14.0, default));
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
        });
}
