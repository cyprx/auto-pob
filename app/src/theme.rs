//! PoE-inspired look: warm near-black panels, square bronze-bordered widgets, gold
//! accents, PoB's stat colours, and Fontin small caps headings (the font the main app
//! already ships).

use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Rect,
    RichText, Sense, Stroke, TextStyle, Theme, ThemePreference, Vec2,
};
use egui::theme::{DefaultStyle, StyleProvider};
use egui::widget_style::{StyleArgs, TextEditStyle, WidgetState};
use std::sync::Arc;

/// Background of the tree view and text fields
pub const BACKGROUND: Color32 = Color32::from_rgb(15, 13, 11);
const PANEL: Color32 = Color32::from_rgb(26, 22, 18);
/// Alternate rows in striped tables
const PANEL_STRIPE: Color32 = Color32::from_rgb(33, 28, 22);
const WIDGET: Color32 = Color32::from_rgb(42, 34, 26);
// Hover and press (a user-requested deviation from PoB, whose controls only turn their
// border white): a lighter fill, a light gold border and brighter text, a step further when
// pressed
const WIDGET_HOVERED: Color32 = Color32::from_rgb(76, 60, 40);
const WIDGET_ACTIVE: Color32 = Color32::from_rgb(100, 78, 48);
const BORDER_HOVERED: Color32 = Color32::from_rgb(236, 204, 134);
const BORDER_ACTIVE: Color32 = Color32::from_rgb(255, 228, 165);
const TEXT_HOVERED: Color32 = Color32::from_rgb(250, 241, 222);
const TEXT_ACTIVE: Color32 = Color32::from_rgb(255, 249, 236);
/// A hovered edit box's background, a step up from `BACKGROUND`
const EDIT_HOVERED: Color32 = Color32::from_rgb(36, 30, 23);
/// A hovered list row: a warm tint, darker than the selection's (110, 80, 35)
const ROW_HOVERED: Color32 = Color32::from_rgb(74, 54, 30);
const ROW_ACTIVE: Color32 = Color32::from_rgb(90, 66, 36);
const SELECTION: Color32 = Color32::from_rgb(110, 80, 35);
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

    install(ctx);
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Name(HEADING_FONT.into())),
        );
        style.spacing.button_padding = Vec2::new(8.0, 3.0);
    });
}

/// The look and behaviour every context of the app shares: the palette, the edit boxes' hover
/// look and the pointing hand over clickable widgets.
pub fn install(ctx: &egui::Context) {
    // the palette is dark only
    ctx.set_theme(ThemePreference::Dark);
    ctx.set_visuals_of(Theme::Dark, visuals());
    ctx.replace_widget_theme::<TextEditStyle>(EditBoxStyle);
    ctx.add_plugin(HandCursor);
}

/// egui's edit box look, with a lighter fill and brighter text under the pointer (egui keeps
/// both as they are). Read-only and focused boxes keep egui's look.
struct EditBoxStyle;

impl StyleProvider<TextEditStyle> for EditBoxStyle {
    fn style(&mut self, args: &StyleArgs<'_>) -> TextEditStyle {
        let mut style = StyleProvider::<TextEditStyle>::style(&mut DefaultStyle, args);
        if args.state == WidgetState::Hovered && !args.has_class(&egui::TextEdit::CLASS_READ_ONLY) {
            style.atom_layout.frame.fill = EDIT_HOVERED;
            style.atom_layout.text_style.color = TEXT_HOVERED;
        }
        style
    }
}

/// The tallest widget the pointing hand shows over: a button or a row, not a click-sensing
/// canvas or backdrop
const HAND_MAX_HEIGHT: f32 = 48.0;

/// Shows the pointing hand over a hovered, enabled, clickable widget that set no cursor of its
/// own (egui's `interact_cursor` reaches buttons only, not check boxes, radio buttons, dropdown
/// buttons or clickable labels). Edit boxes keep their text cursor; drag-sensing widgets (the
/// tree, sliders, draggable rows) keep theirs.
struct HandCursor;

impl egui::Plugin for HandCursor {
    fn debug_name(&self) -> &'static str {
        "auto-pob hand cursor"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        ui.ctx().data_mut(|d| d.remove::<egui::IdSet>(no_hand_id()));
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        hand_cursor(ui.ctx());
    }
}

