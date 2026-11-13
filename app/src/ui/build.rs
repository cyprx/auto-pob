//! The build screen's top bar and tab strip.

use super::name_box::{self, Filter};
use super::tree::TreeTab;
use crate::engine::Command;
use crate::engine::views::{BuildView, is_loadout_marker};
use crate::widgets::{EditBox, EditBoxOptions, item_tooltip_ui, pob_text};
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildAction {
    Back,
    Save,
    SaveAs,
    Undo,
    Redo,
    Options,
    /// The loadouts' "Help >>": the Help window at its seventh section (Build.lua:311-313)
    Help,
    /// PoB's About button: the window on its version history (Main.lua:209-211)
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildTab {
    #[default]
    Tree,
    Skills,
    Items,
    Calcs,
    Config,
    Notes,
    /// PoB's Compare tab; it has no undo of its own: its copies use the target tab's (R8)
    Compare,
    Import,
}

impl BuildTab {
    /// The Config, Tree, Skills, Items and Calcs tabs have undo history; the notes editor has its
    /// own and the Import/Export tab has none.
    pub fn has_undo(self) -> bool {
        matches!(
            self,
            BuildTab::Config
                | BuildTab::Tree
                | BuildTab::Skills
                | BuildTab::Items
                | BuildTab::Calcs
        )
    }
}

/// The top bar's own state: the level box and the New Loadout name being typed.
#[derive(Default)]
pub struct TopBarState {
    level: EditBox,
    /// A keystroke went into the level box while the build was in Auto
    typed_in_auto: bool,
    /// "New Loadout" was chosen: the name being typed
    new_loadout: Option<String>,
    /// The name was edited since the window opened: PoB's Save starts disabled (Build.lua:344)
    new_loadout_edited: bool,
    /// "Help >>" was chosen in the loadouts list
    help: bool,
}

impl TopBarState {
    /// A New Loadout name is being asked for, so shortcuts must wait.
    pub fn has_dialog(&self) -> bool {
        self.new_loadout.is_some()
    }

    /// A level typed but not sent: the build has changes the engine doesn't know about yet.
    pub fn has_pending(&self) -> bool {
        self.level.has_pending() || self.typed_in_auto
    }

    /// Takes the pending level as the command that sends it: also a level retyped unchanged
    /// in Auto, which PoB takes as Manual.
    pub fn flush(&mut self) -> Option<Command> {
        let typed = std::mem::take(&mut self.typed_in_auto);
        match self.level.take_pending() {
            Some(text) => level_command(&text),
            None if typed => level_command(&self.level.text),
            None => None,
        }
    }

    /// Drops the pending level (the user chose not to save).
    pub fn discard(&mut self) {
        self.level.discard();
        self.typed_in_auto = false;
    }
}

/// What the top bar asks for in one frame.
#[derive(Debug, Default)]
pub struct TopBarOutput {
    pub action: Option<BuildAction>,
    pub commands: Vec<Command>,
}

/// The level PoB would take from the box: its `%D` filter keeps digits only, and an empty box
/// is level 1 (Build.lua:218-219); nothing is sent for text that holds no number.
fn level_command(text: &str) -> Option<Command> {
    let digits: String = text.chars().filter(char::is_ascii_digit).take(3).collect();
    let level = if digits.is_empty() {
        1
    } else {
        digits.parse().ok()?
    };
    Some(Command::SetCharacterLevel { level })
}

/// Where a widget was drawn, so tests can click it. Does nothing outside tests.
#[cfg(test)]
fn note(ui: &egui::Ui, key: &str, rect: egui::Rect) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::NULL.with(("top_bar_rect", key.to_owned())), rect));
}

#[cfg(not(test))]
fn note(_ui: &egui::Ui, _key: &str, _rect: egui::Rect) {}

