//! The display panel: PoB's editors for the item being edited, and its tooltip.

use super::crafting::{self, CraftingState};
use super::text_window::{self, Outcome, TextWindow};
use crate::engine::views::ControlTooltipView;
use crate::engine::views::{DisplayItemView, DropList};
use crate::engine::{BuySource, Command, ControlTooltipQuery, PopupKind, SocketColour};
use crate::ui::tree::Rects;
use crate::widgets::{EditBox, EditBoxOptions, item_tooltip_ui_with, pob_text};
use eframe::egui;
use std::sync::Arc;

/// The raw-text window asks about its text once it stops changing this long
const TEXT_PAUSE: f64 = 0.25;

/// PoB's quality boxes hold two characters.
const QUALITY_DIGITS: usize = 2;

/// PoB's quality boxes take digits only (filter `%D`).
fn quality_options() -> EditBoxOptions {
    EditBoxOptions::number(40.0, Some(QUALITY_DIGITS))
}

#[derive(Default)]
pub struct DisplayState {
    pub(super) quality: EditBox,
    pub(super) catalyst_quality: EditBox,
    /// The range line the slider is for
    pub range_line: usize,
    /// PoB's count of range line list rebuilds last seen: each one puts the dropdown back on
    /// the first line (UpdateDisplayItemRangeLines, ItemsTab.lua:2385-2396)
    range_resets: Option<u32>,
    /// A slider value shown until the engine's view catches up
    slider: Option<(usize, f64)>,
    pub text_window: Option<TextWindow>,
    /// The window's text went to the engine; the next view closes it
    pub(super) submitted: Option<u64>,
    pub(super) rects: Rects,
    pub(super) crafting: CraftingState,
    /// The raw-text window's question about its text, the text as it was when it last
    /// changed, and when; the answer for the question (Save's verdict and tooltip)
    pub(super) text_asked: Option<ControlTooltipQuery>,
    text_seen: Option<(ControlTooltipQuery, f64)>,
    text_verdict: Option<Arc<ControlTooltipView>>,
    /// Questions for the engine from the last frame
    pub(super) queries: Vec<Command>,
    /// A text box had the keyboard or a dropdown list was open as the frame began: PoB's
    /// selected control takes the press then, so a mod line under it doesn't toggle
    /// (ControlHost.lua:4-8, ItemsTab.lua:1487, 1521-1539)
    pub(super) control_selected: bool,
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
    pub(super) fn on_view(&mut self, view: Option<&DisplayItemView>) {
        let resets = view.map(|v| v.range_resets);
        if resets != self.range_resets {
            self.range_line = 0;
            self.range_resets = resets;
        }
        self.slider = None;
        self.crafting.on_view();
    }

    /// There is no item (any more) to edit.
    pub(super) fn on_gone(&mut self) {
        self.discard_pending();
        self.slider = None;
        self.range_line = 0;
        self.crafting.on_gone();
    }

    /// The engine's answer about the raw-text window's text, if it is for the latest question
    pub(super) fn on_control_tooltip(&mut self, view: &Arc<ControlTooltipView>) {
        if self.text_asked.as_ref() == Some(&view.query) {
            self.text_verdict = Some(view.clone());
        }
    }

