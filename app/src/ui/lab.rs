use crate::{
    engine::lab_engine::{
        BuildSummary, CONFIG, Command, ConfigKind, ConfigValue, Engine, Event, METRICS, NodeScore,
        STATS, Tree,
    },
    theme,
    ui::tree_view::{Heat, TreeView},
};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Name,
    Distance,
    Gain,
    PathGain,
    PerPoint,
}

pub struct LabApp {
    engine: Engine,
    status: String,
    error: Option<String>,
    /// A command is running on the engine
    busy: bool,
    build: Option<BuildSummary>,
    import_text: String,
    level: u32,
    metric: usize,
    max_distance: u32,
    progress: Option<(usize, usize)>,
    scores: Vec<NodeScore>,
    /// Metric the current scores were computed for
    scores_metric: usize,
    sort_by: SortBy,
    sort_descending: bool,
    tree: Option<Tree>,
    tree_view: TreeView,
    /// Node power by node id, for the tree heatmap
    heat: HashMap<u32, Heat>,
}

impl LabApp {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            status: String::new(),
            error: None,
            busy: true, // until the engine has started
            build: None,
            import_text: String::new(),
            level: 1,
            metric: 0,
            max_distance: 6,
            progress: None,
            scores: Vec::new(),
            scores_metric: 0,
            sort_by: SortBy::PerPoint,
            sort_descending: true,
            tree: None,
            tree_view: TreeView::default(),
            heat: HashMap::new(),
        }
    }

    fn send(&mut self, command: Command) {
        self.busy = true;
        self.error = None;
        self.engine.send(command);
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.engine.events.try_recv() {
            match event {
                Event::Status(status) => self.status = status,
                Event::Tree(tree) => {
                    self.tree = Some(tree);
                    self.tree_view.fit();
                }
                Event::Build(build) => {
                    self.level = build.level;
                    self.build = Some(build);
                    // scores belong to the previous state of the build
                    self.scores.clear();
                    self.heat.clear();
                    self.busy = false;
                }
                Event::Progress { done, total } => self.progress = Some((done, total)),
                Event::NodePower(scores) => {
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
                    self.busy = false;
                }
                Event::Error(error) => {
                    self.error = Some(error);
                    self.progress = None;
                    self.busy = false;
                }
            }
        }
    }

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

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        theme::heading(ui, "Build");
        ui.label("Paste a PoB share code or build XML:");
        ui.add(
            egui::TextEdit::multiline(&mut self.import_text)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui.button("Load").clicked() && !self.import_text.trim().is_empty() {
                self.send(Command::Load(self.import_text.clone()));
            }
            if ui.button("New build").clicked() {
                self.send(Command::NewBuild);
            }
        });

        let Some(build) = &self.build else {
            return;
        };
        ui.separator();
        ui.label(match &build.ascendancy {
            Some(ascendancy) => format!("{} ({})", ascendancy, build.class),
            None => build.class.clone(),
        });

        let slider = ui.add(egui::Slider::new(&mut self.level, 1..=100).text("Level"));
        // only recalculate once the user lets go of the slider
        if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
            let level = self.level;
            self.send(Command::SetLevel(level));
        }

        ui.separator();
        ui.strong("Configuration");
        let mut changed = None;
        let build = self.build.as_ref().expect("checked above");
        for (option, value) in CONFIG.iter().zip(&build.config) {
            match option.kind {
                ConfigKind::Check => {
                    let mut checked = *value == ConfigValue::Bool(true);
                    if ui.checkbox(&mut checked, option.label).changed() {
                        changed = Some((option.var, ConfigValue::Bool(checked)));
                    }
                }
                ConfigKind::List(choices) => {
                    let current = match value {
                        ConfigValue::Text(text) => choices
                            .iter()
                            .find(|(v, _)| v == text)
                            .map_or(text.as_str(), |(_, label)| label),
                        _ => "(default)",
                    };
                    egui::ComboBox::from_label(option.label)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (choice, label) in choices {
                                let selected = *value == ConfigValue::Text((*choice).into());
                                if ui.selectable_label(selected, *label).clicked() {
                                    changed =
                                        Some((option.var, ConfigValue::Text((*choice).into())));
                                }
                            }
                        });
                }
            }
        }

        ui.separator();
        ui.strong("Stats");
        egui::Grid::new("stats").striped(true).show(ui, |ui| {
            for ((key, label), value) in STATS.iter().zip(&build.stats) {
                ui.label(egui::RichText::new(*label).color(stat_color(key)));
                ui.label(value.map_or("-".into(), format_number));
                ui.end_row();
            }
        });

        if let Some((var, value)) = changed {
            self.send(Command::SetConfig(var, value));
        }
    }

    fn node_power_panel(&mut self, ui: &mut egui::Ui) {
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
                self.send(Command::NodePower {
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
            ui.label("Load a build, then rank its unallocated passives by the selected stat.");
            return;
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
    }
}

impl eframe::App for LabApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_events();

        egui::Panel::top("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.busy {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });

        egui::Panel::left("build")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!self.busy, |ui| self.side_panel(ui));
                });
            });

        egui::Panel::right("node_power")
            .resizable(true)
            .default_size(380.0)
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.busy, |ui| self.node_power_panel(ui));
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
            match (&self.tree, &self.build) {
                (Some(tree), Some(build)) => {
                    self.tree_view.show(ui, tree, &build.allocated, &self.heat);
                }
                _ => {
                    ui.label("Loading...");
                }
            }
        });
    }
}

/// PoB's colour for a stat, by calc output key.
fn stat_color(key: &str) -> egui::Color32 {
    match key {
        "Life" => theme::LIFE,
        "Mana" => theme::MANA,
        "EnergyShield" => theme::ENERGY_SHIELD,
        "Armour" => theme::ARMOUR,
        "Evasion" => theme::POSITIVE,
        "FireResist" => theme::FIRE,
        "ColdResist" => theme::COLD,
        "LightningResist" => theme::LIGHTNING,
        "ChaosResist" => theme::CHAOS,
        "CombinedDPS" => theme::UNIQUE,
        _ => theme::HEADING,
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
