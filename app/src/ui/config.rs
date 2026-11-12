//! The Config tab: PoB's configuration options by section, config sets and custom modifiers.
//!
//! The tab never owns the build's state. It draws the latest `ConfigView` and returns
//! commands. While the engine is behind (spec 6.3) a field keeps what the user typed or
//! picked, and goes back to showing the snapshot once the engine has caught up.

use crate::engine::views::{ConfigKind, ConfigOption, ConfigSection, ConfigView, CustomModGroup};
use crate::engine::{Command, ConfigValue, HoverTarget, PopupKind};
use crate::theme;
use crate::ui::hover::HoverTip;
use crate::ui::items::list::reorder_rows;
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
    Rename {
        id: u32,
        title: String,
    },
    /// PoB's manager Copy: the copy's name (NF-4); `edited` once it was typed into, as PoB's
    /// Save stays disabled until then (ConfigSetListControl.lua:44-46)
    Copy {
        id: u32,
        title: String,
        edited: bool,
    },
}

/// What a dragged config set row carries: its position
struct ConfigSetDrag(usize);

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
    /// The set the user asked to delete, until the question is answered
    confirm_delete: Option<u32>,
    confirm_remove: Option<usize>,
    /// Ctrl+F: the search box takes the keyboard when next drawn (ConfigTab.lua:987-988)
    pub focus_search: bool,
    /// The custom-modifier boxes' heights, as their grips left them
    heights: HashMap<String, f32>,
    /// Where widgets were drawn last frame, so tests can click them
    #[cfg(test)]
    pub(crate) rects: HashMap<String, egui::Rect>,
    /// The options' and custom groups' "what if" tooltips (R22)
    pub(crate) hover_tip: HoverTip,
    /// The set manager window is open (NF-4)
    ordering: bool,
    /// The manager's selected set, which Copy copies
    order_selected: Option<u32>,
    /// A row of the manager's list had the keyboard last frame: its keys are the list's
    order_focused: bool,
    /// Hover questions made this frame; `show` hands them out with the commands
    hover_queries: Vec<Command>,
}

impl ConfigUi {
    /// Forgets the build's state, as for another build: PoB makes a new ConfigTab for each.
    /// The custom-modifier boxes keep the heights their grips left them.
    pub fn reset(&mut self) {
        let heights = std::mem::take(&mut self.heights);
        *self = Self {
            heights,
            ..Self::default()
        };
    }

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

    /// The engine answered a hover; only the open question's answer is kept.
    pub fn on_hover(&mut self, view: &std::sync::Arc<crate::engine::views::HoverTooltipView>) {
        self.hover_tip.on_view(view);
    }

    /// The options changed under the hover's answer.
    pub fn forget_hover(&mut self) {
        self.hover_tip.forget();
    }

    /// Draws the tab; returns the commands to send, `HoverTooltip` questions among them.
    pub fn show(&mut self, ui: &mut egui::Ui, view: &ConfigView, behind: bool) -> Vec<Command> {
        let mut commands = Vec::new();
        if !behind {
            self.bools.clear();
            self.picks.clear();
        }
        self.top_row(ui, view, &mut commands);
        self.order_window(&ui.ctx().clone(), view, &mut commands);
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
        commands.append(&mut self.hover_queries);
        commands
    }

    // ---- top row ------------------------------------------------------------------------

