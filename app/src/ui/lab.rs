//! The Lab tab: the passive tree with a node power ranking, driven by `Command::Lab`.

use crate::{
    engine::{LabCommand, LabEvent, NodeScore, Tree},
    theme,
    ui::tree_view::{Heat, TreeView},
};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;

/// Stats node power can be ranked by, as (calc output key, label).
pub const METRICS: &[(&str, &str)] = &[
    ("CombinedDPS", "Combined DPS"),
    ("Life", "Life"),
    ("EnergyShield", "Energy Shield"),
    ("TotalEHP", "Effective HP"),
    ("Armour", "Armour"),
    ("Evasion", "Evasion"),
];

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Name,
    Distance,
    Gain,
    PathGain,
    PerPoint,
}

pub struct LabTab {
    /// The tree needs (re)loading: a different build was opened
    tree_stale: bool,
    /// A `LabCommand::Tree` is on its way
    tree_requested: bool,
    tree: Option<Tree>,
    tree_view: TreeView,
    metric: usize,
    max_distance: u32,
    progress: Option<(usize, usize)>,
    /// A ranking is running on the engine
    ranking: bool,
    scores: Vec<NodeScore>,
    /// Metric the current scores were computed for
    scores_metric: usize,
    sort_by: SortBy,
    sort_descending: bool,
    /// Node power by node id, for the tree heatmap
    heat: HashMap<u32, Heat>,
}

impl Default for LabTab {
    fn default() -> Self {
        Self {
            tree_stale: true,
            tree_requested: false,
            tree: None,
            tree_view: TreeView::default(),
            metric: 0,
            max_distance: 6,
            progress: None,
            ranking: false,
            scores: Vec::new(),
            scores_metric: 0,
            sort_by: SortBy::PerPoint,
            sort_descending: true,
            heat: HashMap::new(),
        }
    }
}

impl LabTab {
    /// A different build was loaded: its tree and any ranking no longer apply.
    pub fn build_changed(&mut self) {
        self.tree_stale = true;
        self.clear_scores();
    }

    #[cfg(test)]
    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// The build's numbers changed, so a ranking computed before is out of date.
    pub fn stats_changed(&mut self) {
        self.clear_scores();
    }

    fn clear_scores(&mut self) {
        self.scores.clear();
        self.heat.clear();
    }

    /// A command failed; stop showing a ranking's progress.
    pub fn failed(&mut self) {
        self.progress = None;
        self.ranking = false;
    }

    pub fn on_event(&mut self, event: LabEvent) {
        match event {
            LabEvent::Tree(tree) => {
                self.tree = Some(tree);
                self.tree_requested = false;
                self.tree_view.fit();
            }
            LabEvent::Progress { done, total } => self.progress = Some((done, total)),
            LabEvent::NodePower(scores) => {
                self.heat = scores
                    .iter()
                    .map(|s| {
                        let heat = Heat {
                            per_point: s.per_point,
                            distance: s.distance,
                        };
                        (s.id, heat)
                    })
                    .collect();
                self.scores = scores;
                self.scores_metric = self.metric;
                self.sort_scores();
                self.progress = None;
                self.ranking = false;
            }
        }
    }

