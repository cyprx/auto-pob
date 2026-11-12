//! The Config tab: PoB's configuration options by section, config sets and custom modifiers.
//!
//! The tab never owns the build's state. It draws the latest `ConfigView` and returns
//! commands. While the engine is behind (spec 6.3) a field keeps what the user typed or
//! picked, and goes back to showing the snapshot once the engine has caught up.

use crate::engine::views::{ConfigKind, ConfigOption, ConfigSection, ConfigView, CustomModGroup};
use crate::engine::{Command, ConfigValue};
use crate::theme;
use crate::widgets::{pob_text, tooltip_ui};
use eframe::egui;
use std::collections::HashMap;

/// Width one column of sections wants.
const COLUMN_WIDTH: f32 = 360.0;
/// Opacity of options PoB hides, drawn under "show all".
const HIDDEN_OPACITY: f32 = 0.45;

/// What a number field holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberInput {
    /// Nothing typed: clears the option
    Empty,
    Valid(f64),
    /// Not a number of this kind; nothing is sent
    Invalid,
}

/// Parses what the user typed into a count, integer or float field. No exponents.
pub fn parse_number_input(text: &str, kind: ConfigKind, allow_zero: bool) -> NumberInput {
    let text = text.trim();
    if text.is_empty() {
        return NumberInput::Empty;
    }
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) if kind != ConfigKind::Count => (true, rest),
        _ => (false, text),
    };
    let digits_only = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let well_formed = match kind {
        ConfigKind::Float => match body.split_once('.') {
            None => digits_only(body),
            Some((int, frac)) => {
                (digits_only(int) || int.is_empty())
                    && (digits_only(frac) || frac.is_empty())
                    && !(int.is_empty() && frac.is_empty())
            }
        },
        _ => digits_only(body),
    };
    if !well_formed {
        return NumberInput::Invalid;
    }
    let Ok(value) = body.parse::<f64>() else {
        return NumberInput::Invalid;
    };
    if kind == ConfigKind::Count && value == 0.0 && !allow_zero {
        return NumberInput::Invalid;
    }
    NumberInput::Valid(if negative { -value } else { value })
}

/// Whether the option's label contains `query`, ignoring case and colour codes. An empty
/// query matches everything.
pub fn matches_search(option: &ConfigOption, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let default = eframe::egui::Color32::WHITE;
    pob_text(&option.label, 14.0, default)
        .text
        .to_lowercase()
        .contains(&query)
}

/// A JSON option value as the text of a number or text field.
fn value_text(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            Some(f) => format!("{f}"),
            None => n.to_string(),
        },
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// An in-progress edit of a config set's title.
enum SetEdit {
    New(String),
    Rename { id: u32, title: String },
}

/// How a text field sends its edits.
#[derive(Clone, Copy, PartialEq)]
enum SendOn {
    /// Every change
    Change,
    /// When focus leaves
    Blur,
}

#[derive(Clone, Copy)]
enum FieldStyle {
    Single { width: f32 },
    Multi { rows: usize },
}

#[derive(Default)]
pub struct ConfigUi {
    search: String,
    show_all: bool,
    /// Typed text by field, kept while the field has focus or the engine is behind
    drafts: HashMap<String, String>,
    /// What each on-blur field last sent, so the same text isn't sent twice while behind
    sent: HashMap<String, String>,
    /// Checkbox states clicked while the engine is behind
    bools: HashMap<String, bool>,
    /// Dropdown choices made while the engine is behind
    picks: HashMap<String, String>,
    set_edit: Option<SetEdit>,
    confirm_delete: bool,
    confirm_remove: Option<usize>,
    /// Ctrl+F: the search box takes the keyboard when next drawn (ConfigTab.lua:987-988)
    pub focus_search: bool,
    /// The custom-modifier boxes' heights, as their grips left them
    heights: HashMap<String, f32>,
    /// F2: the active set's Rename box opens when next drawn
    pub rename: bool,
    /// Where widgets were drawn last frame, so tests can click them
    #[cfg(test)]
    pub(crate) rects: HashMap<String, egui::Rect>,
}

impl ConfigUi {
    #[cfg(test)]
    fn note(&mut self, key: String, response: &egui::Response) {
        self.rects.insert(key, response.rect);
    }

