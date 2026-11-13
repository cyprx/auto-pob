//! The raw-text window: PoB's "Edit..." / "Create custom..." popup.

use crate::engine::Command;
use crate::engine::views::{ItemTooltip, ItemTooltipLine};
use crate::ui::tree::Rects;
use crate::widgets::{item_tooltip_ui, pob_text};
use eframe::egui;

/// PoB's rarity dropdown for the popup.
pub const RARITIES: [&str; 5] = ["Normal", "Magic", "Rare", "Unique", "Relic"];

/// The dropdown's entries in their rarity colours (ItemsTab.lua:20-26)
const RARITY_LABELS: [&str; 5] = [
    "^xC8C8C8Normal",
    "^x8888FFMagic",
    "^xFFFF77Rare",
    "^xAF6025Unique",
    "^x60C060Relic",
];

/// What the window is editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextWindow {
    /// 0 Normal ... 4 Relic
    pub rarity: usize,
    pub text: String,
    /// "Create custom...": there is no item yet
    pub creating: bool,
}

impl TextWindow {
    /// As `EditDisplayItemText()` opens with no display item: Rare and empty.
    pub fn create() -> Self {
        Self {
            rarity: 2,
            text: String::new(),
            creating: true,
        }
    }
}

/// PoB's verdict on the window's text: Save's tooltip, whose "The item is invalid." first
/// line means Save is disabled (ItemsTab.lua:2630-2652). None until the engine answered for
/// this very text: Save stays enabled, and the engine refuses an invalid item.
pub struct SaveVerdict<'a> {
    pub tooltip: Option<&'a ItemTooltip>,
}

/// PoB's first line for a text it can't read as an item
const INVALID: &str = "The item is invalid.";

impl SaveVerdict<'_> {
    fn enabled(&self) -> bool {
        !self.tooltip.is_some_and(|t| {
            matches!(t.lines.first(), Some(ItemTooltipLine::Line { text, .. }) if text == INVALID)
        })
    }
}

pub enum Outcome {
    Open,
    Save(Command),
    Close,
}

pub fn show(
    ctx: &egui::Context,
    window: &mut TextWindow,
    rects: &mut Rects,
    verdict: &SaveVerdict<'_>,
) -> Outcome {
    let mut outcome = Outcome::Open;
    let mut open = true;
    // PoB's popup closes on Escape (it has no escape control, PopupDialog.lua:72-77), but a
    // selected box takes the first Esc, and an outside click does nothing
    let escape_id = egui::Id::unique("items_text_window");
    if crate::widgets::escape::escape(ctx, escape_id) {
        open = false;
    }
    // modal, as PoB's popup: nothing behind it takes the mouse while it is open (R34)
    egui::Modal::new(egui::Id::unique("items_text_window")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.horizontal(|ui| {
            // PoB's titles (ItemsTab.lua:2658)
            ui.heading(if window.creating {
                "Create Custom Item from Text"
            } else {
                "Edit Item Text"
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let x = ui.button("\u{d7}");
                rects.note(&x, || "text_close".into());
                if x.clicked() {
                    open = false;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label("Rarity:");
            let default = ui.visuals().text_color();
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("items_text_rarity")
                .selected_text(pob_text(RARITY_LABELS[window.rarity.min(4)], 14.0, default))
                .show_ui(ui, |ui| {
                    for (i, label) in RARITY_LABELS.iter().enumerate() {
                        let item = crate::widgets::dropdown::value(
                            ui,
                            &mut window.rarity,
                            i,
                            pob_text(label, 14.0, default),
                        );
                        rects.note(&item, || format!("text_rarity_{i}"));
                    }
                });
            rects.note(&combo.response, || "text_rarity".into());
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            let at = window.rarity.min(RARITIES.len() - 1);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, RARITIES.len(), at)
            {
                window.rarity = i;
            }
        });
        let edit = ui.add(
            egui::TextEdit::multiline(&mut window.text)
                .id_salt("items_text_edit")
                .font(egui::TextStyle::Monospace)
                .desired_rows(18)
                .desired_width(f32::INFINITY),
        );
        rects.note(&edit, || "text_edit".into());
        ui.horizontal(|ui| {
            // "Create" when there is no item yet; enabled while PoB reads the text as an item,
            // its tooltip the item's or PoB's hint (ItemsTab.lua:2626-2652)
            let label = if window.creating { "Create" } else { "Save" };
            let mut save = ui.add_enabled(verdict.enabled(), egui::Button::new(label));
            if let Some(tooltip) = verdict.tooltip {
                save = save
                    .on_hover_ui(|ui| {
                        item_tooltip_ui(ui, tooltip);
                    })
                    .on_disabled_hover_ui(|ui| {
                        item_tooltip_ui(ui, tooltip);
                    });
            }
            rects.note(&save, || "text_save".into());
            if save.clicked() {
                outcome = Outcome::Save(Command::SetDisplayItemText {
                    rarity: window.rarity,
                    text: window.text.clone(),
                });
            }
            let cancel = ui.button("Cancel");
            rects.note(&cancel, || "text_cancel".into());
            if cancel.clicked() {
                outcome = Outcome::Close;
            }
        });
    });
    crate::widgets::escape::note(ctx, escape_id);
    if !open {
        outcome = Outcome::Close;
    }
    outcome
}
