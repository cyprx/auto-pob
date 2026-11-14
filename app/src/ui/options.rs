//! The Options window (R12, R13): PoB's own Options popup, drawn by `PopupWindow`, with what
//! only auto-pob has added around it.
//!
//! - Beside each control that edits a setting, an "auto-pob only" box keeps auto-pob's value in
//!   `auto-pob.toml` and leaves `Settings.xml` (shared with the classic app and PoB) alone.
//!   Toggling it sends `SetOverrides` at once; a Cancel puts the saved map back.
//! - Above Save and Cancel: the user folder, the release notice and the trade realm and league.
//! - Save presses PoB's own `save`; when the engine's answer comes back (it carries the
//!   overrides the Save moved), the file is written and the map sent again. Cancel, Escape and
//!   the close box press PoB's cancel and write nothing.

use super::popup::{Decorate, PopupWindow};
use crate::engine::views::{PopupControl, PopupControlKind, PopupView, SettingEffect, SettingRow};
use crate::engine::{Command, SettingsView};
use crate::settings::{AppSettings, REALMS};
use crate::widgets::{EditBox, EditBoxOptions};

/// The window's own text boxes: the Builds folder and the trade league
pub fn box_ids() -> [egui::Id; 2] {
    [
        egui::Id::unique("options_user_dir"),
        egui::Id::unique("options_league"),
    ]
}
use eframe::egui;

/// What the window asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct OptionsOutput {
    pub commands: Vec<Command>,
    pub queries: Vec<Command>,
    /// The index in `commands` of PoB's Save press: the app tells us its seq (`save_sent`)
    pub save_at: Option<usize>,
}

#[derive(Default)]
pub struct OptionsUi {
    pub window: PopupWindow,
    rows: Vec<SettingRow>,
    /// The settings as the file has them
    base: AppSettings,
    /// The settings as the window edits them
    draft: AppSettings,
    folder: EditBox,
    league: EditBox,
    /// PoB's Save went out as this command; the file is written when its answer is here
    saving: Option<u64>,
    /// The file couldn't be written: the next `begin` keeps the draft and shows this
    error: Option<String>,
    retry: bool,
}

/// What the UI itself does with PoB's `Ui` settings (R13).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiEffects {
    pub zoom: f32,
    pub show_title: bool,
    /// "Node Power colours" (`nodePowerTheme`)
    pub power_theme: crate::ui::tree::overlays::PowerTheme,
    /// "Show search circles at viewport edge" (`edgeSearchHighlight`, on in PoB by default)
    pub edge_search: bool,
    /// "Show Animations" (`showAnimations`, on in PoB by default): the tree's jewel rings turn
    pub animations: bool,
    /// "Styled Tooltips with Flavour Text" (`showFlavourText`, on in PoB by default): the
    /// tooltips' header and separator art
    pub styled_tooltips: bool,
}

impl Default for UiEffects {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            show_title: false,
            power_theme: Default::default(),
            edge_search: true,
            animations: true,
            styled_tooltips: true,
        }
    }
}

/// The percentages PoB's "UI scaling override" dropdown offers besides "Use system default"
/// (0; Main.lua:928-937)
const DPI_OVERRIDES: [u32; 7] = [100, 125, 150, 175, 200, 225, 250];

/// The zoom factor, window-title switch and tree view settings the settings ask for. Only a percentage PoB's
/// dropdown offers zooms; anything else (0, "Use system default", a stray 1, garbage) is no
/// override.
pub fn ui_effects(rows: &[SettingRow]) -> UiEffects {
    let value = |key: &str| rows.iter().find(|r| r.key == key).map(|r| r.value.as_str());
    let percent = value("dpiScaleOverridePercent")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|p| p.fract() == 0.0 && DPI_OVERRIDES.contains(&(*p as u32)))
        .unwrap_or(100.0) as f32;
    UiEffects {
        zoom: percent / 100.0,
        show_title: value("showTitlebarName") == Some("true"),
        power_theme: crate::ui::tree::overlays::PowerTheme::parse(
            value("nodePowerTheme").unwrap_or_default(),
        ),
        edge_search: value("edgeSearchHighlight") != Some("false"),
        animations: value("showAnimations") != Some("false"),
        styled_tooltips: value("showFlavourText") != Some("false"),
    }
}