/// Where this pass's [`no_hand`] widgets are kept
fn no_hand_id() -> egui::Id {
    egui::Id::unique("auto-pob no hand")
}

/// Keeps the pointing hand off `response`'s widget this pass: one that senses clicks for
/// something other than a left click (e.g. a right-click menu)
pub fn no_hand(ui: &egui::Ui, response: &egui::Response) {
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<egui::IdSet>(no_hand_id())
            .insert(response.id)
    });
}

fn hand_cursor(ctx: &egui::Context) {
    if ctx.output(|o| o.cursor_icon) != CursorIcon::Default {
        return;
    }
    let hovered = ctx.interaction_snapshot(|s| s.hovered.clone());
    let skipped: egui::IdSet = ctx.data(|d| d.get_temp(no_hand_id()).unwrap_or_default());
    let clickable = hovered.iter().any(|id| {
        !skipped.contains(id)
            && ctx.read_response(*id).is_some_and(|r| {
                r.enabled()
                    && r.hovered()
                    && r.sense.senses_click()
                    && !r.sense.senses_drag()
                    && r.rect.height() <= HAND_MAX_HEIGHT
            })
    });
    if clickable {
        ctx.set_cursor_icon(CursorIcon::PointingHand);
    }
}

/// Draws `add` with the look of a list row: a warm fill under the pointer and no border
/// (PoB's hovered dropdown row has a grey fill and no border, DropDownControl.lua:373-376; the
/// warm tint is a user-requested deviation). The border stays 1 px wide, unseen, so nothing
/// moves. A selected row keeps the selection's fill.
pub fn list_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let widgets = &mut ui.visuals_mut().widgets;
    let old = (widgets.hovered, widgets.active);
    for (state, fill) in [
        (&mut widgets.hovered, ROW_HOVERED),
        (&mut widgets.active, ROW_ACTIVE),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = ROW_BORDER;
    }
    let out = add(ui);
    let widgets = &mut ui.visuals_mut().widgets;
    (widgets.hovered, widgets.active) = old;
    out
}

/// A list row's border: none to see
const ROW_BORDER: Stroke = Stroke {
    width: 1.0,
    color: Color32::TRANSPARENT,
};

/// A list row drawn as a frame (the draggable lists): the selection's fill when `selected`,
/// [`list_row`]'s warm fill under the pointer, nothing otherwise. The frame's 2 px margin is
/// what the rows had before.
pub fn row_frame<R>(
    ui: &mut egui::Ui,
    selected: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut prepared = egui::Frame::NONE.inner_margin(2.0).begin(ui);
    let inner = add(&mut prepared.content_ui);
    let rect = prepared.content_ui.min_rect().expand(2.0);
    prepared.frame.fill = if selected {
        ui.visuals().selection.bg_fill
    } else if ui.is_enabled() && ui.rect_contains_pointer(rect) {
        ROW_HOVERED
    } else {
        Color32::TRANSPARENT
    };
    egui::InnerResponse::new(inner, prepared.end(ui))
}

// The main tab strip (a user-requested look, not PoB's): tabs on a dark band with a line under
// it; the selected tab takes the panel's fill and a gold line along its top, so it reads as
// joined to the content below
/// A main tab's height
pub const TAB_HEIGHT: f32 = 30.0;
/// Text to tab edge, each side: the roomiest that lets the strip fit the panel
const TAB_PADDINGS: [f32; 3] = [18.0, 12.0, 6.0];
/// Between two tabs
const TAB_GAP: f32 = 4.0;
/// The band above the tabs
const TAB_BAND_TOP: f32 = 5.0;
/// The tabs' text size; when even the smallest padding is too wide, the body text's size
const TAB_FONT_SIZE: f32 = 17.0;
/// The selected tab's gold line
pub const TAB_ACCENT: f32 = 2.0;
/// The band, darker than the panels
const TAB_BAND: Color32 = Color32::from_rgb(11, 9, 7);
/// An unselected tab: muted
const TAB: Color32 = Color32::from_rgb(34, 29, 23);
const TAB_TEXT: Color32 = Color32::from_rgb(165, 155, 138);
/// The central panel's margin (`Frame::central_panel`), which the band spans
const PANEL_MARGIN: f32 = 8.0;

