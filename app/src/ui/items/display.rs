//! The display panel: PoB's editors for the item being edited, and its tooltip.

use super::text_window::{self, Outcome, TextWindow};
use crate::engine::views::{DisplayItemView, DropList};
use crate::engine::{Command, SocketColour};
use crate::ui::tree::Rects;
use crate::widgets::{EditBox, EditBoxOptions, item_tooltip_ui_with, pob_text};
use eframe::egui;

/// PoB's quality boxes hold two characters.
const QUALITY_DIGITS: usize = 2;

/// PoB's quality boxes take digits only (filter `%D`).
fn quality_options() -> EditBoxOptions {
    EditBoxOptions {
        width: 40.0,
        char_limit: Some(QUALITY_DIGITS),
        filter: Some(|c| c.is_ascii_digit()),
        hint: None,
    }
}

#[derive(Default)]
pub struct DisplayState {
    pub(super) quality: EditBox,
    pub(super) catalyst_quality: EditBox,
    /// The range line the slider is for
    pub range_line: usize,
    /// A slider value shown until the engine's view catches up
    slider: Option<(usize, f64)>,
    pub text_window: Option<TextWindow>,
    /// The window's text went to the engine; the next view closes it
    pub(super) submitted: Option<u64>,
    pub(super) rects: Rects,
}

impl DisplayState {
    /// Typed edits waiting for Enter or blur, to go out first.
    pub fn take_pending(&mut self) -> Vec<Command> {
        let mut commands = Vec::new();
        if let Some(text) = self.quality.take_pending() {
            commands.push(Command::SetItemQuality { text });
        }
        if let Some(text) = self.catalyst_quality.take_pending() {
            commands.push(Command::SetItemCatalystQuality { text });
        }
        commands
    }

    /// Forgets the typed edits waiting for Enter or blur: the item they were for is going.
    fn discard_pending(&mut self) {
        self.quality.discard();
        self.catalyst_quality.discard();
    }

    /// A new view of the display item arrived.
    pub(super) fn on_view(&mut self) {
        self.slider = None;
    }

    /// There is no item (any more) to edit.
    pub(super) fn on_gone(&mut self) {
        self.discard_pending();
        self.slider = None;
        self.range_line = 0;
    }

    /// The raw-text window, drawn whether or not an item is displayed.
    pub fn text_window(&mut self, ctx: &egui::Context) -> Vec<Command> {
        let Some(window) = &mut self.text_window else {
            return Vec::new();
        };
        match text_window::show(ctx, window, &mut self.rects) {
            Outcome::Open => Vec::new(),
            Outcome::Save(command) => {
                // the app tells which seq it got (`ItemsUi::text_sent`)
                self.submitted = Some(u64::MAX);
                vec![command]
            }
            Outcome::Close => {
                self.text_window = None;
                self.submitted = None;
                Vec::new()
            }
        }
    }
}

fn dropdown(ui: &mut egui::Ui, rects: &mut Rects, key: &str, list: &DropList) -> Option<usize> {
    let default = ui.visuals().text_color();
    let current = list.labels.get(list.selected).map_or("", String::as_str);
    let mut picked = None;
    let combo = ui.add_enabled_ui(list.enabled, |ui| {
        egui::ComboBox::from_id_salt(("items_display", key))
            .selected_text(pob_text(current, 14.0, default))
            .show_ui(ui, |ui| {
                for (i, label) in list.labels.iter().enumerate() {
                    let item =
                        ui.selectable_label(i == list.selected, pob_text(label, 14.0, default));
                    rects.note(&item, || format!("{key}_{i}"));
                    if item.clicked() && i != list.selected {
                        picked = Some(i);
                    }
                }
            })
    });
    rects.note(&combo.inner.response, || key.to_owned());
    picked
}

fn colour(name: &str) -> Option<SocketColour> {
    match name {
        "R" => Some(SocketColour::R),
        "G" => Some(SocketColour::G),
        "B" => Some(SocketColour::B),
        "W" => Some(SocketColour::W),
        _ => None,
    }
}

/// The buttons when nothing is displayed.
pub fn show_empty(ui: &mut egui::Ui, state: &mut DisplayState) {
    // PoB's popup is modal: nothing else on the panel works while the window is open
    let enabled = state.text_window.is_none();
    ui.add_enabled_ui(enabled, |ui| show_empty_inner(ui, state));
}

