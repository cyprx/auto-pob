//! PoB's item tooltip (`ItemTooltip`): sized lines, a rarity title band and clickable mod lines.
//! Fonts, centring and backgrounds are approximated.

use super::pob_text::{pob_colour, pob_text};
use crate::engine::views::{ItemTooltip, ItemTooltipLine, ModLineRef};
use crate::theme;
use eframe::egui::{self, Color32, Sense, Stroke, Vec2};

/// PoB font sizes are drawn at this fraction of their size in points.
const SIZE_SCALE: f32 = 0.9;
/// A centred line this big (and above) at the top is the item's name.
const TITLE_SIZE: u32 = 18;

fn line_label(
    ui: &mut egui::Ui,
    size: u32,
    text: &str,
    font: Option<&str>,
    default: Color32,
    clickable: bool,
) -> egui::Response {
    let mut job = pob_text(text, size as f32 * SIZE_SCALE, default);
    if font.is_some_and(|f| f.contains("ITALIC")) {
        for s in &mut job.sections {
            s.format.italics = true;
        }
    }
    let label = egui::Label::new(job).selectable(false);
    ui.add(if clickable {
        label.sense(Sense::click())
    } else {
        label
    })
}

/// Draws the tooltip; returns the mod line clicked, if any.
pub fn item_tooltip_ui(ui: &mut egui::Ui, tooltip: &ItemTooltip) -> Option<ModLineRef> {
    item_tooltip_ui_with(ui, tooltip, &mut |_, _| {})
}

/// As `item_tooltip_ui`; `seen` is told each text line's index in `tooltip.lines` and its
/// response (for tests to find lines).
pub fn item_tooltip_ui_with(
    ui: &mut egui::Ui,
    tooltip: &ItemTooltip,
    seen: &mut dyn FnMut(usize, &egui::Response),
) -> Option<ModLineRef> {
    if tooltip.children.is_empty() {
        return single(ui, tooltip, seen);
    }
    // PoB draws the child tooltips (Shift's granted skills) beside the item's (R34)
    ui.horizontal_top(|ui| {
        let clicked = single(ui, tooltip, seen);
        for child in &tooltip.children {
            single(ui, child, &mut |_, _| {});
        }
        clicked
    })
    .inner
}

fn single(
    ui: &mut egui::Ui,
    tooltip: &ItemTooltip,
    seen: &mut dyn FnMut(usize, &egui::Response),
) -> Option<ModLineRef> {
    let default = ui.visuals().text_color();
    let border = tooltip
        .colour
        .as_deref()
        .map_or(theme::BRONZE_DIM, |c| pob_colour(c, theme::BRONZE_DIM));
    let mut clicked = None;
    let titles = if tooltip.header.is_some() {
        tooltip
            .lines
            .iter()
            .take_while(|l| {
                matches!(l, ItemTooltipLine::Line { size, center: true, .. } if *size >= TITLE_SIZE)
            })
            .count()
    } else {
        0
    };
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, border))
        .inner_margin(6.0)
        .show(ui, |ui| {
            if titles > 0 {
                egui::Frame::NONE
                    .fill(border.gamma_multiply(0.3))
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            for line in &tooltip.lines[..titles] {
                                if let ItemTooltipLine::Line {
                                    size, text, font, ..
                                } = line
                                {
                                    line_label(ui, *size, text, font.as_deref(), default, false);
                                }
                            }
                            for text in tooltip.influences.iter().chain(&tooltip.foil) {
                                ui.label(pob_text(text, 12.0, default));
                            }
                        });
                    });
            }
            for (index, line) in tooltip.lines.iter().enumerate().skip(titles) {
                match line {
                    ItemTooltipLine::Line {
                        size,
                        text,
                        font,
                        center,
                        background,
                        mod_line,
                    } => {
                        let draw = |ui: &mut egui::Ui| {
                            line_label(
                                ui,
                                *size,
                                text,
                                font.as_deref(),
                                default,
                                mod_line.is_some(),
                            )
                        };
                        let frame = egui::Frame::NONE.fill(if background.is_some() {
                            Color32::from_white_alpha(10)
                        } else {
                            Color32::TRANSPARENT
                        });
                        let response = frame
                            .show(ui, |ui| {
                                if *center {
                                    ui.vertical_centered(draw).inner
                                } else {
                                    draw(ui)
                                }
                            })
                            .inner;
                        seen(index, &response);
                        // a toggleable line lights up under the pointer, and any mouse button
                        // pressed on it toggles it, at the press (ItemsTab.lua:1521-1539)
                        if let Some(m) = mod_line
                            && response.hovered()
                        {
                            ui.painter().rect_filled(
                                response.rect,
                                0.0,
                                Color32::from_white_alpha(38),
                            );
                            let pressed = ui.input(|i| {
                                i.events.iter().any(|e| {
                                    matches!(e, egui::Event::PointerButton { pressed: true, .. })
                                })
                            });
                            if pressed {
                                clicked = Some(*m);
                            }
                        }
                    }
                    ItemTooltipLine::Separator { .. } => {
                        let width = ui.available_width().max(1.0);
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
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::views::ModLineList;
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    fn tooltip() -> ItemTooltip {
        let line = |text: &str, mod_line| ItemTooltipLine::Line {
            size: 14,
            text: text.into(),
            font: None,
            center: false,
            background: None,
            mod_line,
        };
        ItemTooltip {
            header: Some("RARE".into()),
            influences: vec!["Shaper Item".into()],
            foil: None,
            colour: Some("^xFFFF77".into()),
            lines: vec![
                ItemTooltipLine::Line {
                    size: 20,
                    text: "^xFFFF77Doom Loop".into(),
                    font: Some("FONTIN SC".into()),
                    center: true,
                    background: None,
                    mod_line: None,
                },
                ItemTooltipLine::Separator { separator: 10 },
                line(
                    "+10 life",
                    Some(ModLineRef {
                        list: ModLineList::Explicit,
                        index: 1,
                    }),
                ),
            ],
            children: Vec::new(),
        }
    }

    fn run(ctx: &egui::Context, events: Vec<Event>) -> Option<ModLineRef> {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            events,
            ..Default::default()
        };
        let mut hit = None;
        ctx.run_ui(input, |ui| {
            hit = hit.or(item_tooltip_ui(ui, &tooltip()));
        })
        .drop_without_applying_deltas();
        hit
    }

    #[test]
    fn draws_the_band_and_lines_and_returns_a_clicked_mod_line() {
        let ctx = egui::Context::default();
        assert_eq!(run(&ctx, vec![]), None);
        // the mod line is the last row: find it by scanning down the tooltip's left edge
        let mut found = None;
        for y in (10..200).step_by(4) {
            let pos = Pos2::new(20.0, y as f32);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            run(&ctx, vec![Event::PointerMoved(pos)]);
            // PoB toggles at the press
            if let Some(hit) = run(&ctx, vec![button(true)]) {
                found = Some(hit);
                run(&ctx, vec![button(false)]);
                // any button does (ItemsTab.lua:1530: `event.key:match("BUTTON")`)
                let right = run(
                    &ctx,
                    vec![Event::PointerButton {
                        pos,
                        button: PointerButton::Secondary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    }],
                );
                assert_eq!(right, found, "a right press toggles too");
                break;
            }
            run(&ctx, vec![button(false)]);
        }
        assert_eq!(
            found,
            Some(ModLineRef {
                list: ModLineList::Explicit,
                index: 1
            })
        );
    }
}