/// The colours of a main tab: (fill, border, text). A tab with keyboard focus looks hovered;
/// the selected one keeps its fill and takes the hovered border.
fn tab_colours(selected: bool, response: &egui::Response) -> (Color32, Color32, Color32) {
    let focused = response.has_focus();
    if selected {
        let border = if focused {
            BORDER_HOVERED
        } else {
            Color32::TRANSPARENT
        };
        (PANEL, border, TEXT_HOVERED)
    } else if response.is_pointer_button_down_on() {
        (WIDGET_ACTIVE, BORDER_ACTIVE, TEXT_ACTIVE)
    } else if response.hovered() || focused {
        (WIDGET_HOVERED, BORDER_HOVERED, TEXT_HOVERED)
    } else {
        (TAB, Color32::TRANSPARENT, TAB_TEXT)
    }
}

/// The text size and padding that let `labels` fit `width` on one row, roomiest first; the
/// smallest of both when nothing fits (the strip then wraps)
fn tab_metrics(ui: &egui::Ui, labels: &[&str], width: f32) -> (FontId, f32) {
    // the heading font (Fontin small caps once `apply` ran)
    let family = TextStyle::Heading.resolve(ui.style()).family;
    let body = TextStyle::Body.resolve(ui.style()).size.min(TAB_FONT_SIZE);
    let gaps = TAB_GAP * labels.len().saturating_sub(1) as f32;
    let mut last = (FontId::new(body, family.clone()), TAB_PADDINGS[2]);
    for size in [TAB_FONT_SIZE, body] {
        let font = FontId::new(size, family.clone());
        let text: f32 = labels
            .iter()
            .map(|l| {
                ui.painter()
                    .layout_no_wrap((*l).to_owned(), font.clone(), Color32::PLACEHOLDER)
                    .size()
                    .x
            })
            .sum();
        for pad in TAB_PADDINGS {
            // a point to spare, so rounding doesn't wrap a strip that fits
            if text + 2.0 * pad * labels.len() as f32 + gaps <= width - 1.0 {
                return (font, pad);
            }
        }
        last = (font, TAB_PADDINGS[2]);
    }
    last
}

/// One main tab
fn main_tab(
    ui: &mut egui::Ui,
    selected: bool,
    text: &str,
    (font, pad): &(FontId, f32),
) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 2.0 * pad, TAB_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Tab, ui.is_enabled(), selected, text)
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let (fill, border, text_colour) = tab_colours(selected, &response);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
        if border != Color32::TRANSPARENT {
            painter.rect_stroke(
                rect,
                CornerRadius::ZERO,
                Stroke::new(1.0, border),
                egui::StrokeKind::Inside,
            );
        }
        if selected {
            let sides = Stroke::new(1.0, BRONZE);
            painter.vline(rect.left() + 0.5, rect.y_range(), sides);
            painter.vline(rect.right() - 0.5, rect.y_range(), sides);
            let accent = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TAB_ACCENT));
            painter.rect_filled(accent, CornerRadius::ZERO, GOLD);
        }
        let at = rect.center() - galley.size() / 2.0;
        painter.galley(at, galley, text_colour);
    }
    response
}

/// The main tab strip: `labels` as tabs on a dark band spanning the panel, `selected` drawn
/// joined to the content. The tabs shrink their padding, then their text, to fit the panel;
/// past that they wrap, the band and its line under the last row. The tabs' responses, in
/// order.
pub fn main_tabs(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Vec<egui::Response> {
    let clip = ui.clip_rect();
    let top = ui.cursor().top();
    let metrics = tab_metrics(ui, labels, ui.available_width());
    let band = ui.painter().add(egui::Shape::Noop);
    ui.add_space(TAB_BAND_TOP);
    let tabs = ui
        .horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(TAB_GAP);
            labels
                .iter()
                .enumerate()
                .map(|(i, label)| main_tab(ui, i == selected, label, &metrics))
                .collect::<Vec<_>>()
        })
        .inner;
    let bottom = tabs
        .iter()
        .map(|t| t.rect.bottom())
        .fold(top + TAB_BAND_TOP, f32::max);
    let band_rect = Rect::from_x_y_ranges(
        clip.x_range(),
        (top - PANEL_MARGIN).max(clip.top())..=bottom,
    );
    ui.painter().set(
        band,
        egui::epaint::RectShape::filled(band_rect, CornerRadius::ZERO, TAB_BAND),
    );
    // the line under the band, broken by the selected tab when it is on the last row
    let line = Stroke::new(1.0, BRONZE);
    let y = bottom - 0.5;
    match tabs.get(selected).filter(|t| t.rect.bottom() == bottom) {
        Some(tab) => {
            ui.painter()
                .hline(band_rect.left()..=tab.rect.left(), y, line);
            ui.painter()
                .hline(tab.rect.right()..=band_rect.right(), y, line);
        }
        None => {
            ui.painter().hline(band_rect.x_range(), y, line);
        }
    }
    ui.add_space(4.0);
    tabs
}