/// The window title: `auto-pob - <build>` when the setting is on and a build is open.
pub fn window_title(show_name: bool, build: Option<&str>) -> String {
    match build.filter(|_| show_name) {
        Some(name) => format!("auto-pob - {name}"),
        None => "auto-pob".into(),
    }
}

impl OptionsUi {
    #[cfg(test)]
    pub(crate) fn draft_mut(&mut self) -> &mut AppSettings {
        &mut self.draft
    }

    pub fn rows(&self) -> &[SettingRow] {
        &self.rows
    }

    /// The window is about to open over the settings in the file.
    pub fn begin(&mut self, saved: &AppSettings) {
        self.folder = EditBox::default();
        self.league = EditBox::default();
        self.saving = None;
        if std::mem::take(&mut self.retry) {
            return;
        }
        self.error = None;
        self.base = saved.clone();
        self.draft = saved.clone();
    }

    /// The file was written: `next` is what the window shows from now on.
    pub fn commit(&mut self, next: &AppSettings) {
        self.base = next.clone();
        self.draft = next.clone();
        self.error = None;
    }

    /// The file could not be written. The draft (with the overrides PoB's Save moved) is kept
    /// for the reopened window, which shows `error`. Returns the draft's overrides.
    pub fn save_failed(
        &mut self,
        next: AppSettings,
        error: String,
    ) -> std::collections::BTreeMap<String, String> {
        let overrides = next.overrides.clone();
        self.draft = next;
        self.error = Some(error);
        self.retry = true;
        overrides
    }

    /// PoB's Save went out as command `seq`.
    pub fn save_sent(&mut self, seq: u64) {
        self.saving = Some(seq);
    }

    /// The engine refused command `seq`.
    pub fn on_error(&mut self, seq: u64) {
        if self.saving == Some(seq) {
            self.saving = None;
        }
        self.window.on_error(seq);
    }

    /// The settings view of command `seq`. Returns the settings to write when it answers the
    /// Save: the edits plus the overrides PoB's Save moved.
    pub fn on_settings(&mut self, view: &SettingsView, seq: u64) -> Option<AppSettings> {
        self.rows.clone_from(&view.rows);
        if !self.saving.is_some_and(|s| seq >= s) {
            return None;
        }
        self.saving = None;
        let mut next = self.draft.clone();
        next.overrides.extend(view.saved_overrides.clone());
        Some(next)
    }

    /// The commands that end an Options session the window didn't end itself (the app is
    /// closing it): PoB's cancel, and the saved overrides back in place.
    pub fn close(&mut self) -> Vec<Command> {
        let mut commands: Vec<Command> = self.window.flush().into_iter().collect();
        if let Some(close) = self.window.close() {
            commands.push(close);
            self.restore(&mut commands);
        }
        commands
    }

    /// A save PoB made by itself (a sign-in's token) cancelled the window: its edits are gone,
    /// and the overrides it changed go back to the saved ones, as after a Cancel.
    pub fn lost(&mut self) -> Vec<Command> {
        self.folder = EditBox::default();
        self.league = EditBox::default();
        self.saving = None;
        let mut commands = Vec::new();
        self.restore(&mut commands);
        commands
    }

    /// After a Cancel the saved overrides are the ones that count.
    fn restore(&mut self, commands: &mut Vec<Command>) {
        if self.draft.overrides != self.base.overrides {
            commands.push(Command::SetOverrides {
                values: self.base.overrides.clone(),
            });
        }
        self.draft = self.base.clone();
    }

