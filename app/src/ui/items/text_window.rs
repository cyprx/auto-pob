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
            ui.heading(if window.creating {
                "Create custom item"
            } else {
                "Edit item text"
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
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("items_text_rarity")
                .selected_text(RARITIES[window.rarity.min(4)])
                .show_ui(ui, |ui| {
                    for (i, name) in RARITIES.iter().enumerate() {
                        let item =
                            crate::widgets::dropdown::value(ui, &mut window.rarity, i, *name);
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
    crate::widgets::escape::note(ctx, escape_id);
    if !open {
        outcome = Outcome::Close;
    }
    outcome
}
