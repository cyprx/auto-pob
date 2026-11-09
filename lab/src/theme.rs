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
const BRONZE_DIM: Color32 = Color32::from_rgb(58, 46, 32);
const BRONZE: Color32 = Color32::from_rgb(120, 92, 58);
const GOLD_DIM: Color32 = Color32::from_rgb(150, 118, 62);
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
        (&mut widgets.hovered, WIDGET_HOVERED, GOLD_DIM, TEXT_BRIGHT),
        (&mut widgets.active, WIDGET_ACTIVE, GOLD, TEXT_BRIGHT),
        (&mut widgets.open, WIDGET, GOLD_DIM, TEXT),
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