    /// A pending edit in the folder or league box goes into the draft.
    fn flush_boxes(&mut self) {
        if let Some(text) = self.folder.take_pending() {
            set_folder(&mut self.draft, &text);
        }
        if let Some(text) = self.league.take_pending() {
            self.draft.trade_league = text.trim().to_owned();
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, now: f64, busy: bool) -> OptionsOutput {
        let (out, overrides_changed) = {
            let Self {
                window,
                rows,
                base,
                draft,
                folder,
                league,
                error,
                ..
            } = self;
            let mut extras = Extras {
                rows,
                base,
                draft,
                folder,
                league,
                error: error.as_deref(),
                busy,
                overrides_changed: false,
            };
            let out = window.show_with(ctx, now, busy, &mut extras);
            (out, extras.overrides_changed)
        };
        let mut result = OptionsOutput {
            queries: out.queries,
            ..OptionsOutput::default()
        };
        let mut cancelled = false;
        for command in out.commands {
            match &command {
                Command::PressPopupButton { name } if name == "save" => {
                    self.flush_boxes();
                    result.save_at = Some(result.commands.len());
                }
                Command::PressPopupButton { name } if name == "cancel" => cancelled = true,
                Command::ClosePopup => cancelled = true,
                _ => {}
            }
            result.commands.push(command);
        }
        if cancelled {
            self.restore(&mut result.commands);
        } else if overrides_changed {
            result.commands.push(Command::SetOverrides {
                values: self.draft.overrides.clone(),
            });
        }
        result
    }
}

fn set_folder(settings: &mut AppSettings, text: &str) {
    let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
    settings.user_dir =
        (!text.trim().is_empty()).then(|| crate::settings::expand_folder(text, home.as_deref()));
}

/// What the window draws around PoB's controls.
struct Extras<'a> {
    rows: &'a [SettingRow],
    base: &'a AppSettings,
    draft: &'a mut AppSettings,
    folder: &'a mut EditBox,
    league: &'a mut EditBox,
    error: Option<&'a str>,
    busy: bool,
    overrides_changed: bool,
}

impl Extras<'_> {
    fn section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("auto-pob only");
        ui.label(
            egui::RichText::new("Kept in auto-pob.toml, not in Settings.xml.")
                .color(ui.visuals().weak_text_color()),
        );
        egui::Grid::new("options_auto_pob")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("User folder");
                ui.vertical(|ui| {
                    let current = self
                        .draft
                        .user_dir
                        .as_ref()
                        .map(|d| d.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let opts = EditBoxOptions {
                        width: 360.0,
                        hint: Some("the default folder"),
                        ..EditBoxOptions::default()
                    };
                    let (response, sent) =
                        self.folder
                            .show(ui, box_ids()[0], &current, self.busy, &opts);
                    super::import::mark(ui, "options_user_dir", &response);
                    if let Some(text) = sent {
                        set_folder(self.draft, &text);
                    }
                    if std::env::var_os("AUTO_POB_USER_DIR").is_some() {
                        ui.label("AUTO_POB_USER_DIR is set and wins over this folder.");
                    }
                    if self.draft.user_dir != self.base.user_dir {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "Takes effect the next time auto-pob starts.",
                        );
                    }
                });
                ui.end_row();

                ui.label("Release notice");
                let check = ui
                    .checkbox(
                        &mut self.draft.update_check,
                        "Look for new auto-pob releases",
                    )
                    .on_hover_text(
                        "One request to GitHub when auto-pob starts, nothing else is sent. \
                         Takes effect the next time auto-pob starts. \
                         AUTO_POB_UPDATE_CHECK=1 turns it on whatever this says.",
                    );
                super::import::mark(ui, "options_update_check", &check);
                ui.end_row();

                ui.label("Trade realm");
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt("options_realm")
                    .selected_text(self.draft.trade_realm.as_str())
                    .show_ui(ui, |ui| {
                        for realm in REALMS {
                            let item = crate::widgets::dropdown::item(
                                ui,
                                self.draft.trade_realm == realm,
                                realm,
                            );
                            super::import::mark(ui, &format!("options_realm_{realm}"), &item);
                            if item.clicked() {
                                self.draft.trade_realm = realm.to_owned();
                            }
                        }
                    });
                super::import::mark(ui, "options_realm", &combo.response);
                // the wheel and the arrows step it (DropDownControl.lua:476-516)
                let at = REALMS
                    .iter()
                    .position(|r| *r == self.draft.trade_realm)
                    .unwrap_or(0);
                if let Some(i) =
                    crate::widgets::scroll_step::combo_step(ui, &combo.response, REALMS.len(), at)
                {
                    self.draft.trade_realm = REALMS[i].to_owned();
                }
                ui.end_row();

                ui.label("Trade league");
                let opts = EditBoxOptions {
                    width: 200.0,
                    ..EditBoxOptions::default()
                };
                let current = self.draft.trade_league.clone();
                let (response, sent) =
                    self.league
                        .show(ui, box_ids()[1], &current, self.busy, &opts);
                super::import::mark(ui, "options_league", &response);
                if let Some(text) = sent {
                    self.draft.trade_league = text.trim().to_owned();
                }
                ui.end_row();
            });
        if let Some(error) = self.error {
            let label = ui.colored_label(ui.visuals().error_fg_color, error);
            super::import::mark(ui, "options_save_error", &label);
        }
        ui.add_space(6.0);
    }
}

