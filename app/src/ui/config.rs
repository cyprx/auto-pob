//! The Config tab: PoB's configuration options by section, config sets and custom modifiers.
//!
//! The tab never owns the build's state. It draws the latest `ConfigView` and returns
//! commands. While the engine is behind (spec 6.3) a field keeps what the user typed or
//! picked, and goes back to showing the snapshot once the engine has caught up.

use crate::engine::views::{
    ConfigKind, ConfigOption, ConfigSection, ConfigSet, ConfigView, CustomModGroup,
};
use crate::engine::{Command, ConfigValue, HoverTarget, PopupKind};
use crate::theme;
use crate::ui::hover::HoverTip;
use crate::ui::items::list::reorder_rows;
use crate::ui::name_box::{self, Filter};
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
    /// Nothing typed, or text that is no number: clears the option
    Empty,
    Valid(f64),
    /// A number this field doesn't send; nothing is sent
    Invalid,
}

/// Parses what the user typed into a count, integer or float field as PoB does: its box sets
/// the option to `tonumber(buf)` (ConfigTab.lua:273-280), so text that is no number, such as
/// "-" alone, clears it, and a count of 0 is stored. The box's filter leaves digits, "-" and
/// "." only, so no exponents. A negative count is not sent: the countAllowZero box steps no
/// lower than 0 (controller ruling, review I4).
pub fn parse_number_input(text: &str, kind: ConfigKind, _allow_zero: bool) -> NumberInput {
    let text = text.trim();
    if text.is_empty() {
        return NumberInput::Empty;
    }
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits_only = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // Lua's number syntax, as far as the filter lets it be typed
    let well_formed = match body.split_once('.') {
        None => digits_only(body),
        Some((int, frac)) => {
            (digits_only(int) || int.is_empty())
                && (digits_only(frac) || frac.is_empty())
                && !(int.is_empty() && frac.is_empty())
        }
    };
    // tonumber gives nil: PoB's option is cleared
    let Some(value) = well_formed.then(|| body.parse::<f64>().ok()).flatten() else {
        return NumberInput::Empty;
    };
    let value = if negative { -value } else { value };
    match kind {
        // the engine takes whole numbers only for these; PoB's filter has no "." for them
        ConfigKind::Count | ConfigKind::Integer if value.fract() != 0.0 => NumberInput::Invalid,
        ConfigKind::Count if value < 0.0 => NumberInput::Invalid,
        _ => NumberInput::Valid(value),
    }
}

/// Whether the option's label matches `query` as PoB's search does (ConfigTab.lua:172-183):
/// the label without colour codes and the query both lowercased, an all-space query matches
/// everything, and the query is `string.matchOrPattern`'s (Common.lua:976-1016) with every
/// other Lua magic character escaped: `|` separates alternatives, `(a|b)` expands into them,
/// and each alternative is found as plain text.
pub fn matches_search(option: &ConfigOption, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    if query
        .chars()
        .all(|c| c == ' ' || ('\t'..='\r').contains(&c))
    {
        return true;
    }
    let default = eframe::egui::Color32::WHITE;
    let label = pob_text(&option.label, 14.0, default)
        .text
        .to_ascii_lowercase();
    // a parenthesis left over is an unbalanced capture: Lua raises, which fails the whole
    // matchOrPattern (PCall), whatever the other alternatives say
    let patterns = or_patterns(&query);
    if patterns.iter().any(|p| p.contains(['(', ')'])) {
        return false;
    }
    patterns.iter().any(|p| label.contains(p.as_str()))
}