/// `undo`: the tab has an engine history here; `history`: whether its undo and redo have
/// anything to do. The right half is PoB's: points, Auto/Manual, the level box, class,
/// ascendancy, bloodline and loadouts (Build.lua:186-415); the class part is the tree tab's.
pub fn top_bar(
    ui: &mut egui::Ui,
    view: &BuildView,
    undo: bool,
    history: (bool, bool),
    state: &mut TopBarState,
    tree: &mut TreeTab,
    busy: bool,
) -> TopBarOutput {
    let mut out = TopBarOutput::default();
    ui.horizontal_wrapped(|ui| {
        if ui.button("< Builds").clicked() {
            out.action = Some(BuildAction::Back);
        }
        ui.separator();
        let marker = if view.unsaved { " *" } else { "" };
        ui.strong(format!("{}{marker}", view.name));
        ui.separator();
        if ui.button("Save").clicked() {
            out.action = Some(BuildAction::Save);
        }
        if ui.button("Save as…").clicked() {
            out.action = Some(BuildAction::SaveAs);
        }
        ui.separator();
        if ui
            .add_enabled(undo && history.0, egui::Button::new("Undo"))
            .clicked()
        {
            out.action = Some(BuildAction::Undo);
        }
        if ui
            .add_enabled(undo && history.1, egui::Button::new("Redo"))
            .clicked()
        {
            out.action = Some(BuildAction::Redo);
        }
        ui.separator();
        if ui.button("Options").clicked() {
            out.action = Some(BuildAction::Options);
        }
        let about = ui.button("About");
        note(ui, "about", about.rect);
        if about.clicked() {
            out.action = Some(BuildAction::About);
        }
        ui.separator();
        tree.top_bar_points(ui, &view.progress);
        level(ui, view, state, busy, &mut out.commands);
        out.commands.extend(tree.top_bar_class(ui));
        bloodline(ui, view, &mut out.commands);
        loadouts(ui, view, state, &mut out.commands);
        if std::mem::take(&mut state.help) {
            out.action = Some(BuildAction::Help);
        }
    });
    new_loadout_dialog(ui.ctx(), state, &mut out.commands);
    out
}

/// The Auto/Manual button and the level box (`levelScalingButton`, `characterLevel`). PoB
/// applies every keystroke, and its level callback switches to Manual (Build.lua:218-224):
/// while a level is being typed the mode shows Manual, and a press of the button sends that
/// level first.
fn level(
    ui: &mut egui::Ui,
    view: &BuildView,
    state: &mut TopBarState,
    busy: bool,
    commands: &mut Vec<Command>,
) {
    let typing = state.level.has_pending() || state.typed_in_auto;
    let auto = view.level_auto && !typing;
    let mode = ui
        .button(if auto { "Auto" } else { "Manual" })
        .on_hover_text(if auto {
            "The level follows the passive points spent"
        } else {
            "The level is the one typed"
        });
    note(ui, "level_mode", mode.rect);
    if mode.clicked() {
        commands.extend(state.flush());
        commands.push(Command::SetLevelMode { auto: !auto });
    }
    ui.label("Level");
    let opts = EditBoxOptions {
        width: 36.0,
        byte_limit: Some(3),
        filter: Some(|c| c.is_ascii_digit()),
        hint: None,
    };
    let id = egui::Id::unique("top_bar_level");
    // a keystroke in Auto is a Manual level in PoB, even one that retypes the shown level;
    // one the `%D` filter or the length limit drops changes nothing (EditControl.lua:140-181)
    let before = state.level.text.clone();
    let selected = egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|s| s.cursor.char_range())
        .is_some_and(|r| !r.is_empty());
    let current = view.level.to_string();
    let (response, sent) = state.level.show(ui, id, &current, busy, &opts);
    if view.level_auto
        && response.has_focus()
        && response.changed()
        && (state.level.text != before || selected)
        && !state.level.refused
    {
        state.typed_in_auto = true;
    }
    note(ui, "level", response.rect);
    if let Some(tooltip) = &view.level_tooltip {
        response.on_hover_ui(|ui| {
            item_tooltip_ui(ui, tooltip);
        });
    }
    match sent.as_deref().and_then(level_command) {
        Some(command) => {
            state.typed_in_auto = false;
            commands.push(command);
        }
        None if state.typed_in_auto && !ui.memory(|m| m.has_focus(id)) => {
            // Esc puts the text back, but PoB's keystrokes already made it Manual
            state.typed_in_auto = false;
            commands.extend(level_command(&state.level.text));
        }
        None => {}
    }
}

