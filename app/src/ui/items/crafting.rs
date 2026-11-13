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
        | PopupKind::ModBrowser { .. }
        | PopupKind::GameVersion => return None,
    })
}

#[derive(Default)]
pub struct CraftingState {
    /// An affix slider's value shown until the engine's view catches up
    affix_sliders: HashMap<usize, f64>,
    /// The cluster node count slider's value, as 0..=1
    cluster_slider: Option<f64>,
    /// The node count slider's own value as the wheel and the arrows left it: PoB's slider
    /// keeps its unrounded value, so small steps add up to the next count (SliderControl.lua:
    /// 45-53, ItemsTab.lua:711-716)
    cluster_raw: Option<f64>,
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
        self.cluster_raw = None;
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

/// Whether a window's button sits in PoB's "Add modifier" row (the custom section) rather
/// than the enchant row (ItemsTab.lua:460-541, 985-1004)
fn in_custom_section(kind: &PopupKind) -> bool {
    matches!(kind, PopupKind::AddModifier | PopupKind::AddCrucible)
}

/// One row of the windows' buttons, those `pick` takes, in PoB's order. They follow
/// `view.popups`, which holds exactly the buttons PoB shows and enables.
fn openers(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut CraftingState,
    pick: fn(&PopupKind) -> bool,
    out: &mut Vec<Command>,
) {
    let openers: Vec<(&PopupKind, String)> = view
        .popups
        .iter()
        .filter(|kind| pick(kind))
        .filter_map(|kind| opener_label(kind).map(|label| (kind, label)))
        .collect();
    if openers.is_empty() {
        return;
    }
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

/// PoB's enchant section: Change Enchantment, Anoint, Corrupt and Add Implicit, right after
/// the sockets (ItemsTab.lua:460-541)
pub fn enchant_buttons(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut CraftingState,
) -> Vec<Command> {
    let mut out = Vec::new();
    openers(ui, view, state, |k| !in_custom_section(k), &mut out);
    out
}

/// Draws the crafting sections after the catalyst, in PoB's order: the cluster jewel, the
/// modifier sorting, the affixes, then the custom section (its buttons and Remove rows);
/// returns the commands.
pub fn show(
    ui: &mut egui::Ui,
    view: &DisplayItemView,
    state: &mut CraftingState,
    _busy: bool,
) -> Vec<Command> {
    let mut out = Vec::new();
    // the cluster jewel section comes before the sorting (ItemsTab.lua:697-716)
    if let Some(cluster) = &view.cluster {
        cluster_rows(ui, cluster, state, &mut out);
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
    // the custom section: "Add modifier..." and "Add Crucible mod...", then the Remove rows
    openers(ui, view, state, in_custom_section, &mut out);
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
        // the closed dropdown shows its selected entry's tooltip (DropDownControl.lua:288-296,
        // mode "BODY")
        state.tip.note(
            &response,
            ControlTooltipQuery::Affix {
                slot,
                index: Some(affix.list.selected),
            },
        );
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
            // PoB's label, grey while the dropdown is on "None" (ItemsTab.lua:969-971)
            let roll = if affix.list.selected > 0 {
                "^7Roll:"
            } else {
                "^x7F7F7FRoll:"
            };
            let label = ui.label(pob_text(roll, 14.0, default));
            state.rects.note(&label, || format!("affix_roll_{slot}"));
            let mut value = state.affix_sliders.get(&slot).copied().unwrap_or(range);
            let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
            state.rects.note(&slider, || format!("affix_range_{slot}"));
            // its own tooltip, for the value under the pointer, or the value while dragged
            // (SliderControl.lua:117-125)
            let at = if slider.dragged() {
                value
            } else {
                slider.hover_pos().map_or(value, |p| {
                    f64::from((p.x - slider.rect.left()) / slider.rect.width().max(1.0))
                })
            };
            state.tip.note(
                &slider,
                ControlTooltipQuery::AffixSlider {
                    slot,
                    at: (at.clamp(0.0, 1.0) * 1000.0).round() as u16,
                },
            );
            // the wheel and the arrows move it (SliderControl.lua:164-180)
            if let Some(to) = crate::widgets::scroll_step::page_slider_step(ui, &slider, value) {
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
        // the wheel and the arrows move PoB's unrounded value; the count is that value rounded
        // (SliderControl.lua:164-180, ItemsTab.lua:711-716)
        let raw = state
            .cluster_raw
            .filter(|r| count_of(*r) == cluster.count)
            .unwrap_or(current);
        if span > 0
            && let Some(to) = crate::widgets::scroll_step::page_slider_step(ui, &slider, raw)
        {
            state.cluster_raw = Some(to);
            let count = count_of(to);
            if count != cluster.count {
                state.cluster_slider = Some(f64::from(count - cluster.min) / f64::from(span));
                out.push(Command::SetClusterNodeCount { count });
            }
        }
        if slider.dragged() || slider.changed() {
            state.cluster_slider = Some(value);
            state.cluster_raw = Some(value);
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
    for (index, line) in view.custom_mods.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            let remove = ui.small_button("Remove");
            state.rects.note(&remove, || format!("remove_{index}"));
            if remove.clicked() {
                out.push(Command::RemoveCustomMod { index });
            }
            // PoB's label is white: " ^7Crafted:" (ItemsTab.lua:2362)
            ui.label(pob_text(&format!("^7{}:", line.kind), 14.0, default));
            ui.label(pob_text(&line.line, 14.0, default));
        });
    }
}