impl Decorate for Extras<'_> {
    fn before_row(&mut self, ui: &mut egui::Ui, view: &PopupView, row: &[usize]) {
        let has_save = row.iter().any(|&i| {
            let c = &view.controls[i];
            c.kind == PopupControlKind::Button && c.name == "save"
        });
        if has_save {
            self.section(ui);
        }
    }

    fn after_control(&mut self, ui: &mut egui::Ui, control: &PopupControl) {
        if matches!(
            control.kind,
            PopupControlKind::Label | PopupControlKind::Button | PopupControlKind::List
        ) {
            return;
        }
        let Some(row) = self
            .rows
            .iter()
            .find(|r| !r.control.is_empty() && r.control == control.name)
        else {
            return;
        };
        let mut on = self.draft.overrides.contains_key(&row.key);
        let check = ui.checkbox(&mut on, "auto-pob only").on_hover_text(
            "Keep this value only in auto-pob. Settings.xml, which Path of Building \
             shares, keeps its own.",
        );
        super::import::mark(ui, &format!("override_{}", row.key), &check);
        if check.changed() {
            if on {
                self.draft
                    .overrides
                    .insert(row.key.clone(), row.value.clone());
            } else {
                self.draft.overrides.remove(&row.key);
            }
            self.overrides_changed = true;
        }
        if row.effect == SettingEffect::NoEffect {
            let note = ui.label(
                egui::RichText::new("Does nothing in auto-pob.")
                    .color(ui.visuals().weak_text_color()),
            );
            super::import::mark(ui, &format!("note_{}", row.key), &note);
        }
    }
}