/// The bloodline dropdown (`secondaryAscendDrop`); PoB enables it with more than "None"
fn bloodline(ui: &mut egui::Ui, view: &BuildView, commands: &mut Vec<Command>) {
    if view.secondary.is_empty() {
        return;
    }
    let current = view
        .secondary
        .iter()
        .find(|(id, _)| *id == view.secondary_selected)
        .map_or("None", |(_, label)| label.as_str());
    ui.add_enabled_ui(view.secondary.len() > 1, |ui| {
        let combo = crate::widgets::dropdown::Dropdown::from_id_salt("top_bar_bloodline")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (i, (id, label)) in view.secondary.iter().enumerate() {
                    let item =
                        crate::widgets::dropdown::item(ui, *id == view.secondary_selected, label);
                    note(ui, &format!("bloodline_{i}"), item.rect);
                    if item.clicked() && *id != view.secondary_selected {
                        commands.push(Command::SelectSecondaryAscendancy { id: *id });
                    }
                }
            });
        note(ui, "bloodline", combo.response.rect);
        // the wheel and the arrows step it (DropDownControl.lua:476-516)
        let at = view
            .secondary
            .iter()
            .position(|(id, _)| *id == view.secondary_selected)
            .unwrap_or(0);
        if let Some(i) =
            crate::widgets::scroll_step::combo_step(ui, &combo.response, view.secondary.len(), at)
        {
            commands.push(Command::SelectSecondaryAscendancy {
                id: view.secondary[i].0,
            });
        }
    });
}

/// The loadouts dropdown (`buildLoadouts`): PoB's headers are labels, its actions are buttons,
/// and only real loadouts are picked
fn loadouts(
    ui: &mut egui::Ui,
    view: &BuildView,
    state: &mut TopBarState,
    commands: &mut Vec<Command>,
) {
    if view.loadouts.is_empty() {
        return;
    }
    let default = ui.visuals().text_color();
    let current = view
        .loadout_selected
        .and_then(|i| view.loadouts.get(i))
        .map_or("Loadouts", String::as_str);
    let combo = crate::widgets::dropdown::Dropdown::from_id_salt("top_bar_loadouts")
        .selected_text(pob_text(current, 14.0, default))
        .width(160.0)
        .show_ui(ui, |ui| {
            for (i, entry) in view.loadouts.iter().enumerate() {
                let key = format!("loadout_{i}");
                if !is_loadout_marker(entry) {
                    let item = crate::widgets::dropdown::item(
                        ui,
                        view.loadout_selected == Some(i),
                        pob_text(entry, 14.0, default),
                    );
                    note(ui, &key, item.rect);
                    if item.clicked() && view.loadout_selected != Some(i) {
                        commands.push(Command::SelectLoadout {
                            label: entry.clone(),
                        });
                    }
                    continue;
                }
                let response = match &entry[4..] {
                    "New Loadout" => {
                        let r = crate::widgets::dropdown::button(ui, "New Loadout…");
                        if r.clicked() {
                            // PoB's box opens on "New Loadout" (Build.lua:318)
                            state.new_loadout = Some("New Loadout".to_owned());
                            state.new_loadout_edited = false;
                        }
                        r
                    }
                    "Sync" => {
                        let r = crate::widgets::dropdown::button(ui, "Sync")
                            .on_hover_text("Rebuild the list from the sets' names");
                        if r.clicked() {
                            commands.push(Command::SyncLoadouts);
                        }
                        r
                    }
                    "Help >>" => {
                        let r = crate::widgets::dropdown::button(ui, "Help >>");
                        if r.clicked() {
                            state.help = true;
                        }
                        r
                    }
                    "-----" => {
                        ui.separator();
                        continue;
                    }
                    // "Loadouts:" and any other header: a label, never a choice
                    _ => {
                        let weak = ui.visuals().weak_text_color();
                        crate::widgets::dropdown::label(ui, pob_text(entry, 14.0, weak))
                    }
                };
                note(ui, &key, response.rect);
            }
        });
    note(ui, "loadouts", combo.response.rect);
    // the wheel and the arrows step through the loadouts themselves, not the headers and
    // actions between them; a search's matches are noted for the loadouts alone
    let real: Vec<usize> = (0..view.loadouts.len())
        .filter(|&i| !is_loadout_marker(&view.loadouts[i]))
        .collect();
    let at = view
        .loadout_selected
        .and_then(|s| real.iter().position(|&i| i == s));
    if let Some(at) = at
        && let Some(i) =
            crate::widgets::scroll_step::combo_step(ui, &combo.response, real.len(), at)
    {
        commands.push(Command::SelectLoadout {
            label: view.loadouts[real[i]].clone(),
        });
    }
}

