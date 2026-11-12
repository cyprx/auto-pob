//! The display panel's crafting sections: the buttons that open PoB's popups, the affix
//! editor, the cluster jewel controls and the custom mod list (spec 5).

use crate::engine::views::{AffixView, ClusterView, DisplayItemView, DropList};
use crate::engine::{Command, ControlTooltipQuery, PopupKind, Snapshot};
use crate::ui::popup::HoverTip;
use crate::ui::tree::Rects;
use crate::widgets::{pob_text, search_combo_ui};
use eframe::egui;
use std::collections::HashMap;

/// PoB's label for the button that opens `kind` from a displayed item. `Craft` belongs to the
/// empty panel, and the text editor and node replacement have no button here.
pub fn opener_label(kind: &PopupKind) -> Option<String> {
    Some(match kind {
        PopupKind::Enchant { second: false } => "Change Enchantment...".into(),
        PopupKind::Enchant { second: true } => "Change Enchantment 2...".into(),
        PopupKind::Anoint { slot: 0 } => "Anoint...".into(),
        PopupKind::Anoint { slot } => format!("Anoint {}...", slot + 1),
        PopupKind::Corrupt => "Corrupt...".into(),
        PopupKind::AddModifier => "Add modifier...".into(),
        PopupKind::AddImplicit => "Add Implicit...".into(),
        PopupKind::AddCrucible => "Add Crucible mod...".into(),
        PopupKind::Craft
        | PopupKind::EditText
        | PopupKind::NodeReplacement { .. }
        | PopupKind::Options
        | PopupKind::BuySimilar { .. }
        | PopupKind::TimelessJewel
        | PopupKind::SpectreLibrary
        | PopupKind::ModBrowser { .. } => return None,
    })
}

#[derive(Default)]
pub struct CraftingState {
    /// An affix slider's value shown until the engine's view catches up
    affix_sliders: HashMap<usize, f64>,
    /// The cluster node count slider's value, as 0..=1
    cluster_slider: Option<f64>,
    tip: HoverTip,
    queries: Vec<Command>,
    pub(crate) rects: Rects,
}

impl CraftingState {
    /// A new view of the display item arrived.
    pub(super) fn on_view(&mut self) {
        self.affix_sliders.clear();
        self.cluster_slider = None;
    }

    /// There is no item (any more) to craft.
    pub(super) fn on_gone(&mut self) {
        self.on_view();
        self.tip.reset();
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        if let Snapshot::ControlTooltip(view) = snapshot {
            self.tip.on_reply(view);
        }
    }

    /// The tooltip questions of the last frame; the app sends them without waiting.
    pub fn take_queries(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.queries)
    }
}

/// "Craft item...", shown while no item is displayed.
pub fn craft_button(ui: &mut egui::Ui, state: &mut CraftingState) -> Vec<Command> {
    let craft = ui.button("Craft item...");
    state.rects.note(&craft, || "craft".into());
    if craft.clicked() {
        vec![Command::OpenPopup(PopupKind::Craft)]
    } else {
        Vec::new()
    }
}

/// Draws the sections for the displayed item; returns the commands. The windows' buttons
/// follow `view.popups`, which holds exactly the buttons PoB shows and enables.
pub fn show(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut CraftingState,
    _busy: bool,
) -> Vec<Command> {
    let mut out = Vec::new();
    let openers: Vec<(&PopupKind, String)> = view
        .popups
        .iter()
        .filter_map(|kind| opener_label(kind).map(|label| (kind, label)))
        .collect();
    if !openers.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in openers {
                let button = ui.button(&label);
                state.rects.note(&button, || format!("open_{label}"));
                if button.clicked() {
                    out.push(Command::OpenPopup(kind.clone()));
                }
            }
        });
    }

    if let Some(sort) = &view.crafting_sort {
        ui.horizontal(|ui| {
            ui.label("Modifier sorting:");
            let (response, picked) = combo(ui, state, "crafting_sort", sort, None);
            state.rects.note(&response, || "crafting_sort".into());
            if let Some(index) = picked {
                out.push(Command::SetCraftingSort { index });
            }
        });
    }

    for affix in &view.affixes {
        affix_row(ui, affix, state, &mut out);
    }
    if let Some(cluster) = &view.cluster {
        cluster_rows(ui, cluster, state, &mut out);
    }
    custom_mods(ui, view, state, &mut out);

    let now = ui.input(|i| i.time);
    if let Some(query) = state.tip.end_frame(ui.ctx(), now) {
        state.queries.push(query);
    }
    out
}