/// A gold Fontin heading.
pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.heading(RichText::new(text).color(HEADING));
}

/// The emphasised button for a panel's main action: PoB's unique colour, lighter with a light
/// gold border under the pointer and when pressed, like every button.
pub fn primary_button(text: &str) -> impl egui::Widget + use<> {
    let text = RichText::new(text).color(TEXT_BRIGHT).strong();
    move |ui: &mut egui::Ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        let old = (widgets.inactive, widgets.hovered, widgets.active);
        for (state, fill, border) in [
            (&mut widgets.inactive, UNIQUE.gamma_multiply(0.55), UNIQUE),
            (
                &mut widgets.hovered,
                UNIQUE.gamma_multiply(0.75),
                BORDER_HOVERED,
            ),
            (
                &mut widgets.active,
                UNIQUE.gamma_multiply(0.9),
                BORDER_ACTIVE,
            ),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, border);
        }
        let response = ui.add(egui::Button::new(text));
        let widgets = &mut ui.visuals_mut().widgets;
        (widgets.inactive, widgets.hovered, widgets.active) = old;
        response
    }
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
    visuals.selection.bg_fill = SELECTION;
    visuals.interact_cursor = Some(CursorIcon::PointingHand);
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
        // PoB's controls only turn their border white under the pointer (EditControl.lua:
        // 246-248, ButtonControl.lua:45-46, CheckBoxControl.lua:40-41); at the user's request
        // ours light up: a lighter fill, a light gold border, brighter text
        (
            &mut widgets.hovered,
            WIDGET_HOVERED,
            BORDER_HOVERED,
            TEXT_HOVERED,
        ),
        (
            &mut widgets.active,
            WIDGET_ACTIVE,
            BORDER_ACTIVE,
            TEXT_ACTIVE,
        ),
        // a dropped dropdown keeps the hovered look (PoB's border is white,
        // DropDownControl.lua:250, 274)
        (
            &mut widgets.open,
            WIDGET_HOVERED,
            BORDER_HOVERED,
            TEXT_HOVERED,
        ),
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

    /// A painted rectangle: where, its fill and its border's colour (transparent when none)
    #[derive(Debug, Clone, Copy)]
    struct Painted {
        rect: Rect,
        fill: Color32,
        border: Color32,
    }

    fn rects(shape: &egui::Shape, out: &mut Vec<Painted>) {
        match shape {
            egui::Shape::Rect(r) => out.push(Painted {
                rect: r.rect,
                fill: r.fill,
                border: if r.stroke.width > 0.0 {
                    r.stroke.color
                } else {
                    Color32::TRANSPARENT
                },
            }),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| rects(s, out)),
            _ => {}
        }
    }

    struct Frame {
        /// The rectangle `draw` returned
        rect: Rect,
        painted: Vec<Painted>,
        cursor: CursorIcon,
    }

    impl Frame {
        /// The rectangles painted around `rect` (the widget's own frame)
        fn around(&self, rect: Rect) -> Vec<Painted> {
            self.painted
                .iter()
                .copied()
                .filter(|p| (p.rect.center() - rect.center()).length() < 2.0)
                .collect()
        }

        fn has_border(&self, colour: Color32) -> bool {
            self.around(self.rect).iter().any(|p| p.border == colour)
        }

        fn has_fill(&self, colour: Color32) -> bool {
            self.around(self.rect).iter().any(|p| p.fill == colour)
        }
    }

    /// Runs `draw` in a themed context for `frames`, frame by frame; the last frame
    fn run(frames: Vec<Vec<Event>>, mut draw: impl FnMut(&mut egui::Ui) -> Rect) -> Frame {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut last = None;
        for events in frames {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 300.0))),
                events,
                ..Default::default()
            };
            let mut rect = Rect::NOTHING;
            let output = ctx.run_ui(input, |ui| rect = draw(ui));
            let mut painted = Vec::new();
            for clipped in &output.shapes {
                rects(&clipped.shape, &mut painted);
            }
            last = Some(Frame {
                rect,
                painted,
                cursor: output.platform_output.cursor_icon,
            });
            output.drop_without_applying_deltas();
        }
        last.unwrap()
    }

    const AT: Pos2 = Pos2::new(20.0, 12.0);

    /// Frames with the pointer away, or over `AT`
    fn frames(hover: bool) -> Vec<Vec<Event>> {
        let at = if hover { AT } else { Pos2::new(290.0, 290.0) };
        vec![vec![], vec![Event::PointerMoved(at)], vec![]]
    }

    fn press(pressed: bool) -> Event {
        Event::PointerButton {
            pos: AT,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A hovered button lights up: a lighter fill and a light gold border (a user-requested
    /// deviation from PoB's white border), a step further while pressed
    #[test]
    fn a_hovered_button_has_a_lighter_fill_and_a_gold_border() {
        let button = |ui: &mut egui::Ui| ui.button("a button").rect;
        let idle = run(frames(false), button);
        assert!(
            idle.has_fill(WIDGET) && idle.has_border(BRONZE),
            "{:?}",
            idle.painted
        );
        let hovered = run(frames(true), button);
        assert!(hovered.rect.contains(AT), "{:?}", hovered.rect);
        assert!(
            hovered.has_fill(WIDGET_HOVERED) && hovered.has_border(BORDER_HOVERED),
            "{:?}",
            hovered.painted
        );
        assert_ne!(WIDGET_HOVERED, WIDGET);
        assert_ne!(BORDER_HOVERED, Color32::WHITE);
        let mut pressing = frames(true);
        pressing.push(vec![press(true)]);
        pressing.push(vec![]);
        let pressed = run(pressing, button);
        assert!(
            pressed.has_fill(WIDGET_ACTIVE) && pressed.has_border(BORDER_ACTIVE),
            "{:?}",
            pressed.painted
        );
    }

    /// Every edit box lights up under the pointer too: a lighter fill and the gold border
    #[test]
    fn a_hovered_edit_box_has_a_lighter_fill_and_a_gold_border() {
        let edit = |ui: &mut egui::Ui| {
            let mut text = String::from("abc");
            ui.add(egui::TextEdit::singleline(&mut text)).rect
        };
        let idle = run(frames(false), edit);
        assert!(idle.has_fill(BACKGROUND) && !idle.has_border(BORDER_HOVERED));
        let hovered = run(frames(true), edit);
        assert!(hovered.rect.contains(AT), "{:?}", hovered.rect);
        assert!(
            hovered.has_fill(EDIT_HOVERED) && hovered.has_border(BORDER_HOVERED),
            "{:?}",
            hovered.painted
        );
        // the text cursor stays
        assert_eq!(hovered.cursor, CursorIcon::Text);
    }

    /// A hovered row of a list or a dropped list gets a warm fill and no box; a selected row
    /// keeps the selection's fill, a different colour
    #[test]
    fn a_hovered_list_row_has_a_warm_fill_and_no_border() {
        let row = |selected| {
            move |ui: &mut egui::Ui| list_row(ui, |ui| ui.selectable_label(selected, "a row")).rect
        };
        let hovered = run(frames(true), row(false));
        assert!(hovered.rect.contains(AT), "{:?}", hovered.rect);
        assert!(hovered.has_fill(ROW_HOVERED), "{:?}", hovered.painted);
        assert!(
            hovered
                .around(hovered.rect)
                .iter()
                .all(|p| p.border == Color32::TRANSPARENT),
            "{:?}",
            hovered.painted
        );
        let selected = run(frames(true), row(true));
        assert!(selected.has_fill(SELECTION) && !selected.has_fill(ROW_HOVERED));
        assert_ne!(ROW_HOVERED, SELECTION);
        // outside a list a selectable label is a button: it gets the gold border
        let tab = run(frames(true), |ui| ui.selectable_label(false, "a tab").rect);
        assert!(tab.has_border(BORDER_HOVERED), "{:?}", tab.painted);
    }

    /// A framed row (the draggable lists) gets the same warm fill under the pointer
    #[test]
    fn a_hovered_framed_row_has_the_warm_fill() {
        let row = |ui: &mut egui::Ui| {
            row_frame(ui, false, |ui| {
                ui.set_min_width(200.0);
                ui.label("a row");
            })
            .response
            .rect
        };
        assert!(!run(frames(false), row).has_fill(ROW_HOVERED));
        let hovered = run(frames(true), row);
        assert!(hovered.has_fill(ROW_HOVERED), "{:?}", hovered.painted);
    }

    /// A disabled widget looks the same under the pointer and shows no hand
    #[test]
    fn a_disabled_widget_does_not_change_on_hover() {
        let disabled =
            |ui: &mut egui::Ui| ui.add_enabled(false, egui::Button::new("a button")).rect;
        let idle = run(frames(false), disabled);
        let hovered = run(frames(true), disabled);
        assert!(hovered.rect.contains(AT), "{:?}", hovered.rect);
        let looks = |f: &Frame| {
            f.around(f.rect)
                .iter()
                .map(|p| (p.fill, p.border))
                .collect::<Vec<_>>()
        };
        assert_eq!(looks(&idle), looks(&hovered));
        assert!(!hovered.has_border(BORDER_HOVERED));
        assert_eq!(hovered.cursor, CursorIcon::Default);
        let primary = |ui: &mut egui::Ui| ui.add_enabled(false, primary_button("Go")).rect;
        assert_eq!(
            looks(&run(frames(false), primary)),
            looks(&run(frames(true), primary))
        );
    }

    /// The main action's button lights up under the pointer like any other
    #[test]
    fn a_hovered_primary_button_lights_up() {
        let primary = |ui: &mut egui::Ui| ui.add(primary_button("Go")).rect;
        let idle = run(frames(false), primary);
        let hovered = run(frames(true), primary);
        assert!(idle.has_border(UNIQUE), "{:?}", idle.painted);
        assert!(hovered.has_border(BORDER_HOVERED), "{:?}", hovered.painted);
        assert!(!hovered.has_fill(UNIQUE.gamma_multiply(0.55)));
    }

    /// Clickable widgets show the pointing hand: buttons, and check boxes, which egui's own
    /// setting doesn't reach
    #[test]
    fn clickable_widgets_show_the_pointing_hand() {
        let hovered = run(frames(true), |ui| ui.button("a button").rect);
        assert_eq!(hovered.cursor, CursorIcon::PointingHand);
        let hovered = run(frames(true), |ui| {
            let mut on = false;
            ui.checkbox(&mut on, "a check box").rect
        });
        assert!(hovered.rect.contains(AT), "{:?}", hovered.rect);
        assert_eq!(hovered.cursor, CursorIcon::PointingHand);
        let away = run(frames(false), |ui| ui.button("a button").rect);
        assert_eq!(away.cursor, CursorIcon::Default);
        // a large click-sensing area (a canvas, a backdrop) keeps the arrow
        let canvas = run(frames(true), |ui| {
            ui.allocate_exact_size(Vec2::splat(200.0), egui::Sense::click())
                .1
                .rect
        });
        assert_eq!(canvas.cursor, CursorIcon::Default);
    }

    /// A widget opted out with `no_hand` keeps the arrow though it senses clicks
    #[test]
    fn a_no_hand_widget_keeps_the_arrow() {
        let label = |opt_out: bool| {
            move |ui: &mut egui::Ui| {
                let r = ui.add(
                    egui::Label::new("Helmet:")
                        .selectable(false)
                        .sense(egui::Sense::click()),
                );
                if opt_out {
                    no_hand(ui, &r);
                }
                r.rect
            }
        };
        let hand = run(frames(true), label(false));
        assert!(hand.rect.contains(AT), "{:?}", hand.rect);
        assert_eq!(hand.cursor, CursorIcon::PointingHand);
        assert_eq!(run(frames(true), label(true)).cursor, CursorIcon::Default);
    }

    /// A dropped dropdown keeps the gold border (PoB's is white)
    #[test]
    fn a_dropped_dropdown_has_a_gold_border() {
        let frames = vec![
            vec![],
            vec![Event::PointerMoved(AT)],
            vec![press(true)],
            vec![press(false)],
            vec![Event::PointerMoved(Pos2::new(250.0, 250.0))],
            vec![],
        ];
        let open = run(frames, |ui| {
            egui::ComboBox::from_id_salt("dd")
                .selected_text("one")
                .show_ui(ui, |ui| {
                    ui.label("two");
                })
                .response
                .rect
        });
        assert!(
            open.has_border(BORDER_HOVERED),
            "{open:?}",
            open = open.painted
        );
    }
}