/// "New Loadout"'s name popup ("Set Name"): Save waits for an edit, then needs a name with a
/// visible character PoB's box keeps (Build.lua:319-344)
fn new_loadout_dialog(ctx: &egui::Context, state: &mut TopBarState, commands: &mut Vec<Command>) {
    let Some(name) = &mut state.new_loadout else {
        return;
    };
    let edited = &mut state.new_loadout_edited;
    let mut close = false;
    egui::Window::new("Set Name")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Enter name for this loadout:");
            // the box drops what PoB's drops, and takes 100 bytes (Build.lua:319)
            let (field, changed) =
                name_box::edit(ui, name, Filter::Plain, name_box::NAME_LIMIT, |t| t);
            note(ui, "new_loadout_name", field.rect);
            *edited |= changed;
            // the window opens on its box, as PoB's does
            if ui.memory(|m| m.focused().is_none()) {
                field.request_focus();
            }
            // Esc is PoB's Cancel (its popup closes on Escape)
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let valid = *edited && name_box::has_visible(name);
            ui.horizontal(|ui| {
                let save = ui.add_enabled(valid, egui::Button::new("Save"));
                note(ui, "new_loadout_save", save.rect);
                if (save.clicked() || enter) && valid {
                    commands.push(Command::NewLoadout { name: name.clone() });
                    close = true;
                }
                close |= ui.button("Cancel").clicked();
            });
        });
    if close {
        state.new_loadout = None;
    }
}

