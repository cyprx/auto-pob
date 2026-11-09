//! Item/node tooltips: PoB's `{ size, text }` lines and separators.

use super::pob_text::pob_text;
use crate::engine::views::TooltipLine;
use crate::theme;
use eframe::egui::{self, Sense, Stroke, Vec2};

/// PoB font sizes (14 body, 16-20 headings) are drawn at this fraction of their size in points.
const SIZE_SCALE: f32 = 0.9;

/// Draws `lines` top to bottom. Text honours PoB colour codes; separators are a thin rule.
pub fn tooltip_ui(ui: &mut egui::Ui, lines: &[TooltipLine]) {
    let default = ui.visuals().text_color();
    for line in lines {
        match line {
            TooltipLine::Line { size, text } => {
                let job = pob_text(text, *size as f32 * SIZE_SCALE, default);
                ui.label(job);
            }
            TooltipLine::Separator { .. } => {
                let width = ui.available_width().max(1.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0, theme::BRONZE_DIM),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_lines_and_separators_without_panicking() {
        let lines = vec![
            TooltipLine::Line {
                size: 20,
                text: "^xAF6025Kaom's Heart".into(),
            },
            TooltipLine::Separator { separator: 10 },
            TooltipLine::Line {
                size: 14,
                text: "50^ life".into(),
            },
        ];
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |ui| {
            tooltip_ui(ui, &lines);
        })
        .drop_without_applying_deltas();
    }
}