fn show_empty_inner(ui: &mut egui::Ui, state: &mut DisplayState) {
    ui.horizontal(|ui| {
        let create = ui.button("Create custom\u{2026}");
        state.rects.note(&create, || "create".into());
        if create.clicked() {
            state.text_window = Some(TextWindow::create());
            state.submitted = None;
        }
    });
    ui.weak("Paste an item (Ctrl+V) or double-click one in the list.");
}

/// Draws the panel for the item; returns the commands.
/// `busy`: the engine hasn't caught up with what was sent.
pub fn show(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut DisplayState,
    busy: bool,
) -> Vec<Command> {
    // PoB's popup is modal: no editor works while the raw-text window is open
    let enabled = state.text_window.is_none();
    ui.add_enabled_ui(enabled, |ui| panel(ui, view, state, busy))
        .inner
}

fn panel(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut DisplayState,
    busy: bool,
) -> Vec<Command> {
    let mut out = Vec::new();
    ui.horizontal_wrapped(|ui| {
        // a Save on an item the engine is still deleting would orphan it
        let save = ui.add_enabled(!busy, egui::Button::new(&view.save_label));
        state.rects.note(&save, || "save".into());
        if save.clicked() {
            // a box still being typed in goes first
            out.extend(state.take_pending());
            out.push(Command::SaveDisplayItem);
        }
        let edit = ui.button("Edit\u{2026}");
        state.rects.note(&edit, || "edit".into());
        if edit.clicked() {
            // the window opens on the view's text, which lacks a box still being typed in: that
            // edit goes first and the click is refused, so the next one opens the new text
            let pending = state.take_pending();
            if pending.is_empty() {
                state.text_window = Some(TextWindow {
                    rarity: view.edit_rarity,
                    text: view.edit_text.clone(),
                    creating: false,
                });
                state.submitted = None;
            } else {
                out.extend(pending);
            }
        }
        let cancel = ui.button("Cancel");
        state.rects.note(&cancel, || "cancel".into());
        if cancel.clicked() {
            // the item is going: an edit still being typed must not follow the cancel
            state.discard_pending();
            out.push(Command::CancelDisplayItem { if_id: None });
        }
    });

    // variants
    if view.version.is_some() || !view.variants.is_empty() {
        ui.horizontal_wrapped(|ui| {
            if let Some(version) = &view.version {
                ui.label("Version:");
                if let Some(index) = dropdown(ui, &mut state.rects, "version", version) {
                    out.push(Command::SetItemVersion { index });
                }
            }
            for v in &view.variants {
                // a combo may not wrap by itself, so each gets its own line
                ui.end_row();
                if let Some(index) = dropdown(
                    ui,
                    &mut state.rects,
                    &format!("variant_{}", v.control),
                    &v.list,
                ) {
                    out.push(Command::SetItemVariant {
                        control: v.control,
                        index,
                    });
                }
            }
        });
    }

    // sockets
    let shown: Vec<usize> = (0..view.sockets.len())
        .filter(|&i| view.sockets[i].shown)
        .collect();
    if !shown.is_empty() || view.can_add_socket {
        ui.horizontal_wrapped(|ui| {
            ui.label("Sockets:");
            for &i in &shown {
                let socket = &view.sockets[i];
                let current = colour(&socket.colour);
                let combo = egui::ComboBox::from_id_salt(("items_socket", i))
                    .width(40.0)
                    .selected_text(&socket.colour)
                    .show_ui(ui, |ui| {
                        for (name, c) in [
                            ("R", SocketColour::R),
                            ("G", SocketColour::G),
                            ("B", SocketColour::B),
                            ("W", SocketColour::W),
                        ] {
                            let item = ui.selectable_label(current == Some(c), name);
                            state.rects.note(&item, || format!("socket_{i}_{name}"));
                            if item.clicked() && current != Some(c) {
                                out.push(Command::SetItemSocket {
                                    index: i,
                                    colour: c,
                                });
                            }
                        }
                    });
                state.rects.note(&combo.response, || format!("socket_{i}"));
                if let Some(linked) = socket.linked {
                    let mut on = linked;
                    let link = ui.checkbox(&mut on, "");
                    state.rects.note(&link, || format!("link_{i}"));
                    if link.changed() {
                        out.push(Command::SetItemSocketLink {
                            index: i,
                            linked: on,
                        });
                    }
                }
            }
            if view.can_add_socket {
                let add = ui.button("+");
                state.rects.note(&add, || "add_socket".into());
                if add.clicked() {
                    out.push(Command::AddItemSocket);
                }
            }
        });
    }

    // item properties
    if !view.influences.is_empty()
        || view.quality.is_some()
        || view.catalyst.is_some()
        || view.catalyst_quality.is_some()
    {
        ui.horizontal_wrapped(|ui| {
            for (which, list) in view.influences.iter().enumerate() {
                if let Some(index) =
                    dropdown(ui, &mut state.rects, &format!("influence_{which}"), list)
                {
                    out.push(Command::SetItemInfluence { which, index });
                }
            }
            ui.end_row();
            if let Some(quality) = &view.quality {
                ui.label("Quality:");
                let id = ui.scope_id().with("items_box_quality");
                let (response, sent) =
                    state
                        .quality
                        .show(ui, id, quality, busy, &quality_options());
                state.rects.note(&response, || "quality".into());
                if let Some(text) = sent {
                    out.push(Command::SetItemQuality { text });
                }
            } else {
                // the item has no such box: an edit for another item must not follow
                state.quality.discard();
            }
            ui.end_row();
            if let Some(catalyst) = &view.catalyst {
                ui.label("Catalyst:");
                if let Some(index) = dropdown(ui, &mut state.rects, "catalyst", catalyst) {
                    out.push(Command::SetItemCatalyst { index });
                }
            }
            ui.end_row();
            if let Some(quality) = &view.catalyst_quality {
                ui.label("Catalyst quality:");
                let id = ui.scope_id().with("items_box_catalyst_quality");
                let (response, sent) =
                    state
                        .catalyst_quality
                        .show(ui, id, quality, busy, &quality_options());
                state.rects.note(&response, || "catalyst_quality".into());
                if let Some(text) = sent {
                    out.push(Command::SetItemCatalystQuality { text });
                }
            } else {
                state.catalyst_quality.discard();
            }
        });
    } else {
        state.discard_pending();
    }

    // range lines
    if !view.range_lines.is_empty() {
        let default = ui.visuals().text_color();
        state.range_line = state.range_line.min(view.range_lines.len() - 1);
        let line = state.range_line;
        ui.horizontal_wrapped(|ui| {
            ui.label("Range:");
            let current = &view.range_lines[line];
            let combo = egui::ComboBox::from_id_salt("items_range_line")
                .width(260.0)
                .selected_text(pob_text(&current.label, 14.0, default))
                .show_ui(ui, |ui| {
                    for (i, l) in view.range_lines.iter().enumerate() {
                        let item =
                            ui.selectable_label(i == line, pob_text(&l.label, 14.0, default));
                        state.rects.note(&item, || format!("range_line_{i}"));
                        if item.clicked() {
                            state.range_line = i;
                            state.slider = None;
                        }
                    }
                });
            state.rects.note(&combo.response, || "range_lines".into());
        });
        let current = &view.range_lines[state.range_line];
        let line = state.range_line;
        ui.horizontal_wrapped(|ui| {
            if current.slider
                && let Some(range) = current.range
            {
                let mut value = match state.slider {
                    Some((l, v)) if l == line => v,
                    _ => range,
                };
                let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
                state.rects.note(&slider, || "range_slider".into());
                if slider.dragged() || slider.changed() {
                    state.slider = Some((line, value));
                }
                if slider.drag_stopped()
                    || (slider.changed() && !slider.is_pointer_button_down_on())
                {
                    out.push(Command::SetRangeLine { line, value });
                }
            }
            if current.mutable {
                let mut on = current.mutated;
                let mutated = ui.checkbox(&mut on, "Foulborn");
                state.rects.note(&mutated, || "range_mutated".into());
                if mutated.changed() {
                    out.push(Command::SetRangeLineMutated { line, on });
                }
            }
        });
    }

    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("items_display_scroll")
        .auto_shrink(false)
        .show(ui, |ui| {
            let rects = &mut state.rects;
            let mut note = |i: usize, r: &egui::Response| rects.note(r, || format!("tip_line_{i}"));
            if let Some(line) = item_tooltip_ui_with(ui, &view.tooltip, &mut note) {
                out.push(Command::ToggleModLine { line });
            }
        });
    out
}