pub fn tab_strip(ui: &mut egui::Ui, tab: &mut BuildTab) {
    ui.horizontal(|ui| {
        ui.selectable_value(tab, BuildTab::Tree, "Tree");
        ui.selectable_value(tab, BuildTab::Skills, "Skills");
        ui.selectable_value(tab, BuildTab::Items, "Items");
        ui.selectable_value(tab, BuildTab::Calcs, "Calcs");
        ui.selectable_value(tab, BuildTab::Config, "Config");
        ui.selectable_value(tab, BuildTab::Notes, "Notes");
        ui.selectable_value(tab, BuildTab::Compare, "Compare");
        ui.selectable_value(tab, BuildTab::Import, "Import/Export");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers, PointerButton, RawInput, Rect};

    struct Harness {
        ctx: egui::Context,
        view: BuildView,
        state: TopBarState,
        tree: TreeTab,
        /// The last action the top bar asked for
        action: Option<BuildAction>,
    }

    impl Harness {
        fn new(view: BuildView) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                view,
                state: TopBarState::default(),
                tree: TreeTab::default(),
                action: None,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<String> {
            let input = RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(2000.0, 800.0),
                )),
                ..Default::default()
            };
            let mut out = TopBarOutput::default();
            let (view, state, tree) = (&self.view, &mut self.state, &mut self.tree);
            self.ctx
                .run_ui(input, |ui| {
                    egui::Panel::top("top").show(ui, |ui| {
                        out = top_bar(ui, view, true, (false, false), state, tree, false);
                    });
                })
                .drop_without_applying_deltas();
            self.action = out.action.or(self.action);
            out.commands.iter().map(|c| format!("{c:?}")).collect()
        }

        fn rect(&self, key: &str) -> Option<Rect> {
            self.ctx
                .data(|d| d.get_temp(egui::Id::NULL.with(("top_bar_rect", key.to_owned()))))
        }

        fn click(&mut self, key: &str) -> Vec<String> {
            let pos = self
                .rect(key)
                .unwrap_or_else(|| panic!("no widget {key}"))
                .center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = Vec::new();
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

        fn key(&mut self, key: Key) -> Vec<String> {
            let mut all = Vec::new();
            for pressed in [true, false] {
                all.extend(self.frame(vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]));
            }
            all
        }

        /// Selects everything in the focused box, then types `text`
        fn retype(&mut self, text: &str) -> Vec<String> {
            let mut sent = self.frame(vec![Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }]);
            sent.extend(self.frame(vec![Event::Text(text.into())]));
            sent
        }

        /// A click where nothing is drawn
        fn click_away(&mut self) -> Vec<String> {
            let pos = egui::pos2(1900.0, 700.0);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = Vec::new();
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

        /// Opens a combo box and lets its popup lay out
        fn open(&mut self, key: &str) -> Vec<String> {
            let mut sent = self.click(key);
            for _ in 0..3 {
                sent.extend(self.frame(vec![]));
            }
            sent
        }
    }

    fn view() -> BuildView {
        BuildView {
            name: "b".into(),
            class: "Witch".into(),
            level: 90,
            secondary: vec![(0, "None".into()), (8, "Aul Bloodline".into())],
            loadouts: [
                "^7^7Loadouts:",
                "Default",
                "L2",
                "^7^7-----",
                "^7^7New Loadout",
                "^7^7Sync",
                "^7^7Help >>",
            ]
            .map(String::from)
            .to_vec(),
            loadout_selected: Some(1),
            ..BuildView::default()
        }
    }

    #[test]
    fn level_box_sends_on_enter() {
        let mut h = Harness::new(view());
        let mut sent = h.click("level");
        // the box keeps PoB's `%D` filter and its three characters
        sent.extend(h.frame(vec![Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]));
        sent.extend(h.frame(vec![Event::Text("7x5".into())]));
        assert!(sent.is_empty(), "nothing goes out while typing: {sent:?}");
        assert!(h.state.has_pending());
        sent.extend(h.key(Key::Enter));
        assert_eq!(sent, ["SetCharacterLevel { level: 75 }"]);

        // Escape puts back the build's level and sends nothing
        let mut h = Harness::new(view());
        let mut sent = h.click("level");
        sent.extend(h.frame(vec![Event::Text("1".into())]));
        sent.extend(h.key(Key::Escape));
        assert!(sent.is_empty());
        assert!(!h.state.has_pending());
        assert_eq!(h.state.level.text, "90");
    }

    #[test]
    fn level_mode_toggles() {
        let mut h = Harness::new(view());
        assert_eq!(h.click("level_mode"), ["SetLevelMode { auto: true }"]);
        let mut h = Harness::new(BuildView {
            level_auto: true,
            ..view()
        });
        assert_eq!(h.click("level_mode"), ["SetLevelMode { auto: false }"]);
    }

    #[test]
    fn loadout_headers_not_selectable() {
        let mut h = Harness::new(BuildView {
            loadout_selected: None,
            ..view()
        });
        let mut sent = h.open("loadouts");
        assert!(h.rect("loadout_0").is_some(), "the list opened");
        // the "Loadouts:" header is a label: clicking it picks nothing
        sent.extend(h.click("loadout_0"));
        assert!(sent.is_empty(), "{sent:?}");
        sent.extend(h.open("loadouts"));
        sent.extend(h.click("loadout_2"));
        assert_eq!(sent, ["SelectLoadout { label: \"L2\" }"]);
        let mut sent = h.open("loadouts");
        sent.extend(h.click("loadout_5"));
        assert_eq!(sent, ["SyncLoadouts"]);
        // New Loadout asks for a name first
        let mut sent = h.open("loadouts");
        sent.extend(h.click("loadout_4"));
        assert!(sent.is_empty());
        assert!(h.state.has_dialog());
        let mut sent = h.retype("L3");
        sent.extend(h.click("new_loadout_save"));
        assert_eq!(sent, ["NewLoadout { name: \"L3\" }"]);
        assert!(!h.state.has_dialog());
    }

    #[test]
    fn a_loadouts_search_steps_through_its_matches_only() {
        // PoB steps through the drop list, which a search narrows (DropDownControl.lua:57-107);
        // the headers and actions among the loadouts don't break that
        let mut loadouts = view().loadouts;
        loadouts.insert(3, "Mapping".into());
        let mut h = Harness::new(BuildView { loadouts, ..view() });
        h.open("loadouts");
        h.frame(vec![Event::Text("map".into())]);
        h.frame(vec![]);
        assert_eq!(
            h.key(Key::ArrowDown),
            ["SelectLoadout { label: \"Mapping\" }"]
        );
    }

    #[test]
    fn loadouts_help_opens_the_help_window_and_about_its_history() {
        let mut h = Harness::new(view());
        let mut sent = h.open("loadouts");
        sent.extend(h.click("loadout_6"));
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(h.action, Some(BuildAction::Help));
        h.action = None;
        h.click("about");
        assert_eq!(h.action, Some(BuildAction::About));
    }

    #[test]
    fn new_loadout_window_opens_on_pobs_name_and_closes_on_escape() {
        let mut h = Harness::new(view());
        h.open("loadouts");
        h.click("loadout_4");
        assert_eq!(h.state.new_loadout.as_deref(), Some("New Loadout"));
        // PoB's Save starts disabled, on its default name too (Build.lua:344)
        let sent = h.click("new_loadout_save");
        assert!(sent.is_empty(), "{sent:?}");
        assert!(h.state.has_dialog());
        // any edit enables it, even one back to the default name
        let mut sent = h.retype("New Loadout");
        sent.extend(h.click("new_loadout_save"));
        assert_eq!(sent, ["NewLoadout { name: \"New Loadout\" }"]);
        // Esc cancels
        h.open("loadouts");
        h.click("loadout_4");
        let sent = h.key(Key::Escape);
        assert!(sent.is_empty(), "{sent:?}");
        assert!(!h.state.has_dialog());
    }

    #[test]
    fn a_new_loadout_name_pob_filters_to_nothing_keeps_save_disabled() {
        let mut h = Harness::new(view());
        h.open("loadouts");
        h.click("loadout_4");
        // PoB's box drops every non-ASCII byte under LuaJIT; over a selection it refuses a
        // text with one, so the name stays as it opened and is not edited
        for name in ["\u{65e5}\u{672c}\u{8a9e}", "\u{3a9}\u{3bc}"] {
            let mut sent = h.retype(name);
            sent.extend(h.click("new_loadout_save"));
            assert!(sent.is_empty(), "{name}: {sent:?}");
            assert_eq!(
                h.state.new_loadout.as_deref(),
                Some("New Loadout"),
                "{name}"
            );
        }
        // emptied, nothing is left to save
        h.retype("");
        h.frame(vec![Event::Key {
            key: Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(h.click("new_loadout_save").is_empty());
        // typed in, the box keeps what PoB's does and Save sends it untrimmed
        h.click("new_loadout_name");
        let mut sent = h.frame(vec![Event::Text("\u{65e5}\u{672c} a ".into())]);
        assert_eq!(h.state.new_loadout.as_deref(), Some(" a "));
        sent.extend(h.click("new_loadout_save"));
        assert_eq!(sent, ["NewLoadout { name: \" a \" }"]);
        assert!(!h.state.has_dialog());
        // 100 bytes at most; what would pass it is refused
        h.open("loadouts");
        h.click("loadout_4");
        h.retype("");
        h.frame(vec![Event::Key {
            key: Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Event::Text("x".repeat(100))]);
        h.frame(vec![Event::Text("y".into())]);
        assert_eq!(h.state.new_loadout.as_deref().map(str::len), Some(100));
    }

    #[test]
    fn mode_press_sends_the_typed_level_first() {
        // PoB applies every keystroke and its callback switches to Manual, so the press that
        // follows ends in Auto
        let mut h = Harness::new(view());
        h.click("level");
        h.retype("75");
        let sent = h.click("level_mode");
        assert_eq!(
            sent,
            [
                "SetCharacterLevel { level: 75 }",
                "SetLevelMode { auto: true }"
            ]
        );
        assert!(!h.state.has_pending());

        // in Auto, typing shows Manual, and the press goes back to Auto
        let mut h = Harness::new(BuildView {
            level_auto: true,
            ..view()
        });
        h.click("level");
        h.retype("75");
        let sent = h.click("level_mode");
        assert_eq!(
            sent,
            [
                "SetCharacterLevel { level: 75 }",
                "SetLevelMode { auto: true }"
            ]
        );
    }

    #[test]
    fn level_box_sends_on_blur() {
        let mut h = Harness::new(view());
        h.click("level");
        h.retype("60");
        assert_eq!(h.click_away(), ["SetCharacterLevel { level: 60 }"]);
    }

    #[test]
    fn retyping_the_level_in_auto_makes_it_manual() {
        // a keystroke in Auto is a Manual level in PoB, even the level already shown
        let mut h = Harness::new(BuildView {
            level_auto: true,
            ..view()
        });
        h.click("level");
        h.retype("90");
        assert!(h.state.has_pending());
        assert_eq!(h.click_away(), ["SetCharacterLevel { level: 90 }"]);
        assert!(!h.state.has_pending());
        // in Manual, an unchanged level sends nothing
        let mut h = Harness::new(view());
        h.click("level");
        h.retype("90");
        assert!(h.click_away().is_empty());
    }

    #[test]
    fn a_non_digit_in_auto_stays_auto() {
        // PoB's `%D` filter drops the key before its callback runs (EditControl.lua:162-168)
        let mut h = Harness::new(BuildView {
            level_auto: true,
            ..view()
        });
        h.click("level");
        h.frame(vec![Event::Text("x".into())]);
        h.frame(vec![]);
        assert!(!h.state.typed_in_auto, "a dropped key made it Manual");
        assert!(h.click_away().is_empty());
        // a fourth digit is past the limit and changes nothing either
        let mut h = Harness::new(BuildView {
            level: 100,
            level_auto: true,
            ..view()
        });
        h.click("level");
        h.key(Key::End);
        h.frame(vec![Event::Text("5".into())]);
        h.frame(vec![]);
        assert!(!h.state.typed_in_auto, "a refused key made it Manual");
        assert!(h.click_away().is_empty());
        // over a selection, ReplaceSel refuses a non-digit whole: the text stays, and so Auto
        let mut h = Harness::new(BuildView {
            level_auto: true,
            ..view()
        });
        h.click("level");
        h.retype("x");
        h.frame(vec![]);
        assert!(
            !h.state.typed_in_auto,
            "a refused key over a selection made it Manual"
        );
        assert_eq!(h.state.level.text, "90");
        assert!(h.click_away().is_empty());
    }

    #[test]
    fn bloodline_sends_its_id() {
        let mut h = Harness::new(view());
        let mut sent = h.open("bloodline");
        sent.extend(h.click("bloodline_1"));
        assert_eq!(sent, ["SelectSecondaryAscendancy { id: 8 }"]);
    }

    /// The top bar's dropdowns are PoB's searchable ones too (SearchHost)
    #[test]
    fn the_bloodline_dropdown_filters_as_typed() {
        let mut h = Harness::new(view());
        h.open("bloodline");
        h.frame(vec![Event::Text("aul".into())]);
        h.frame(vec![]);
        assert_eq!(
            h.rect("bloodline_0").unwrap().height(),
            0.0,
            "None wasn't filtered out"
        );
        let sent = h.click("bloodline_1");
        assert_eq!(sent, ["SelectSecondaryAscendancy { id: 8 }"]);
    }
}
