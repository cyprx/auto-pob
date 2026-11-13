//! PoE-inspired look: warm near-black panels, square bronze-bordered widgets, gold
//! accents, PoB's stat colours, and Fontin small caps headings (the font the main app
//! already ships).

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke,
    TextStyle, Theme, ThemePreference, Vec2,
};
use std::sync::Arc;

/// Background of the tree view and text fields
pub const BACKGROUND: Color32 = Color32::from_rgb(15, 13, 11);
const PANEL: Color32 = Color32::from_rgb(26, 22, 18);
/// Alternate rows in striped tables
const PANEL_STRIPE: Color32 = Color32::from_rgb(33, 28, 22);
const WIDGET: Color32 = Color32::from_rgb(42, 34, 26);
const WIDGET_HOVERED: Color32 = Color32::from_rgb(60, 48, 34);
const WIDGET_ACTIVE: Color32 = Color32::from_rgb(80, 62, 40);
pub const BRONZE_DIM: Color32 = Color32::from_rgb(58, 46, 32);
const BRONZE: Color32 = Color32::from_rgb(120, 92, 58);
const GOLD: Color32 = Color32::from_rgb(200, 160, 80);
const TEXT: Color32 = Color32::from_rgb(200, 192, 176);
const TEXT_BRIGHT: Color32 = Color32::from_rgb(235, 225, 205);
/// Secondary text, e.g. zero values
pub const TEXT_DIM: Color32 = Color32::from_rgb(130, 122, 108);

// Colours from PoB's `colorCodes` (Data/Global.lua), so stats look like they do in PoB
pub const LIFE: Color32 = Color32::from_rgb(0xE0, 0x50, 0x30);
pub const MANA: Color32 = Color32::from_rgb(0x70, 0x70, 0xFF);
pub const ENERGY_SHIELD: Color32 = Color32::from_rgb(0x88, 0xFF, 0xFF);
pub const ARMOUR: Color32 = Color32::from_rgb(0xC8, 0xC8, 0xC8);
pub const FIRE: Color32 = Color32::from_rgb(0xB9, 0x71, 0x23);
pub const COLD: Color32 = Color32::from_rgb(0x3F, 0x6D, 0xB3);
pub const LIGHTNING: Color32 = Color32::from_rgb(0xAD, 0xAA, 0x47);
pub const CHAOS: Color32 = Color32::from_rgb(0xD0, 0x20, 0x90);
pub const POSITIVE: Color32 = Color32::from_rgb(0x33, 0xFF, 0x77);
pub const NEGATIVE: Color32 = Color32::from_rgb(0xDD, 0x00, 0x22);
/// PoB's unique item colour; used for the main action
pub const UNIQUE: Color32 = Color32::from_rgb(0xAF, 0x60, 0x25);
pub const HEADING: Color32 = GOLD;

const HEADING_FONT: &str = "fontin-smallcaps";

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        HEADING_FONT.into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/fontin-smallcaps.ttf"
        ))),
    );
    fonts.families.insert(
        FontFamily::Name(HEADING_FONT.into()),
        vec![HEADING_FONT.into()],
    );
    ctx.set_fonts(fonts);

    // the palette is dark only
    ctx.set_theme(ThemePreference::Dark);
    ctx.set_visuals_of(Theme::Dark, visuals());
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Name(HEADING_FONT.into())),
        );
        style.spacing.button_padding = Vec2::new(8.0, 3.0);
    });
}

/// A gold Fontin heading.
/// Draws `add` with the look of a row of PoB's lists: a hovered row of a dropped list gets a
/// grey fill and no border (DropDownControl.lua:373-376), a ListControl row no white box
/// (ListControl.lua:236-241). The border stays 1 px wide, unseen, so nothing moves.
pub fn list_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let old = ui.visuals().widgets.hovered.bg_stroke;
    ui.visuals_mut().widgets.hovered.bg_stroke = ROW_HOVER;
    let out = add(ui);
    ui.visuals_mut().widgets.hovered.bg_stroke = old;
    out
}

/// A list row's border under the pointer: none to see
const ROW_HOVER: Stroke = Stroke {
    width: 1.0,
    color: Color32::TRANSPARENT,
};

pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.heading(RichText::new(text).color(HEADING));
}

/// The emphasised button for a panel's main action.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(TEXT_BRIGHT).strong())
        .fill(UNIQUE.gamma_multiply(0.55))
        .stroke(Stroke::new(1.0, UNIQUE))
}

/// Colour for a value that's a change: green for gains, red for losses.
pub fn change_color(value: f64) -> Color32 {
    if value > 0.0 {
        POSITIVE
    } else if value < 0.0 {
        NEGATIVE
    } else {
        TEXT_DIM
    }
}

fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.faint_bg_color = PANEL_STRIPE;
    visuals.window_stroke = Stroke::new(1.0, BRONZE);
    visuals.hyperlink_color = GOLD;
    visuals.selection.bg_fill = Color32::from_rgb(110, 80, 35);
    visuals.selection.stroke = Stroke::new(1.0, GOLD);
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.menu_corner_radius = CornerRadius::ZERO;
    visuals.error_fg_color = Color32::from_rgb(225, 85, 60);
    visuals.warn_fg_color = Color32::from_rgb(225, 170, 60);

    let widgets = &mut visuals.widgets;
    // (state, fill, border, text). `noninteractive` also styles separators and labels.
    for (widget, fill, border, text) in [
        (&mut widgets.noninteractive, PANEL, BRONZE_DIM, TEXT),
        (&mut widgets.inactive, WIDGET, BRONZE, TEXT),
        // PoB's controls draw their border white under the pointer (EditControl.lua:246-248,
        // ButtonControl.lua:45-46, CheckBoxControl.lua:40-41)
        (
            &mut widgets.hovered,
            WIDGET_HOVERED,
            Color32::WHITE,
            TEXT_BRIGHT,
        ),
        (&mut widgets.active, WIDGET_ACTIVE, GOLD, TEXT_BRIGHT),
        // a dropped dropdown's border is white (DropDownControl.lua:250, 274)
        (&mut widgets.open, WIDGET, Color32::WHITE, TEXT),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, border);
        widget.fg_stroke = Stroke::new(1.0, text);
        // square corners, no grow-on-hover: a harder, PoB-like look
        widget.corner_radius = CornerRadius::ZERO;
        widget.expansion = 0.0;
    }
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Pos2, RawInput, Rect};

    fn rect_strokes(shape: &egui::Shape, out: &mut Vec<(Rect, Color32)>) {
        match shape {
            egui::Shape::Rect(r) if r.stroke.width > 0.0 => out.push((r.rect, r.stroke.color)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| rect_strokes(s, out)),
            _ => {}
        }
    }

    /// PoB's EditControl draws its border white while the pointer is over it, grey otherwise
    /// (EditControl.lua:246-254): every edit box of ours does too
    #[test]
    fn a_hovered_edit_box_has_pobs_white_border() {
        let ctx = egui::Context::default();
        ctx.set_theme(ThemePreference::Dark);
        ctx.set_visuals_of(Theme::Dark, visuals());
        let mut rect = Rect::NOTHING;
        let mut strokes = Vec::new();
        for events in [
            vec![],
            vec![],
            vec![Event::PointerMoved(Pos2::new(40.0, 20.0))],
            vec![],
        ] {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 200.0))),
                events,
                ..Default::default()
            };
            let mut text = String::from("abc");
            let output = ctx.run_ui(input, |ui| {
                rect = ui.add(egui::TextEdit::singleline(&mut text)).rect;
            });
            strokes.clear();
            for clipped in &output.shapes {
                rect_strokes(&clipped.shape, &mut strokes);
            }
            output.drop_without_applying_deltas();
        }
        assert!(rect.contains(Pos2::new(40.0, 20.0)), "{rect:?}");
        assert!(
            strokes
                .iter()
                .any(|(r, c)| *c == Color32::WHITE && r.intersects(rect)),
            "{rect:?} {strokes:?}"
        );
    }

    /// Runs `draw` with the theme for `events`, frame by frame; the last frame's outlined
    /// rectangles and the rectangle `draw` returned
    fn run(
        frames: Vec<Vec<Event>>,
        mut draw: impl FnMut(&mut egui::Ui) -> Rect,
    ) -> (Rect, Vec<(Rect, Color32)>) {
        let ctx = egui::Context::default();
        ctx.set_theme(ThemePreference::Dark);
        ctx.set_visuals_of(Theme::Dark, visuals());
        let mut rect = Rect::NOTHING;
        let mut strokes = Vec::new();
        for events in frames {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 300.0))),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| rect = draw(ui));
            strokes.clear();
            for clipped in &output.shapes {
                rect_strokes(&clipped.shape, &mut strokes);
            }
            output.drop_without_applying_deltas();
        }
        (rect, strokes)
    }

    fn white_around(rect: Rect, strokes: &[(Rect, Color32)]) -> bool {
        strokes
            .iter()
            .any(|(r, c)| *c == Color32::WHITE && (r.center() - rect.center()).length() < 2.0)
    }

    /// A hovered row of a list or a dropped list gets no white box (DropDownControl.lua:
    /// 373-376, ListControl.lua:236-241); a hovered tab, a ButtonControl, does
    #[test]
    fn a_hovered_list_row_has_no_white_border() {
        let at = Pos2::new(20.0, 10.0);
        let frames = || vec![vec![], vec![Event::PointerMoved(at)], vec![]];
        let (rect, strokes) = run(frames(), |ui| {
            list_row(ui, |ui| ui.selectable_label(false, "a row")).rect
        });
        assert!(rect.contains(at), "{rect:?}");
        assert!(!white_around(rect, &strokes), "{strokes:?}");
        let (rect, strokes) = run(frames(), |ui| ui.selectable_label(false, "a tab").rect);
        assert!(white_around(rect, &strokes), "{strokes:?}");
    }

    /// A dropped dropdown draws its border white (DropDownControl.lua:250, 274)
    #[test]
    fn a_dropped_dropdown_has_a_white_border() {
        let at = Pos2::new(30.0, 10.0);
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let frames = vec![
            vec![],
            vec![Event::PointerMoved(at)],
            vec![press(true)],
            vec![press(false)],
            vec![Event::PointerMoved(Pos2::new(250.0, 250.0))],
            vec![],
        ];
        let (rect, strokes) = run(frames, |ui| {
            egui::ComboBox::from_id_salt("dd")
                .selected_text("one")
                .show_ui(ui, |ui| {
                    ui.label("two");
                })
                .response
                .rect
        });
        assert!(white_around(rect, &strokes), "{rect:?} {strokes:?}");
    }
}