/// A searchable dropdown over `list`; `slot` is the affix whose entries have tooltips.
fn combo(
    ui: &mut egui::Ui,
    state: &mut CraftingState,
    key: &str,
    list: &DropList,
    slot: Option<usize>,
) -> (egui::Response, Option<usize>) {
    let id = ui.make_persistent_id(("items_crafting", key));
    let (rects, tip) = (&mut state.rects, &mut state.tip);
    let mut shown = None;
    ui.add_enabled_ui(list.enabled, |ui| {
        shown = Some(search_combo_ui(
            ui,
            id,
            &list.labels,
            list.selected,
            Some(300.0),
            &mut |i, entry| {
                rects.note(entry, || format!("{key}_{i}"));
                if let Some(slot) = slot {
                    tip.note(
                        entry,
                        ControlTooltipQuery::Affix {
                            slot,
                            index: Some(i),
                        },
                    );
                }
            },
        ));
    });
    shown.expect("the closure ran")
}

fn affix_row(
    ui: &mut egui::Ui,
    affix: &AffixView,
    state: &mut CraftingState,
    out: &mut Vec<Command>,
) {
    let slot = affix.slot;
    let default = ui.visuals().text_color();
    ui.horizontal(|ui| {
        ui.label(pob_text(&affix.label, 14.0, default));
        let key = format!("affix_{slot}");
        let (response, picked) = combo(ui, state, &key, &affix.list, Some(slot));
        state.rects.note(&response, || key.clone());
        state
            .tip
            .note(&response, ControlTooltipQuery::Affix { slot, index: None });
        if let Some(index) = picked {
            out.push(Command::SetAffix {
                slot,
                index,
                label: affix.list.labels[index].clone(),
            });
        }
    });
    if let Some(range) = affix.range {
        ui.horizontal(|ui| {
            let mut value = state.affix_sliders.get(&slot).copied().unwrap_or(range);
            let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
            state.rects.note(&slider, || format!("affix_range_{slot}"));
            // the wheel and the arrows move it (SliderControl.lua:164-180)
            if let Some(to) = crate::widgets::scroll_step::slider_step(ui, &slider, value) {
                state.affix_sliders.insert(slot, to);
                out.push(Command::SetAffixRange { slot, value: to });
            }
            if let Some(tiers) = affix.tiers {
                ui.weak(format!("{tiers} tiers"));
            }
            if slider.dragged() || slider.changed() {
                state.affix_sliders.insert(slot, value);
            }
            // it sends on release, not on every step of the drag
            if slider.drag_stopped() || (slider.changed() && !slider.is_pointer_button_down_on()) {
                out.push(Command::SetAffixRange { slot, value });
            }
        });
    }
}

fn cluster_rows(
    ui: &mut egui::Ui,
    cluster: &ClusterView,
    state: &mut CraftingState,
    out: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        ui.label("Cluster jewel skill:");
        let (response, picked) = combo(ui, state, "cluster_skill", &cluster.skill, None);
        state.rects.note(&response, || "cluster_skill".into());
        if let Some(index) = picked {
            out.push(Command::SetClusterSkill { index });
        }
    });
    let span = cluster.max.saturating_sub(cluster.min);
    let count_of = |v: f64| cluster.min + (v * f64::from(span)).round() as u32;
    let current = if span == 0 {
        0.0
    } else {
        f64::from(cluster.count.clamp(cluster.min, cluster.max) - cluster.min) / f64::from(span)
    };
    ui.horizontal(|ui| {
        ui.label("Added Passives:");
        let mut value = state.cluster_slider.unwrap_or(current);
        let slider = ui.add_enabled(
            span > 0,
            egui::Slider::new(&mut value, 0.0..=1.0)
                .show_value(false)
                .step_by(if span > 0 { 1.0 / f64::from(span) } else { 0.0 }),
        );
        state.rects.note(&slider, || "cluster_count".into());
        // the wheel and the arrows move it, to its steps (SliderControl.lua:164-180)
        if span > 0
            && let Some(to) = crate::widgets::scroll_step::slider_step(ui, &slider, value)
        {
            let to = (to * f64::from(span)).round() / f64::from(span);
            state.cluster_slider = Some(to);
            out.push(Command::SetClusterNodeCount {
                count: count_of(to),
            });
        }
        if slider.dragged() || slider.changed() {
            state.cluster_slider = Some(value);
        }
        ui.label(count_of(value).to_string());
        if slider.drag_stopped() || (slider.changed() && !slider.is_pointer_button_down_on()) {
            out.push(Command::SetClusterNodeCount {
                count: count_of(value),
            });
        }
    });
}

fn custom_mods(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut CraftingState,
    out: &mut Vec<Command>,
) {
    if view.custom_mods.is_empty() {
        return;
    }
    let default = ui.visuals().text_color();
    ui.separator();
    for (index, line) in view.custom_mods.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            let remove = ui.small_button("Remove");
            state.rects.note(&remove, || format!("remove_{index}"));
            if remove.clicked() {
                out.push(Command::RemoveCustomMod { index });
            }
            ui.weak(format!("{}:", line.kind));
            ui.label(pob_text(&line.line, 14.0, default));
        });
    }
}