    /// Asks the engine about the window's text once it stops changing for a moment; the first
    /// text of a window at once
    fn ask_about_text(&mut self, ctx: &egui::Context, query: ControlTooltipQuery) {
        if self.text_asked.as_ref() == Some(&query) {
            return;
        }
        let now = ctx.input(|i| i.time);
        let since = match &self.text_seen {
            Some((seen, since)) if *seen == query => *since,
            _ => {
                self.text_seen = Some((query.clone(), now));
                now
            }
        };
        if self.text_asked.is_none() || now - since >= TEXT_PAUSE {
            self.text_asked = Some(query.clone());
            self.queries.push(Command::ControlTooltip(query));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// The raw-text window, drawn whether or not an item is displayed.
    pub fn text_window(&mut self, ctx: &egui::Context) -> Vec<Command> {
        let Some(window) = &self.text_window else {
            self.text_asked = None;
            self.text_seen = None;
            self.text_verdict = None;
            return Vec::new();
        };
        let query = ControlTooltipQuery::ItemText {
            rarity: window.rarity,
            text: window.text.clone(),
        };
        self.ask_about_text(ctx, query.clone());
        let verdict = self.text_verdict.clone().filter(|v| v.query == query);
        let Some(window) = &mut self.text_window else {
            return Vec::new();
        };
        let save = text_window::SaveVerdict {
            tooltip: verdict.as_ref().map(|v| &v.tooltip),
        };
        match text_window::show(ctx, window, &mut self.rects, &save) {
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
    dropdown_with(ui, rects, key, list).0
}

/// As `dropdown`, with the dropdown's button
fn dropdown_with(
    ui: &mut egui::Ui,
    rects: &mut Rects,
    key: &str,
    list: &DropList,
) -> (Option<usize>, egui::Response) {
    let default = ui.visuals().text_color();
    let current = list.labels.get(list.selected).map_or("", String::as_str);
    let mut picked = None;
    let combo = ui.add_enabled_ui(list.enabled, |ui| {
        crate::widgets::dropdown::Dropdown::from_id_salt(("items_display", key))
            .selected_text(pob_text(current, 14.0, default))
            .show_ui(ui, |ui| {
                for (i, label) in list.labels.iter().enumerate() {
                    let item = crate::widgets::dropdown::item(
                        ui,
                        i == list.selected,
                        pob_text(label, 14.0, default),
                    );
                    rects.note(&item, || format!("{key}_{i}"));
                    if item.clicked() && i != list.selected {
                        picked = Some(i);
                    }
                }
            })
    });
    rects.note(&combo.inner.response, || key.to_owned());
    // the wheel and the arrows step it (DropDownControl.lua:476-516)
    let picked = picked.or_else(|| {
        crate::widgets::scroll_step::combo_step(
            ui,
            &combo.inner.response,
            list.labels.len(),
            list.selected,
        )
    });
    (picked, combo.inner.response)
}

/// A socket's entry as PoB's list colours it: STRENGTH, DEXTERITY, INTELLIGENCE, SCION
/// (ItemsTab.lua:28-33, Global.lua colorCodes)
pub(super) fn socket_label(name: &str) -> String {
    let code = match name {
        "R" => "^xE05030",
        "G" => "^x70FF70",
        "B" => "^x7070FF",
        "W" => "^xFFF0F0",
        _ => "",
    };
    format!("{code}{name}")
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

/// PoB's hint under the buttons while nothing is displayed (ItemsTab.lua:311-322)
const DISPLAY_TIP: &str = "^7Double-click an item from one of the lists,
or copy and paste an item from in game
(hover over the item and Ctrl+C) to view or edit
the item and add it to your build. You can
also clone an item within Path of Building by
copying and pasting it with Ctrl+C and Ctrl+V.

You can Control + Click an item to equip it, or
drag it onto the slot.  This will also add it to
your build if it's from the unique/template list.
If there's 2 slots an item can go in,
holding Shift will put it in the second.";

/// PoB's tooltip for the influence dropdowns (ItemsTab.lua:603)
const INFLUENCE_TIP: &str = "Selecting an influence here will also allow the modifier dropdowns to contain influenced mods.";

/// The buttons when nothing is displayed.
pub fn show_empty(ui: &mut egui::Ui, state: &mut DisplayState) -> Vec<Command> {
    // PoB's popup is modal: nothing else on the panel works while the window is open
    let enabled = state.text_window.is_none();
    ui.add_enabled_ui(enabled, |ui| show_empty_inner(ui, state))
        .inner
}

fn show_empty_inner(ui: &mut egui::Ui, state: &mut DisplayState) -> Vec<Command> {
    let mut out = Vec::new();
    // "Craft item..." then "Create custom..." (ItemsTab.lua:301-310)
    ui.horizontal(|ui| {
        out.extend(crafting::craft_button(ui, &mut state.crafting));
        let create = ui.button("Create custom...");
        state.rects.note(&create, || "create".into());
        if create.clicked() {
            state.text_window = Some(TextWindow::create());
            state.submitted = None;
        }
    });
    let default = ui.visuals().text_color();
    let tip = ui.label(pob_text(DISPLAY_TIP, 14.0, default));
    state.rects.note(&tip, || "display_tip".into());
    out
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
        // PoB's trade search window for the item (R10); the app sends a box still being typed
        // in before any window opens
        let buy = ui.button("Buy similar");
        state.rects.note(&buy, || "buy_similar".into());
        if buy.clicked() {
            out.push(Command::OpenPopup(PopupKind::BuySimilar {
                source: BuySource::DisplayItem,
            }));
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
                let default = ui.visuals().text_color();
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt(("items_socket", i))
                    .width(40.0)
                    .selected_text(pob_text(&socket_label(&socket.colour), 14.0, default))
                    .show_ui(ui, |ui| {
                        for (name, c) in [
                            ("R", SocketColour::R),
                            ("G", SocketColour::G),
                            ("B", SocketColour::B),
                            ("W", SocketColour::W),
                        ] {
                            let item = crate::widgets::dropdown::item(
                                ui,
                                current == Some(c),
                                pob_text(&socket_label(name), 14.0, default),
                            );
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
                // the wheel and the arrows step it (DropDownControl.lua:476-516)
                let colours = [
                    SocketColour::R,
                    SocketColour::G,
                    SocketColour::B,
                    SocketColour::W,
                ];
                let at = colours
                    .iter()
                    .position(|c| current == Some(*c))
                    .unwrap_or(0);
                if let Some(to) =
                    crate::widgets::scroll_step::combo_step(ui, &combo.response, colours.len(), at)
                {
                    out.push(Command::SetItemSocket {
                        index: i,
                        colour: colours[to],
                    });
                }
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

    // PoB's enchant section: Change Enchantment, Anoint, Corrupt, Add Implicit
    out.extend(crafting::enchant_buttons(ui, view, &mut state.crafting));

    // item properties
    if !view.influences.is_empty()
        || view.quality.is_some()
        || view.catalyst.is_some()
        || view.catalyst_quality.is_some()
    {
        ui.horizontal_wrapped(|ui| {
            for (which, list) in view.influences.iter().enumerate() {
                // PoB's tooltip on both (ItemsTab.lua:603-617)
                let (picked, response) =
                    dropdown_with(ui, &mut state.rects, &format!("influence_{which}"), list);
                response
                    .on_hover_text(INFLUENCE_TIP)
                    .on_disabled_hover_text(INFLUENCE_TIP);
                if let Some(index) = picked {
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
                    let stepped = state.quality.stepped;
                    out.push(crate::widgets::edit_box::as_step(
                        Command::SetItemQuality { text },
                        stepped,
                    ));
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
                    let stepped = state.catalyst_quality.stepped;
                    out.push(crate::widgets::edit_box::as_step(
                        Command::SetItemCatalystQuality { text },
                        stepped,
                    ));
                }
            } else {
                state.catalyst_quality.discard();
            }
        });
    } else {
        state.discard_pending();
    }

    out.extend(crafting::show(ui, view, &mut state.crafting, busy));

    // range lines: PoB's stacked rows, one Foulborn box, slider and line each (the default
    // `showAllItemAffixes` on a unique, ItemsTab.lua:1089-1173)
    if view.stacked {
        stacked_range_lines(ui, view, state, &mut out);
    } else if !view.range_lines.is_empty() {
        let default = ui.visuals().text_color();
        state.range_line = state.range_line.min(view.range_lines.len() - 1);
        let line = state.range_line;
        ui.horizontal_wrapped(|ui| {
            ui.label("Range:");
            let current = &view.range_lines[line];
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("items_range_line")
                .width(260.0)
                .selected_text(pob_text(&current.label, 14.0, default))
                .show_ui(ui, |ui| {
                    for (i, l) in view.range_lines.iter().enumerate() {
                        let item = crate::widgets::dropdown::item(
                            ui,
                            i == line,
                            pob_text(&l.label, 14.0, default),
                        );
                        state.rects.note(&item, || format!("range_line_{i}"));
                        if item.clicked() {
                            state.range_line = i;
                            state.slider = None;
                        }
                    }
                });
            state.rects.note(&combo.response, || "range_lines".into());
            if let Some(i) = crate::widgets::scroll_step::combo_step(
                ui,
                &combo.response,
                view.range_lines.len(),
                line,
            ) {
                state.range_line = i;
                state.slider = None;
            }
        });
        let current = &view.range_lines[state.range_line];
        let line = state.range_line;
        ui.horizontal_wrapped(|ui| {
            // PoB's Foulborn box comes before the slider (ItemsTab.lua:1033-1074)
            if current.mutable {
                let mut on = current.mutated;
                let mutated = ui.checkbox(&mut on, "Foulborn");
                state.rects.note(&mutated, || "range_mutated".into());
                if mutated.changed() {
                    out.push(Command::SetRangeLineMutated { line, on });
                }
            }
            if current.slider
                && let Some(range) = current.range
            {
                let mut value = match state.slider {
                    Some((l, v)) if l == line => v,
                    _ => range,
                };
                let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
                state.rects.note(&slider, || "range_slider".into());
                // the wheel and the arrows move it (SliderControl.lua:164-180)
                if let Some(to) = crate::widgets::scroll_step::page_slider_step(ui, &slider, value) {
                    state.slider = Some((line, to));
                    out.push(Command::SetRangeLine { line, value: to });
                }
                if slider.dragged() || slider.changed() {
                    state.slider = Some((line, value));
                }
                if slider.drag_stopped()
                    || (slider.changed() && !slider.is_pointer_button_down_on())
                {
                    out.push(Command::SetRangeLine { line, value });
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
            if let Some(line) = item_tooltip_ui_with(ui, &view.tooltip, &mut note)
                && !state.control_selected
            {
                out.push(Command::ToggleModLine { line });
            }
        });
    out
}

/// PoB's colour for a mutated (Foulborn) line (`colorCodes.MUTATED`, Global.lua:46)
const MUTATED: &str = "^xCD2285";

/// One row per range line: the Foulborn box where the line can mutate, the slider where it has
/// a range, then the line in PoB's colour.
fn stacked_range_lines(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut DisplayState,
    out: &mut Vec<Command>,
) {
    let default = ui.visuals().text_color();
    for (line, current) in view.range_lines.iter().enumerate() {
        // PoB hides the row's line when neither its slider nor its box shows
        if !(current.mutable || (current.slider && current.range.is_some())) {
            continue;
        }
        ui.horizontal(|ui| {
            if current.mutable {
                let mut on = current.mutated;
                let mutated = ui.checkbox(&mut on, "");
                state
                    .rects
                    .note(&mutated, || format!("stacked_mutated_{line}"));
                if mutated.changed() {
                    out.push(Command::SetRangeLineMutated { line, on });
                }
            }
            if current.slider
                && let Some(range) = current.range
            {
                let mut value = match state.slider {
                    Some((l, v)) if l == line => v,
                    _ => range,
                };
                let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
                state
                    .rects
                    .note(&slider, || format!("stacked_slider_{line}"));
                // the wheel and the arrows move it, as every slider (SliderControl.lua:164-180)
                if let Some(to) = crate::widgets::scroll_step::page_slider_step(ui, &slider, value) {
                    state.slider = Some((line, to));
                    out.push(Command::SetRangeLine { line, value: to });
                }
                if slider.dragged() || slider.changed() {
                    state.slider = Some((line, value));
                }
                if slider.drag_stopped()
                    || (slider.changed() && !slider.is_pointer_button_down_on())
                {
                    out.push(Command::SetRangeLine { line, value });
                }
            }
            let colour = if current.mutated { MUTATED } else { "^7" };
            let label = ui.label(pob_text(
                &format!("{colour}{}", current.label),
                14.0,
                default,
            ));
            state.rects.note(&label, || format!("stacked_line_{line}"));
        });
    }
}