    /// Draws the tab; returns the commands to send. `busy` is true while the engine is behind.
    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool) -> Vec<LabCommand> {
        let mut commands = Vec::new();
        if self.tree_stale && !self.tree_requested {
            self.tree_stale = false;
            self.tree_requested = true;
            commands.push(LabCommand::Tree);
        }

        egui::Panel::right("node_power")
            .resizable(true)
            .default_size(380.0)
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy && !self.ranking, |ui| {
                    if let Some(command) = self.node_power_panel(ui) {
                        self.ranking = true;
                        commands.push(command);
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                theme::heading(ui, "Passive tree");
                if ui.button("Fit").clicked() {
                    self.tree_view.fit();
                }
                if !self.heat.is_empty() {
                    ui.label(format!(
                        "Heatmap: {} per point (blue = low, red = high)",
                        METRICS[self.scores_metric].1
                    ));
                }
            });
            match &self.tree {
                Some(tree) => self.tree_view.show(ui, tree, &tree.allocated, &self.heat),
                None => {
                    ui.label("Loading...");
                }
            }
        });
        commands
    }

    fn node_power_panel(&mut self, ui: &mut egui::Ui) -> Option<LabCommand> {
        let mut command = None;
        theme::heading(ui, "Node power");
        ui.horizontal(|ui| {
            ui.label("Rank by");
            egui::ComboBox::from_id_salt("rank_by")
                .selected_text(METRICS[self.metric].1)
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in METRICS.iter().enumerate() {
                        ui.selectable_value(&mut self.metric, i, *label);
                    }
                });
            if ui.add(theme::primary_button("Rank nodes")).clicked() {
                command = Some(LabCommand::NodePower {
                    metric: METRICS[self.metric].0,
                    max_distance: self.max_distance,
                });
            }
        });
        ui.add(egui::Slider::new(&mut self.max_distance, 1..=15).text("Max points away"));

        if let Some((done, total)) = self.progress {
            ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .text(format!("{done} / {total} nodes")),
            );
        }

        if self.scores.is_empty() {
            ui.label("Rank this build's unallocated passives by the selected stat.");
            return command;
        }
        ui.label(format!(
            "{} nodes ranked by {}. \"Path\" includes the points needed to reach the node.",
            self.scores.len(),
            METRICS[self.scores_metric].1
        ));

        let mut clicked_header = None;
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::remainder().at_least(160.0))
            .column(Column::auto())
            .columns(Column::auto().at_least(70.0), 4)
            .header(22.0, |mut header| {
                for (label, sort) in [
                    ("Node", Some(SortBy::Name)),
                    ("Type", None),
                    ("Points", Some(SortBy::Distance)),
                    ("Node gain", Some(SortBy::Gain)),
                    ("Path gain", Some(SortBy::PathGain)),
                    ("Per point", Some(SortBy::PerPoint)),
                ] {
                    header.col(|ui| {
                        let marker = match sort {
                            Some(s) if s == self.sort_by && self.sort_descending => " ⏷",
                            Some(s) if s == self.sort_by => " ⏶",
                            _ => "",
                        };
                        let text = egui::RichText::new(format!("{label}{marker}"))
                            .strong()
                            .color(theme::HEADING);
                        match sort {
                            Some(s) => {
                                if ui.button(text).clicked() {
                                    clicked_header = Some(s);
                                }
                            }
                            None => {
                                ui.label(text);
                            }
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(20.0, self.scores.len(), |mut row| {
                    let score = &self.scores[row.index()];
                    row.col(|ui| {
                        ui.label(&score.name);
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(&score.kind).color(kind_color(&score.kind)));
                    });
                    row.col(|ui| {
                        ui.label(score.distance.to_string());
                    });
                    for value in [score.gain, score.path_gain, score.per_point] {
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(format_number(value))
                                    .color(theme::change_color(value)),
                            );
                        });
                    }
                });
            });

        if let Some(sort_by) = clicked_header {
            if sort_by == self.sort_by {
                self.sort_descending = !self.sort_descending;
            } else {
                self.sort_by = sort_by;
                self.sort_descending = !matches!(sort_by, SortBy::Name | SortBy::Distance);
            }
            self.sort_scores();
        }
        command
    }
}

impl LabTab {
    fn sort_scores(&mut self) {
        let key = |s: &NodeScore| match self.sort_by {
            SortBy::Name | SortBy::Distance => 0.0,
            SortBy::Gain => s.gain,
            SortBy::PathGain => s.path_gain,
            SortBy::PerPoint => s.per_point,
        };
        match self.sort_by {
            SortBy::Name => self.scores.sort_by(|a, b| a.name.cmp(&b.name)),
            SortBy::Distance => self.scores.sort_by_key(|s| s.distance),
            _ => self.scores.sort_by(|a, b| key(a).total_cmp(&key(b))),
        }
        if self.sort_descending {
            self.scores.reverse();
        }
    }
}

/// Node types: keystones stand out the most, like on the tree.
fn kind_color(kind: &str) -> egui::Color32 {
    match kind {
        "Keystone" => theme::UNIQUE,
        "Notable" => theme::HEADING,
        _ => theme::TEXT_DIM,
    }
}

/// Formats a stat for display: thousands separators for large values, decimals for small ones.
fn format_number(value: f64) -> String {
    if value.abs() >= 1000.0 {
        let rounded = format!("{:.0}", value.abs());
        let mut grouped = String::new();
        for (i, c) in rounded.chars().enumerate() {
            if i > 0 && (rounded.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        if value < 0.0 {
            format!("-{grouped}")
        } else {
            grouped
        }
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