/// Views of the Options popup for the tests of this window and the app around it.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub fn control(name: &str, kind: PopupControlKind, pos: [f32; 2]) -> PopupControl {
        PopupControl {
            name: name.into(),
            kind,
            label: name.into(),
            pos,
            size: [120.0, 20.0],
            shown: true,
            enabled: true,
            choices: vec![],
            selected: 0,
            value: 0.0,
            divisions: None,
            checked: false,
            text: String::new(),
            ascii_only: false,
            digits: false,
            signed: false,
            numeric: false,
            limit: None,
            tooltip_text: None,
            hover_tooltip: false,
            rows: vec![],
            row_selected: None,
            row_highlight: None,
            note: None,
            reorder: false,
        }
    }

    /// Two checkboxes (one has no effect in auto-pob) and Save and Cancel
    pub fn options_view() -> PopupView {
        PopupView {
            kind: crate::engine::PopupKind::Options,
            title: "Options".into(),
            enter: None,
            controls: vec![
                control("showFlavourText", PopupControlKind::Check, [0.0, 0.0]),
                control("showAnimations", PopupControlKind::Check, [0.0, 30.0]),
                control("save", PopupControlKind::Button, [0.0, 80.0]),
                control("cancel", PopupControlKind::Button, [130.0, 80.0]),
            ],
        }
    }

    pub fn row(key: &str, control: &str, value: &str, effect: SettingEffect) -> SettingRow {
        SettingRow {
            key: key.into(),
            control: control.into(),
            label: key.into(),
            shared: value.into(),
            value: value.into(),
            overridden: false,
            effect,
        }
    }

    pub fn settings_view(saved: &[(&str, &str)]) -> SettingsView {
        SettingsView {
            rows: vec![
                row(
                    "showFlavourText",
                    "showFlavourText",
                    "true",
                    SettingEffect::PobText,
                ),
                row(
                    "showAnimations",
                    "showAnimations",
                    "false",
                    SettingEffect::NoEffect,
                ),
            ],
            saved_overrides: saved
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            options_lost: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::ui::import::{clear_marks, marked};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    struct Harness {
        ui: OptionsUi,
        ctx: egui::Context,
        time: f64,
    }

    impl Harness {
        fn new(saved: &AppSettings) -> Self {
            let mut ui = OptionsUi::default();
            ui.begin(saved);
            ui.on_settings(&settings_view(&[]), 0);
            ui.window
                .on_snapshot(&crate::engine::Snapshot::Popup(Some(std::sync::Arc::new(
                    options_view(),
                ))));
            let mut h = Self {
                ui,
                ctx: egui::Context::default(),
                time: 1.0,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> OptionsOutput {
            clear_marks();
            self.time += 0.2;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut all = OptionsOutput::default();
            let time = self.time;
            let out = self.ctx.run_ui(input, |ui| {
                let pass = self.ui.show(ui.ctx(), time, false);
                if let Some(at) = pass.save_at {
                    all.save_at = Some(all.commands.len() + at);
                }
                all.commands.extend(pass.commands);
            });
            out.drop_without_applying_deltas();
            all
        }

        fn click_at(&mut self, pos: Pos2) -> OptionsOutput {
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = OptionsOutput::default();
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                let pass = self.frame(events);
                if let Some(at) = pass.save_at {
                    all.save_at = Some(all.commands.len() + at);
                }
                all.commands.extend(pass.commands);
            }
            all
        }

        fn click_mark(&mut self, key: &str) -> OptionsOutput {
            let pos = marked(key).unwrap_or_else(|| panic!("no {key}")).center();
            self.click_at(pos)
        }

        fn click_button(&mut self, name: &str) -> OptionsOutput {
            let pos = self.ui.window.rects.map[&format!("btn_{name}")].center();
            self.click_at(pos)
        }
    }

    fn debug(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|c| format!("{c:?}")).collect()
    }

    fn overrides(pairs: &[(&str, &str)]) -> Command {
        Command::SetOverrides {
            values: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn override_checkbox_sends_set_overrides() {
        let mut h = Harness::new(&AppSettings::default());
        // only the controls with a setting get the box; Save and Cancel do not
        assert!(marked("override_showFlavourText").is_some());
        assert!(marked("override_showAnimations").is_some());
        assert_eq!(marked("override_save"), None);
        let out = h.click_mark("override_showFlavourText");
        assert_eq!(
            debug(&out.commands),
            debug(&[overrides(&[("showFlavourText", "true")])])
        );
        // a second box adds to the map; the first stays
        let out = h.click_mark("override_showAnimations");
        assert_eq!(
            debug(&out.commands),
            debug(&[overrides(&[
                ("showAnimations", "false"),
                ("showFlavourText", "true")
            ])])
        );
        // unticking takes only that key out
        let out = h.click_mark("override_showFlavourText");
        assert_eq!(
            debug(&out.commands),
            debug(&[overrides(&[("showAnimations", "false")])])
        );
    }

    #[test]
    fn a_saved_override_shows_ticked_and_untick_clears_it() {
        let mut saved = AppSettings::default();
        saved
            .overrides
            .insert("showFlavourText".into(), "false".into());
        let mut h = Harness::new(&saved);
        let out = h.click_mark("override_showFlavourText");
        assert_eq!(debug(&out.commands), debug(&[overrides(&[])]));
    }

    #[test]
    fn options_save_presses_pob_then_waits_for_its_answer() {
        let mut h = Harness::new(&AppSettings::default());
        h.click_mark("override_showFlavourText");
        let out = h.click_button("save");
        assert_eq!(
            debug(&out.commands),
            debug(&[Command::PressPopupButton {
                name: "save".into()
            }])
        );
        assert_eq!(out.save_at, Some(0));
        h.ui.save_sent(9);
        // another answer, from before the press, writes nothing
        assert_eq!(h.ui.on_settings(&settings_view(&[]), 8), None);
        // the answer to the press carries what PoB's Save moved into the overrides
        let next =
            h.ui.on_settings(&settings_view(&[("showAnimations", "true")]), 9)
                .expect("the file is written");
        assert_eq!(next.overrides["showAnimations"], "true");
        assert_eq!(next.overrides["showFlavourText"], "true");
        // and only once
        assert_eq!(h.ui.on_settings(&settings_view(&[]), 10), None);
    }

    /// Drives the four auto-pob-only fields with real input.
    fn edit_auto_pob_fields(h: &mut Harness) {
        h.click_mark("options_update_check");
        h.click_mark("options_realm");
        h.click_mark("options_realm_PS4");
        h.click_mark("options_league");
        // typing lands after the old league
        h.frame(vec![Event::Text("-Settlers".into())]);
        h.click_mark("options_user_dir");
        h.frame(vec![Event::Text("/typed/folder".into())]);
    }

    #[test]
    fn auto_pob_only_fields_reach_the_saved_settings() {
        let mut h = Harness::new(&AppSettings::default());
        edit_auto_pob_fields(&mut h);
        // the folder box still has the focus: Save has to take what was typed
        let out = h.click_button("save");
        assert!(out.save_at.is_some());
        h.ui.save_sent(3);
        let next = h.ui.on_settings(&settings_view(&[]), 3).unwrap();
        assert!(next.update_check);
        assert_eq!(next.trade_realm, "PS4");
        assert_eq!(next.trade_league, "Standard-Settlers");
        assert_eq!(
            next.user_dir,
            Some(std::path::PathBuf::from("/typed/folder"))
        );
    }

    #[test]
    fn cancel_drops_the_auto_pob_only_edits() {
        let mut h = Harness::new(&AppSettings::default());
        edit_auto_pob_fields(&mut h);
        let out = h.click_button("cancel");
        assert_eq!(out.save_at, None);
        assert_eq!(h.ui.on_settings(&settings_view(&[]), 9), None);
        assert_eq!(h.ui.draft, AppSettings::default());
    }

    #[test]
    fn the_restart_note_shows_for_a_changed_folder() {
        let mut h = Harness::new(&AppSettings::default());
        h.click_mark("options_user_dir");
        h.frame(vec![Event::Text("/typed/folder".into())]);
        h.click_mark("options_league");
        assert_eq!(
            h.ui.draft.user_dir,
            Some(std::path::PathBuf::from("/typed/folder"))
        );
    }

    #[test]
    fn save_stays_reachable_when_the_restart_note_appears() {
        let mut h = Harness::new(&AppSettings::default());
        h.ui.draft.user_dir = Some("/elsewhere".into());
        h.frame(vec![]);
        h.frame(vec![]);
        let out = h.click_button("save");
        assert_eq!(out.save_at, Some(0), "{:?}", out.commands);
    }

    #[test]
    fn options_cancel_writes_nothing() {
        let mut h = Harness::new(&AppSettings::default());
        h.click_mark("override_showFlavourText");
        h.ui.draft.update_check = true;
        let out = h.click_button("cancel");
        // PoB's cancel, then the saved map (none) back in place; never a Save
        assert_eq!(
            debug(&out.commands),
            debug(&[
                Command::PressPopupButton {
                    name: "cancel".into()
                },
                overrides(&[]),
            ])
        );
        assert_eq!(out.save_at, None);
        // no answer after it is a Save's
        assert_eq!(
            h.ui.on_settings(&settings_view(&[("showAnimations", "x")]), 50),
            None
        );
        assert!(!h.ui.draft.update_check, "the edits are dropped");
    }

    #[test]
    fn escape_cancels_and_restores_the_saved_overrides() {
        let mut h = Harness::new(&AppSettings::default());
        h.click_mark("override_showFlavourText");
        let out = h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!(
            debug(&out.commands),
            debug(&[Command::ClosePopup, overrides(&[])])
        );
        assert_eq!(out.save_at, None);
    }

    #[test]
    fn no_effect_rows_say_so() {
        let _h = Harness::new(&AppSettings::default());
        // both settings get the box; only the one PoB's option does nothing for gets the note
        assert!(marked("override_showAnimations").is_some());
        assert!(marked("note_showAnimations").is_some());
        assert!(marked("override_showFlavourText").is_some());
        assert_eq!(marked("note_showFlavourText"), None);
    }

    #[test]
    fn ui_rows_set_the_zoom_and_the_title() {
        let rows = |dpi: &str, title: &str| {
            vec![
                row(
                    "dpiScaleOverridePercent",
                    "dpiScaleOverride",
                    dpi,
                    SettingEffect::Ui,
                ),
                row("showTitlebarName", "titlebarName", title, SettingEffect::Ui),
            ]
        };
        assert_eq!(ui_effects(&[]), UiEffects::default());
        let fx = ui_effects(&rows("150", "true"));
        assert!((fx.zoom - 1.5).abs() < 1e-6);
        assert!(fx.show_title);
        // 0 is "no override"
        assert!((ui_effects(&rows("0", "false")).zoom - 1.0).abs() < 1e-6);
        assert!((ui_effects(&rows("garbage", "false")).zoom - 1.0).abs() < 1e-6);
        // only PoB's own percentages zoom: a fresh folder's stray 1 once shrank the UI to 25%
        for other in ["1", "-100", "99", "1000", "150.5", ""] {
            assert!(
                (ui_effects(&rows(other, "false")).zoom - 1.0).abs() < 1e-6,
                "{other}"
            );
        }
        for percent in [100, 125, 175, 200, 225, 250] {
            let zoom = ui_effects(&rows(&percent.to_string(), "false")).zoom;
            assert!((zoom - percent as f32 / 100.0).abs() < 1e-6, "{percent}");
        }
        assert_eq!(window_title(true, Some("Witch")), "auto-pob - Witch");
        assert_eq!(window_title(false, Some("Witch")), "auto-pob");
        assert_eq!(window_title(true, None), "auto-pob");
    }

    #[test]
    fn ui_rows_set_the_heat_map_colours_and_edge_circles() {
        // Main.lua:957-964, 1011-1013: PoB's "Node Power colours" and edge search circles,
        // which the tree view follows
        use crate::ui::tree::overlays::PowerTheme;
        let fx = ui_effects(&[
            row(
                "nodePowerTheme",
                "nodePowerTheme",
                "RED/GREEN",
                SettingEffect::Ui,
            ),
            row(
                "edgeSearchHighlight",
                "edgeSearchHighlight",
                "false",
                SettingEffect::Ui,
            ),
            row(
                "showAnimations",
                "showAnimations",
                "false",
                SettingEffect::Ui,
            ),
        ]);
        assert_eq!(fx.power_theme, PowerTheme::RedGreen);
        assert!(!fx.edge_search);
        // the jewel rings stand still (PassiveTreeView.lua:1250-1255)
        assert!(!fx.animations);
        // PoB's defaults
        assert_eq!(ui_effects(&[]).power_theme, PowerTheme::RedBlue);
        assert!(ui_effects(&[]).edge_search);
        assert!(ui_effects(&[]).animations);
    }
}