/// `matchOrPattern`'s alternatives of `pattern` (its `generateOrPatterns`)
fn or_patterns(pattern: &str) -> Vec<String> {
    // the balanced groups `%b()` finds, scanning left to right
    let bytes = pattern.as_bytes();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut depth = 0;
            let close = (i..bytes.len()).find(|&j| {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                depth == 0
            });
            if let Some(close) = close {
                groups.push((i, close));
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    let mut generated = vec![pattern[..groups.first().map_or(pattern.len(), |g| g.0)].to_owned()];
    for (n, &(open, close)) in groups.iter().enumerate() {
        let next = groups.get(n + 1).map_or(pattern.len(), |g| g.0);
        let regular = &pattern[close + 1..next];
        let inner = or_patterns(&pattern[open + 1..close]);
        generated = generated
            .iter()
            .flat_map(|g| inner.iter().map(move |sub| format!("{g}{sub}{regular}")))
            .collect();
    }
    let mut out: Vec<String> = Vec::new();
    for g in &generated {
        for piece in g.split('|').filter(|p| !p.is_empty()) {
            if !out.iter().any(|o| o == piece) {
                out.push(piece.to_owned());
            }
        }
    }
    out
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
    /// `edited` once it was typed into, as PoB's Save stays disabled until then too
    Rename {
        id: u32,
        title: String,
        edited: bool,
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

/// How an integer box steps (EditControl.lua:680-700)
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// `%D`: down only from a number above 0
    Unsigned,
    /// `^%-%d`: down below 0
    Signed,
    /// countAllowZero's `^%-%d` box, whose value can't be negative here (parse_number_input):
    /// it steps as PoB's but never below 0, placeholder or not (controller ruling, review I4)
    Floor0,
}

/// What a field's PoB box takes: its filter and byte limit, and for an integer box whether
/// it goes below 0 (ConfigTab.lua:273: `^%-%d` for integer and countAllowZero, `%D` for count,
/// `^%d.` for float, at most 10 characters)
#[derive(Clone, Copy)]
struct Accept {
    keeps: fn(char) -> bool,
    limit: usize,
    /// An integer box, which steps on + and -, Up, Down and the wheel
    number: Option<Step>,
}

impl Accept {
    fn for_option(kind: ConfigKind, allow_zero: bool) -> Option<Self> {
        let (keeps, number): (fn(char) -> bool, _) = match kind {
            ConfigKind::Integer => (|c| c.is_ascii_digit() || c == '-', Some(Step::Signed)),
            // PoB's countAllowZero box takes "-" and steps below 0, but a negative count is no
            // value here (parse_number_input), so its step stops at 0
            ConfigKind::Count if allow_zero => {
                (|c| c.is_ascii_digit() || c == '-', Some(Step::Floor0))
            }
            ConfigKind::Count => (|c| c.is_ascii_digit(), Some(Step::Unsigned)),
            ConfigKind::Float => (|c| c.is_ascii_digit() || c == '.', None),
            // "^%C\t\n" (ConfigTab.lua:289)
            ConfigKind::Text => {
                return Some(Self {
                    keeps: |c| !c.is_control() || c == '\t' || c == '\n',
                    limit: usize::MAX,
                    number: None,
                });
            }
            _ => return None,
        };
        Some(Self {
            keeps,
            limit: 10,
            number,
        })
    }
}

/// PoB's step of an integer box with `placeholder` (EditControl.lua:680-700): up adds one to
/// its number, else to the placeholder's, else makes it 1; down takes one from its number
/// (above 0 in a `%D` box), else from the placeholder's, else makes it 0
fn step_number(text: &str, placeholder: &str, up: bool, step: Step) -> String {
    let number = |t: &str| t.trim().parse::<i64>().ok();
    let delta = if up { 1 } else { -1 };
    let stepped = match number(text) {
        Some(n) if up || step != Step::Unsigned || n > 0 => n + delta,
        _ => match number(placeholder) {
            Some(p) => p + delta,
            None => i64::from(up),
        },
    };
    let stepped = if step == Step::Floor0 {
        stepped.max(0)
    } else {
        stepped
    };
    stepped.to_string()
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
enum FieldStyle<'a> {
    Single {
        width: f32,
    },
    /// `colours`: PoB's colour code for each line of the view's text while the box isn't
    /// selected (the custom modifier box's `inactiveText`, ConfigTab.lua:78-86)
    Multi {
        rows: usize,
        colours: &'a [String],
    },
}

/// PoB's unselected custom modifier box: each line of `text` in its colour code, a line past
/// the codes in `default` (ConfigTab.lua:78-86). DrawString still reads the codes inside a
/// line; with Alt held the line shows stripped of them and trimmed.
fn coloured_lines(
    text: &str,
    colours: &[String],
    font: egui::FontId,
    default: egui::Color32,
    alt: bool,
) -> egui::text::LayoutJob {
    use crate::widgets::pob_text::{append_pob_text, pob_colour, strip_escapes};
    let mut job = egui::text::LayoutJob::default();
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let colour = colours
            .get(i)
            .map_or(default, |code| pob_colour(code, default));
        let (body, end) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        let shown = if alt {
            let lua_space = |c: char| c == ' ' || ('\t'..='\r').contains(&c);
            strip_escapes(body).trim_matches(lua_space).to_owned()
        } else {
            body.to_owned()
        };
        append_pob_text(&mut job, &format!("{shown}{end}"), &font, colour);
    }
    job.keep_trailing_whitespace = true;
    job
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
    /// Ctrl+F: the search box takes the keyboard when next drawn (ConfigTab.lua:987-988)
    pub focus_search: bool,
    /// The custom-modifier boxes' heights, as their grips left them
    heights: HashMap<String, f32>,
    /// The field that had the keyboard last frame: PoB colours a custom modifier box's lines
    /// only while it isn't selected
    field_focus: Option<String>,
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

    /// "Manage Config Sets" is open
    #[cfg(test)]
    pub(crate) fn manager_open(&self) -> bool {
        self.ordering
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
            // PoB's selector takes a pick only with more than one set (ConfigTab.lua:157-159)
            let combo = ui
                .add_enabled_ui(view.sets.len() > 1, |ui| {
                    crate::widgets::dropdown::Dropdown::from_id_salt("config_set")
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
                        })
                })
                .inner;
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
            self.set_buttons(ui, view, active.map(ConfigSet::name_box_text), commands);
            ui.separator();
            let before = self.search.clone();
            let pasted = name_box::pasted(ui);
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search options")
                    .desired_width(160.0),
            );
            self.note("search".into(), &search);
            crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &search, &self.search);
            // PoB's box drops control characters and takes 100 bytes (ConfigTab.lua:165)
            if search.changed() {
                let raw = std::mem::take(&mut self.search);
                self.search = name_box::spliced_by(&before, &raw, |c| !c.is_control(), 100, pasted);
                name_box::place_caret(ui.ctx(), search.id, &raw, &self.search);
            }
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
        active_title: Option<String>,
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
                // PoB's question (ConfigSetListControl.lua:92)
                format!(
                    "Are you sure you want to delete '{}'?",
                    title.unwrap_or("Default")
                ),
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
            // PoB's name box: `EditControl`'s own filter, 100 bytes, and a Save that waits for
            // an edit and a visible character (ConfigSetListControl.lua:43-69)
            let (title, edited) = match edit {
                SetEdit::New(t) => (t, None),
                SetEdit::Rename {
                    title: t, edited, ..
                }
                | SetEdit::Copy {
                    title: t, edited, ..
                } => (t, Some(edited)),
            };
            let (field, changed) =
                name_box::edit(ui, title, Filter::Plain, name_box::NAME_LIMIT, |t| {
                    t.hint_text("Set name").desired_width(140.0)
                });
            field.request_focus();
            let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let typed = match edited {
                Some(edited) => {
                    *edited |= changed;
                    *edited
                }
                None => true,
            };
            let valid = typed && name_box::has_visible(title);
            let ok = ui.add_enabled(valid, theme::primary_button("OK"));
            let cancel = ui.button("Cancel");
            if (ok.clicked() || submit) && valid {
                // PoB saves the box as it stands
                let title = title.clone();
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
                title: active_title.unwrap_or_default(),
                edited: false,
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
                .map_or(String::new(), |s| s.name_box_text());
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
                        title: set.name_box_text(),
                        edited: false,
                    });
                } else if delete && len > 1 {
                    self.confirm_delete = Some(set.id);
                }
            }
        }
        // Esc closes it, as PoB's popup (PopupDialog.lua:72-77), once a box is left; a
        // question it asked takes Esc first
        let window = egui::Id::unique("config_set_manager");
        let mut open = self.set_edit.is_some()
            || self.confirm_delete.is_some()
            || !crate::widgets::escape::escape(ctx, window);
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
        crate::widgets::escape::note(ctx, window);
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
                // a number box's + and - sit beside it
                let (response, text) = ui
                    .horizontal(|ui| {
                        self.draft_field(
                            ui,
                            var.to_owned(),
                            &current,
                            FieldStyle::Single { width: 100.0 },
                            &hint,
                            SendOn::Change,
                            behind,
                            Accept::for_option(option.kind, option.allow_zero),
                        )
                    })
                    .inner;
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
        style: FieldStyle<'_>,
        hint: &str,
        send_on: SendOn,
        behind: bool,
        accept: Option<Accept>,
    ) -> (egui::Response, Option<String>) {
        let mut text = self
            .drafts
            .get(&key)
            .cloned()
            .unwrap_or_else(|| current.to_owned());
        let before = text.clone();
        let pasted = name_box::pasted(ui);
        let edit = match style {
            FieldStyle::Single { width } => {
                egui::TextEdit::singleline(&mut text).desired_width(width)
            }
            FieldStyle::Multi { rows, .. } => egui::TextEdit::multiline(&mut text)
                .desired_rows(rows)
                .desired_width(f32::INFINITY)
                .min_size(egui::vec2(0.0, self.box_height(&key))),
        };
        // unselected and showing the view's text, a custom modifier box colours each line as
        // PoB reads it
        let colours = match style {
            FieldStyle::Multi { colours, .. }
                if !colours.is_empty()
                    && before == current
                    && self.field_focus.as_deref() != Some(key.as_str()) =>
            {
                Some(colours)
            }
            _ => None,
        };
        // the box lays its own text out unseen, so a click still finds its place in the text,
        // and the coloured lines, codes read, are drawn over it
        let wrap = std::cell::Cell::new(f32::INFINITY);
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let font = egui::FontSelection::default().resolve(ui.style());
            wrap.set(wrap_width);
            let mut job = egui::text::LayoutJob::simple(
                buf.as_str().to_owned(),
                font,
                egui::Color32::TRANSPARENT,
                wrap_width,
            );
            job.keep_trailing_whitespace = true;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let edit = if colours.is_some() {
            edit.layouter(&mut layouter)
        } else {
            edit
        };
        let output = edit.hint_text(hint).show(ui);
        let response = output.response.response;
        if let Some(colours) = colours
            && !text.is_empty()
        {
            let font = egui::FontSelection::default().resolve(ui.style());
            let default = ui.visuals().text_color();
            let alt = ui.input(|i| i.modifiers.alt);
            let mut job = coloured_lines(&text, colours, font, default, alt);
            job.wrap.max_width = wrap.get();
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter()
                .with_clip_rect(output.text_clip_rect.intersect(ui.clip_rect()))
                .galley(output.galley_pos, galley, default);
        }
        if response.has_focus() {
            self.field_focus = Some(key.clone());
        } else if self.field_focus.as_deref() == Some(key.as_str()) {
            self.field_focus = None;
        }
        crate::widgets::url_click::open_on_ctrl_click(ui, &response, &text);
        if matches!(style, FieldStyle::Single { .. }) {
            crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &response, &text);
        }
        if matches!(style, FieldStyle::Multi { .. }) {
            crate::widgets::edit_box::page_keys(ui, &response, &text);
        }
        let mut changed = response.changed();
        if changed && let Some(accept) = accept {
            // the box's own filter and limit, for what an edit inserts
            let raw = std::mem::take(&mut text);
            text = name_box::spliced_by(&before, &raw, accept.keeps, accept.limit, pasted);
            name_box::place_caret(ui.ctx(), response.id, &raw, &text);
            // typing over the text with itself is an edit; one the box drops is none
            changed = text != before || raw == before;
        }
        if let Some(kind) = accept.and_then(|a| a.number) {
            let mut step = crate::widgets::scroll_step::number_step(ui, &response);
            let (plus, minus) = ui
                .add_enabled_ui(response.enabled(), |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    (ui.small_button("+"), ui.small_button("-"))
                })
                .inner;
            #[cfg(test)]
            self.rects.insert(format!("plus:{key}"), plus.rect);
            #[cfg(test)]
            self.rects.insert(format!("minus:{key}"), minus.rect);
            step += i32::from(plus.clicked()) - i32::from(minus.clicked());
            for _ in 0..step.unsigned_abs() {
                text = step_number(&text, hint, step > 0, kind);
            }
            if step != 0 {
                // SetText(..., true): sent at once (EditControl.lua:96-103)
                changed = true;
            }
        }
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
        if changed {
            self.drafts.insert(key.clone(), text.clone());
        }
        let send = match send_on {
            SendOn::Change => changed,
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
                Some(Accept {
                    keeps: |c| Filter::Plain.keeps(c),
                    limit: usize::MAX,
                    number: None,
                }),
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
            // PoB's red X removes the group at once; Undo brings it back (ConfigTab.lua:23-33)
            let remove = ui.button(egui::RichText::new("X").color(theme::NEGATIVE));
            self.note(format!("mod_remove:{i}"), &remove);
            if remove.clicked() {
                commands.push(Command::RemoveCustomModGroup { group: i });
            }
        });
        let (response, text) = self.draft_field(
            ui,
            format!("mod_text:{i}"),
            &group.text,
            FieldStyle::Multi {
                rows: 4,
                colours: &group.line_colours,
            },
            "One modifier per line, e.g. +10% to all Elemental Resistances",
            SendOn::Blur,
            behind,
            None,
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

    /// While it isn't selected, the custom modifier box shows each line in PoB's colour for it:
    /// MAGIC for a line PoB reads as a modifier, UNSUPPORTED for one it can't
    /// (ConfigTab.lua:78-86); selected, it is plain text
    /// DrawString still reads the colour codes inside a custom modifier line, after the
    /// line's own MAGIC or UNSUPPORTED; with Alt held the line shows stripped and trimmed
    /// (ConfigTab.lua:78-86)
    #[test]
    fn custom_mod_lines_read_their_own_colour_codes() {
        let mut h = Harness::new();
        h.view.custom_mods[0].text = " ^1+5 to ^xAABBCCStrength\nplain".into();
        h.view.custom_mods[0].line_colours = vec!["^x8888FF".into(), "^xF05050".into()];
        h.frame(vec![]);
        let magic = egui::Color32::from_rgb(0x88, 0x88, 0xFF);
        let red = egui::Color32::from_rgb(255, 0, 0);
        let custom = egui::Color32::from_rgb(0xAA, 0xBB, 0xCC);
        let unsupported = egui::Color32::from_rgb(0xF0, 0x50, 0x50);
        assert_eq!(
            h.text_colours(" +5 to Strength\nplain"),
            [
                (" ".to_owned(), magic),
                ("+5 to ".to_owned(), red),
                ("Strength\n".to_owned(), custom),
                ("plain".to_owned(), unsupported)
            ]
        );
        assert_eq!(
            h.text_colours_with("+5 to Strength\nplain", Modifiers::ALT),
            [
                ("+5 to Strength\n".to_owned(), magic),
                ("plain".to_owned(), unsupported)
            ]
        );
    }

    #[test]
    fn custom_mod_lines_take_pobs_colours_while_unselected() {
        let mut h = Harness::new();
        let text = "+5 to Strength\nnot a mod";
        h.view.custom_mods[0].text = text.into();
        h.view.custom_mods[0].line_colours = vec!["^x8888FF".into(), "^xF05050".into()];
        h.frame(vec![]);
        let magic = egui::Color32::from_rgb(0x88, 0x88, 0xFF);
        let unsupported = egui::Color32::from_rgb(0xF0, 0x50, 0x50);
        assert_eq!(
            h.text_colours(text),
            [
                ("+5 to Strength\n".to_owned(), magic),
                ("not a mod".to_owned(), unsupported)
            ]
        );
        // an empty box, as every new build has, draws through the colours too
        h.view.custom_mods[0].text = String::new();
        h.view.custom_mods[0].line_colours = vec!["^xF05050".into()];
        h.frame(vec![]);
        h.frame(vec![]);
        h.view.custom_mods[0].text = text.into();
        h.view.custom_mods[0].line_colours = vec!["^x8888FF".into(), "^xF05050".into()];
        h.frame(vec![]);
        h.click("mod_text:0");
        let runs = h.text_colours(text);
        assert!(!runs.is_empty(), "the box wasn't drawn");
        assert!(
            runs.iter().all(|(_, c)| *c != magic && *c != unsupported),
            "{runs:?}"
        );
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
        // PoB stores tonumber(buf): a count of 0 is a value, allowed to be zero or not
        assert_eq!(
            parse_number_input("0", ConfigKind::Count, false),
            Valid(0.0)
        );
        assert_eq!(parse_number_input("0", ConfigKind::Count, true), Valid(0.0));
        assert_eq!(parse_number_input("-5", ConfigKind::Count, false), Invalid);
        assert_eq!(parse_number_input("-5", ConfigKind::Count, true), Invalid);
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
        assert_eq!(
            parse_number_input("5.", ConfigKind::Float, false),
            Valid(5.0)
        );
        assert_eq!(
            parse_number_input(".5", ConfigKind::Float, false),
            Valid(0.5)
        );
        // tonumber gives nil for these: PoB's option is cleared
        for text in ["-", ".", "1e9", "1.2.3", "12a", "5-", "--5"] {
            for kind in [ConfigKind::Integer, ConfigKind::Float, ConfigKind::Count] {
                assert_eq!(
                    parse_number_input(text, kind, true),
                    Empty,
                    "{text} {kind:?}"
                );
            }
        }
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
    fn search_takes_pobs_alternatives() {
        // string.matchOrPattern (Common.lua:976-1016): `|` and `(a|b)` groups
        let o = option("Is the enemy Shocked?");
        assert!(matches_search(&o, "chilled|shocked"));
        assert!(matches_search(&o, "enemy (chilled|shocked)"));
        assert!(!matches_search(&o, "enemy (chilled|frozen)"));
        // every other character is plain text, spaces too
        assert!(!matches_search(&o, "shocked "));
        assert!(!matches_search(&o, "s.ocked"));
        // a parenthesis left open is an error to Lua: nothing matches
        assert!(!matches_search(&o, "enemy (shocked"));
        assert!(!matches_search(&o, "shocked|(x"), "the whole match fails");
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
                        untitled: false,
                    },
                    ConfigSet {
                        id: 2,
                        title: "Second".into(),
                        untitled: false,
                    },
                ],
                active_set: 1,
                custom_mods: vec![CustomModGroup {
                    title: "Default".into(),
                    enabled: true,
                    text: "+5 to Strength".into(),
                    line_colours: vec![],
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

        /// One frame; the colours of the drawn text whose whole text is `text`, a run each
        fn text_colours(&mut self, text: &str) -> Vec<(String, egui::Color32)> {
            self.text_colours_with(text, Modifiers::NONE)
        }

        /// `text_colours` with `modifiers` held
        fn text_colours_with(
            &mut self,
            text: &str,
            modifiers: Modifiers,
        ) -> Vec<(String, egui::Color32)> {
            self.time += 0.06;
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                time: Some(self.time),
                events: vec![Event::ModifiersChanged(modifiers)],
                ..RawInput::default()
            };
            let view = self.view.clone();
            let output = self.ctx.run_ui(input, |ui| {
                self.ui.show(ui, &view, false);
            });
            let mut runs = vec![];
            for clipped in &output.shapes {
                if let egui::Shape::Text(t) = &clipped.shape
                    && t.galley.job.text == text
                {
                    // the box's own text is laid out unseen under the coloured lines
                    for section in &t.galley.job.sections {
                        if section.format.color == egui::Color32::TRANSPARENT {
                            continue;
                        }
                        let range = section.byte_range.start.0..section.byte_range.end.0;
                        let part = t.galley.job.text[range].to_owned();
                        runs.push((part, section.format.color));
                    }
                }
            }
            output.drop_without_applying_deltas();
            runs
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

    /// A countAllowZero box at 0 steps down to 0, not to its placeholder less one
    /// (controller ruling, review I4)
    #[test]
    fn a_count_allow_zero_box_steps_down_no_lower_than_0() {
        let mut h = with_number(ConfigKind::Count, Some(0.0), Some(15.0));
        let n = h.view.sections[0].options.len() - 1;
        h.view.sections[0].options[n].allow_zero = true;
        h.frame(vec![]);
        assert_eq!(
            debug(&h.click("minus:n")),
            ["SetConfig { var: \"n\", value: Number(0.0) }"]
        );
    }

    fn with_number(kind: ConfigKind, value: Option<f64>, placeholder: Option<f64>) -> Harness {
        let mut h = Harness::new();
        let mut number = option("Enemy count");
        number.var = "n".into();
        number.kind = kind;
        number.value = value.map(|v| serde_json::json!(v));
        number.placeholder = placeholder.map(|v| serde_json::json!(v));
        h.view.sections[0].options.push(number);
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    /// PoB's count box is a `%D` box of 10 characters (ConfigTab.lua:273): it drops what isn't
    /// a digit, and its + steps the number, or the placeholder's, and sends it at once
    /// (EditControl.lua:680-700)
    #[test]
    fn a_count_box_takes_digits_and_steps_on_plus() {
        let mut h = with_number(ConfigKind::Count, None, Some(3.0));
        assert_eq!(
            debug(&h.click("plus:n")),
            ["SetConfig { var: \"n\", value: Number(4.0) }"]
        );
        h.click("field:n");
        let sent = h.frame(vec![Event::Text("a1b2345678901".into())]);
        assert_eq!(
            debug(&sent),
            ["SetConfig { var: \"n\", value: Number(1234567890.0) }"]
        );
    }

    /// PoB's number box sets its option to tonumber(buf) (ConfigTab.lua:273-280): a count of
    /// "0" is stored, and "-" alone, no number, clears the option
    #[test]
    fn a_count_of_0_is_sent_and_a_lone_minus_clears() {
        let mut h = with_number(ConfigKind::Count, Some(2.0), None);
        h.click("field:n");
        h.frame(vec![Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        let sent = h.frame(vec![Event::Text("0".into())]);
        assert_eq!(
            debug(&sent),
            ["SetConfig { var: \"n\", value: Number(0.0) }"]
        );
        let mut h = with_number(ConfigKind::Integer, None, None);
        h.click("field:n");
        let sent = h.frame(vec![Event::Text("-".into())]);
        assert_eq!(debug(&sent), ["SetConfig { var: \"n\", value: Clear }"]);
        let sent = h.frame(vec![Event::Text("5".into())]);
        assert_eq!(
            debug(&sent),
            ["SetConfig { var: \"n\", value: Number(-5.0) }"]
        );
    }

    #[test]
    fn a_number_steps_from_its_value_then_its_placeholder() {
        use Step::*;
        assert_eq!(step_number("5", "", true, Unsigned), "6");
        assert_eq!(step_number("", "3", true, Unsigned), "4");
        assert_eq!(step_number("", "", true, Unsigned), "1");
        // a `%D` box at 0 falls back to the placeholder, a signed one goes below
        assert_eq!(step_number("0", "", false, Unsigned), "0");
        assert_eq!(step_number("0", "7", false, Unsigned), "6");
        assert_eq!(step_number("0", "", false, Signed), "-1");
        // countAllowZero stops at 0, with a placeholder too (review I4)
        assert_eq!(step_number("0", "15", false, Floor0), "0");
        assert_eq!(step_number("0", "0", false, Floor0), "0");
        assert_eq!(step_number("", "15", false, Floor0), "14");
        assert_eq!(step_number("3", "15", false, Floor0), "2");
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
    fn the_search_box_filters_and_limits_as_pobs() {
        // ConfigTab.lua:165: "%c" and 100 bytes
        let mut h = Harness::new();
        h.click("search");
        h.frame(vec![Event::Text("a\u{7}b".into())]);
        assert_eq!(h.ui.search, "ab");
        h.frame(vec![Event::Text("x".repeat(120))]);
        assert_eq!(h.ui.search.len(), 100);
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

    /// A set's name box is PoB's (ConfigSetListControl.lua:43-69): Save waits for an edit, the
    /// box drops what `EditControl`'s filter rejects, and the name is saved as it stands
    #[test]
    fn a_set_name_follows_pobs_box() {
        let mut h = Harness::new();
        h.click("set_order");
        h.frame(vec![]);
        h.click("order_row:1");
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        // as it opened, Save is off
        assert!(h.click("set_ok").is_empty());
        assert!(h.ui.set_edit.is_some());
        // empties the box: over a selection PoB refuses a text with a character it drops
        let select_all = |h: &mut Harness| {
            h.frame(vec![Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }]);
            key(h, egui::Key::Backspace);
        };
        // PoB's box under LuaJIT drops every non-ASCII byte: nothing is left to save
        for name in ["\u{65e5}\u{672c}", "\u{3a9}\u{3bc}"] {
            select_all(&mut h);
            h.frame(vec![Event::Text(name.into())]);
            h.frame(vec![]);
            assert!(h.click("set_ok").is_empty(), "{name}");
            assert!(h.ui.set_edit.is_some(), "{name}");
        }
        // a character a key adds is kept, the rest dropped, and nothing is trimmed
        select_all(&mut h);
        h.frame(vec![Event::Text("\u{65e5}\u{672c} a ".into())]);
        h.frame(vec![]);
        assert_eq!(
            debug(&h.click("set_ok")),
            ["RenameConfigSet { id: 2, title: \" a \" }"]
        );
        // 100 bytes at most
        h.frame(vec![]);
        h.frame(vec![]);
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        select_all(&mut h);
        h.frame(vec![Event::Text("x".repeat(130))]);
        h.frame(vec![Event::Text("x".repeat(100))]);
        h.frame(vec![Event::Text("y".into())]);
        h.frame(vec![]);
        let Some(SetEdit::Rename { title, .. }) = &h.ui.set_edit else {
            panic!("no name box");
        };
        assert_eq!(title.len(), 100);
    }

    /// A set with no title (a build never saved: ConfigTab.lua:133) opens PoB's rename box empty
    #[test]
    fn renaming_an_untitled_config_set_starts_empty() {
        let mut h = Harness::new();
        h.view.sets[0].untitled = true;
        h.click("set_order");
        h.frame(vec![]);
        h.click("order_row:0");
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        h.frame(vec![Event::Text("x".into())]);
        assert_eq!(
            debug(&h.click("set_ok")),
            ["RenameConfigSet { id: 1, title: \"x\" }"]
        );
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

    /// Esc closes the manager, as PoB's popup (PopupDialog.lua:72-77)
    #[test]
    fn esc_closes_the_config_manager() {
        let mut h = Harness::new();
        h.click("set_order");
        h.frame(vec![]);
        assert!(h.ui.ordering);
        key(&mut h, egui::Key::Escape);
        h.frame(vec![]);
        assert!(!h.ui.ordering, "Esc left it open");
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