    fn top_row(&mut self, ui: &mut egui::Ui, view: &ConfigView, commands: &mut Vec<Command>) {
        let active = view.sets.iter().find(|s| s.id == view.active_set);
        ui.horizontal_wrapped(|ui| {
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("config_set")
                .selected_text(active.map_or("", |s| s.title.as_str()))
                .show_ui(ui, |ui| {
                    for set in &view.sets {
                        let selected = set.id == view.active_set;
                        let item = crate::widgets::dropdown::item(ui, selected, &set.title);
                        self.note(format!("set:{}", set.id), &item);
                        if item.clicked() && !selected {
                            commands.push(Command::SelectConfigSet { id: set.id });
                        }
                    }
                });
            self.note("set_combo".into(), &combo.response);
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            let at = view
                .sets
                .iter()
                .position(|s| s.id == view.active_set)
                .unwrap_or(0);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, view.sets.len(), at)
            {
                commands.push(Command::SelectConfigSet {
                    id: view.sets[i].id,
                });
            }
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
        if let Some(id) = self.confirm_delete {
            let title = view
                .sets
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.title.as_str());
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Delete {}?", title.unwrap_or("this set")),
            );
            let yes = ui.add(theme::primary_button("Delete"));
            self.note("set_delete_yes".into(), &yes);
            if yes.clicked() {
                commands.push(Command::DeleteConfigSet { id });
                self.confirm_delete = None;
            }
            if ui.button("Cancel").clicked() {
                self.confirm_delete = None;
            }
            return;
        }
        if let Some(edit) = &mut self.set_edit {
            let (title, edited) = match edit {
                SetEdit::New(t) | SetEdit::Rename { title: t, .. } => (t, None),
                SetEdit::Copy {
                    title: t, edited, ..
                } => (t, Some(edited)),
            };
            let field = ui.add(
                egui::TextEdit::singleline(title)
                    .hint_text("Set name")
                    .desired_width(140.0),
            );
            field.request_focus();
            let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let typed = match edited {
                Some(edited) => {
                    *edited |= field.changed();
                    *edited
                }
                None => true,
            };
            let valid = typed && !title.trim().is_empty();
            let ok = ui.add_enabled(valid, theme::primary_button("OK"));
            let cancel = ui.button("Cancel");
            if (ok.clicked() || submit) && valid {
                let title = title.trim().to_owned();
                commands.push(match edit {
                    SetEdit::New(_) => Command::NewConfigSet { title },
                    SetEdit::Rename { id, .. } => Command::RenameConfigSet { id: *id, title },
                    SetEdit::Copy { id, .. } => Command::CopyConfigSet { id: *id, title },
                });
                self.set_edit = None;
            } else if cancel.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.set_edit = None;
            }
            // the gone box gives the keyboard back, so the manager's list keys work again
            if self.set_edit.is_none() {
                field.surrender_focus();
            }
            self.note("set_ok".into(), &ok);
            return;
        }
        let new = ui.button("New");
        self.note("set_new".into(), &new);
        if new.clicked() {
            self.set_edit = Some(SetEdit::New(String::new()));
        }
        if ui.button("Rename").clicked() {
            self.set_edit = Some(SetEdit::Rename {
                id: view.active_set,
                title: active_title.unwrap_or_default().to_owned(),
            });
        }
        // the last set can't be deleted
        let delete = ui.add_enabled(view.sets.len() > 1, egui::Button::new("Delete"));
        self.note("set_delete".into(), &delete);
        if delete.clicked() {
            self.confirm_delete = Some(view.active_set);
        }
        // PoB's manager also copies a set and reorders the list (NF-4)
        let copy = ui.button("Copy");
        self.note("set_copy".into(), &copy);
        if copy.clicked() {
            // the open manager's selected row, else the current set
            let id = self
                .order_selected
                .filter(|id| self.ordering && view.sets.iter().any(|s| s.id == *id))
                .unwrap_or(view.active_set);
            let title = view
                .sets
                .iter()
                .find(|s| s.id == id)
                .map_or(String::new(), |s| s.title.clone());
            self.set_edit = Some(SetEdit::Copy {
                id,
                title,
                edited: false,
            });
        }
        let order = ui.button("Manage\u{2026}");
        self.note("set_order".into(), &order);
        if order.clicked() {
            self.ordering = !self.ordering;
            // the list opens on the current set, and its selection goes with it
            self.order_selected = self.ordering.then_some(view.active_set);
        }
    }

    /// The set order window: PoB's manager list, whose rows drag to a new place (NF-4).
    fn order_window(
        &mut self,
        ctx: &egui::Context,
        view: &ConfigView,
        commands: &mut Vec<Command>,
    ) {
        if !self.ordering {
            return;
        }
        // its list's keys (ListControl.lua:375-392): the arrows, Home and End pick a row, F2
        // renames it (ConfigSetListControl.lua's OnSelKeyDown) and Delete or Backspace asks to
        // delete it. They are the list's own: only while it is the selected control (one of
        // its rows has the keyboard), and they wait for a question and a name being typed.
        if self.order_focused
            && self.set_edit.is_none()
            && self.confirm_delete.is_none()
            && !ctx.text_edit_focused()
        {
            let at = view
                .sets
                .iter()
                .position(|s| Some(s.id) == self.order_selected);
            let len = view.sets.len();
            if let Some(i) = ctx.input_mut(|i| crate::ui::shortcuts::list_key(i, len, at)) {
                self.order_selected = Some(view.sets[i].id);
            }
            let (f2, delete) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::F2),
                    crate::ui::shortcuts::list_delete(i),
                )
            });
            if let Some(set) = view.sets.iter().find(|s| Some(s.id) == self.order_selected) {
                if f2 {
                    self.set_edit = Some(SetEdit::Rename {
                        id: set.id,
                        title: set.title.clone(),
                    });
                } else if delete && len > 1 {
                    self.confirm_delete = Some(set.id);
                }
            }
        }
        let mut open = true;
        egui::Window::new("Manage Config Sets")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Click a set to pick it for Copy; drag it to a new place.");
                let selected = self.order_selected;
                let rows = reorder_rows(
                    ui,
                    "config_sets",
                    view.sets.len(),
                    true,
                    false,
                    ConfigSetDrag,
                    |p: &ConfigSetDrag| p.0,
                    |ui, i| {
                        let set = &view.sets[i];
                        let label = if set.id == view.active_set {
                            format!("{} (current)", set.title)
                        } else {
                            set.title.clone()
                        };
                        let fill = if selected == Some(set.id) {
                            ui.visuals().selection.bg_fill
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        egui::Frame::NONE
                            .fill(fill)
                            .inner_margin(2.0)
                            .show(ui, |ui| {
                                ui.set_min_width(220.0);
                                ui.add(egui::Label::new(label).selectable(false));
                            });
                    },
                );
                // a click on a row selects the list; a click anywhere else, but in the questions
                // it opens, selects another control (ControlHost.lua:47-52)
                let press = ui.input(|i| {
                    i.pointer
                        .button_pressed(egui::PointerButton::Primary)
                        .then(|| i.pointer.interact_pos())
                        .flatten()
                });
                for (i, row) in rows.rows.iter().enumerate() {
                    self.note(format!("order_row:{i}"), row);
                    if row.clicked() {
                        self.order_selected = Some(view.sets[i].id);
                    }
                    if self.order_focused {
                        crate::ui::shortcuts::keep_arrows(ui.ctx(), row.id);
                    }
                }
                if let Some(pos) = press
                    && self.set_edit.is_none()
                    && self.confirm_delete.is_none()
                {
                    self.order_focused = rows.rows.iter().any(|r| r.rect.contains(pos));
                }
                if let Some((from, to)) = rows.moved {
                    commands.push(Command::MoveConfigSet { from, to });
                }
            });
        if !open {
            self.ordering = false;
            self.order_selected = None;
            self.order_focused = false;
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
        // PoB's own tooltip, with its comparison, replaces the static one once it is here
        let now = ui.input(|i| i.time);
        let target = HoverTarget::ConfigOption {
            var: option.var.clone(),
        };
        let shown = self
            .hover_tip
            .track(&widget, target, now, &mut self.hover_queries);
        if !shown && !option.tooltip.is_empty() {
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
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt(("config_list", var))
                    .selected_text(pob_text(shown, 14.0, default))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for choice in &option.choices {
                            let selected = choice.value == current;
                            let item = crate::widgets::dropdown::item(
                                ui,
                                selected,
                                pob_text(&choice.label, 14.0, default),
                            );
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
                let at = option
                    .choices
                    .iter()
                    .position(|c| c.value == current)
                    .unwrap_or(0);
                if let Some(i) = crate::widgets::scroll_step::combo_step(
                    ui,
                    &combo.response,
                    option.choices.len(),
                    at,
                ) {
                    let value = option.choices[i].value.clone();
                    self.picks.insert(var.to_owned(), value.clone());
                    commands.push(Command::SetConfig {
                        var: var.to_owned(),
                        value: ConfigValue::Choice(value),
                    });
                }
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

    /// A resizable box's height: PoB's 80 until its grip moves it (ConfigTab.lua:70)
    fn box_height(&self, key: &str) -> f32 {
        self.heights.get(key).copied().unwrap_or(80.0)
    }

    /// A text field that shows what the user typed until the engine has caught up. Returns
    /// the text to send, if the field's edit is due to be sent.
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
            let now = ui.input(|i| i.time);
            let target = HoverTarget::CustomModEnabled { group: i };
            self.hover_tip
                .track(&toggle, target, now, &mut self.hover_queries);
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
            // PoB's "Add Mod": its Mod Browser appends the picked modifier (R28)
            let add_mod = ui.button("Add Mod\u{2026}");
            self.note(format!("mod_add:{i}"), &add_mod);
            if add_mod.clicked() {
                commands.push(Command::OpenPopup(PopupKind::ModBrowser { group: i }));
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

    // ---- Task 13: hover tooltips (R22), set Copy and order (NF-4) ----

    use crate::engine::HoverTarget;
    use crate::engine::views::{ConfigSet, CustomModGroup, HoverTooltipView, TooltipLine};
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
    use std::sync::Arc;

    struct Harness {
        ui: ConfigUi,
        view: ConfigView,
        ctx: egui::Context,
        time: f64,
    }

    impl Harness {
        fn new() -> Harness {
            let mut check = option("Do you have Onslaught?");
            check.var = "buffOnslaught".into();
            let view = ConfigView {
                sets: vec![
                    ConfigSet {
                        id: 1,
                        title: "First".into(),
                    },
                    ConfigSet {
                        id: 2,
                        title: "Second".into(),
                    },
                ],
                active_set: 1,
                custom_mods: vec![CustomModGroup {
                    title: "Default".into(),
                    enabled: true,
                    text: "+5 to Strength".into(),
                }],
                sections: vec![ConfigSection {
                    name: "General".into(),
                    column: 1,
                    options: vec![check],
                }],
            };
            let mut h = Harness {
                ui: ConfigUi::default(),
                view,
                ctx: egui::Context::default(),
                time: 1.0,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<Command> {
            self.time += 0.06;
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                time: Some(self.time),
                events,
                ..RawInput::default()
            };
            let mut out = vec![];
            let view = self.view.clone();
            let output = self.ctx.run_ui(input, |ui| {
                // egui may run a frame twice; each pass's output counts
                out.extend(self.ui.show(ui, &view, false));
            });
            output.drop_without_applying_deltas();
            out
        }

        fn pos(&self, key: &str) -> Pos2 {
            self.ui
                .rects
                .get(key)
                .unwrap_or_else(|| panic!("no widget {key}"))
                .center()
        }

        fn click(&mut self, key: &str) -> Vec<Command> {
            let pos = self.pos(key);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = vec![];
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                all.extend(self.frame(events));
            }
            all
        }

        /// Frames with the pointer over `key` until a hover question comes out
        fn hover(&mut self, key: &str) -> Vec<Command> {
            let pos = self.pos(key);
            let mut all = vec![];
            for _ in 0..4 {
                all.extend(self.frame(vec![Event::PointerMoved(pos)]));
            }
            all
        }
    }

    fn debug(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|c| format!("{c:?}")).collect()
    }

    #[test]
    fn config_hover_asks_once_and_shows_the_answer() {
        let mut h = Harness::new();
        let pos = h.pos("check:buffOnslaught");
        // under PoB's delay nothing is asked
        assert!(h.frame(vec![Event::PointerMoved(pos)]).is_empty());
        let target = HoverTarget::ConfigOption {
            var: "buffOnslaught".into(),
        };
        let sent = h.hover("check:buffOnslaught");
        assert_eq!(
            debug(&sent),
            debug(&[Command::HoverTooltip(target.clone())])
        );
        assert_eq!(h.ui.hover_tip.asked(), Some(&target));
        h.ui.on_hover(&Arc::new(HoverTooltipView {
            target: target.clone(),
            lines: vec![TooltipLine::Line {
                size: 14,
                text: "^7Toggling this option will give you:".into(),
            }],
        }));
        assert!(h.ui.hover_tip.answered(), "the answer was dropped");
        assert!(h.hover("check:buffOnslaught").is_empty(), "asked once");
        // a stale answer to another question isn't kept
        h.ui.forget_hover();
        h.ui.on_hover(&Arc::new(HoverTooltipView {
            target: HoverTarget::ConfigOption {
                var: "other".into(),
            },
            lines: vec![],
        }));
        assert!(!h.ui.hover_tip.answered());
        // a new view of the options asks again
        h.ui.forget_hover();
        assert_eq!(
            debug(&h.hover("check:buffOnslaught")),
            debug(&[Command::HoverTooltip(target)])
        );
    }

    #[test]
    fn custom_group_enable_hover_names_the_group() {
        let mut h = Harness::new();
        assert_eq!(
            debug(&h.hover("mod_enabled:0")),
            debug(&[Command::HoverTooltip(HoverTarget::CustomModEnabled {
                group: 0
            })])
        );
    }

    #[test]
    fn copy_asks_for_a_name_and_sends_copy_config_set() {
        let mut h = Harness::new();
        assert!(h.click("set_copy").is_empty());
        // PoB's Save waits for the name to be edited
        assert!(h.click("set_ok").is_empty());
        h.frame(vec![Event::Text("2".into())]);
        let sent = h.click("set_ok");
        assert_eq!(debug(&sent), ["CopyConfigSet { id: 1, title: \"First2\" }"]);
        // a row picked in the manager is what Copy copies
        h.click("set_order");
        h.frame(vec![]);
        h.click("order_row:1");
        h.click("set_copy");
        h.frame(vec![Event::Text("!".into())]);
        assert_eq!(
            debug(&h.click("set_ok")),
            ["CopyConfigSet { id: 2, title: \"Second!\" }"]
        );
    }

    fn key(h: &mut Harness, key: egui::Key) -> Vec<Command> {
        h.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }])
    }

    /// F2 in "Manage Config Sets" renames its selected row, not the active set
    /// (ConfigSetListControl.lua's OnSelKeyDown); off the window F2 does nothing
    #[test]
    fn f2_in_the_config_manager_renames_the_selected_set() {
        let mut h = Harness::new();
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        assert!(h.ui.set_edit.is_none(), "F2 off the manager renamed");
        h.click("set_order");
        h.frame(vec![]);
        h.click("order_row:1");
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        h.frame(vec![Event::Text("!".into())]);
        assert_eq!(
            debug(&h.click("set_ok")),
            ["RenameConfigSet { id: 2, title: \"Second!\" }"]
        );
        // Up then F2 picks the first set, once the name box is gone
        h.frame(vec![]);
        h.frame(vec![]);
        key(&mut h, egui::Key::ArrowUp);
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        assert!(matches!(h.ui.set_edit, Some(SetEdit::Rename { id: 1, .. })));
    }

    /// PoB's Copy belongs to the manager's list: its selected row counts only while the
    /// window is open
    #[test]
    fn copy_takes_the_managers_row_only_while_it_is_open() {
        let mut h = Harness::new();
        h.click("set_order");
        h.frame(vec![]);
        h.click("order_row:1");
        h.click("set_copy");
        assert!(matches!(h.ui.set_edit, Some(SetEdit::Copy { id: 2, .. })));
        h.ui.set_edit = None;
        // closed, Copy copies the current set
        h.click("set_order");
        h.frame(vec![]);
        h.click("set_copy");
        assert!(matches!(h.ui.set_edit, Some(SetEdit::Copy { id: 1, .. })));
    }

    /// Delete in the manager asks about its selected set, which is what Delete then removes
    #[test]
    fn delete_in_the_config_manager_asks_about_the_selected_set() {
        let mut h = Harness::new();
        h.click("set_order");
        h.frame(vec![]);
        // the list's keys wait until it is selected
        key(&mut h, egui::Key::Delete);
        h.frame(vec![]);
        assert!(
            h.ui.confirm_delete.is_none(),
            "Delete asked before the list was selected"
        );
        h.click("order_row:0");
        key(&mut h, egui::Key::ArrowDown);
        key(&mut h, egui::Key::Delete);
        h.frame(vec![]);
        assert_eq!(
            debug(&h.click("set_delete_yes")),
            ["DeleteConfigSet { id: 2 }"]
        );
        // a click elsewhere on the tab selects another control: Delete there asks nothing
        h.click("order_row:0");
        h.click("show_all");
        key(&mut h, egui::Key::Delete);
        h.frame(vec![]);
        assert!(h.ui.confirm_delete.is_none(), "Delete off the list asked");
    }

    #[test]
    fn dragging_a_config_set_sends_move_config_set() {
        let mut h = Harness::new();
        h.click("set_order");
        h.frame(vec![]);
        let from = h.pos("order_row:0");
        let r = h.ui.rects["order_row:1"];
        let to = Pos2::new(r.center().x, r.bottom() - 2.0);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut all = vec![];
        for events in [
            vec![Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![Event::PointerMoved(Pos2::new(from.x + 30.0, from.y + 6.0))],
            vec![Event::PointerMoved(to)],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
            vec![],
        ] {
            all.extend(h.frame(events));
        }
        let moves: Vec<_> = debug(&all)
            .into_iter()
            .filter(|c| c.starts_with("Move"))
            .collect();
        assert_eq!(moves, ["MoveConfigSet { from: 0, to: 1 }"]);
    }
}