    #[cfg(test)]
    pub(crate) fn draft(&self, key: &str) -> Option<&str> {
        self.drafts.get(key).map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn set_show_all(&mut self, on: bool) {
        self.show_all = on;
    }

    #[cfg(not(test))]
    fn note(&mut self, _key: String, _response: &egui::Response) {}

    /// Draws the tab; returns the commands to send.
    pub fn show(&mut self, ui: &mut egui::Ui, view: &ConfigView, behind: bool) -> Vec<Command> {
        let mut commands = Vec::new();
        if !behind {
            self.bools.clear();
            self.picks.clear();
        }
        self.top_row(ui, view, &mut commands);
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.sections(ui, view, behind, &mut commands);
                ui.add_space(8.0);
                self.custom_mods(ui, &view.custom_mods, behind, &mut commands);
            });
        // a structural change renumbers or replaces what the drafts were for
        if commands.iter().any(|c| {
            matches!(
                c,
                Command::SelectConfigSet { .. }
                    | Command::DeleteConfigSet { .. }
                    | Command::NewConfigSet { .. }
                    | Command::AddCustomModGroup
                    | Command::RemoveCustomModGroup { .. }
            )
        }) {
            self.drafts.clear();
            self.sent.clear();
            self.bools.clear();
            self.picks.clear();
        }
        commands
    }

    // ---- top row ------------------------------------------------------------------------

    fn top_row(&mut self, ui: &mut egui::Ui, view: &ConfigView, commands: &mut Vec<Command>) {
        let active = view.sets.iter().find(|s| s.id == view.active_set);
        ui.horizontal_wrapped(|ui| {
            let combo = egui::ComboBox::from_id_salt("config_set")
                .selected_text(active.map_or("", |s| s.title.as_str()))
                .show_ui(ui, |ui| {
                    for set in &view.sets {
                        let selected = set.id == view.active_set;
                        let item = ui.selectable_label(selected, &set.title);
                        self.note(format!("set:{}", set.id), &item);
                        if item.clicked() && !selected {
                            commands.push(Command::SelectConfigSet { id: set.id });
                        }
                    }
                });
            self.note("set_combo".into(), &combo.response);
            self.set_buttons(ui, view, active.map(|s| s.title.as_str()), commands);
            ui.separator();
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search options")
                    .desired_width(160.0),
            );
            if std::mem::take(&mut self.focus_search) {
                search.request_focus();
            }
            if !self.search.is_empty() && ui.small_button("x").clicked() {
                self.search.clear();
            }
            let show_all = ui.checkbox(&mut self.show_all, "Show all");
            self.note("show_all".into(), &show_all);
        });
    }

    fn set_buttons(
        &mut self,
        ui: &mut egui::Ui,
        view: &ConfigView,
        active_title: Option<&str>,
        commands: &mut Vec<Command>,
    ) {
        if self.confirm_delete {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Delete {}?", active_title.unwrap_or("this set")),
            );
            let yes = ui.add(theme::primary_button("Delete"));
            self.note("set_delete_yes".into(), &yes);
            if yes.clicked() {
                commands.push(Command::DeleteConfigSet {
                    id: view.active_set,
                });
                self.confirm_delete = false;
            }
            if ui.button("Cancel").clicked() {
                self.confirm_delete = false;
            }
            return;
        }
        if let Some(edit) = &mut self.set_edit {
            let title = match edit {
                SetEdit::New(t) | SetEdit::Rename { title: t, .. } => t,
            };
            let field = ui.add(
                egui::TextEdit::singleline(title)
                    .hint_text("Set name")
                    .desired_width(140.0),
            );
            field.request_focus();
            let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let valid = !title.trim().is_empty();
            let ok = ui.add_enabled(valid, theme::primary_button("OK"));
            let cancel = ui.button("Cancel");
            if (ok.clicked() || submit) && valid {
                let title = title.trim().to_owned();
                commands.push(match edit {
                    SetEdit::New(_) => Command::NewConfigSet { title },
                    SetEdit::Rename { id, .. } => Command::RenameConfigSet { id: *id, title },
                });
                self.set_edit = None;
            } else if cancel.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.set_edit = None;
            }
            return;
        }
        let new = ui.button("New");
        self.note("set_new".into(), &new);
        if new.clicked() {
            self.set_edit = Some(SetEdit::New(String::new()));
        }
        // F2 too (ConfigSetListControl.lua's rename)
        if ui.button("Rename").clicked() || std::mem::take(&mut self.rename) {
            self.set_edit = Some(SetEdit::Rename {
                id: view.active_set,
                title: active_title.unwrap_or_default().to_owned(),
            });
        }
        // the last set can't be deleted
        let delete = ui.add_enabled(view.sets.len() > 1, egui::Button::new("Delete"));
        self.note("set_delete".into(), &delete);
        if delete.clicked() {
            self.confirm_delete = true;
        }
    }

    // ---- sections -----------------------------------------------------------------------

    fn sections(
        &mut self,
        ui: &mut egui::Ui,
        view: &ConfigView,
        behind: bool,
        commands: &mut Vec<Command>,
    ) {
        let columns = ((ui.available_width() / COLUMN_WIDTH) as usize).max(1);
        // PoB's column order, then its order within a column
        let mut order: Vec<&ConfigSection> = view.sections.iter().collect();
        order.sort_by_key(|s| s.column);
        let mut buckets: Vec<Vec<(&ConfigSection, Vec<&ConfigOption>)>> = vec![vec![]; columns];
        for section in order {
            let options: Vec<&ConfigOption> = section
                .options
                .iter()
                .filter(|o| (o.shown || self.show_all) && matches_search(o, &self.search))
                .collect();
            if options.is_empty() {
                continue;
            }
            let bucket = (section.column.max(1) as usize - 1) % columns;
            buckets[bucket].push((section, options));
        }
        ui.columns(columns, |uis| {
            for (ui, bucket) in uis.iter_mut().zip(&buckets) {
                for (section, options) in bucket {
                    egui::CollapsingHeader::new(
                        egui::RichText::new(&section.name).color(theme::HEADING),
                    )
                    .id_salt(("config_section", &section.name))
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(("config_grid", &section.name))
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                for option in options {
                                    self.option_row(ui, option, behind, commands);
                                    ui.end_row();
                                }
                            });
                    });
                }
            }
        });
    }

    fn option_row(
        &mut self,
        ui: &mut egui::Ui,
        option: &ConfigOption,
        behind: bool,
        commands: &mut Vec<Command>,
    ) {
        let dim = !option.shown;
        let default = if option.invalid {
            theme::NEGATIVE
        } else {
            ui.visuals().text_color()
        };
        scoped(ui, dim, |ui| {
            let label = ui.label(pob_text(&option.label, 14.0, default));
            if !option.tooltip.is_empty() {
                label.on_hover_ui(|ui| tooltip_ui(ui, &option.tooltip));
            }
        });
        let widget = scoped(ui, dim, |ui| {
            self.option_widget(ui, option, behind, commands)
        });
        if !option.tooltip.is_empty() {
            widget.on_hover_ui(|ui| tooltip_ui(ui, &option.tooltip));
        }
    }

    fn option_widget(
        &mut self,
        ui: &mut egui::Ui,
        option: &ConfigOption,
        behind: bool,
        commands: &mut Vec<Command>,
    ) -> egui::Response {
        let var = option.var.as_str();
        match option.kind {
            ConfigKind::Check => {
                let snapshot = option.value.as_ref().and_then(|v| v.as_bool());
                let mut on = self
                    .bools
                    .get(var)
                    .copied()
                    .unwrap_or(snapshot.unwrap_or(false));
                let response = ui.add(egui::Checkbox::without_text(&mut on));
                self.note(format!("check:{var}"), &response);
                if response.changed() {
                    self.bools.insert(var.to_owned(), on);
                    commands.push(Command::SetConfig {
                        var: var.to_owned(),
                        value: ConfigValue::Bool(on),
                    });
                }
                response
            }
            ConfigKind::List => {
                let current = self
                    .picks
                    .get(var)
                    .or(option.selected.as_ref())
                    .cloned()
                    .unwrap_or_default();
                let shown = option
                    .choices
                    .iter()
                    .find(|c| c.value == current)
                    .map_or(current.as_str(), |c| c.label.as_str());
                let default = ui.visuals().text_color();
                let combo = egui::ComboBox::from_id_salt(("config_list", var))
                    .selected_text(pob_text(shown, 14.0, default))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for choice in &option.choices {
                            let selected = choice.value == current;
                            let item = ui
                                .selectable_label(selected, pob_text(&choice.label, 14.0, default));
                            self.note(format!("list:{var}:{}", choice.value), &item);
                            if item.clicked() && !selected {
                                self.picks.insert(var.to_owned(), choice.value.clone());
                                commands.push(Command::SetConfig {
                                    var: var.to_owned(),
                                    value: ConfigValue::Choice(choice.value.clone()),
                                });
                            }
                        }
                    });
                self.note(format!("list:{var}"), &combo.response);
                combo.response
            }
            ConfigKind::Count | ConfigKind::Integer | ConfigKind::Float | ConfigKind::Text => {
                let current = value_text(option.value.as_ref());
                let hint = value_text(option.placeholder.as_ref());
                let (response, text) = self.draft_field(
                    ui,
                    var.to_owned(),
                    &current,
                    FieldStyle::Single { width: 100.0 },
                    &hint,
                    SendOn::Change,
                    behind,
                );
                self.note(format!("field:{var}"), &response);
                if let Some(text) = text {
                    let value = if option.kind == ConfigKind::Text {
                        Some(if text.is_empty() {
                            ConfigValue::Clear
                        } else {
                            ConfigValue::Text(text)
                        })
                    } else {
                        match parse_number_input(&text, option.kind, option.allow_zero) {
                            NumberInput::Empty => Some(ConfigValue::Clear),
                            NumberInput::Valid(n) => Some(ConfigValue::Number(n)),
                            NumberInput::Invalid => None,
                        }
                    };
                    if let Some(value) = value {
                        commands.push(Command::SetConfig {
                            var: var.to_owned(),
                            value,
                        });
                    }
                }
                response
            }
        }
    }

    /// A text field that shows what the user typed until the engine has caught up. Returns
    /// the text to send, if the field's edit is due to be sent.
    /// A resizable box's height: PoB's 80 until its grip moves it (ConfigTab.lua:70)
    fn box_height(&self, key: &str) -> f32 {
        self.heights.get(key).copied().unwrap_or(80.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn draft_field(
        &mut self,
        ui: &mut egui::Ui,
        key: String,
        current: &str,
        style: FieldStyle,
        hint: &str,
        send_on: SendOn,
        behind: bool,
    ) -> (egui::Response, Option<String>) {
        let mut text = self
            .drafts
            .get(&key)
            .cloned()
            .unwrap_or_else(|| current.to_owned());
        let edit = match style {
            FieldStyle::Single { width } => {
                egui::TextEdit::singleline(&mut text).desired_width(width)
            }
            FieldStyle::Multi { rows } => egui::TextEdit::multiline(&mut text)
                .desired_rows(rows)
                .desired_width(f32::INFINITY)
                .min_size(egui::vec2(0.0, self.box_height(&key))),
        };
        let response = ui.add(edit.hint_text(hint));
        if matches!(style, FieldStyle::Multi { .. }) {
            // PoB's custom-modifier box has a size grip (ConfigTab.lua:70: 80 high, 40 to 600)
            let mut height = self.box_height(&key);
            let grip = crate::widgets::size_grip::size_grip(
                ui,
                ui.make_persistent_id(("config_grip", &key)),
                response.rect,
                &mut height,
                (40.0, 600.0),
            );
            #[cfg(test)]
            self.rects.insert(format!("grip:{key}"), grip.rect);
            let _ = &grip;
            self.heights.insert(key.clone(), height);
        }
        if response.changed() {
            self.drafts.insert(key.clone(), text.clone());
        }
        let send = match send_on {
            SendOn::Change => response.changed(),
            SendOn::Blur => {
                response.lost_focus()
                    && text != current
                    && self.sent.get(&key).map(String::as_str) != Some(text.as_str())
            }
        };
        if send && send_on == SendOn::Blur {
            self.sent.insert(key.clone(), text.clone());
        }
        if !response.has_focus() && !behind && !send {
            self.drafts.remove(&key);
            self.sent.remove(&key);
        }
        (response, send.then_some(text))
    }

    // ---- custom modifiers ---------------------------------------------------------------

    fn custom_mods(
        &mut self,
        ui: &mut egui::Ui,
        groups: &[CustomModGroup],
        behind: bool,
        commands: &mut Vec<Command>,
    ) {
        egui::CollapsingHeader::new(egui::RichText::new("Custom modifiers").color(theme::HEADING))
            .id_salt("config_custom_mods")
            .default_open(true)
            .show(ui, |ui| {
                for (i, group) in groups.iter().enumerate() {
                    self.custom_mod_group(ui, i, group, behind, commands);
                    ui.add_space(6.0);
                }
                let add = ui.button("Add group");
                self.note("add_group".into(), &add);
                if add.clicked() {
                    commands.push(Command::AddCustomModGroup);
                }
            });
    }

    fn custom_mod_group(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        group: &CustomModGroup,
        behind: bool,
        commands: &mut Vec<Command>,
    ) {
        ui.horizontal(|ui| {
            let enabled_key = format!("mod_enabled:{i}");
            let mut enabled = self
                .bools
                .get(&enabled_key)
                .copied()
                .unwrap_or(group.enabled);
            let toggle = ui.add(egui::Checkbox::without_text(&mut enabled));
            self.note(enabled_key.clone(), &toggle);
            if toggle.changed() {
                self.bools.insert(enabled_key, enabled);
                commands.push(Command::SetCustomModEnabled { group: i, enabled });
            }
            let (response, title) = self.draft_field(
                ui,
                format!("mod_title:{i}"),
                &group.title,
                FieldStyle::Single { width: 220.0 },
                "Group title",
                SendOn::Blur,
                behind,
            );
            self.note(format!("mod_title:{i}"), &response);
            if let Some(title) = title {
                commands.push(Command::SetCustomModTitle { group: i, title });
            }
            if self.confirm_remove == Some(i) {
                ui.colored_label(ui.visuals().warn_fg_color, "Remove this group?");
                let yes = ui.add(theme::primary_button("Remove"));
                self.note(format!("mod_remove_yes:{i}"), &yes);
                if yes.clicked() {
                    commands.push(Command::RemoveCustomModGroup { group: i });
                    self.confirm_remove = None;
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_remove = None;
                }
            } else {
                let remove = ui.button("Remove");
                self.note(format!("mod_remove:{i}"), &remove);
                if remove.clicked() {
                    self.confirm_remove = Some(i);
                }
            }
        });
        let (response, text) = self.draft_field(
            ui,
            format!("mod_text:{i}"),
            &group.text,
            FieldStyle::Multi { rows: 4 },
            "One modifier per line, e.g. +10% to all Elemental Resistances",
            SendOn::Blur,
            behind,
        );
        self.note(format!("mod_text:{i}"), &response);
        if let Some(text) = text {
            commands.push(Command::SetCustomModText { group: i, text });
        }
    }
}

/// Runs `add`, dimmed if `dim`.
fn scoped<R>(ui: &mut egui::Ui, dim: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if dim {
        ui.scope(|ui| {
            ui.set_opacity(HIDDEN_OPACITY);
            add(ui)
        })
        .inner
    } else {
        add(ui)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(label: &str) -> ConfigOption {
        ConfigOption {
            var: "v".into(),
            kind: ConfigKind::Check,
            allow_zero: false,
            label: label.into(),
            shown: true,
            invalid: false,
            value: None,
            placeholder: None,
            selected: None,
            choices: vec![],
            tooltip: vec![],
        }
    }

    #[test]
    fn parse_number_input_cases() {
        use NumberInput::*;
        assert_eq!(parse_number_input("", ConfigKind::Count, false), Empty);
        assert_eq!(parse_number_input("  ", ConfigKind::Float, false), Empty);
        assert_eq!(
            parse_number_input(" 12 ", ConfigKind::Count, false),
            Valid(12.0)
        );
        assert_eq!(parse_number_input("0", ConfigKind::Count, false), Invalid);
        assert_eq!(parse_number_input("0", ConfigKind::Count, true), Valid(0.0));
        assert_eq!(parse_number_input("-5", ConfigKind::Count, false), Invalid);
        assert_eq!(
            parse_number_input("-5", ConfigKind::Integer, false),
            Valid(-5.0)
        );
        assert_eq!(
            parse_number_input("3.5", ConfigKind::Integer, false),
            Invalid
        );
        assert_eq!(
            parse_number_input("3.5", ConfigKind::Float, false),
            Valid(3.5)
        );
        assert_eq!(
            parse_number_input("-3.5", ConfigKind::Float, false),
            Valid(-3.5)
        );
        assert_eq!(parse_number_input("1e9", ConfigKind::Float, false), Invalid);
        assert_eq!(
            parse_number_input("1.2.3", ConfigKind::Float, false),
            Invalid
        );
        assert_eq!(parse_number_input("-", ConfigKind::Integer, false), Invalid);
        assert_eq!(parse_number_input(".", ConfigKind::Float, false), Invalid);
        assert_eq!(parse_number_input("12a", ConfigKind::Count, false), Invalid);
    }

    #[test]
    fn search_ignores_case_and_colour_codes() {
        let o = option("^7Enemy is a ^xFF0000Boss");
        assert!(matches_search(&o, "enemy is a boss"));
        assert!(matches_search(&o, "BOSS"));
        assert!(!matches_search(&o, "curse"));
        assert!(!matches_search(&o, "xFF0000"));
    }

    #[test]
    fn empty_search_matches_everything() {
        assert!(matches_search(&option("anything"), ""));
        assert!(matches_search(&option("anything"), "   "));
    }
}
