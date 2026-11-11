//! The raw-text window: PoB's "Edit..." / "Create custom..." popup.

use crate::engine::Command;
use crate::ui::tree::Rects;
use eframe::egui;

/// PoB's rarity dropdown for the popup.
pub const RARITIES: [&str; 5] = ["Normal", "Magic", "Rare", "Unique", "Relic"];

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

pub enum Outcome {
    Open,
    Save(Command),
    Close,
}

pub fn show(ctx: &egui::Context, window: &mut TextWindow, rects: &mut Rects) -> Outcome {
    let mut outcome = Outcome::Open;
    let mut open = true;
    egui::Window::new(if window.creating {
        "Create custom item"
    } else {
        "Edit item text"
    })
    .open(&mut open)
    .collapsible(false)
    .default_width(520.0)
    .show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Rarity:");
            let combo = egui::ComboBox::from_id_salt("items_text_rarity")
                .selected_text(RARITIES[window.rarity.min(4)])
                .show_ui(ui, |ui| {
                    for (i, name) in RARITIES.iter().enumerate() {
                        let item = ui.selectable_value(&mut window.rarity, i, *name);
                        rects.note(&item, || format!("text_rarity_{i}"));
                    }
                });
            rects.note(&combo.response, || "text_rarity".into());
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
            let save = ui.button("Save");
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
    if !open {
        outcome = Outcome::Close;
    }
    outcome
}
