//! Item/node tooltips: PoB's `{ size, text }` lines and separators, and for a node's or a gem's
//! PoB's header and separator art and a notable's oils.

use super::art;
use super::pob_text::pob_text;
use super::tooltip_art::{self, LineArt, TooltipArt};
use crate::engine::views::TooltipLine;
use crate::theme;
use eframe::egui::{self, Sense, Stroke, Vec2};

/// PoB font sizes (14 body, 16-20 headings) are drawn at this fraction of their size in points.
const SIZE_SCALE: f32 = 0.9;

/// Draws `lines` top to bottom. Text honours PoB colour codes; separators are a thin rule.
pub fn tooltip_ui(ui: &mut egui::Ui, lines: &[TooltipLine]) {
    styled_tooltip_ui(ui, lines, &Look::default());
}

/// What PoB knows of a tooltip besides its lines
#[derive(Debug, Default, Clone, Copy)]
pub struct Look<'a> {
    /// `tooltipHeader`: the header art (NOTABLE, GEM, ...)
    pub header: Option<&'a str>,
    /// An anointable notable's oils, after its name
    pub recipe: &'a [String],
    /// `tooltip.center`: every line centred (a node's tooltip)
    pub center: bool,
}

/// The images PoB draws on a tooltip of these lines (see [`tooltip_art::tooltip_art`])
pub fn tooltip_lines_art(
    lines: &[TooltipLine],
    look: &Look,
    styled: bool,
) -> Vec<tooltip_art::Drawn> {
    tooltip_art::tooltip_art(
        &TooltipArt {
            header: look.header,
            influences: &[],
            foil: None,
            recipe: look.recipe,
        },
        styled,
        lines.iter().map(|l| match l {
            TooltipLine::Line { .. } => LineArt::Text { background: None },
            TooltipLine::Separator { .. } => LineArt::Separator,
        }),
    )
}

/// `tooltip_ui` with PoB's header art, separator images and oils when "Styled Tooltips with
/// Flavour Text" is on (Tooltip.lua:221-298, 466-602)
pub fn styled_tooltip_ui(ui: &mut egui::Ui, lines: &[TooltipLine], look: &Look) {
    let default = ui.visuals().text_color();
    let styled = tooltip_art::styled(ui.ctx());
    let header = look
        .header
        .filter(|_| styled && matches!(lines.first(), Some(TooltipLine::Line { .. })))
        .map(tooltip_art::header_art);
    let header_slot = ui.painter().add(egui::Shape::Noop);
    if let Some(art) = &header {
        ui.add_space(art.text_offset);
    }
    // a gem's first separator is left out with the option on, header art or not
    // (Tooltip.lua:573-585)
    let mut skip_separator = styled && look.header.is_some_and(tooltip_art::skips_first_separator);
    let centred = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
        if look.center {
            ui.vertical_centered(|ui| add(ui));
        } else {
            add(ui);
        }
    };
    for (i, line) in lines.iter().enumerate() {
        match line {
            TooltipLine::Line { size, text } => {
                let job = pob_text(text, *size as f32 * SIZE_SCALE, default);
                if i == 0 && !look.recipe.is_empty() {
                    centred(ui, &mut |ui| {
                        ui.horizontal(|ui| {
                            ui.label(job.clone());
                            oils(ui, *size as f32 * SIZE_SCALE, look.recipe, styled, default);
                        });
                    });
                } else {
                    centred(ui, &mut |ui| {
                        ui.label(job.clone());
                    });
                }
            }
            TooltipLine::Separator { separator } => {
                let width = ui.available_width().max(1.0);
                match look.header.filter(|_| styled) {
                    Some(name) => {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(width, *separator as f32),
                            Sense::hover(),
                        );
                        if std::mem::take(&mut skip_separator) {
                            continue;
                        }
                        art::paint(
                            ui.ctx(),
                            ui.painter(),
                            tooltip_art::separator_art(name),
                            rect,
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
                        ui.painter().hline(
                            rect.x_range(),
                            rect.center().y,
                            Stroke::new(1.0, theme::BRONZE_DIM),
                        );
                    }
                }
            }
        }
    }
    if let Some(art) = &header {
        let m = ui.style().spacing.menu_margin;
        let inner = ui.min_rect();
        let outer = egui::Rect::from_min_max(
            egui::pos2(
                inner.left() - f32::from(m.left),
                inner.top() - f32::from(m.top),
            ),
            egui::pos2(inner.right() + f32::from(m.right), inner.bottom()),
        );
        let shapes = tooltip_art::header_shapes(ui.ctx(), outer, art, [1.0; 3], &[]);
        ui.painter().set(header_slot, egui::Shape::Vec(shapes));
    }
}

/// An anointable notable's oils after its name: each oil's name without "Oil", 3/4 the title's
/// size, and its image the title's size (Tooltip.lua:229-268)
fn oils(ui: &mut egui::Ui, size: f32, recipe: &[String], styled: bool, default: egui::Color32) {
    for oil in recipe {
        let name = oil
            .strip_suffix("Oil")
            .filter(|n| !n.is_empty())
            .unwrap_or(oil);
        let colour = if styled {
            egui::Color32::from_rgb(0xF8, 0xE6, 0xCA)
        } else {
            default
        };
        ui.label(
            egui::RichText::new(name)
                .size((size * 3.0 / 4.0).floor())
                .color(colour),
        );
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        art::paint(
            ui.ctx(),
            ui.painter(),
            &format!("TreeData/{oil}.png"),
            rect,
            egui::Color32::WHITE,
        );
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
