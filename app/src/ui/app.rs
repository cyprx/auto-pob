//! The application: a screen state machine over the engine's events.

use std::sync::Arc;

use crate::engine::commands::DeleteKind;
use crate::engine::views::{BuildListView, BuildView, ConfigView, RequestsView, StatsView};
use crate::engine::{BackgroundTask, Command, Engine, Event, ImportMode, PopupKind, Snapshot, Tab};
use crate::game::Game;
use crate::pob_files::DownloadProgress;
use crate::settings::{self, AppSettings};
use crate::theme;
use crate::ui::build::{self, BuildAction, BuildTab};
use crate::ui::build_list::{BuildListUi, ListScreenAction};
use crate::ui::calcs::{CalcsOutput, CalcsUi};
use crate::ui::compare::{CompareOutput, CompareUi};
use crate::ui::config::ConfigUi;
use crate::ui::folder_picker::{FolderPicker, PickerOutput};
use crate::ui::help::{self, HelpStart, HelpUi};
use crate::ui::import::{Confirm, ImportOutput, ImportUi, Plan};
use crate::ui::items::{ItemsOutput, ItemsUi};
use crate::ui::name_box::{self, Filter};
use crate::ui::notes::NotesUi;
use crate::ui::options::{self, OptionsUi};
use crate::ui::popup::{self, PopupWindow};
use crate::ui::requests;
use crate::ui::shortcuts::{self, ShortcutAction, ShortcutState};
use crate::ui::skills::{SkillsOutput, SkillsUi};
use crate::ui::tree::{TreeOutput, TreeTab};
use crate::ui::unsaved::{Pending, PromptChoice, Resolution, needs_prompt, resolve};
use crate::ui::{sidebar, startup};
use crate::update_notice::Release;
use eframe::egui;

/// Reads the system clipboard's text.
pub type ClipboardRead = Box<dyn Fn() -> Option<String>>;

/// Starts fetching PoB's help files in the background; the answer comes on the channel.
pub type HelpDownload = Box<dyn Fn() -> std::sync::mpsc::Receiver<Result<(), String>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    List,
    Build,
}

/// Whether PoB's Save As would be on: a visible name, and no build of that name in the folder
/// (Build.lua:1384-1396)
fn save_as_is_open(list: Option<&BuildListView>, dialog: &SaveAsDialog) -> bool {
    if !name_box::has_visible(&dialog.name) {
        return false;
    }
    list.is_none_or(|list| {
        !list
            .root
            .join(&dialog.folder)
            .join(format!("{}.xml", dialog.name))
            .exists()
    })
}

/// The Save as dialog.
struct SaveAsDialog {
    folder: String,
    name: String,
    /// What to do once saved.
    then: Option<Pending>,
    /// The engine refused to overwrite `name`; ask.
    confirm_overwrite: bool,
    /// A `SaveAs` is on its way.
    waiting: bool,
    /// PoB's folder list: where in the Builds tree the build goes
    picker: FolderPicker,
    /// A New Folder in the picker is on its way: the folder to go back to if it is refused
    new_folder: Option<(u64, String)>,
}

/// A question asked before the tab or the pending line acts.
struct AppConfirm {
    message: String,
    yes: &'static str,
    no: &'static str,
    then: ConfirmThen,
}

enum ConfirmThen {
    /// Start a character import
    Plan(Plan),
    /// Send an import of a link or code into the open build
    Send(Command),
    /// Cancel a request
    Cancel(u64),
    /// Go on with an action that cancels a running OAuth import
    Leave(Pending),
    /// Ask the usual questions (OAuth import, unsaved changes), then act
    Request(Pending),
}

/// A `Save` or `SaveAs` whose outcome we are waiting for.
struct PendingSave {
    seq: u64,
    then: Option<Pending>,
    is_save_as: bool,
}

pub struct AutoPobApp {
    spawn: Box<dyn Fn() -> Engine>,
    engine: Engine,
    pob_version: &'static str,

    // startup
    ready: bool,
    progress: Option<DownloadProgress>,
    startup_error: Option<String>,

    // the latest snapshots
    build: Option<Arc<BuildView>>,
    list: Option<Arc<BuildListView>>,
    stats: Option<Arc<StatsView>>,
    config: Option<Arc<ConfigView>>,
    /// Whether the build was unsaved when we last heard, for the "engine stopped" screen.
    last_unsaved: bool,

    /// Highest command number sent that produces snapshots, and highest snapshot received.
    last_sent: u64,
    last_snapshot: u64,

    screen: Screen,
    /// A build is being opened; the build screen waits for this command's snapshots.
    entering: Option<u64>,
    tab: BuildTab,
    tree: TreeTab,
    calcs: CalcsUi,
    skills: SkillsUi,
    items: ItemsUi,
    /// The crafting popup window, whichever tab opened it
    popup: PopupWindow,
    /// The tab that opened the popup: leaving it closes the window
    popup_tab: Option<BuildTab>,
    /// A failed open or import (its seq) and the build generation before it: if the build that
    /// follows is another one, PoB's Init failed and PoB would show its list (`on_failed_open`)
    failed_open: Option<(u64, u64)>,
    /// The trade link a popup press asked for: opened and copied for that press only (R20)
    trade_link: popup::TradeLink,
    /// The Options window, which any screen can open and a build change doesn't end (R19)
    options: OptionsUi,
    /// auto-pob.toml as it is on disk, and where it goes (None: nowhere, as in the tests)
    settings: AppSettings,
    config_path: Option<std::path::PathBuf>,
    /// Why the file may not be written in this run (it could not be read)
    settings_locked: Option<String>,
    /// The zoom factor and window title the settings last asked for
    applied_zoom: f32,
    applied_title: String,
    /// The seq of the raw item text last sent, to keep its window open on an error
    text_seq: Option<u64>,
    sidebar: sidebar::SidebarState,
    /// The top bar's level box and New Loadout name
    top_bar: build::TopBarState,
    notes: NotesUi,
    compare: CompareUi,
    /// `SetCompareOverlay { on: true }` is the engine's state: the Compare tab's Tree view is up
    compare_overlay: bool,
    import_ui: ImportUi,
    /// The pending network requests (the finished ones are turned into notices)
    requests: RequestsView,
    confirm: Option<AppConfirm>,
    /// The `ImportFromUrl` whose immediate answer replaces the build
    import_seq: Option<u64>,
    /// The confirm dialog's identity, so its modal layer keeps its state across frames
    confirm_id: egui::Id,
    config_ui: ConfigUi,
    list_ui: BuildListUi,
    /// The `MoveBuild` of the last paste: if it is refused the cut mark comes back
    list_move_seq: Option<u64>,
    /// A `pob://` link from the command line, asked about once the engine is ready (R4)
    startup_link: Option<String>,
    /// The release check's answer, and the newer release it found (until dismissed)
    release_rx: Option<std::sync::mpsc::Receiver<Release>>,
    release: Option<Release>,

    notices: Vec<String>,
    pending_save: Option<PendingSave>,
    save_as: Option<SaveAsDialog>,
    prompt: Option<Pending>,
    import: Option<String>,
    /// Actions waiting for the next frame (they need the egui context).
    ready_actions: Vec<Pending>,
    allow_close: bool,
    /// PoB's About window: its help and version history (R25)
    help: HelpUi,
    /// Where the top bar and the sidebar were drawn last: a box there is Build's own control
    bar_rects: Vec<egui::Rect>,
    /// Starts fetching PoB's missing help files (the user's click), answering on the channel
    help_download: Option<HelpDownload>,
    help_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// Reads the system clipboard, for a right-click into a text box (EditControl.lua:527);
    /// read only on that click
    clipboard_read: Option<ClipboardRead>,
    /// The text a right-click pastes into the box it focused, at the next frame's start
    paste_next: Option<String>,
    /// PoB's options for the wheel on dropdowns and sliders, as its settings last said
    scroll_options: crate::widgets::scroll_step::ScrollOptions,
    /// PoB's foot labels as drawn, for tests
    #[cfg(test)]
    label_texts: Vec<String>,
    /// Names of the commands sent through `send`, for tests.
    #[cfg(test)]
    sent_log: Vec<String>,
    /// The same commands in full (`{:?}`), for tests that check arguments.
    #[cfg(test)]
    sent_full: Vec<String>,
}

impl AutoPobApp {
    /// `spawn` starts an engine; it is called now and again for Retry and Restart.
    pub fn new(spawn: Box<dyn Fn() -> Engine>) -> Self {
        let engine = spawn();
        AutoPobApp {
            spawn,
            engine,
            pob_version: Game::Poe1.pob_version(),
            ready: false,
            progress: None,
            startup_error: None,
            build: None,
            list: None,
            stats: None,
            config: None,
            last_unsaved: false,
            last_sent: 0,
            last_snapshot: 0,
            screen: Screen::List,
            entering: None,
            tab: BuildTab::default(),
            tree: TreeTab::default(),
            calcs: CalcsUi::default(),
            skills: SkillsUi::default(),
            items: ItemsUi::default(),
            popup: PopupWindow::default(),
            popup_tab: None,
            failed_open: None,
            trade_link: popup::TradeLink::default(),
            options: OptionsUi::default(),
            settings: AppSettings::default(),
            config_path: None,
            settings_locked: None,
            applied_zoom: 1.0,
            applied_title: "auto-pob".into(),
            text_seq: None,
            sidebar: sidebar::SidebarState::default(),
            top_bar: build::TopBarState::default(),
            notes: NotesUi::default(),
            compare: CompareUi::default(),
            compare_overlay: false,
            import_ui: ImportUi::default(),
            requests: RequestsView::default(),
            confirm: None,
            import_seq: None,
            confirm_id: egui::Id::unique("confirm_dialog"),
            config_ui: ConfigUi::default(),
            list_ui: BuildListUi::default(),
            list_move_seq: None,
            startup_link: None,
            release_rx: None,
            release: None,
            notices: Vec::new(),
            pending_save: None,
            save_as: None,
            prompt: None,
            import: None,
            ready_actions: Vec::new(),
            allow_close: false,
            help: HelpUi::default(),
            bar_rects: Vec::new(),
            help_download: None,
            help_rx: None,
            clipboard_read: None,
            paste_next: None,
            scroll_options: Default::default(),
            #[cfg(test)]
            label_texts: Vec::new(),
            #[cfg(test)]
            sent_log: Vec::new(),
            #[cfg(test)]
            sent_full: Vec::new(),
        }
    }

    /// How a right-click into a text box reads the clipboard.
    #[must_use]
    pub fn with_clipboard_reader(mut self, read: ClipboardRead) -> Self {
        self.clipboard_read = Some(read);
        self
    }

    /// How the Help window fetches PoB's help files when the user asks.
    #[must_use]
    pub fn with_help_download(mut self, download: HelpDownload) -> Self {
        self.help_download = Some(download);
        self
    }

    /// A `pob://` link to import after the user confirms it.
    #[must_use]
    pub fn with_startup_link(mut self, link: Option<String>) -> Self {
        self.startup_link = link;
        self
    }

    /// Shows a banner when the release check finds a newer release.
    #[must_use]
    pub fn with_release_check(mut self, rx: std::sync::mpsc::Receiver<Release>) -> Self {
        self.release_rx = Some(rx);
        self
    }

    /// The contents of `auto-pob.toml` (defaults when there is none) and the file to write them
    /// to. The engine gets the overrides once it is ready.
    #[must_use]
    pub fn with_settings(mut self, settings: AppSettings, path: std::path::PathBuf) -> Self {
        self.settings = settings;
        self.config_path = Some(path);
        self
    }

    /// The settings file could not be read (`reason`): it is never written in this run.
    #[must_use]
    pub fn with_locked_settings(mut self, reason: Option<String>) -> Self {
        self.settings_locked = reason;
        self
    }

    /// A message shown at the top until dismissed.
    #[must_use]
    pub fn with_notice(mut self, notice: String) -> Self {
        self.notices.push(notice);
        self
    }

    fn restart(&mut self) {
        let spawn = std::mem::replace(&mut self.spawn, Box::new(|| unreachable!()));
        let settings = std::mem::take(&mut self.settings);
        let config_path = self.config_path.take();
        let locked = self.settings_locked.take();
        *self = AutoPobApp::new(spawn);
        self.settings = settings;
        self.config_path = config_path;
        self.settings_locked = locked;
    }

    /// The app's own values for PoB's settings go to the engine (R12).
    fn send_overrides(&mut self) {
        if !self.settings.overrides.is_empty() {
            self.send_overrides_always();
        }
    }

    fn send_overrides_always(&mut self) {
        let values = self.settings.overrides.clone();
        self.send(Command::SetOverrides { values });
    }

    /// The trade realm and league go to the engine, for PoB's trade windows (R10). Nothing
    /// answers it, so the UI doesn't wait.
    fn send_trade_defaults(&mut self) {
        self.engine.send(Command::SetTradeDefaults {
            realm: self.settings.trade_realm.clone(),
            league: self.settings.trade_league.clone(),
        });
    }

    /// Writes `settings` to `auto-pob.toml`. A damaged file moved aside is told to the user;
    /// a failure is returned.
    fn write_settings(&mut self, settings: &AppSettings) -> Result<(), String> {
        if let Some(reason) = &self.settings_locked {
            return Err(format!(
                "the settings file wasn't read, so it is left alone ({reason})"
            ));
        }
        let Some(path) = self.config_path.clone() else {
            return Ok(());
        };
        match settings::save_noting(&path, settings) {
            Ok(None) => Ok(()),
            Ok(Some(backup)) => {
                self.notices.push(format!(
                    "{} could not be read, so it was moved to {} and a new one written.",
                    path.display(),
                    backup.display()
                ));
                Ok(())
            }
            Err(e) => Err(format!("Couldn't save {}: {e:#}", path.display())),
        }
    }

    fn open_options(&mut self) {
        self.send(Command::OpenPopup(PopupKind::Options));
    }

    /// PoB's settings, as the engine sees them now; the answer to an Options Save also carries
    /// the overrides it moved, and then the file is written and the map sent again (R12).
    fn on_settings(&mut self, view: &crate::engine::SettingsView, seq: u64) {
        let on = |key: &str| view.rows.iter().any(|r| r.key == key && r.value == "true");
        self.scroll_options = crate::widgets::scroll_step::ScrollOptions {
            disable: on("disableScrollControlInteraction"),
            invert_slider: on("invertSliderScrollDirection"),
        };
        if view.options_lost {
            for command in self.options.lost() {
                self.send(command);
            }
            self.notices.push(
                "Options closed without saving: PoB saved its settings after a sign-in, so \
                 the changes in the window were dropped."
                    .into(),
            );
        }
        let Some(next) = self.options.on_settings(view, seq) else {
            return;
        };
        match self.write_settings(&next) {
            Ok(()) => {
                self.options.commit(&next);
                self.settings = next;
                self.send_overrides_always();
                self.send_trade_defaults();
            }
            Err(error) => {
                // not saved: the old settings stay, the draft comes back in a reopened window
                let draft = self.options.save_failed(next, error);
                self.send(Command::OpenPopup(PopupKind::Options));
                self.send(Command::SetOverrides { values: draft });
            }
        }
    }

    fn on_popup_snapshot(&mut self, snapshot: &Snapshot, seq: u64) {
        match snapshot {
            Snapshot::Popup(Some(view)) if matches!(view.kind, PopupKind::Options) => {
                self.options.window.on_snapshot_at(snapshot, seq);
            }
            Snapshot::Popup(Some(_)) => self.popup.on_snapshot_at(snapshot, seq),
            _ => {
                self.popup.on_snapshot_at(snapshot, seq);
                self.options.window.on_snapshot_at(snapshot, seq);
            }
        }
    }

    /// Draws the Options window and sends what it asks.
    fn options_window(&mut self, ui: &mut egui::Ui) {
        if !self.options.window.is_blocking() {
            return;
        }
        let now = ui.input(|i| i.time);
        let behind = self.behind();
        let out = self.options.show(ui.ctx(), now, behind);
        let save_at = out.save_at;
        for (i, command) in out.commands.into_iter().enumerate() {
            let seq = self.send(command);
            if save_at == Some(i) {
                self.options.save_sent(seq);
            }
        }
        // questions never leave the app waiting
        for query in out.queries {
            self.engine.send(query);
        }
    }

    /// What the UI does for PoB's settings: the zoom, the window title and the tree view's
    /// heat map colours and edge search circles (R13).
    fn apply_ui_settings(&mut self, ctx: &egui::Context) {
        let effects = options::ui_effects(self.options.rows());
        self.tree
            .set_ui_settings(effects.power_theme, effects.edge_search);
        if (effects.zoom - self.applied_zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(effects.zoom);
            self.applied_zoom = effects.zoom;
        }
        let build = (self.screen == Screen::Build && self.entering.is_none())
            .then(|| self.build.as_ref().map(|b| b.name.as_str()))
            .flatten();
        let title = options::window_title(effects.show_title, build);
        if title != self.applied_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.applied_title = title;
        }
    }

    /// Sends a command whose snapshots the UI waits for.
    fn send(&mut self, command: Command) -> u64 {
        if let Command::OpenPopup(kind) = &command {
            // a late edit would end the new session (R4), so every pending one goes first
            self.flush_edits();
            if matches!(kind, PopupKind::Options) {
                self.options.begin(&self.settings);
            } else {
                self.popup_tab = Some(self.tab);
            }
        }
        #[cfg(test)]
        self.sent_full.push(format!("{command:?}"));
        #[cfg(test)]
        self.sent_log.push(
            format!("{command:?}")
                .split(['(', ' '])
                .next()
                .unwrap_or_default()
                .to_owned(),
        );
        // a new open or import starts afresh: an earlier failure's check is for its own build
        // view only, which may never have come (no build was loaded)
        if matches!(
            command,
            Command::OpenBuild { .. } | Command::NewBuild | Command::ImportShareCode { .. }
        ) || matches!(&command, Command::ImportFromUrl { mode, .. } if *mode != ImportMode::Comparison)
        {
            self.failed_open = None;
        }
        let is_notes = matches!(command, Command::SetNotes { .. });
        // a comparison keeps the build: its answers never replace it
        let is_import = matches!(
            command,
            Command::ImportFromUrl { mode, .. } if mode != ImportMode::Comparison
        );
        // the Import tab follows the answers to these
        let echo = matches!(
            command,
            Command::SignIn
                | Command::ExportShareCode
                | Command::SetImportCode { .. }
                | Command::ImportCharacter { .. }
                | Command::ImportPublicCharacter { .. }
        )
        .then(|| command.clone());
        let is_close = matches!(command, Command::ClosePopup);
        let is_press = matches!(command, Command::PressPopupButton { .. });
        // only the trade windows' link buttons answer with a link (R20)
        let is_link_press = matches!(
            &command,
            Command::PressPopupButton { name } if name == "search" || name == "searchTradeButton"
        );
        let is_open = matches!(command, Command::OpenPopup(_));
        let is_options = matches!(command, Command::OpenPopup(PopupKind::Options));
        let seq = self.engine.send(command);
        if is_options {
            self.options.window.open_sent(seq);
        } else if is_open {
            self.popup.open_sent(seq);
        }
        if is_close {
            self.popup.close_sent(seq);
            self.options.window.close_sent(seq);
        }
        if is_press {
            self.popup.press_sent(seq);
        }
        if is_link_press {
            self.trade_link.pressed(seq);
        }
        self.last_sent = seq;
        if let Some(command) = echo {
            self.import_ui.sent(seq, &command);
        }
        if is_import {
            self.import_seq = Some(seq);
        }
        if is_notes {
            self.notes.sent(seq);
        }
        seq
    }

    /// True while the engine hasn't caught up with what we sent.
    fn behind(&self) -> bool {
        self.last_sent > self.last_snapshot
    }

    fn current_unsaved(&self) -> bool {
        self.screen == Screen::Build
            && self.entering.is_none()
            && (self.notes.has_pending()
                || self.skills.has_pending()
                || self.sidebar.has_pending()
                || self.top_bar.has_pending()
                || self.calcs.has_pending()
                || self.build.as_ref().is_some_and(|b| b.unsaved))
    }

    fn list_folder(&self) -> String {
        self.list
            .as_ref()
            .map(|l| l.folder.clone())
            .unwrap_or_default()
    }

    // ---- events -------------------------------------------------------------------------

    fn process_events(&mut self) {
        while let Ok(event) = self.engine.events.try_recv() {
            self.on_event(event);
        }
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Downloading(p) => self.progress = Some(p),
            Event::Ready => {
                self.ready = true;
                self.send_overrides();
                self.send_trade_defaults();
            }
            Event::StartupFailed { message } => self.startup_error = Some(message),
            Event::Snapshot { seq, snapshot } => {
                self.last_snapshot = self.last_snapshot.max(seq);
                // The events are in order, so everything from the previous build has arrived
                // by the new build's first snapshot. Only now is the tree tab's state for the
                // old build (and any late power events) dropped.
                if self.entering.is_some_and(|e| seq >= e) {
                    self.entering = None;
                    self.reset_build_state();
                }
                // an import that answered at once replaced the build: its first view comes
                // before the new build's others
                if matches!(snapshot, Snapshot::Build(_)) && self.import_seq == Some(seq) {
                    self.import_seq = None;
                    self.reset_for_new_build();
                }
                // a link's answer is its request, with no new build yet: the import is over as
                // far as the reset goes (a code's has no request left, every one was cancelled)
                if self.import_seq == Some(seq)
                    && matches!(&snapshot, Snapshot::Requests(view) if !view.pending.is_empty())
                {
                    self.import_seq = None;
                }
                self.import_ui.on_snapshot(&snapshot, seq);
                self.compare.on_snapshot(&snapshot);
                // PoB switched to its Compare tab: an Import-tab comparison landed (R5). PoB
                // switches at the user's Import click; a link's comparison lands later, so
                // a window open by then keeps its tab, which leaving would end
                if self.compare.take_switched() {
                    if self.tab == BuildTab::Compare || !(self.modal() || self.popup.is_open()) {
                        self.tab = BuildTab::Compare;
                    } else {
                        self.notices
                            .push("Comparison imported: see the Compare tab".to_owned());
                    }
                }
                match snapshot {
                    Snapshot::Build(view) => {
                        let replaced = self
                            .failed_open
                            .take_if(|(at, _)| seq >= *at)
                            .is_some_and(|(_, before)| view.generation != before);
                        self.build = Some(view);
                        if replaced {
                            self.on_failed_open();
                        }
                        self.last_unsaved = self.current_unsaved();
                        if self.pending_save.as_ref().is_some_and(|p| seq >= p.seq) {
                            self.save_succeeded();
                        }
                    }
                    Snapshot::BuildList(view) => {
                        // PoB reopened the build its last exit recorded: it starts on that
                        // build, not on its list
                        if seq == 0
                            && view.build_open
                            && self.screen == Screen::List
                            && self.entering.is_none()
                        {
                            self.screen = Screen::Build;
                            self.tab = BuildTab::default();
                        }
                        self.list = Some(view);
                    }
                    Snapshot::Stats(view) => {
                        // a new calculation: the "what if" answers compared against the old one
                        self.skills.forget_hover();
                        self.config_ui.forget_hover();
                        self.sidebar.forget_hover();
                        self.tree.forget_hover();
                        self.calcs.group_hover.forget();
                        self.stats = Some(view);
                    }
                    Snapshot::Config(view) => {
                        // the answers to hovers were about the options before
                        self.config_ui.forget_hover();
                        self.config = Some(view);
                    }
                    Snapshot::Calcs(_) => self.calcs.on_snapshot(&snapshot),
                    Snapshot::GroupTooltip(ref t) => {
                        // the Skills list, the sidebar and the Calcs selector each ask
                        self.sidebar.group_hover.on_tooltip(t);
                        self.calcs.group_hover.on_tooltip(t);
                        self.skills.on_snapshot(&snapshot);
                    }
                    Snapshot::Skills(_)
                    | Snapshot::SocketGroupText(_)
                    | Snapshot::GemSearch(_)
                    | Snapshot::GemTooltip(_) => self.skills.on_snapshot(&snapshot),
                    // the Skills and Config tabs each keep the answers to their own hovers
                    Snapshot::HoverTooltip(view) => {
                        self.skills
                            .on_snapshot(&Snapshot::HoverTooltip(view.clone()));
                        self.sidebar.on_hover(&view);
                        self.tree.on_hover(&view);
                        self.config_ui.on_hover(&view);
                    }
                    Snapshot::Items(_)
                    | Snapshot::DisplayItem(_)
                    | Snapshot::ItemTooltip(_)
                    | Snapshot::ItemText(_)
                    | Snapshot::ItemDb(_) => self.items.on_snapshot_at(&snapshot, seq),
                    Snapshot::Breakdown(_) => self.calcs.on_snapshot(&snapshot),
                    Snapshot::Popup(_) => self.on_popup_snapshot(&snapshot, seq),
                    // the popup window, the display panel and the database panel each keep the
                    // answers to their own questions
                    Snapshot::ControlTooltip(_) => {
                        self.on_popup_snapshot(&snapshot, seq);
                        self.items.on_snapshot_at(&snapshot, seq);
                    }
                    Snapshot::Notes(_) => self.notes.on_snapshot(&snapshot, seq),
                    Snapshot::Requests(view) => self.on_requests(&view),
                    Snapshot::Settings(view) => self.on_settings(&view, seq),
                    // the Import tab keeps its own
                    Snapshot::Share(_) | Snapshot::ShareCode(_) | Snapshot::Account(_) => {}
                    // the Compare tab took it above
                    Snapshot::Compare(_) => {}
                    Snapshot::ExternalUrl(link) => self.trade_link.on_link(seq, &link.url),
                    Snapshot::Help(view) => self.help.on_view(view),
                    // power results of the build being left
                    Snapshot::NodePower(_) | Snapshot::PowerReport(_)
                        if self.entering.is_some() => {}
                    snapshot => self.tree.on_snapshot(&snapshot),
                }
            }
            Event::Error { seq, message } => self.on_error(seq, message),
            // power events of the build being left arrive before the new build's snapshots
            Event::Progress { .. } | Event::BackgroundFailed { .. } if self.entering.is_some() => {}
            Event::Progress {
                task: BackgroundTask::GemDps,
                percent,
            } => self.skills.on_gem_progress(percent),
            // the picker offers Retry; a failure with no picker open has nothing to tell
            Event::BackgroundFailed {
                task: BackgroundTask::GemDps,
                ..
            } => self.skills.on_gem_failed(),
            Event::Progress {
                task: BackgroundTask::AnointSort,
                percent,
            } => self.popup.on_progress(percent),
            Event::BackgroundFailed {
                task: BackgroundTask::AnointSort,
                message,
            } => self.popup.on_failed(message),
            Event::Progress {
                task: BackgroundTask::Timeless,
                percent,
            } => self.popup.on_search_progress(percent),
            Event::BackgroundFailed {
                task: BackgroundTask::Timeless,
                message,
            } => self.popup.on_search_failed(message),
            Event::Progress {
                task: BackgroundTask::ItemDb,
                percent,
            } => self.items.on_db_progress(percent),
            Event::BackgroundFailed {
                task: BackgroundTask::ItemDb,
                message,
            } => self.items.on_db_failed(message),
            Event::Progress { percent, .. } => self.tree.on_progress(percent),
            Event::BackgroundFailed { message, .. } => {
                self.tree.power_failed();
                self.notices.push(message);
            }
            // PoB's message popups (TreeTab:610's "Tree Converted") become notices
            Event::Message { title, text } => self.notices.push(format!("{title}: {text}")),
        }
    }

    /// Forgets the build's per-build UI state, as for another build.
    fn reset_build_state(&mut self) {
        self.tree.reset();
        self.calcs.reset();
        self.skills.reset();
        self.items.reset();
        self.popup.reset();
        self.popup_tab = None;
        self.sidebar = sidebar::SidebarState::default();
        self.top_bar = build::TopBarState::default();
        self.notes.reset();
        self.config_ui.reset();
        self.import_ui.reset();
        self.compare.reset();
        // the engine dropped the overlay with the comparisons
        self.compare_overlay = false;
    }

    /// A network step replaced the build (R14): the build screen starts over, as for
    /// `enter_build`.
    fn reset_for_new_build(&mut self) {
        self.entering = None;
        self.screen = Screen::Build;
        self.tab = BuildTab::default();
        self.reset_build_state();
    }

    fn on_requests(&mut self, view: &RequestsView) {
        self.requests = RequestsView {
            pending: view.pending.clone(),
            finished: Vec::new(),
        };
        for done in &view.finished {
            // a cancel is the user's own doing
            if let Some(error) = done.error.as_ref().filter(|_| !done.cancelled) {
                self.notices.push(format!("{}: {error}", done.label));
            }
        }
        // a link whose build PoB couldn't open: a new one stands in, which PoB never shows
        if view
            .finished
            .iter()
            .any(|f| f.new_build && f.error.is_some() && !f.cancelled)
        {
            self.on_failed_open();
        } else if view.finished.iter().any(|f| f.new_build) {
            // the step's other views follow this one (the engine sends the requests first)
            self.reset_for_new_build();
        }
    }

    /// Shows a passive node on the tree tab.
    fn show_node(&mut self, id: u32, now: f64) {
        self.tab = BuildTab::Tree;
        self.tree.focus(id, now);
    }

    fn on_error(&mut self, seq: u64, message: String) {
        // the Help window says it itself
        if self.help.on_error(seq, &message) {
            return;
        }
        if let Some(dialog) = &mut self.save_as
            && let Some((_, back)) = dialog.new_folder.take_if(|(s, _)| *s == seq)
        {
            dialog.folder = back;
        }
        if self.list_move_seq == Some(seq) {
            self.list_move_seq = None;
            self.list_ui.move_refused();
        }
        self.import_ui.on_error(seq);
        if self.import_seq == Some(seq) || self.entering == Some(seq) {
            let generation = self.build.as_ref().map_or(0, |b| b.generation);
            self.failed_open = Some((seq, generation));
        }
        if self.import_seq == Some(seq) {
            self.import_seq = None;
        }
        // the window stays open for another try (R4)
        self.popup.on_error(seq);
        self.options.on_error(seq);
        if self.text_seq == Some(seq) {
            self.text_seq = None;
            // the raw-text window stays open for another try
            self.items.text_refused();
        }
        if self.entering == Some(seq) {
            // the build never opened; go back to where the user was
            self.entering = None;
            self.screen = Screen::List;
        }
        if self.pending_save.as_ref().is_some_and(|p| p.seq == seq) {
            let pending = self.pending_save.take().expect("checked above");
            if let Some(dialog) = &mut self.save_as {
                dialog.waiting = false;
                if pending.is_save_as && message.contains("exists") {
                    dialog.confirm_overwrite = true;
                    return;
                }
            }
        }
        self.notices.push(message);
    }

    fn save_succeeded(&mut self) {
        let Some(pending) = self.pending_save.take() else {
            return;
        };
        self.save_as = None;
        if let Some(then) = pending.then {
            self.ready_actions.push(then);
        }
    }

    // ---- actions ------------------------------------------------------------------------

    /// Asks first if there are unsaved changes.
    fn request(&mut self, action: Pending) {
        // leaving or replacing the build cancels its requests (R13); cancelling a running
        // OAuth fetch or import may sign the user out in both apps
        if !matches!(action, Pending::Close) && self.import_ui.pending_signs_out() {
            self.confirm = Some(AppConfirm {
                message: "Leaving now cancels your Path of Exile import, which may sign you out of Path of Exile in both apps."
                    .into(),
                yes: "Leave",
                no: "Stay",
                then: ConfirmThen::Leave(action),
            });
            return;
        }
        self.request_checked(action);
    }

    fn request_checked(&mut self, action: Pending) {
        if needs_prompt(self.current_unsaved(), &action) {
            self.prompt = Some(action);
        } else {
            self.ready_actions.push(action);
        }
    }

    /// Sends the notes editor's pending edit, so it precedes whatever command comes next.
    fn flush_notes(&mut self) {
        if let Some(command) = self.notes.flush() {
            self.send(command);
        }
    }

    /// Sends every editor's pending edit (notes, then a Skills field still being typed in).
    fn flush_edits(&mut self) {
        self.flush_notes();
        if let Some(command) = self.sidebar.flush() {
            self.send(command);
        }
        if let Some(command) = self.top_bar.flush() {
            self.send(command);
        }
        if let Some(command) = self.calcs.flush() {
            self.send(command);
        }
        if let Some(command) = self.skills.flush() {
            self.send(command);
        }
        for command in self.items.flush() {
            self.send(command);
        }
        if let Some(command) = self.popup.flush() {
            self.send(command);
        }
        if let Some(command) = self.options.window.flush() {
            self.send(command);
        }
    }

    fn run_action(&mut self, ctx: &egui::Context, action: Pending) {
        self.flush_edits();
        match action {
            Pending::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::BackToList => self.back_to_list(),
            Pending::Open(path) => self.enter_build(Command::OpenBuild { path }),
            Pending::New => self.enter_build(Command::NewBuild),
            Pending::ImportNew(url) => {
                self.send(Command::ImportFromUrl {
                    url,
                    mode: ImportMode::NewBuild,
                });
            }
        }
    }

    fn back_to_list(&mut self) {
        self.screen = Screen::List;
        // the heat map would go on calculating for a build nobody is looking at
        if let Some(stop) = self.tree.stop_heatmap() {
            self.send(stop);
        }
        let folder = self.list_folder();
        self.send(Command::ListBuilds { folder });
    }

    /// An open or import failed after PoB's Init ran: the engine put a new build in the failed
    /// one's place. PoB shows its build list, with its message (already a notice), so the app
    /// does too, and forgets the old build's tab state.
    fn on_failed_open(&mut self) {
        self.entering = None;
        self.import_seq = None;
        self.reset_build_state();
        self.tab = BuildTab::default();
        self.back_to_list();
    }

    /// Sends a command that loads a different build and shows the build screen when it's done.
    fn enter_build(&mut self, command: Command) {
        let seq = self.send(command);
        self.entering = Some(seq);
        self.screen = Screen::Build;
        self.tab = BuildTab::default();
        // the tree tab resets when the build's first snapshot arrives (see `on_event`)
    }

    fn save(&mut self, then: Option<Pending>) {
        self.flush_edits();
        if self.build.as_ref().is_some_and(|b| b.path.is_some()) {
            let seq = self.send(Command::Save);
            self.pending_save = Some(PendingSave {
                seq,
                then,
                is_save_as: false,
            });
        } else {
            self.open_save_as(then);
        }
    }

    fn open_save_as(&mut self, then: Option<Pending>) {
        // PoB's box opens on the build's name with the characters it forbids turned into "-"
        // (Build.lua:1400)
        let name = self
            .build
            .as_ref()
            .map(|b| b.save_as_name.clone())
            .unwrap_or_default();
        self.save_as = Some(SaveAsDialog {
            folder: self.list_folder(),
            name,
            then,
            confirm_overwrite: false,
            waiting: false,
            picker: FolderPicker::default(),
            new_folder: None,
        });
    }

    fn send_save_as(&mut self, overwrite: bool) {
        self.flush_edits();
        let Some(dialog) = &mut self.save_as else {
            return;
        };
        dialog.waiting = true;
        dialog.confirm_overwrite = false;
        let command = Command::SaveAs {
            folder: dialog.folder.clone(),
            name: dialog.name.clone(),
            overwrite,
        };
        let then = dialog.then.clone();
        let seq = self.send(command);
        self.pending_save = Some(PendingSave {
            seq,
            then,
            is_save_as: true,
        });
    }

    /// The engine history Undo and Redo work on here, if any.
    fn undo_tab(&self) -> Option<Tab> {
        match self.tab {
            BuildTab::Config => Some(Tab::Config),
            BuildTab::Tree => Some(Tab::Tree),
            BuildTab::Skills => Some(Tab::Skills),
            BuildTab::Items => Some(Tab::Items),
            BuildTab::Calcs => Some(Tab::Calcs),
            // the Compare tab's Tree view edits our tree, as the Tree tab does
            BuildTab::Compare if self.compare.tree_shown() => Some(Tab::Tree),
            // the notes editor keeps its own history; the Compare tab's copies use the
            // target tab's undo (R8)
            BuildTab::Notes | BuildTab::Compare | BuildTab::Import => None,
        }
    }

    fn undo_redo(&mut self, redo: bool) {
        let Some(tab) = self.undo_tab() else {
            return;
        };
        self.send(if redo {
            Command::Redo(tab)
        } else {
            Command::Undo(tab)
        });
    }

    // ---- drawing ------------------------------------------------------------------------

    /// Draws one frame. (`eframe::App::ui` calls this.)
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.process_events();
        crate::widgets::scroll_step::set_options(&ctx, self.scroll_options);
        self.right_click_paste(&ctx);

        // closing the window with unsaved changes asks first
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.engine.is_alive()
            && self.current_unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.prompt = Some(Pending::Close);
            // the prompt sits under the window's modal layer, so the window goes first
            if let Some(command) = self.popup.close() {
                self.send(command);
            }
            for command in self.options.close() {
                self.send(command);
            }
        }

        for action in std::mem::take(&mut self.ready_actions) {
            self.run_action(&ctx, action);
        }

        if self.startup_error.is_some() || (!self.ready && self.engine.is_alive()) {
            let action = startup::show(
                ui,
                self.pob_version,
                self.progress,
                self.startup_error.as_deref(),
            );
            if action.is_some() {
                self.restart();
            }
            return;
        }
        if !self.engine.is_alive() {
            self.stopped_screen(ui);
            return;
        }

        if self.import_ui.take_focus() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        for command in self.import_ui.take_ready() {
            self.send(command);
        }
        if let Some(url) = self.import_ui.take_open_url() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        // the user's own press asked for it; PoB copies it too (R20)
        if let Some(url) = self.trade_link.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url.clone()));
            ctx.copy_text(url);
        }
        self.ask_about_startup_link();
        if let Some(rx) = &self.release_rx
            && let Ok(release) = rx.try_recv()
        {
            self.release = Some(release);
        }
        self.notices_panel(ui);
        self.pending_panel(ui);
        self.main_labels(ui);
        // the app's own windows sit above the screen's: Esc is theirs while one is open
        crate::widgets::escape::hold(&ctx, self.app_window_open());
        match self.screen {
            Screen::List => self.list_screen(ui),
            Screen::Build => self.build_screen(ui),
        }
        crate::widgets::escape::hold(&ctx, false);
        self.options_window(ui);
        self.help_window(&ctx);
        self.apply_ui_settings(&ctx);
        self.dialogs(&ctx);
    }

    /// The Help window, and the fetch of PoB's help files its button asks for (I3): no
    /// request is made without that click.
    fn help_window(&mut self, ctx: &egui::Context) {
        if self.help.show(ctx) && self.help_rx.is_none() {
            match &self.help_download {
                Some(download) => {
                    self.help_rx = Some(download());
                    self.help.download_started();
                }
                None => {
                    self.help
                        .download_done(Err("downloading isn't available here".into()));
                }
            }
        }
        if let Some(rx) = &self.help_rx
            && let Ok(result) = rx.try_recv()
        {
            self.help_rx = None;
            if let Some(query) = self.help.download_done(result) {
                let seq = self.engine.send(query);
                self.help.asked(seq);
            }
        }
    }

    /// Opens PoB's About window at `start`; its text is asked of the engine the first time.
    /// A question: it never leaves the app waiting.
    fn open_help(&mut self, start: HelpStart) {
        if let Some(query) = self.help.open(start) {
            #[cfg(test)]
            self.sent_log.push("Help".into());
            let seq = self.engine.send(query);
            self.help.asked(seq);
        }
    }

    /// What F1 would open on the wiki on this tab: the hovered node, item or gem.
    fn wiki_target(&self) -> Option<String> {
        match self.tab {
            BuildTab::Tree => self.tree.hovered_wiki(),
            BuildTab::Items => self.items.hovered_wiki(),
            BuildTab::Skills => self.skills.hovered_wiki(),
            _ => None,
        }
    }

    /// Carries out a shortcut of the build screen; Ctrl+W and Mouse4 come back as Back.
    fn run_shortcut(&mut self, ctx: &egui::Context, action: ShortcutAction) -> Option<BuildAction> {
        use ShortcutAction as A;
        match action {
            A::Tab(tab) => self.tab = tab,
            A::Import => {
                self.tab = BuildTab::Import;
                self.import_ui.focus_code();
            }
            A::Close => return Some(BuildAction::Back),
            A::Help => self.open_help(HelpStart::Tab(help::tab_name(Some(self.tab)))),
            // the user's own key asked for it (R20)
            A::Wiki(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            A::Search => match self.tab {
                BuildTab::Tree => self.tree.focus_search(),
                BuildTab::Config => self.config_ui.focus_search = true,
                BuildTab::Compare => self.compare.focus_search(),
                BuildTab::Items => {
                    if let Some(command) = self.items.focus_db_search(ctx) {
                        self.send(command);
                    }
                }
                _ => {}
            },
            A::CopyNode => {
                if let Some(text) = self.tree.hovered_node_text() {
                    ctx.copy_text(text);
                }
            }
            A::EditSlot => {
                if let Some(command) = self.items.edit_hovered_slot() {
                    self.send(command);
                }
            }
            tree => {
                if let Some(command) = self.tree.on_shortcut(&tree, ctx) {
                    self.send(command);
                }
            }
        }
        None
    }

    /// PoB's right-click into a text box selects it and pastes the clipboard there
    /// (EditControl.lua:527). The box under the pointer is last frame's; it takes the focus
    /// now and the paste at the next frame's start, when it has it.
    fn right_click_paste(&mut self, ctx: &egui::Context) {
        if let Some(text) = self.paste_next.take() {
            ctx.input_mut(|i| i.events.insert(0, egui::Event::Paste(text)));
            shortcuts::mark_right_click_paste(ctx);
        }
        if !ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary)) {
            return;
        }
        let Some(read) = &self.clipboard_read else {
            return;
        };
        let hovered: Vec<egui::Id> =
            ctx.interaction_snapshot(|s| s.hovered.iter().copied().collect());
        let target = hovered
            .into_iter()
            .find(|id| egui::TextEdit::load_state(ctx, *id).is_some())
            // a read-only or disabled box takes no paste (EditControl.lua:438)
            .filter(|id| {
                ctx.read_response(*id)
                    .is_some_and(|r| r.enabled() && r.sense.senses_click())
            });
        if let Some(id) = target
            && let Some(text) = read().filter(|t| !t.is_empty())
        {
            ctx.memory_mut(|m| m.request_focus(id));
            self.paste_next = Some(text.replace("\r\n", "\n"));
            ctx.request_repaint();
        }
    }

    /// PoB's labels at the foot of every screen (Main.lua:230-237): the fork and its version.
    fn main_labels(&mut self, ui: &mut egui::Ui) {
        let grey = crate::widgets::pob_text::pob_colour("^8", ui.visuals().weak_text_color());
        egui::Panel::bottom("pob_labels").show(ui, |ui| {
            ui.horizontal(|ui| {
                let fork = "PoB Community Fork".to_owned();
                let version = format!("Version: {}", self.pob_version);
                ui.colored_label(grey, &fork);
                ui.colored_label(grey, &version);
                #[cfg(test)]
                {
                    self.label_texts = vec![fork, version];
                }
            });
        });
    }

    /// The `pob://` argument never imports by itself: the user is asked first (R4).
    fn ask_about_startup_link(&mut self) {
        if self.confirm.is_some() {
            return;
        }
        if let Some(link) = self.startup_link.take() {
            self.confirm = Some(AppConfirm {
                message: format!("Import the build from this link?\n{link}"),
                yes: "Yes",
                no: "No",
                // it replaces the build shown by the time the answer comes, so the answer
                // goes through the same questions as the Import tab's new-build import
                then: ConfirmThen::Request(Pending::ImportNew(link)),
            });
        }
    }

    /// The release banner at the top of the build list.
    fn release_banner(&mut self, ui: &mut egui::Ui) {
        let Some(release) = &self.release else {
            return;
        };
        let mut dismiss = false;
        let mut open = false;
        egui::Panel::top("release_banner").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("auto-pob v{} is available", release.version));
                let link = ui.link("See the release");
                crate::ui::import::mark(ui, "release_link", &link);
                open = link.clicked();
                let close = ui.small_button("\u{00d7}");
                crate::ui::import::mark(ui, "release_dismiss", &close);
                dismiss = close.clicked();
            });
        });
        if open {
            ui.ctx()
                .open_url(egui::OpenUrl::new_tab(release.url.clone()));
        }
        if dismiss {
            self.release = None;
            self.release_rx = None;
        }
    }

    fn stopped_screen(&mut self, ui: &mut egui::Ui) {
        let mut restart = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                theme::heading(ui, "Engine stopped");
                ui.add_space(8.0);
                ui.label("The Path of Building engine has stopped working.");
                if self.last_unsaved {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Changes you made since the last save are lost.",
                    );
                }
                ui.add_space(8.0);
                if ui.add(theme::primary_button("Restart")).clicked() {
                    restart = true;
                }
            });
        });
        if restart {
            self.restart();
        }
    }

    fn notices_panel(&mut self, ui: &mut egui::Ui) {
        if self.notices.is_empty() {
            return;
        }
        let mut dismiss = None;
        egui::Panel::top("notices").show(ui, |ui| {
            for (i, notice) in self.notices.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("x").clicked() {
                        dismiss = Some(i);
                    }
                    ui.colored_label(ui.visuals().error_fg_color, notice);
                });
            }
        });
        if let Some(i) = dismiss {
            self.notices.remove(i);
        }
    }

    /// The pending-request line, on every screen.
    fn pending_panel(&mut self, ui: &mut egui::Ui) {
        if self.requests.pending.is_empty() {
            return;
        }
        let mut cancel = None;
        egui::Panel::bottom("pending_requests").show(ui, |ui| {
            cancel = requests::show(ui, &self.requests);
        });
        if let Some(Command::CancelRequest { id }) = cancel {
            let label = self
                .requests
                .pending
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.label.clone())
                .unwrap_or_default();
            // PoB signs out when a token refresh is cancelled, in both apps (they share
            // Settings.xml)
            if self.import_ui.cancel_signs_out(&label) {
                self.confirm = Some(AppConfirm {
                    message: "Cancelling now may sign you out of Path of Exile in both apps."
                        .into(),
                    yes: "Cancel request",
                    no: "Keep waiting",
                    then: ConfirmThen::Cancel(id),
                });
            } else {
                self.send(Command::CancelRequest { id });
            }
        }
    }

    fn list_screen(&mut self, ui: &mut egui::Ui) {
        // a Game Version window met on the list (the build PoB opened at start) is cancelled:
        // PoB's Cancel goes to the list, where the user already is (Build.lua:1344-1347)
        if matches!(self.popup.kind(), Some(PopupKind::GameVersion))
            && self.list.as_ref().is_some_and(|l| !l.build_open)
            && let Some(command) = self.popup.close()
        {
            self.send(command);
        }
        // PoB's edit box has no Ctrl+W (EditControl.lua:500-640): egui's word delete never
        // sees it
        strip_ctrl_w(ui);
        self.release_banner(ui);
        let mut action = None;
        let blocked = self.prompt.is_some()
            || self.save_as.is_some()
            || self.import.is_some()
            || self.confirm.is_some()
            || self.popup.is_blocking()
            || self.options.window.is_blocking()
            || self.help.is_open();
        crate::widgets::scroll_step::set_scope(
            ui.ctx(),
            crate::widgets::scroll_step::FocusScope {
                typing: ui.ctx().text_edit_focused(),
                modal: blocked,
                ..Default::default()
            },
        );
        // F1: the Help window ("Build List tab" matches no section: the first, Main.lua:441)
        let state = ShortcutState {
            typing: ui.ctx().text_edit_focused(),
            modal: blocked,
            ..ShortcutState::default()
        };
        let shortcut = ui.input_mut(|i| {
            let a = shortcuts::shortcut_action(&state, i);
            if let Some(a) = &a {
                shortcuts::consume(a, i);
            }
            a
        });
        if shortcut == Some(ShortcutAction::Help) {
            self.open_help(HelpStart::Tab(help::tab_name(None)));
        }
        egui::CentralPanel::default().show(ui, |ui| match self.list.clone() {
            Some(view) => action = self.list_ui.show(ui, &view, blocked),
            None => {
                ui.spinner();
            }
        });
        if let Some(action) = action {
            self.on_list_action(action);
        }
    }

    fn on_list_action(&mut self, action: ListScreenAction) {
        match action {
            ListScreenAction::OpenFolder(folder) => {
                self.send(Command::ListBuilds { folder });
            }
            ListScreenAction::OpenBuild(path) => self.request(Pending::Open(path)),
            ListScreenAction::New => self.request(Pending::New),
            ListScreenAction::Import => self.import = Some(String::new()),
            ListScreenAction::Options => self.open_options(),
            // a refusal comes back as PoB's own text and is shown like any other error
            ListScreenAction::NewFolder(name) => {
                self.send(Command::NewFolder { name });
            }
            ListScreenAction::Rename { path, name } => {
                self.send(Command::RenameBuild { path, name });
            }
            ListScreenAction::Copy { path, name } => {
                self.send(Command::CopyBuild { path, name });
            }
            ListScreenAction::Delete { path, kind } => {
                self.send(Command::DeleteBuild { path, kind });
            }
            ListScreenAction::Move { path, folder } => {
                let seq = self.send(Command::MoveBuild { path, folder });
                self.list_move_seq = Some(seq);
            }
            ListScreenAction::CopyTo { path, folder } => {
                self.send(Command::CopyBuildTo { path, folder });
            }
            ListScreenAction::SetSort(sort) => {
                self.send(Command::SetBuildSort { sort });
            }
            ListScreenAction::SetFilter(text) => {
                self.send(Command::SetBuildFilter { text });
            }
            ListScreenAction::About => self.open_help(HelpStart::VersionHistory),
        }
    }

    /// One of the app's own windows is open, drawn after the screen and above its windows:
    /// the questions, Save as, Import code, a popup session, Options and Help. The top bar's
    /// Set Name takes its own Esc before the tabs' windows are drawn.
    fn app_window_open(&self) -> bool {
        self.top_bar.has_dialog()
            || self.prompt.is_some()
            || self.save_as.is_some()
            || self.import.is_some()
            || self.confirm.is_some()
            || self.popup.is_blocking()
            || self.options.window.is_blocking()
            || self.help.is_open()
    }

    /// A window is open on the build screen that PoB would hold as a popup: nothing behind
    /// it gets a key (Main.lua:365-367)
    fn modal(&self) -> bool {
        self.prompt.is_some()
            || self.save_as.is_some()
            || self.import.is_some()
            || self.confirm.is_some()
            || self.tree.has_dialog()
            || self.skills.has_dialog()
            || self.items.has_dialog()
            || self.compare.has_dialog()
            || self.top_bar.has_dialog()
            || self.popup.is_blocking()
            || self.options.window.is_blocking()
            // PoB's About is a popup too
            || self.help.is_open()
    }

    fn build_screen(&mut self, ui: &mut egui::Ui) {
        let (Some(view), None) = (self.build.clone(), self.entering) else {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    ui.spinner();
                    ui.label("Opening build…");
                });
            });
            return;
        };

        // Shortcuts. Nothing fires under a modal. While a text field has focus, Ctrl+Z and
        // Ctrl+Y are left unconsumed for the field's own undo; Ctrl+S still saves.
        let modal = self.modal();
        // a dropped list of the top bar's or the sidebar's is Build's selected control: it
        // takes every key, as a Build box does (Build.lua:1230, ControlHost.lua:35-45)
        let build_dropped = crate::widgets::scroll_step::take_build_dropped(ui.ctx());
        // a focused button or row doesn't take Ctrl+Z from the tab
        let typing = ui.ctx().text_edit_focused();
        let mut action = None;
        if !modal {
            let command = egui::Modifiers::COMMAND;
            ui.input_mut(|i| {
                if i.consume_key(command, egui::Key::S) {
                    action = Some(BuildAction::Save);
                }
                if !typing && self.undo_tab().is_some() {
                    // PoB's tabs read Ctrl+Z as undo whatever else is held, Ctrl+Shift+Z too
                    // (TreeTab.lua:359, SkillsTab.lua:622): `consume_key` ignores extra Shift
                    if i.consume_key(command, egui::Key::Z) {
                        action = action.or(Some(BuildAction::Undo));
                    } else if i.consume_key(command, egui::Key::Y) {
                        action = action.or(Some(BuildAction::Redo));
                    }
                }
            });
        }
        // PoB's other shortcuts (R25)
        let ctx = ui.ctx().clone();
        let build_box = build_dropped
            || (typing
                && ctx
                    .memory(|m| m.focused())
                    .and_then(|id| ctx.read_response(id))
                    .is_some_and(|r| self.bar_rects.iter().any(|b| b.contains(r.rect.center()))));
        let typing = typing || build_dropped;
        // the dropdowns and sliders hold their keys while their host has a box focused, and
        // the panels' ones under a window
        crate::widgets::scroll_step::set_scope(
            &ctx,
            crate::widgets::scroll_step::FocusScope {
                typing,
                build_box,
                build_host: false,
                modal,
            },
        );
        // under a window only the gem list's own rows open a page
        let wiki = if modal {
            (self.tab == BuildTab::Skills && self.skills.picker_open())
                .then(|| self.skills.hovered_wiki())
                .flatten()
        } else {
            self.wiki_target()
        };
        let state = ShortcutState {
            tab: Some(self.tab),
            typing,
            modal,
            spec_select_busy: self.tree.spec_select_busy(),
            wiki,
            build_box,
            copy_node: self.tab == BuildTab::Tree && self.tree.hovered_node_text().is_some(),
            hover_wiki: self.tab != BuildTab::Tree,
        };
        let shortcut = ui.input_mut(|i| {
            let a = shortcuts::shortcut_action(&state, i);
            // while typing PoB's keys reach the box too
            if let Some(a) = &a
                && !typing
            {
                shortcuts::consume(a, i);
            }
            a
        });
        // PoB's edit box has no Ctrl+W (EditControl.lua:500-640): it closes the build and
        // edits nothing, so egui's word delete never sees it, under a window too
        strip_ctrl_w(ui);
        if let Some(shortcut) = shortcut {
            action = self.run_shortcut(ui.ctx(), shortcut).or(action);
        }

        let behind = self.behind();
        let mut top_commands = Vec::new();
        // the top bar and the sidebar are Build's own controls (Build.lua:1230)
        crate::widgets::scroll_step::set_build_host(ui.ctx(), true);
        // typed edits not yet sent count: PoB's boxes apply every keystroke
        let unsaved = self.current_unsaved();
        let top = egui::Panel::top("top_bar").show(ui, |ui| {
            let history = match self.undo_tab() {
                Some(Tab::Tree) => self.tree.history(),
                Some(Tab::Skills) => self.skills.history(),
                Some(Tab::Items) => self.items.history(),
                Some(Tab::Calcs) => self.calcs.history(),
                _ => (view.can_undo, view.can_redo),
            };
            let undo = self.undo_tab().is_some();
            let out = build::top_bar(
                ui,
                &view,
                undo,
                history,
                &mut self.top_bar,
                &mut self.tree,
                behind,
                unsaved,
            );
            action = out.action.or(action);
            top_commands = out.commands;
        });
        for command in top_commands {
            self.send(command);
        }
        self.skills.set_modal(modal);
        self.items.set_modal(modal);
        self.import_ui.set_modal(modal);
        let mut sidebar_commands = Vec::new();
        let side = egui::Panel::left("stats_sidebar")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                sidebar_commands =
                    sidebar::show(ui, self.stats.as_deref(), behind, &mut self.sidebar);
            });
        self.bar_rects = vec![top.response.rect, side.response.rect];
        crate::widgets::scroll_step::set_build_host(ui.ctx(), false);
        for command in sidebar_commands {
            self.send(command);
        }
        // questions never leave the app waiting
        for query in self.sidebar.take_queries() {
            self.engine.send(query);
        }

        let mut config_commands = Vec::new();
        let mut tree_out = TreeOutput::default();
        let mut calcs_out = CalcsOutput::default();
        let mut skills_out = SkillsOutput::default();
        let mut items_out = ItemsOutput::default();
        let mut notes_command = None;
        let mut import_out = ImportOutput::default();
        let mut compare_out = CompareOutput::default();
        let config = self.config.clone();
        egui::CentralPanel::default().show(ui, |ui| {
            build::tab_strip(ui, &mut self.tab);
            ui.separator();
            match self.tab {
                BuildTab::Tree => {
                    self.tree.set_enabled_sockets(self.items.enabled_sockets());
                    tree_out = self.tree.show(ui, behind);
                }
                BuildTab::Skills => {
                    let now = ui.input(|i| i.time);
                    skills_out = self.skills.show(ui, now, behind);
                }
                BuildTab::Items => {
                    let now = ui.input(|i| i.time);
                    items_out = self.items.show(ui, now, behind);
                }
                BuildTab::Calcs => calcs_out = self.calcs.show(ui, behind),
                BuildTab::Notes => {
                    let now = ui.input(|i| i.time);
                    notes_command = self.notes.show(ui, now);
                }
                BuildTab::Compare => {
                    compare_out = self.compare.show(ui, behind);
                    // R7's overlay: our tree, coloured against the comparison
                    if compare_out.tree {
                        self.tree.set_enabled_sockets(self.items.enabled_sockets());
                        tree_out = self.tree.show_compare(ui, behind);
                    }
                }
                BuildTab::Import => {
                    let now = epoch_now();
                    import_out = self.import_ui.show(ui, now);
                }
                BuildTab::Config => match &config {
                    Some(config) => {
                        config_commands = self.config_ui.show(ui, config, behind);
                    }
                    None => {
                        ui.spinner();
                    }
                },
            }
        });
        for command in config_commands {
            // a hover is a question: it never leaves the app waiting
            if matches!(command, Command::HoverTooltip(_)) {
                self.engine.send(command);
            } else {
                self.send(command);
            }
        }
        if let Some(command) = notes_command {
            self.send(command);
        } else if self.tab != BuildTab::Notes {
            // leaving the tab
            self.flush_notes();
        }
        self.import_output(ui.ctx(), import_out);
        if self.tab != BuildTab::Import
            && let Some(command) = self.import_ui.leave()
        {
            self.send(command);
        }
        // the Items tab's Manage Passive Trees window keeps its own dialogs
        if self.tab != BuildTab::Tree && !self.tree.spec_manager_open() {
            // the Compare tab's tree view asks the Class Change question too
            let compare_tree = self.tab == BuildTab::Compare && self.compare.tree_shown();
            self.tree.close_dialog_off_tab(compare_tree);
        }
        if self.tab != BuildTab::Compare {
            // leaving the tab sends a count still being typed and closes its windows
            if let Some(command) = self.compare.flush() {
                self.send(command);
            }
            self.compare.close_dialog();
        }
        for command in compare_out.commands {
            self.send(command);
        }
        // questions never leave the app waiting
        for query in compare_out.queries {
            self.engine.send(query);
        }
        // the overlay is on while the Compare tab's Tree view shows (R7)
        let overlay = self.tab == BuildTab::Compare && self.compare.tree_shown();
        if overlay != self.compare_overlay {
            self.compare_overlay = overlay;
            self.send(Command::SetCompareOverlay { on: overlay });
        }
        if self.tab != BuildTab::Skills {
            // leaving the tab sends an edit still being typed and closes its dialogs
            if let Some(command) = self.skills.flush() {
                self.send(command);
            }
            if let Some(command) = self.skills.close_dialogs() {
                self.send(command);
            }
        }
        if self.tab != BuildTab::Items {
            // leaving the tab sends an edit still being typed and closes its dialogs
            for command in self.items.flush() {
                self.send(command);
            }
            self.items.close_dialogs();
            if let Some(command) = self.items.leave() {
                self.send(command);
            }
        }
        // PoB's Game Version window is main's, not a tab's (Build.lua:1323-1350): no tab
        // switch ends it
        let game_version = matches!(self.popup.kind(), Some(PopupKind::GameVersion));
        // leaving the tab that opened the window ends its session
        if self.popup.is_open() && self.popup_tab != Some(self.tab) && !game_version {
            for command in self.popup.flush().into_iter().chain(self.popup.close()) {
                self.send(command);
            }
        }
        let popup_out = if self.popup.is_blocking() {
            let now = ui.input(|i| i.time);
            self.popup.show(ui.ctx(), now, behind)
        } else {
            popup::PopupOutput::default()
        };
        // the Game Version window ending any way but Convert is PoB's Cancel, which goes back
        // to the build list (Build.lua:1344-1347)
        let mut to_list = false;
        for command in popup_out.commands {
            to_list |= game_version
                && match &command {
                    Command::PressPopupButton { name } => name != "convert",
                    Command::ClosePopup => true,
                    _ => false,
                };
            self.send(command);
        }
        if to_list {
            self.run_action(ui.ctx(), Pending::BackToList);
        }
        // questions never leave the app waiting
        for query in popup_out.queries {
            self.engine.send(query);
        }
        // the Items tab's "Manage..." under its tree dropdown: PoB's Manage Passive Trees window
        if items_out.manage_trees {
            self.tree.open_spec_manager();
        }
        // the Tree tab draws its own; on the Items tab it is that tab's window, and leaving
        // for any other tab closes it
        if self.tab == BuildTab::Items {
            for command in self.tree.spec_manager(ui.ctx()) {
                self.send(command);
            }
        } else if self.tab != BuildTab::Tree && self.tree.spec_manager_open() {
            self.tree.close_spec_manager();
        }
        for command in items_out.commands {
            let is_text = matches!(command, Command::SetDisplayItemText { .. });
            let seq = self.send(command);
            if is_text {
                self.text_seq = Some(seq);
                self.items.text_sent(seq);
            }
        }
        // questions never leave the app waiting
        for query in items_out.queries {
            self.engine.send(query);
        }
        for command in skills_out.commands {
            self.send(command);
        }
        // questions never leave the app waiting
        for query in skills_out.queries {
            self.engine.send(query);
        }
        for command in tree_out.commands {
            self.send(command);
        }
        // PoB's right click on a jewel socket: the Items tab with that slot focused
        if let Some(node) = tree_out.jump_to_socket {
            self.tab = BuildTab::Items;
            self.items.focus_slot(node);
        }
        // questions are answered with snapshots but never leave the app waiting
        for query in tree_out.queries {
            self.engine.send(query);
        }

        // the Calcs sections popped out float over every tab, as PoB's overlay panes (R24)
        if self.tab != BuildTab::Calcs {
            calcs_out.commands.extend(self.calcs.show_popouts(ui.ctx()));
        }
        for command in calcs_out.commands {
            self.send(command);
        }
        // only the latest question matters; the others are about cells the pointer has left
        if let Some(query) = calcs_out.queries.pop() {
            self.engine.send(query);
        }
        for query in std::mem::take(&mut self.calcs.group_hover.queries) {
            self.engine.send(query);
        }
        if let Some(id) = calcs_out.focus_node {
            let now = ui.input(|i| i.time);
            self.show_node(id, now);
        }

        // Back goes through the unsaved prompt, which knows about the pending edit
        if matches!(action, Some(BuildAction::Save | BuildAction::SaveAs)) {
            self.flush_edits();
        }
        match action {
            Some(BuildAction::Back) => self.request(Pending::BackToList),
            Some(BuildAction::Save) => self.save(None),
            Some(BuildAction::SaveAs) => self.open_save_as(None),
            Some(BuildAction::Undo) => self.undo_redo(false),
            Some(BuildAction::Redo) => self.undo_redo(true),
            Some(BuildAction::Options) => self.open_options(),
            Some(BuildAction::Help) => self.open_help(HelpStart::Section(7)),
            Some(BuildAction::About) => self.open_help(HelpStart::VersionHistory),
            None => {}
        }
    }

    /// Carries out what the Import tab asked for: commands, questions, the browser and the
    /// clipboard (the engine does neither, R1).
    fn import_output(&mut self, ctx: &egui::Context, out: ImportOutput) {
        for command in out.commands {
            self.send(command);
        }
        match out.confirm {
            Some(Confirm::Ask { message, plan }) => {
                let then = match plan.single() {
                    Some(command) => ConfirmThen::Send(command),
                    None => ConfirmThen::Plan(plan),
                };
                let mut message = String::from(message);
                if self.import_ui.pending_signs_out() {
                    message.push_str(" It also cancels your running Path of Exile import, which may sign you out in both apps.");
                }
                self.confirm = Some(AppConfirm {
                    message,
                    yes: "Import",
                    no: "Cancel",
                    then,
                });
            }
            Some(Confirm::NewBuild { url }) => self.request(Pending::ImportNew(url)),
            None => {}
        }
        if let Some(url) = out.open_url {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        if let Some(text) = out.copy {
            ctx.copy_text(text);
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        // a cancel whose request ended while the question was open has nothing left to ask
        if let Some(AppConfirm {
            then: ConfirmThen::Cancel(id),
            ..
        }) = &self.confirm
            && !self.requests.pending.iter().any(|p| p.id == *id)
        {
            self.confirm = None;
        }
        let Some(confirm) = &self.confirm else {
            return;
        };
        // Esc answers No, as PoB's confirm popup (PopupDialog.lua:72-77)
        let window = egui::Id::unique("app_confirm");
        let mut choice = crate::widgets::escape::escape(ctx, window).then_some(false);
        // modal: the screen behind can't be used while the question is open, so the answer
        // is about the state the question was asked in
        egui::Modal::new(self.confirm_id).show(ctx, |ui| {
            ui.heading("Are you sure?");
            {
                ui.label(&confirm.message);
                ui.horizontal(|ui| {
                    let yes = ui.add(theme::primary_button(confirm.yes));
                    crate::ui::import::mark(ui, "confirm_yes", &yes);
                    if yes.clicked() {
                        choice = Some(true);
                    }
                    let no = ui.button(confirm.no);
                    crate::ui::import::mark(ui, "confirm_no", &no);
                    if no.clicked() {
                        choice = Some(false);
                    }
                });
            }
        });
        crate::widgets::escape::note(ctx, window);
        if let Some(yes) = choice {
            self.answer_confirm(yes);
        }
    }

    /// Carries out the answer to the open question.
    fn answer_confirm(&mut self, yes: bool) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        if !yes {
            return;
        }
        match confirm.then {
            ConfirmThen::Leave(action) => self.request_checked(action),
            ConfirmThen::Request(action) => self.request(action),
            ConfirmThen::Plan(plan) => {
                self.flush_edits();
                let command = self.import_ui.begin(plan);
                self.send(command);
            }
            ConfirmThen::Send(command) => {
                self.flush_edits();
                self.send(command);
            }
            ConfirmThen::Cancel(id) => {
                self.send(Command::CancelRequest { id });
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.confirm_dialog(ctx);
        self.prompt_dialog(ctx);
        self.save_as_dialog(ctx);
        self.import_dialog(ctx);
    }

    fn prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.prompt.clone() else {
            return;
        };
        // Esc is the Cancel (PopupDialog.lua:72-77)
        let window = egui::Id::unique("app_unsaved");
        let mut choice =
            crate::widgets::escape::escape(ctx, window).then_some(PromptChoice::Cancel);
        // PoB's Save Changes popup (Build.lua:1352-1379): "now?" when leaving the build,
        // "before exiting?" when the window closes
        let when = if matches!(pending, Pending::Close) {
            "before exiting?"
        } else {
            "now?"
        };
        egui::Window::new("Save Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "This build has unsaved changes.\nDo you want to save them {when}"
                ));
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("Save")).clicked() {
                        choice = Some(PromptChoice::Save);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(PromptChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(PromptChoice::Cancel);
                    }
                });
            });
        crate::widgets::escape::note(ctx, window);
        let Some(choice) = choice else {
            return;
        };
        self.answer_prompt(pending, choice);
    }

    /// The unsaved prompt's answer for what was about to happen.
    fn answer_prompt(&mut self, pending: Pending, choice: PromptChoice) {
        self.prompt = None;
        let has_path = self.build.as_ref().is_some_and(|b| b.path.is_some());
        match resolve(choice, has_path) {
            Resolution::SaveThen => self.save(Some(pending)),
            Resolution::SaveAsThen => self.open_save_as(Some(pending)),
            Resolution::Proceed => {
                // "Don't save" drops the edit still waiting to be sent
                self.notes.discard();
                self.skills.discard();
                self.sidebar.discard();
                self.top_bar.discard();
                self.calcs.discard();
                self.ready_actions.push(pending);
            }
            Resolution::Stay => {}
        }
    }

    fn save_as_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.save_as else {
            return;
        };
        let mut submit = false;
        let mut overwrite = false;
        // Esc is the Cancel, once the name box is left (PopupDialog.lua:72-77)
        let window = egui::Id::unique("app_save_as");
        let mut cancel = !dialog.waiting && crate::widgets::escape::escape(ctx, window);
        let mut picker_out = None;
        let list = self.list.clone();
        // PoB's title: "Save As" for a build with a file, else "Save" (Build.lua:1449)
        let title = if self.build.as_ref().is_some_and(|b| b.path.is_some()) {
            "Save As"
        } else {
            "Save"
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_enabled_ui(!dialog.waiting, |ui| {
                    ui.label("Enter new build name:");
                    // PoB's box drops `\/:*?"<>|` and control characters, 100 bytes at most
                    let (name, _) = name_box::edit(
                        ui,
                        &mut dialog.name,
                        Filter::Path,
                        name_box::NAME_LIMIT,
                        |t| t.hint_text("Build name"),
                    );
                    crate::ui::import::mark(ui, "save_as_name", &name);
                    if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    if let Some(view) = &list {
                        picker_out = dialog.picker.show(ui, &mut dialog.folder, view);
                    }
                    if dialog.confirm_overwrite {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!(
                                "A build named {} already exists. Overwrite it?",
                                dialog.name
                            ),
                        );
                        ui.horizontal(|ui| {
                            if ui.add(theme::primary_button("Overwrite")).clicked() {
                                overwrite = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            let valid = save_as_is_open(list.as_deref(), dialog);
                            let save = ui.add_enabled(valid, theme::primary_button("Save"));
                            crate::ui::import::mark(ui, "save_as_save", &save);
                            if save.clicked() {
                                submit = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    }
                });
            });
        crate::widgets::escape::note(ctx, window);
        if let Some(out) = picker_out {
            self.on_picker(out);
        }
        let Some(dialog) = &mut self.save_as else {
            return;
        };
        if cancel {
            self.save_as = None;
        } else if overwrite {
            self.send_save_as(true);
        } else if submit && save_as_is_open(list.as_deref(), dialog) {
            self.send_save_as(false);
        }
    }

    /// What the Save As folder list asked for. A refusal comes back as the engine's own text.
    fn on_picker(&mut self, out: PickerOutput) {
        match out {
            PickerOutput::Notice(text) => self.notices.push(text),
            PickerOutput::DeleteFolder(path) => {
                let Some(root) = self.list.as_ref().map(|l| l.root.clone()) else {
                    return;
                };
                let full = format!("{}{}", root.display(), path.trim_end_matches('/'));
                self.send(Command::DeleteBuild {
                    path: full.into(),
                    kind: DeleteKind::EmptyFolder,
                });
            }
            PickerOutput::NewFolder { folder, name } => {
                let seq = self.send(Command::NewFolderIn {
                    folder: folder.clone(),
                    name: name.clone(),
                });
                // PoB goes into the folder it made; if that fails the dialog comes back
                if let Some(dialog) = &mut self.save_as {
                    dialog.new_folder = Some((seq, folder.clone()));
                    dialog.folder = format!("{folder}{name}/");
                }
            }
        }
    }

    fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(text) = &mut self.import else {
            return;
        };
        let mut import = false;
        // Esc is the Cancel, once the code box is left (PopupDialog.lua:72-77)
        let window = egui::Id::unique("app_import_code");
        let mut cancel = crate::widgets::escape::escape(ctx, window);
        egui::Window::new("Import code")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Paste a PoB share code or a build link:");
                ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(4)
                        .desired_width(400.0),
                );
                ui.horizontal(|ui| {
                    let go =
                        ui.add_enabled(!text.trim().is_empty(), theme::primary_button("Import"));
                    crate::ui::import::mark(ui, "list_import", &go);
                    if go.clicked() {
                        import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        crate::widgets::escape::note(ctx, window);
        if import {
            let code = std::mem::take(text);
            self.import = None;
            if is_link(&code) {
                // the list stays until the engine reports a new build (R15)
                self.request(Pending::ImportNew(code.trim().to_owned()));
            } else {
                self.enter_build(Command::ImportShareCode { code });
            }
        } else if cancel {
            self.import = None;
        }
    }
}

/// Text the list's import sends as a link (http(s) or pob:) rather than as a share code.
fn is_link(text: &str) -> bool {
    let text = text.trim();
    let starts = |prefix: &str| {
        text.get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    };
    starts("http") || starts("pob:")
}

/// Seconds since the epoch, for the sign-in and retry countdowns.
fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl eframe::App for AutoPobApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

#[cfg(test)]
#[path = "app_options_tests.rs"]
mod options_tests;

#[cfg(test)]
#[path = "app_compare_tests.rs"]
mod compare_tests;

#[cfg(test)]
#[path = "app_trade_tests.rs"]
mod trade_tests;

/// Takes Ctrl+W out of the frame, so no text box deletes a word on it
fn strip_ctrl_w(ui: &egui::Ui) {
    ui.input_mut(|i| {
        i.events.retain(|e| {
            !matches!(e, egui::Event::Key { key: egui::Key::W, modifiers, .. }
                if modifiers.command || modifiers.ctrl)
        })
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::views::{
        BuildListEntry, ConfigKind, ConfigOption, ConfigSection, ConfigSet, CustomModGroup,
        StatRow, TooltipLine,
    };
    use crate::engine::{Affected, Backend, ConfigValue};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A backend that behaves like the real one closely enough to drive every screen.
    #[derive(Default)]
    struct Fake {
        path: Option<PathBuf>,
        unsaved: bool,
        folder: String,
        /// The build has one tree instead of two
        single_spec: bool,
        /// The node power calculation fails on its first step
        fail_power: bool,
        heatmap: Option<String>,
        /// A node power calculation is running
        power_work: bool,
        /// The tree layout went out once; the engine resends it only when it changes
        layout_sent: bool,
    }

    impl Backend for Fake {
        fn release_memory(&mut self) {}

        fn apply(&mut self, command: Command, _send: &dyn Fn(Event)) -> anyhow::Result<Affected> {
            let all = Affected {
                build: true,
                list: true,
                stats: true,
                config: true,
                tree_layout: !self.layout_sent,
                tree: true,
                ..Affected::default()
            };
            match command {
                Command::SetHeatmap { stat, .. } => {
                    self.power_work = stat.is_some();
                    self.heatmap = stat;
                    Ok(Affected {
                        tree: true,
                        power: true,
                        ..Affected::default()
                    })
                }
                Command::SetTreeSearch { text } if text == "boom" => {
                    anyhow::bail!("search broke")
                }
                Command::SetTreeSearch { .. } => Ok(Affected {
                    tree: true,
                    ..Affected::default()
                }),
                // an allocation or reset restarts the calculation of a heat map that is on
                Command::ResetTree => {
                    self.power_work = self.heatmap.is_some();
                    Ok(Affected {
                        tree: true,
                        power: true,
                        ..Affected::default()
                    })
                }
                Command::ExportTreeLink => Ok(Affected {
                    link: true,
                    ..Affected::default()
                }),
                Command::ShrinkTreeLink => Ok(Affected {
                    short_link: true,
                    requests: true,
                    ..Affected::default()
                }),
                Command::NewBuild => {
                    self.layout_sent = true;
                    self.path = None;
                    self.unsaved = false;
                    Ok(all)
                }
                Command::OpenBuild { path } => {
                    if path.to_string_lossy().contains("bad") {
                        anyhow::bail!("couldn't open that build");
                    }
                    self.layout_sent = true;
                    self.path = Some(path);
                    self.unsaved = false;
                    Ok(all)
                }
                Command::ListBuilds { folder } => {
                    self.folder = folder;
                    Ok(Affected {
                        list: true,
                        ..Affected::default()
                    })
                }
                Command::SetConfig { .. } => {
                    self.unsaved = true;
                    Ok(Affected {
                        build: true,
                        ..Affected::default()
                    })
                }
                Command::Save => {
                    self.unsaved = false;
                    Ok(Affected {
                        build: true,
                        ..Affected::default()
                    })
                }
                Command::SaveAs {
                    folder,
                    name,
                    overwrite,
                } => {
                    if name == "dup" && !overwrite {
                        anyhow::bail!("a build named 'dup' exists");
                    }
                    self.path = Some(format!("/b/{folder}{name}.xml").into());
                    self.unsaved = false;
                    Ok(Affected {
                        build: true,
                        list: true,
                        ..Affected::default()
                    })
                }
                Command::Help => Ok(Affected {
                    help: true,
                    ..Affected::default()
                }),
                _ => Ok(Affected::default()),
            }
        }

        fn recalculate(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn build_view(&mut self) -> anyhow::Result<BuildView> {
            Ok(BuildView {
                name: "Fake build".into(),
                save_as_name: "Fake build".into(),
                path: self.path.clone(),
                class: "Witch".into(),
                ascendancy: Some("Necromancer".into()),
                level: 90,
                unsaved: self.unsaved,
                can_undo: true,
                can_redo: false,
                ..BuildView::default()
            })
        }

        fn build_loaded(&mut self) -> anyhow::Result<bool> {
            Ok(true)
        }

        fn build_list_view(&mut self) -> anyhow::Result<BuildListView> {
            let entry = |name: &str, is_folder| BuildListEntry {
                name: name.into(),
                is_folder,
                folder: self.folder.clone(),
                path: format!("/b/{}{name}.xml", self.folder).into(),
                class: Some("Witch".into()),
                ascendancy: None,
                level: Some(90),
                delete_prompt: String::new(),
            };
            Ok(BuildListView {
                folder: self.folder.clone(),
                entries: vec![entry("sub", true), entry("one", false), entry("bad", false)],
                sort: Default::default(),
                filter: String::new(),
                ..Default::default()
            })
        }

        fn stats_view(&mut self) -> anyhow::Result<StatsView> {
            let row = |label: &str| StatRow {
                key: label.into(),
                label: label.into(),
                value: 1.0,
                text: "1,000".into(),
                color: Some("^xE05030".into()),
                notes: vec![],
            };
            Ok(StatsView {
                stats: vec![row("Life")],
                minion: vec![row("Minion Life")],
                ..StatsView::default()
            })
        }

        fn config_view(&mut self) -> anyhow::Result<ConfigView> {
            Ok(ConfigView {
                sets: vec![],
                active_set: 1,
                custom_mods: vec![],
                sections: vec![],
            })
        }

        fn tree_layout_view(&mut self) -> anyhow::Result<crate::engine::TreeLayout> {
            let node = |id, x| crate::engine::TreeNode {
                id,
                pos: [x, 0.0],
                kind: "Normal".into(),
                name: format!("Node {id}"),
                stats: vec![],
                orbit: None,
                group_center: None,
                art: crate::engine::NodeArt::default(),
                ascendancy: None,
                replaceable: false,
                mastery_effects: false,
            };
            Ok(crate::engine::TreeLayout {
                version: "3_28".into(),
                nodes: vec![node(1, 0.0), node(2, 600.0)],
                edges: vec![(0, 1)],
                sheets: vec![],
                key: "fake".into(),
            })
        }

        fn calcs_view(&mut self) -> anyhow::Result<crate::engine::CalcsView> {
            Ok(crate::engine::CalcsView::default())
        }

        fn skills_view(&mut self) -> anyhow::Result<crate::engine::SkillsView> {
            Ok(crate::engine::SkillsView::default())
        }

        fn items_view(&mut self) -> anyhow::Result<crate::engine::ItemsView> {
            Ok(crate::engine::ItemsView::default())
        }

        fn display_item_view(&mut self) -> anyhow::Result<Option<crate::engine::DisplayItemView>> {
            Ok(None)
        }

        fn socket_group_text(
            &mut self,
            group: usize,
        ) -> anyhow::Result<crate::engine::SocketGroupText> {
            Ok(crate::engine::SocketGroupText {
                group,
                text: String::new(),
            })
        }

        fn group_tooltip_view(
            &mut self,
            group: usize,
        ) -> anyhow::Result<crate::engine::GroupTooltipView> {
            Ok(crate::engine::GroupTooltipView {
                group,
                lines: vec![],
            })
        }

        fn notes_view(&mut self) -> anyhow::Result<crate::engine::NotesView> {
            Ok(crate::engine::NotesView::default())
        }

        fn tree_state_view(&mut self) -> anyhow::Result<crate::engine::TreeState> {
            let specs = if self.single_spec {
                serde_json::json!([{ "title": "Main", "version": "3_28", "points": 1 }])
            } else {
                serde_json::json!([
                    { "title": "Main", "version": "3_28", "points": 1 },
                    { "title": "Other", "version": "3_28", "points": 0 },
                ])
            };
            Ok(serde_json::from_value(serde_json::json!({
                "version": "3_28", "version_label": "3.28", "class_id": 0, "ascendancy_id": 0,
                "classes": [
                    { "id": 0, "name": "Witch", "needs_confirm": false, "ascendancies": [
                        { "id": 0, "name": "None", "ascend_id": "None" },
                        { "id": 1, "name": "Necromancer", "ascend_id": "Necromancer" } ] },
                    { "id": 1, "name": "Marauder", "needs_confirm": true },
                    { "id": 2, "name": "Ranger", "needs_confirm": false },
                ],
                "points": { "used": 1, "available": 122, "ascendancy_used": 0,
                            "ascendancy_available": 8 },
                "allocated": [1],
                "specs": specs, "active_spec": 0,
                "power_stats": ["Life", "Full DPS"], "heatmap": self.heatmap,
                "can_undo": true, "can_redo": false,
            }))?)
        }

        fn next_background(&self) -> Option<crate::engine::BackgroundTask> {
            self.power_work
                .then_some(crate::engine::BackgroundTask::NodePower)
        }

        fn background_step(
            &mut self,
            _task: crate::engine::BackgroundTask,
            send: &dyn Fn(Event),
        ) -> anyhow::Result<Affected> {
            self.power_work = false;
            if self.fail_power {
                anyhow::bail!("power calculation failed");
            }
            send(Event::Progress {
                task: crate::engine::BackgroundTask::NodePower,
                percent: 50,
            });
            Ok(Affected {
                power: true,
                ..Affected::default()
            })
        }

        fn gem_search_view(&mut self) -> anyhow::Result<Option<crate::engine::GemSearchView>> {
            Ok(None)
        }

        fn item_db_view(&mut self) -> anyhow::Result<crate::engine::DbSnapshot> {
            Ok(crate::engine::DbSnapshot::Quiet)
        }

        fn gem_tooltip_view(
            &mut self,
            query: &crate::engine::GemTooltipQuery,
        ) -> anyhow::Result<crate::engine::GemTooltipView> {
            Ok(crate::engine::GemTooltipView {
                query: query.clone(),
                lines: vec![],
            })
        }

        fn item_tooltip_view(
            &mut self,
            query: &crate::engine::ItemTooltipQuery,
        ) -> anyhow::Result<Option<crate::engine::ItemTooltipView>> {
            Ok(Some(crate::engine::ItemTooltipView {
                query: query.clone(),
                tooltip: crate::engine::ItemTooltip::default(),
            }))
        }

        fn popup_view(&mut self) -> anyhow::Result<Option<crate::engine::PopupView>> {
            Ok(None)
        }

        fn control_tooltip_view(
            &mut self,
            _query: &crate::engine::ControlTooltipQuery,
        ) -> anyhow::Result<Option<crate::engine::ControlTooltipView>> {
            Ok(None)
        }

        fn item_text(&mut self, id: u32) -> anyhow::Result<crate::engine::ItemText> {
            Ok(crate::engine::ItemText {
                id,
                text: String::new(),
                shared: false,
            })
        }

        fn node_power_view(&mut self) -> anyhow::Result<crate::engine::NodePower> {
            Ok(crate::engine::NodePower {
                stat: self.heatmap.clone(),
                nodes: vec![],
                max_single: 0.0,
                max_offence: 0.0,
                max_defence: 0.0,
                finished: !self.power_work,
            })
        }

        fn power_report_view(&mut self) -> anyhow::Result<crate::engine::PowerReport> {
            anyhow::bail!("unused")
        }

        fn node_tooltip_view(
            &mut self,
            _id: u32,
        ) -> anyhow::Result<Option<crate::engine::NodeTooltip>> {
            Ok(None)
        }

        fn breakdown_view(
            &mut self,
            cell: &crate::engine::CellRef,
        ) -> anyhow::Result<crate::engine::BreakdownView> {
            Ok(crate::engine::BreakdownView {
                cell: cell.clone(),
                sections: vec![],
                error: None,
            })
        }

        fn tree_link_view(&mut self) -> anyhow::Result<String> {
            Ok("https://example.test/tree".into())
        }

        fn short_tree_link_view(&mut self) -> anyhow::Result<Option<String>> {
            Ok(None)
        }

        fn share_view(&mut self) -> anyhow::Result<crate::engine::ShareView> {
            Ok(crate::engine::ShareView::default())
        }

        fn share_code(&mut self) -> anyhow::Result<String> {
            Ok(String::new())
        }

        fn account_view(&mut self) -> anyhow::Result<crate::engine::AccountView> {
            Ok(crate::engine::AccountView::default())
        }

        fn compare_view(&mut self) -> anyhow::Result<Option<crate::engine::CompareView>> {
            Ok(None)
        }

        fn browse_view(
            &mut self,
            folder: &str,
        ) -> anyhow::Result<crate::engine::views::BuildBrowseView> {
            Ok(crate::engine::views::BuildBrowseView {
                folder: folder.to_owned(),
                entries: vec![],
                error: None,
            })
        }

        fn settings_view(&mut self) -> anyhow::Result<crate::engine::SettingsView> {
            Ok(crate::engine::SettingsView::default())
        }
        fn requests_view(&mut self) -> anyhow::Result<crate::engine::RequestsView> {
            Ok(crate::engine::RequestsView::default())
        }
        fn hover_tooltip_view(
            &mut self,
            _target: &crate::engine::HoverTarget,
        ) -> anyhow::Result<Option<crate::engine::HoverTooltipView>> {
            Ok(None)
        }
        fn help_view(&mut self) -> anyhow::Result<crate::engine::HelpView> {
            Ok(fake_help())
        }
    }

    /// PoB's help.txt section titles, with a line each
    fn fake_help() -> crate::engine::HelpView {
        let titles = [
            "Help File Introduction",
            "Contents",
            "FAQ",
            "Hotkeys",
            "Other Notable Things",
            "Timeless Jewels",
            "Loadouts",
            "Party Tab",
            "Skills Tab",
            "Items Tab",
        ];
        crate::engine::HelpView {
            sections: titles
                .iter()
                .map(|t| crate::engine::HelpSection {
                    title: (*t).into(),
                    lines: vec![format!("^7About {t}"), "^7Ctrl+1\t^7Tree tab".into()],
                })
                .collect(),
            header: vec!["^7Path of Building Community Fork v2.67.2".into()],
            changelog: vec![crate::engine::HelpSection {
                title: "Version 2.67.2 (2026/08/07)".into(),
                lines: vec!["^7* Fix a thing".into()],
            }],
            missing: vec![],
        }
    }

    fn app_with(
        start: impl Fn() -> anyhow::Result<Box<dyn Backend>> + Send + Sync + 'static,
    ) -> (AutoPobApp, egui::Context) {
        let start = Arc::new(start);
        let app = AutoPobApp::new(Box::new(move || {
            let start = start.clone();
            Engine::spawn_with(move |_| start(), || {})
        }));
        (app, egui::Context::default())
    }

    fn fake_app() -> (AutoPobApp, egui::Context) {
        app_with(|| Ok(Box::new(Fake::default()) as Box<dyn Backend>))
    }

    fn frame(app: &mut AutoPobApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| app.show(ui))
            .drop_without_applying_deltas();
    }

    /// Runs frames until `done` holds.
    fn pump_until(
        app: &mut AutoPobApp,
        ctx: &egui::Context,
        what: &str,
        done: impl Fn(&AutoPobApp) -> bool,
    ) {
        let start = Instant::now();
        while !done(app) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out: {what}"
            );
            frame(app, ctx, vec![]);
            std::thread::sleep(Duration::from_millis(2));
        }
        frame(app, ctx, vec![]);
    }

    fn started() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = fake_app();
        pump_until(&mut app, &ctx, "list", |a| a.ready && a.list.is_some());
        (app, ctx)
    }

    fn ctrl(key: egui::Key) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]
    }

    #[test]
    fn list_actions_become_their_commands_and_refusals_show_pobs_text() {
        use crate::engine::commands::DeleteKind;
        use crate::engine::views::BuildSort;
        let (mut app, _ctx) = started();
        let path: PathBuf = "/b/one.xml".into();
        let actions = vec![
            ListScreenAction::NewFolder("f".into()),
            ListScreenAction::Rename {
                path: path.clone(),
                name: "n".into(),
            },
            ListScreenAction::Copy {
                path: path.clone(),
                name: "c".into(),
            },
            ListScreenAction::Delete {
                path: path.clone(),
                kind: DeleteKind::Folder,
            },
            ListScreenAction::Move {
                path,
                folder: "sub/".into(),
            },
            ListScreenAction::CopyTo {
                path: "/b/one.xml".into(),
                folder: "sub/".into(),
            },
            ListScreenAction::SetSort(BuildSort::Level),
            ListScreenAction::SetFilter("class:witch".into()),
        ];
        let before = app.sent_full.len();
        for action in actions {
            app.on_list_action(action);
        }
        assert_eq!(
            sent_since(&app, before),
            [
                r#"NewFolder { name: "f" }"#,
                r#"RenameBuild { path: "/b/one.xml", name: "n" }"#,
                r#"CopyBuild { path: "/b/one.xml", name: "c" }"#,
                r#"DeleteBuild { path: "/b/one.xml", kind: Folder }"#,
                r#"MoveBuild { path: "/b/one.xml", folder: "sub/" }"#,
                r#"CopyBuildTo { path: "/b/one.xml", folder: "sub/" }"#,
                "SetBuildSort { sort: Level }",
                r#"SetBuildFilter { text: "class:witch" }"#,
            ]
        );
        // a refusal is the engine's own sentence, shown like any other error
        let seq = app.send(Command::RenameBuild {
            path: "/b/one.xml".into(),
            name: "two".into(),
        });
        app.on_event(Event::Error {
            seq,
            message: "a build with that name already exists".into(),
        });
        assert_eq!(app.notices, ["a build with that name already exists"]);
    }

    /// A list view for the Save As picker: `sub/` holds things, `sub/inner/` is empty
    fn picker_list() -> Snapshot {
        use crate::engine::views::FolderNode;
        let node = |path: &str, empty| FolderNode {
            path: path.into(),
            modified: 1,
            empty,
        };
        Snapshot::BuildList(Arc::new(BuildListView {
            root: "/b/".into(),
            tree: vec![node("sub/", false), node("sub/inner/", true)],
            ..BuildListView::default()
        }))
    }

    fn double_click_mark(app: &mut AutoPobApp, ctx: &egui::Context, key: &str) {
        crate::ui::import::clear_marks();
        frame(app, ctx, vec![]);
        let pos = marked(key)
            .unwrap_or_else(|| panic!("no widget {key}"))
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, ctx, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false, true, false] {
            frame(app, ctx, vec![button(pressed)]);
        }
        frame(app, ctx, vec![]);
    }

    #[test]
    fn ctrl_w_in_the_list_search_deletes_nothing() {
        // PoB's edit box has no Ctrl+W (EditControl.lua:500-640)
        let (mut app, ctx) = started();
        click_mark(&mut app, &ctx, "search");
        frame(&mut app, &ctx, vec![egui::Event::Text("witch".into())]);
        assert_eq!(app.list_ui.filter_for_test(), "witch");
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::W,
                egui::Modifiers::COMMAND | egui::Modifiers::CTRL,
            ),
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.list_ui.filter_for_test(), "witch");
    }

    #[test]
    fn esc_cancels_save_as_once_its_name_box_is_left() {
        let (mut app, ctx) = started();
        deliver(&mut app, picker_list());
        app.open_save_as(None);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let esc = key_with(egui::Key::Escape, egui::Modifiers::NONE);
        frame(&mut app, &ctx, esc.clone());
        frame(&mut app, &ctx, vec![]);
        assert!(app.save_as.is_none(), "Esc left it open");
        assert!(app.sent_full.iter().all(|c| !c.starts_with("SaveAs")));
    }

    /// Save As's Save is off while a build of that name is in the folder (Build.lua:1384-1396)
    #[test]
    fn save_as_is_off_while_a_build_of_that_name_exists() {
        let dir = crate::test_dir::TestDir::new("saveas-exists");
        std::fs::write(dir.join("mine.xml"), "").unwrap();
        let (mut app, ctx) = started();
        deliver(
            &mut app,
            Snapshot::BuildList(Arc::new(BuildListView {
                root: dir.to_path_buf(),
                ..BuildListView::default()
            })),
        );
        app.open_save_as(None);
        frame(&mut app, &ctx, vec![]);
        app.save_as.as_mut().unwrap().name = "mine".into();
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "save_as_save");
        assert_eq!(sent_since(&app, before), [] as [&str; 0]);
        app.save_as.as_mut().unwrap().name = "other".into();
        click_mark(&mut app, &ctx, "save_as_save");
        assert_eq!(
            sent_since(&app, before),
            [r#"SaveAs { folder: "", name: "other", overwrite: false }"#]
        );
    }

    /// Save As opens on the name the engine gives it (Build.lua:1400)
    #[test]
    fn save_as_opens_on_the_engines_name() {
        let (mut app, _ctx) = items_app_typing();
        let mut view = (**app.build.as_ref().unwrap()).clone();
        view.save_as_name = "Acct - Char".into();
        app.build = Some(Arc::new(view));
        app.open_save_as(None);
        assert_eq!(app.save_as.as_ref().unwrap().name, "Acct - Char");
    }

    /// Save As's name box is PoB's (Build.lua:1399-1400): it drops `\/:*?"<>|` and control
    /// characters but not other text, and what is left is saved as it stands
    #[test]
    fn save_as_names_follow_pobs_box() {
        let (mut app, ctx) = started();
        deliver(&mut app, picker_list());
        app.open_save_as(None);
        frame(&mut app, &ctx, vec![]);
        app.save_as.as_mut().unwrap().name.clear();
        click_mark(&mut app, &ctx, "save_as_name");
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("a/b:c*d?\"<>|  ".into())],
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.save_as.as_ref().unwrap().name, "abcd  ");
        // only spaces: Save is off
        let clear = || {
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        click_mark(&mut app, &ctx, "save_as_name");
        frame(&mut app, &ctx, clear());
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text(" /\u{65e5} ".into())],
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.save_as.as_ref().unwrap().name, " \u{65e5} ");
        frame(&mut app, &ctx, clear());
        frame(&mut app, &ctx, vec![egui::Event::Text("/ :".into())]);
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "save_as_save");
        assert_eq!(sent_since(&app, before), [] as [&str; 0]);
        // a name PoB keeps is saved as it stands
        click_mark(&mut app, &ctx, "save_as_name");
        frame(&mut app, &ctx, clear());
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("\u{65e5}\u{672c} x ".into())],
        );
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "save_as_save");
        assert_eq!(
            sent_since(&app, before),
            ["SaveAs { folder: \"\", name: \"\u{65e5}\u{672c} x \", overwrite: false }"]
        );
    }

    #[test]
    fn save_as_picks_a_folder_makes_one_and_deletes_an_empty_one() {
        let (mut app, ctx) = started();
        deliver(&mut app, picker_list());
        app.open_save_as(None);
        frame(&mut app, &ctx, vec![]);
        // walking into a folder and saving puts the build there
        double_click_mark(&mut app, &ctx, "picker_row:sub/");
        assert_eq!(app.save_as.as_ref().unwrap().folder, "sub/");
        app.save_as.as_mut().unwrap().name = "mine".into();
        let before = app.sent_full.len();
        app.send_save_as(false);
        assert_eq!(
            sent_since(&app, before),
            [r#"SaveAs { folder: "sub/", name: "mine", overwrite: false }"#]
        );
        // the engine saved it: the box is gone; open it again for the folder tests
        pump_until(&mut app, &ctx, "saved", |a| a.save_as.is_none());
        deliver(&mut app, picker_list());
        app.open_save_as(None);
        frame(&mut app, &ctx, vec![]);
        app.save_as.as_mut().unwrap().folder = "sub/".into();
        // New Folder names its folder, goes into it, and comes back if refused
        click_mark(&mut app, &ctx, "picker_new");
        click_mark(&mut app, &ctx, "picker_name");
        frame(&mut app, &ctx, vec![egui::Event::Text("de/ep".into())]);
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "picker_create");
        assert_eq!(
            sent_since(&app, before),
            [r#"NewFolderIn { folder: "sub/", name: "deep" }"#]
        );
        assert_eq!(app.save_as.as_ref().unwrap().folder, "sub/deep/");
        let seq = app.save_as.as_ref().unwrap().new_folder.as_ref().unwrap().0;
        app.on_event(Event::Error {
            seq,
            message: "Couldn't create 'deep': File exists".into(),
        });
        assert_eq!(app.save_as.as_ref().unwrap().folder, "sub/");
        assert_eq!(app.notices, ["Couldn't create 'deep': File exists"]);
        // a folder with things in it is refused with PoB's text; an empty one is asked about
        app.notices.clear();
        click_mark(&mut app, &ctx, "picker_row:sub/inner/");
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "picker_delete");
        assert!(
            sent_since(&app, before).is_empty(),
            "deleted without asking"
        );
        click_mark(&mut app, &ctx, "picker_delete_yes");
        assert_eq!(
            sent_since(&app, before),
            [r#"DeleteBuild { path: "/b/sub/inner", kind: EmptyFolder }"#]
        );
        // the Builds folder holds `sub/`, which has things in it
        click_mark(&mut app, &ctx, "picker_crumb:0");
        click_mark(&mut app, &ctx, "picker_row:sub/");
        click_mark(&mut app, &ctx, "picker_delete");
        assert_eq!(app.notices, ["The folder is not empty."]);
    }

    #[test]
    fn a_refused_paste_move_gives_the_cut_mark_back() {
        let (mut app, _ctx) = started();
        // a mark, then the paste that spends it
        let entry = BuildListEntry {
            name: "one".into(),
            is_folder: false,
            folder: String::new(),
            path: "/b/one.xml".into(),
            class: None,
            ascendancy: None,
            level: None,
            delete_prompt: String::new(),
        };
        app.list_ui.mark_for_test(entry);
        app.list_ui.spend_for_test();
        assert!(app.list_ui.marked_path().is_none());
        let seq = app.send(Command::MoveBuild {
            path: "/b/one.xml".into(),
            folder: "sub/".into(),
        });
        app.list_move_seq = Some(seq);
        app.on_event(Event::Error {
            seq,
            message: "couldn't move that build".into(),
        });
        assert_eq!(
            app.list_ui.marked_path(),
            Some(std::path::Path::new("/b/one.xml"))
        );
    }

    #[test]
    fn startup_then_list() {
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(Some(gate));
        let (mut app, ctx) = app_with(move || {
            let gate = gate.lock().unwrap().take();
            if let Some(gate) = gate {
                let _ = gate.recv();
            }
            Ok(Box::new(Fake::default()) as Box<dyn Backend>)
        });
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert!(!app.ready);
        release.send(()).unwrap();
        pump_until(&mut app, &ctx, "ready", |a| a.ready && a.list.is_some());
        assert_eq!(app.screen, Screen::List);
    }

    #[test]
    fn startup_failure_shows_the_message_and_retry_respawns() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();
        let (mut app, ctx) = app_with(move || {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(anyhow::anyhow!("no network").context("download failed"))
            } else {
                Ok(Box::new(Fake::default()) as Box<dyn Backend>)
            }
        });
        pump_until(&mut app, &ctx, "failure", |a| a.startup_error.is_some());
        assert_eq!(
            app.startup_error.as_deref(),
            Some("download failed: no network")
        );
        app.restart();
        pump_until(&mut app, &ctx, "ready", |a| a.ready && a.list.is_some());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert!(app.startup_error.is_none());
    }

    #[test]
    fn opens_a_build_and_draws_every_tab() {
        let (mut app, ctx) = started();
        app.request(Pending::Open("/b/one.xml".into()));
        pump_until(&mut app, &ctx, "build screen", |a| {
            a.screen == Screen::Build && a.entering.is_none() && a.stats.is_some()
        });
        assert!(!app.behind());
        app.tab = BuildTab::Tree;
        frame(&mut app, &ctx, vec![]);
        assert!(!app.behind());
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
    }

    #[test]
    fn failed_open_returns_to_the_list_with_a_notice() {
        let (mut app, ctx) = started();
        app.request(Pending::Open("/b/bad.xml".into()));
        pump_until(&mut app, &ctx, "notice", |a| !a.notices.is_empty());
        assert_eq!(app.screen, Screen::List);
        assert_eq!(app.notices, ["couldn't open that build"]);
    }

    /// The texts one frame draws
    fn drawn_texts(app: &mut AutoPobApp, ctx: &egui::Context) -> Vec<String> {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        // a window lays itself out in its first frame and shows in the next
        frame(app, ctx, vec![]);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.show(ui));
        let mut out = Vec::new();
        output.shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        output.textures_delta.clear();
        out
    }

    #[test]
    fn the_save_questions_have_pobs_texts() {
        // Build.lua:1352-1379 "Save Changes" and :1449 "Save" / "Save As"
        let (mut app, ctx) = started();
        app.request(Pending::Open("/b/one.xml".into()));
        pump_until(&mut app, &ctx, "build", |a| {
            a.screen == Screen::Build && a.entering.is_none()
        });
        app.prompt = Some(Pending::BackToList);
        let texts = drawn_texts(&mut app, &ctx);
        assert!(texts.iter().any(|t| t == "Save Changes"), "{texts:?}");
        assert!(
            texts
                .iter()
                .any(|t| t == "This build has unsaved changes.\nDo you want to save them now?")
        );
        assert!(texts.iter().any(|t| t == "Don't Save"));
        app.prompt = Some(Pending::Close);
        let texts = drawn_texts(&mut app, &ctx);
        assert!(
            texts.iter().any(|t| t
                == "This build has unsaved changes.\nDo you want to save them before exiting?")
        );
        app.prompt = None;
        // a build with a file: "Save As"
        app.open_save_as(None);
        let texts = drawn_texts(&mut app, &ctx);
        assert!(texts.iter().any(|t| t == "Save As"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Enter new build name:"));
        app.save_as = None;
        // one never saved: "Save"
        let mut view = (**app.build.as_ref().unwrap()).clone();
        view.path = None;
        app.build = Some(Arc::new(view));
        app.open_save_as(None);
        let texts = drawn_texts(&mut app, &ctx);
        assert!(!texts.iter().any(|t| t == "Save As"), "{texts:?}");
        // the top bar's Save, the title and the dialog's Save
        assert!(
            texts.iter().filter(|t| *t == "Save").count() >= 3,
            "{texts:?}"
        );
    }

    #[test]
    fn unsaved_changes_prompt_before_leaving() {
        let (mut app, ctx) = started();
        app.request(Pending::Open("/b/one.xml".into()));
        pump_until(&mut app, &ctx, "build", |a| {
            a.screen == Screen::Build && a.entering.is_none()
        });
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            app.screen,
            Screen::List,
            "saved build leaves without a prompt"
        );
        assert!(app.prompt.is_none());
        app.request(Pending::Open("/b/one.xml".into()));
        pump_until(&mut app, &ctx, "build again", |a| {
            a.screen == Screen::Build && a.entering.is_none()
        });

        // an edit makes it unsaved
        app.send(Command::SetConfig {
            var: "x".into(),
            value: ConfigValue::Bool(true),
        });
        pump_until(&mut app, &ctx, "unsaved", |a| {
            a.build.as_ref().is_some_and(|b| b.unsaved)
        });
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.prompt, Some(Pending::BackToList));
        assert_eq!(app.screen, Screen::Build);

        // a close request is intercepted the same way
        app.prompt = None;
        frame(&mut app, &ctx, vec![]);
        ctx.run_ui(
            egui::RawInput {
                viewports: std::iter::once((
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        events: vec![egui::ViewportEvent::Close],
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.prompt, Some(Pending::Close));
        assert!(!app.allow_close);
    }

    #[test]
    fn ctrl_s_on_a_new_build_asks_for_a_name_and_confirms_overwrite() {
        let (mut app, ctx) = started();
        app.request(Pending::New);
        pump_until(&mut app, &ctx, "new build", |a| {
            a.screen == Screen::Build && a.entering.is_none() && a.stats.is_some()
        });
        frame(&mut app, &ctx, ctrl(egui::Key::S));
        let dialog = app.save_as.as_mut().expect("Save as dialog");
        dialog.name = "dup".into();
        app.send_save_as(false);
        pump_until(&mut app, &ctx, "overwrite prompt", |a| {
            a.save_as.as_ref().is_some_and(|d| d.confirm_overwrite)
        });
        assert!(
            app.notices.is_empty(),
            "'exists' is a question, not an error"
        );
        app.send_save_as(true);
        pump_until(&mut app, &ctx, "saved", |a| a.save_as.is_none());
        assert!(app.build.as_ref().unwrap().path.is_some());
    }

    #[test]
    fn save_then_continue_runs_the_pending_action() {
        let (mut app, ctx) = started();
        app.request(Pending::Open("/b/one.xml".into()));
        pump_until(&mut app, &ctx, "build", |a| {
            a.screen == Screen::Build && a.entering.is_none()
        });
        app.send(Command::SetConfig {
            var: "x".into(),
            value: ConfigValue::Bool(true),
        });
        pump_until(&mut app, &ctx, "unsaved", |a| {
            a.build.as_ref().is_some_and(|b| b.unsaved)
        });
        app.save(Some(Pending::BackToList));
        pump_until(&mut app, &ctx, "list again", |a| a.screen == Screen::List);
        assert!(!app.last_unsaved);
    }

    #[test]
    fn a_stopped_engine_shows_the_stopped_screen_and_restarts() {
        let (mut app, ctx) = started();
        app.request(Pending::New);
        pump_until(&mut app, &ctx, "build", |a| {
            a.screen == Screen::Build && a.entering.is_none()
        });
        app.engine.send(Command::Shutdown);
        pump_until(&mut app, &ctx, "stopped", |a| !a.engine.is_alive());
        frame(&mut app, &ctx, vec![]);
        app.restart();
        pump_until(&mut app, &ctx, "ready again", |a| {
            a.ready && a.list.is_some()
        });
        assert_eq!(app.screen, Screen::List);
    }

    fn key_with(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }]
    }

    fn open_new_build() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = started();
        app.request(Pending::New);
        pump_until(&mut app, &ctx, "build", |a| {
            a.screen == Screen::Build && a.entering.is_none() && a.stats.is_some()
        });
        (app, ctx)
    }

    #[test]
    fn ctrl_shift_z_undoes_as_pob_and_ctrl_y_redoes() {
        let (mut app, ctx) = open_new_build();
        let before = app.last_sent;
        // PoB checks only Ctrl on its "z" (TreeTab.lua:359): Ctrl+Shift+Z is undo too
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
        assert_eq!(app.last_sent, before + 1);
        assert_eq!(app.sent_log.last().map(String::as_str), Some("Undo"));
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Y, egui::Modifiers::COMMAND),
        );
        assert_eq!(app.sent_log.last().map(String::as_str), Some("Redo"));
    }

    #[test]
    fn typing_keeps_undo_but_ctrl_s_still_saves() {
        let (mut app, ctx) = open_new_build();
        // give the build a file so Save is a plain Save
        app.build = Some(Arc::new(BuildView {
            path: Some("/b/x.xml".into()),
            ..(**app.build.as_ref().unwrap()).clone()
        }));
        let mut text = String::from("hello");
        let id = egui::Id::unique("typing");
        let mut time = 0.0;
        let mut run = |app: &mut AutoPobApp, text: &mut String, events| {
            time += 2.0; // far apart, so the field's undoer sees separate edits
            ctx.run_ui(
                egui::RawInput {
                    events,
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    app.show(ui);
                    egui::Window::new("field").show(ui.ctx(), |ui| {
                        ui.add(egui::TextEdit::singleline(text).id(id));
                    });
                },
            )
            .drop_without_applying_deltas();
        };
        run(&mut app, &mut text, vec![]);
        ctx.memory_mut(|m| m.request_focus(id));
        run(&mut app, &mut text, vec![]);
        run(&mut app, &mut text, vec![]);
        assert!(ctx.egui_wants_keyboard_input());
        run(&mut app, &mut text, vec![egui::Event::Text("!".into())]);
        run(&mut app, &mut text, vec![]);
        assert_eq!(text, "hello!");
        let before = app.sent_log.len();
        // the field's own undo reverts the edit, so the key must reach it
        run(
            &mut app,
            &mut text,
            key_with(egui::Key::Z, egui::Modifiers::COMMAND),
        );
        assert_eq!(text, "hello", "Ctrl+Z was taken from the text field");
        run(
            &mut app,
            &mut text,
            key_with(egui::Key::Y, egui::Modifiers::COMMAND),
        );
        assert_eq!(text, "hello!", "Ctrl+Y was taken from the text field");
        assert_eq!(app.sent_log.len(), before, "no Undo or Redo command");
        run(
            &mut app,
            &mut text,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(app.sent_log.last().map(String::as_str), Some("Save"));
    }

    #[test]
    fn shortcuts_do_nothing_under_a_modal() {
        let (mut app, ctx) = open_new_build();
        app.open_save_as(None);
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_log.len();
        for key in [egui::Key::S, egui::Key::Z, egui::Key::Y] {
            frame(&mut app, &ctx, key_with(key, egui::Modifiers::COMMAND));
        }
        assert_eq!(app.sent_log.len(), before);
        // and under the unsaved prompt
        app.save_as = None;
        app.prompt = Some(Pending::BackToList);
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(app.sent_log.len(), before);
    }

    // ---- the Config tab -----------------------------------------------------------------

    fn choice(label: &str, value: &str) -> crate::engine::views::Choice {
        crate::engine::views::Choice {
            label: label.into(),
            value: value.into(),
        }
    }

    fn option(var: &str, kind: ConfigKind, label: &str) -> ConfigOption {
        ConfigOption {
            var: var.into(),
            kind,
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

    /// Every kind, a hidden option, an invalid one, two sets and two custom mod groups.
    fn sample_config() -> ConfigView {
        let mut boss = option("enemyIsBoss", ConfigKind::List, "Enemy is a ^xFF0000Boss");
        boss.choices = vec![
            choice("None", "None"),
            choice("Boss", "Boss"),
            choice("Pinnacle", "Pinnacle"),
        ];
        boss.selected = Some("Boss".into());
        boss.value = Some(serde_json::json!("Boss"));
        let mut power = option("usePowerCharges", ConfigKind::Check, "Use power charges?");
        power.value = Some(serde_json::json!(true));
        power.tooltip = vec![TooltipLine::Line {
            size: 14,
            text: "Charges".into(),
        }];
        let mut count = option("overrideCount", ConfigKind::Count, "Charge count");
        count.value = Some(serde_json::json!(7));
        count.placeholder = Some(serde_json::json!(3));
        let mut hidden = option("hiddenCheck", ConfigKind::Check, "A hidden option");
        hidden.shown = false;
        let mut invalid = option("staleCheck", ConfigKind::Check, "A stale option");
        invalid.invalid = true;
        invalid.value = Some(serde_json::json!(true));
        let mut float = option("multiplier", ConfigKind::Float, "Multiplier");
        float.value = Some(serde_json::json!(1.5));
        let mut text = option("note", ConfigKind::Text, "Note");
        text.value = Some(serde_json::json!("hi"));
        ConfigView {
            sets: vec![
                ConfigSet {
                    id: 1,
                    title: "Default".into(),
                    untitled: false,
                },
                ConfigSet {
                    id: 2,
                    title: "Mapping".into(),
                    untitled: false,
                },
            ],
            active_set: 1,
            custom_mods: vec![
                CustomModGroup {
                    title: "First".into(),
                    enabled: true,
                    text: "+1 to all".into(),
                },
                CustomModGroup {
                    title: "Second".into(),
                    enabled: false,
                    text: String::new(),
                },
            ],
            sections: vec![
                ConfigSection {
                    name: "General".into(),
                    column: 1,
                    options: vec![power, boss, count, hidden],
                },
                ConfigSection {
                    name: "Other".into(),
                    column: 2,
                    options: vec![
                        invalid,
                        float,
                        option("integerThing", ConfigKind::Integer, "Integer"),
                        text,
                    ],
                },
            ],
        }
    }

    fn config_app(view: ConfigView) -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = open_new_build();
        app.config = Some(Arc::new(view));
        app.tab = BuildTab::Config;
        // two frames: the first lays the widgets out, the second records where they are
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    fn rect_of(app: &AutoPobApp, key: &str) -> egui::Rect {
        *app.config_ui
            .rects
            .get(key)
            .or_else(|| app.tree.rects.map.get(key))
            .unwrap_or_else(|| panic!("no widget {key}"))
    }

    fn click(app: &mut AutoPobApp, ctx: &egui::Context, key: &str) {
        let pos = rect_of(app, key).center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, ctx, vec![egui::Event::PointerMoved(pos)]);
        frame(app, ctx, vec![button(true)]);
        frame(app, ctx, vec![button(false)]);
        frame(app, ctx, vec![]);
    }

    fn type_text(app: &mut AutoPobApp, ctx: &egui::Context, text: &str) {
        frame(app, ctx, vec![egui::Event::Text(text.into())]);
    }

    fn sent_since(app: &AutoPobApp, before: usize) -> Vec<&str> {
        app.sent_full[before..].iter().map(String::as_str).collect()
    }

    #[test]
    fn config_tab_draws_every_kind_and_hides_hidden_options() {
        let (mut app, ctx) = config_app(sample_config());
        for key in [
            "check:usePowerCharges",
            "list:enemyIsBoss",
            "field:overrideCount",
            "field:multiplier",
            "field:integerThing",
            "field:note",
            "check:staleCheck",
            "set_combo",
            "mod_enabled:0",
            "mod_text:1",
            "add_group",
        ] {
            assert!(app.config_ui.rects.contains_key(key), "missing {key}");
        }
        assert!(!app.config_ui.rects.contains_key("check:hiddenCheck"));
        // an invalid or hidden option is drawn, not sent
        app.config_ui.rects.clear();
        app.config_ui.set_show_all(true);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert!(app.config_ui.rects.contains_key("check:hiddenCheck"));
        assert_eq!(app.sent_log.iter().filter(|c| *c == "SetConfig").count(), 0);
    }

    #[test]
    fn config_tab_narrow_and_wide_both_draw() {
        let (mut app, ctx) = config_app(sample_config());
        for width in [300.0, 1600.0] {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| app.show(ui),
            )
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn clicking_a_checkbox_sends_the_toggled_value() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "check:usePowerCharges");
        assert_eq!(
            sent_since(&app, before),
            ["SetConfig { var: \"usePowerCharges\", value: Bool(false) }"]
        );
    }

    #[test]
    fn choosing_a_dropdown_entry_sends_its_value() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "list:enemyIsBoss");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "list:enemyIsBoss:Pinnacle");
        assert_eq!(
            sent_since(&app, before),
            ["SetConfig { var: \"enemyIsBoss\", value: Choice(\"Pinnacle\") }"]
        );
    }

    #[test]
    fn number_fields_send_valid_input_ignore_invalid_and_clear_when_emptied() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "field:overrideCount");
        // "7" is there; a valid digit extends it
        type_text(&mut app, &ctx, "2");
        assert_eq!(
            sent_since(&app, before),
            ["SetConfig { var: \"overrideCount\", value: Number(72.0) }"]
        );
        // PoB's `%D` filter drops a letter: nothing changes, nothing is sent
        let before = app.sent_full.len();
        type_text(&mut app, &ctx, "x");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        assert_eq!(app.config_ui.draft("overrideCount"), Some("72"));
        // emptying the field clears the option
        let backspace = |key| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![backspace(egui::Key::Backspace)]);
        }
        assert_eq!(
            sent_since(&app, before).last().copied(),
            Some("SetConfig { var: \"overrideCount\", value: Clear }")
        );
    }

    #[test]
    fn custom_mod_groups_add_toggle_edit_and_remove() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "add_group");
        click(&mut app, &ctx, "mod_enabled:1");
        assert_eq!(
            sent_since(&app, before),
            [
                "AddCustomModGroup",
                "SetCustomModEnabled { group: 1, enabled: true }"
            ]
        );

        // text is sent when focus leaves
        let before = app.sent_full.len();
        click(&mut app, &ctx, "mod_text:1");
        type_text(&mut app, &ctx, "+5 life");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click(&mut app, &ctx, "mod_title:1");
        assert_eq!(
            sent_since(&app, before),
            ["SetCustomModText { group: 1, text: \"+5 life\" }"]
        );

        // PoB's X removes the group at once (ConfigTab.lua:23-33)
        let before = app.sent_full.len();
        click(&mut app, &ctx, "mod_remove:0");
        assert_eq!(
            sent_since(&app, before),
            ["RemoveCustomModGroup { group: 0 }"]
        );
    }

    /// A custom modifier group's title box has `EditControl`'s own filter (ConfigTab.lua:35)
    #[test]
    fn a_custom_mod_title_drops_what_pobs_box_drops() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "mod_title:0");
        type_text(&mut app, &ctx, "\u{65e5}\u{672c}x\u{e9}");
        click(&mut app, &ctx, "mod_text:0");
        let sent = sent_since(&app, before);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(
            sent[0].starts_with("SetCustomModTitle { group: 0, title: \"")
                && sent[0].ends_with("x\" }")
                && sent[0].is_ascii(),
            "{sent:?}"
        );
    }

    #[test]
    fn deleting_a_config_set_asks_first_and_the_last_set_stays() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "set_delete");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click(&mut app, &ctx, "set_delete_yes");
        assert_eq!(sent_since(&app, before), ["DeleteConfigSet { id: 1 }"]);

        let mut one = sample_config();
        one.sets.truncate(1);
        let (mut app, ctx) = config_app(one);
        let before = app.sent_full.len();
        click(&mut app, &ctx, "set_delete");
        assert!(app.config_ui.rects.contains_key("set_delete"));
        assert!(!app.config_ui.rects.contains_key("set_delete_yes"));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
    }

    #[test]
    fn another_build_starts_with_no_config_set_question() {
        // PoB makes a new ConfigTab for each build: an unanswered Delete question doesn't
        // carry over to a set of the next build with the same id
        let (mut app, ctx) = config_app(sample_config());
        click(&mut app, &ctx, "set_delete");
        assert!(app.config_ui.rects.contains_key("set_delete_yes"));
        app.request(Pending::New);
        pump_until(&mut app, &ctx, "another build", |a| {
            a.screen == Screen::Build && a.entering.is_none()
        });
        app.config = Some(Arc::new(sample_config()));
        app.tab = BuildTab::Config;
        app.config_ui.rects.clear();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert!(app.config_ui.rects.contains_key("set_delete"));
        assert!(!app.config_ui.rects.contains_key("set_delete_yes"));
    }

    #[test]
    fn picking_another_config_set_selects_it() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "set_combo");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "set:2");
        assert_eq!(sent_since(&app, before), ["SelectConfigSet { id: 2 }"]);
    }

    // ---- the Tree tab -------------------------------------------------------------------

    fn tree_app_with(
        fake: impl Fn() -> Fake + Send + Sync + 'static,
    ) -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = app_with(move || Ok(Box::new(fake()) as Box<dyn Backend>));
        pump_until(&mut app, &ctx, "list", |a| a.ready && a.list.is_some());
        app.request(Pending::New);
        pump_until(&mut app, &ctx, "tree tab", |a| {
            a.screen == Screen::Build
                && a.entering.is_none()
                && a.tree.rects.map.contains_key("reset")
        });
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    fn tree_app() -> (AutoPobApp, egui::Context) {
        tree_app_with(Fake::default)
    }

    /// Runs frames and collects the text egui was asked to copy.
    fn copied_within(app: &mut AutoPobApp, ctx: &egui::Context, frames: usize) -> Vec<String> {
        let mut copied = Vec::new();
        for _ in 0..frames {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| app.show(ui));
            copied.extend(
                output
                    .platform_output
                    .commands
                    .iter()
                    .filter_map(|c| match c {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    }),
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        copied
    }

    #[test]
    fn tree_is_the_default_tab_and_draws() {
        let (app, _ctx) = tree_app();
        assert_eq!(app.tab, BuildTab::Tree);
        for key in [
            "spec_combo",
            "class_combo",
            "reset",
            "search",
            "heat_stat",
            "link_menu",
        ] {
            assert!(app.tree.rects.map.contains_key(key), "missing {key}");
        }
        assert!(app.tree.rect.is_some(), "the canvas was drawn");
    }

    #[test]
    fn ctrl_z_on_tree_tab_sends_tree_undo() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        frame(&mut app, &ctx, ctrl(egui::Key::Y));
        assert_eq!(sent_since(&app, before), ["Undo(Tree)", "Redo(Tree)"]);
    }

    #[test]
    fn focus_node_switches_to_tree() {
        let (mut app, ctx) = tree_app();
        app.tab = BuildTab::Calcs;
        frame(&mut app, &ctx, vec![]);
        app.show_node(42, 1.0);
        assert_eq!(app.tab, BuildTab::Tree);
    }

    #[test]
    fn errors_are_notices_and_a_failed_breakdown_answer_is_not() {
        let (mut app, _ctx) = tree_app();
        let notices = app.notices.len();
        app.on_event(Event::Error {
            seq: 76,
            message: "other".into(),
        });
        assert!(!app.calcs.breakdown_unavailable());
        assert_eq!(app.notices.len(), notices + 1);
        let failed = crate::engine::BreakdownView {
            cell: crate::engine::CellRef {
                section: "s".into(),
                sub: 0,
                row: 0,
                cell: 0,
            },
            sections: vec![],
            error: Some("boom".into()),
        };
        app.on_event(Event::Snapshot {
            seq: 77,
            snapshot: Snapshot::Breakdown(Arc::new(failed)),
        });
        assert_eq!(app.notices.len(), notices + 1, "no notice for a hover");
    }

    #[test]
    fn ctrl_z_on_calcs_sends_calcs_undo() {
        let (mut app, ctx) = tree_app();
        app.tab = BuildTab::Calcs;
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        frame(&mut app, &ctx, ctrl(egui::Key::Y));
        assert_eq!(sent_since(&app, before), ["Undo(Calcs)", "Redo(Calcs)"]);
    }

    fn skills_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = tree_app();
        app.on_event(Event::Snapshot {
            seq: 90,
            snapshot: Snapshot::Skills(Arc::new(crate::ui::skills::tests::sample())),
        });
        app.tab = BuildTab::Skills;
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    #[test]
    fn ctrl_z_on_skills_sends_skills_undo() {
        let (mut app, ctx) = skills_app();
        // a focused row must not count as typing
        let pos = app.skills.rect("row_0").center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(ctx.egui_wants_keyboard_input() && !ctx.text_edit_focused());
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        frame(&mut app, &ctx, ctrl(egui::Key::Y));
        assert_eq!(sent_since(&app, before), ["Undo(Skills)", "Redo(Skills)"]);
    }

    fn items_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = tree_app();
        app.on_event(Event::Snapshot {
            seq: 90,
            snapshot: Snapshot::Items(Arc::new(crate::ui::items::tests::sample())),
        });
        app.tab = BuildTab::Items;
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    fn click_item_key(app: &mut AutoPobApp, ctx: &egui::Context, key: &str) {
        let pos = app.items.rect(key).center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(app, ctx, events);
        }
    }

    fn display_with_quality() -> crate::engine::DisplayItemView {
        crate::engine::DisplayItemView {
            id: Some(2),
            name: "Kaom's Heart".into(),
            save_label: "Save".into(),
            quality: Some("2".into()),
            ..Default::default()
        }
    }

    /// The items tab with a quality typed ("25") but not sent.
    fn items_app_typing() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = items_app();
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(display_with_quality()))),
        });
        frame(&mut app, &ctx, vec![]);
        click_item_key(&mut app, &ctx, "quality");
        frame(&mut app, &ctx, vec![egui::Event::Text("5".into())]);
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    const QUALITY: &str = "SetItemQuality { text: \"25\" }";

    #[test]
    fn a_typed_quality_is_sent_when_leaving_the_items_tab() {
        let (mut app, ctx) = items_app_typing();
        let before = app.sent_full.len();
        app.tab = BuildTab::Notes;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before), [QUALITY]);
    }

    #[test]
    fn a_typed_quality_precedes_save_as_save_and_build_changes() {
        let (mut app, _ctx) = items_app_typing();
        let before = app.sent_full.len();
        app.open_save_as(None);
        app.send_save_as(false);
        let sent = sent_since(&app, before);
        assert_eq!(sent[0], QUALITY);
        assert!(sent[1].starts_with("SaveAs"), "{sent:?}");

        let (mut app, _ctx) = items_app_typing();
        let before = app.sent_full.len();
        app.save(None);
        assert_eq!(sent_since(&app, before)[0], QUALITY);

        let (mut app, ctx) = items_app_typing();
        let before = app.sent_full.len();
        app.run_action(&ctx, Pending::BackToList);
        let sent = sent_since(&app, before);
        assert_eq!(sent[0], QUALITY);
        assert!(sent[1].starts_with("ListBuilds"), "{sent:?}");

        let (mut app, ctx) = items_app_typing();
        let before = app.sent_full.len();
        app.run_action(&ctx, Pending::Open("/b/y.xml".into()));
        let sent = sent_since(&app, before);
        assert_eq!(sent[0], QUALITY);
        assert!(sent[1].starts_with("OpenBuild"), "{sent:?}");

        let (mut app, ctx) = items_app_typing();
        let before = app.sent_full.len();
        app.run_action(&ctx, Pending::Close);
        assert_eq!(sent_since(&app, before)[0], QUALITY);
    }

    #[test]
    fn the_raw_text_window_follows_the_text_commands_seq() {
        let (mut app, ctx) = items_app();
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(display_with_quality()))),
        });
        frame(&mut app, &ctx, vec![]);
        click_item_key(&mut app, &ctx, "edit");
        let before = app.sent_full.len();
        click_item_key(&mut app, &ctx, "text_save");
        assert!(sent_since(&app, before)[0].starts_with("SetDisplayItemText"));
        let seq = app.text_seq.expect("the text command's seq is remembered");
        assert_eq!(seq, app.last_sent);
        let open = |app: &AutoPobApp| app.items.text_window_open();

        // a view from before it leaves the window open
        app.on_event(Event::Snapshot {
            seq: seq - 1,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(display_with_quality()))),
        });
        assert!(open(&app));
        // its error keeps it open over its own views
        app.on_event(Event::Error {
            seq,
            message: "bad item".into(),
        });
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(display_with_quality()))),
        });
        assert!(open(&app));
        // another command's error doesn't matter; a resend succeeding closes it
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        click_item_key(&mut app, &ctx, "text_save");
        assert!(sent_since(&app, before)[0].starts_with("SetDisplayItemText"));
        let seq = app.text_seq.unwrap();
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(display_with_quality()))),
        });
        assert!(!open(&app));
    }

    #[test]
    fn ctrl_z_on_items_sends_items_undo() {
        let (mut app, ctx) = items_app();
        // a focused row must not count as typing
        let pos = app.items.rect("row_1").center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(ctx.egui_wants_keyboard_input() && !ctx.text_edit_focused());
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        frame(&mut app, &ctx, ctrl(egui::Key::Y));
        assert_eq!(sent_since(&app, before), ["Undo(Items)", "Redo(Items)"]);
    }

    #[test]
    fn ctrl_z_does_nothing_while_the_raw_text_window_is_open() {
        let (mut app, ctx) = items_app();
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(display_with_quality()))),
        });
        frame(&mut app, &ctx, vec![]);
        click_item_key(&mut app, &ctx, "edit");
        assert!(app.items.text_window_open());
        // focus is outside the window's text box
        assert!(!ctx.text_edit_focused());
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        frame(&mut app, &ctx, ctrl(egui::Key::Y));
        assert_eq!(sent_since(&app, before), Vec::<String>::new());
    }

    #[test]
    fn a_vanished_display_item_is_cancelled_while_another_tab_shows() {
        let (mut app, ctx) = items_app();
        app.tab = BuildTab::Tree;
        frame(&mut app, &ctx, vec![]);
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(crate::engine::DisplayItemView {
                id: Some(2),
                ..Default::default()
            }))),
        });
        let mut gone = crate::ui::items::tests::sample();
        gone.items.retain(|e| e.id != 2);
        app.on_event(Event::Snapshot {
            seq: 92,
            snapshot: Snapshot::Items(Arc::new(gone)),
        });
        let before = app.sent_full.len();
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            sent_since(&app, before),
            ["CancelDisplayItem { if_id: Some(2) }"]
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before).len(), 1);
    }

    #[test]
    fn leaving_the_items_tab_closes_its_dialogs() {
        let (mut app, ctx) = items_app();
        let pos = app.items.rect("manage").center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(app.items.has_dialog());
        app.tab = BuildTab::Notes;
        frame(&mut app, &ctx, vec![]);
        assert!(!app.items.has_dialog());
    }

    #[test]
    fn ctrl_z_while_typing_does_not_undo() {
        let (mut app, ctx) = skills_app();
        let pos = app.skills.rect("default_quality").center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(ctx.text_edit_focused(), "the quality box has focus");
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
    }

    #[test]
    fn back_or_close_with_a_pending_skills_edit_asks_first() {
        let (mut app, ctx) = skills_app();
        let pos = app.skills.rect("g0_label").center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![egui::Event::Text("x".into())],
            vec![],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(app.skills.has_pending());
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.prompt, Some(Pending::BackToList));
        app.prompt = None;
        ctx.run_ui(
            egui::RawInput {
                viewports: std::iter::once((
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        events: vec![egui::ViewportEvent::Close],
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.prompt, Some(Pending::Close));
        // "Don't save" drops the edit
        app.skills.discard();
        assert!(!app.skills.has_pending());
    }

    #[test]
    fn class_change_with_nodes_asks_and_connect_sends_connect_true() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        click(&mut app, &ctx, "class_combo");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "class:1");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new(), "asks first");
        click(&mut app, &ctx, "class_connect");
        assert_eq!(
            sent_since(&app, before),
            ["SelectClass { id: 1, connect: true }"]
        );

        // "Reset tree" keeps nothing
        click(&mut app, &ctx, "class_combo");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "class:1");
        click(&mut app, &ctx, "class_reset");
        assert_eq!(
            sent_since(&app, before)[1],
            "SelectClass { id: 1, connect: false }"
        );

        // a class that needs no confirmation is chosen at once
        click(&mut app, &ctx, "class_combo");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "class:2");
        assert_eq!(
            sent_since(&app, before)[2],
            "SelectClass { id: 2, connect: false }"
        );
    }

    #[test]
    fn class_question_is_asked_on_any_tab() {
        let (mut app, ctx) = tree_app();
        // PoB's class dropdown is in the top bar on every tab
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        click(&mut app, &ctx, "class_combo");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "class:1");
        assert!(app.tree.has_dialog(), "the question is open");
        // a frame off the Tree tab keeps it
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "class_connect");
        assert_eq!(
            sent_since(&app, before),
            ["SelectClass { id: 1, connect: true }"]
        );
    }

    #[test]
    fn reset_asks_first() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        click(&mut app, &ctx, "reset");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click(&mut app, &ctx, "reset_yes");
        assert_eq!(sent_since(&app, before), ["ResetTree"]);
    }

    #[test]
    fn delete_spec_asks_and_last_spec_cannot_be_deleted() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        click(&mut app, &ctx, "spec_delete");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click(&mut app, &ctx, "spec_delete_yes");
        assert_eq!(sent_since(&app, before), ["DeleteSpec { index: 0 }"]);

        let (mut app, ctx) = tree_app_with(|| Fake {
            single_spec: true,
            ..Fake::default()
        });
        let before = app.sent_full.len();
        click(&mut app, &ctx, "spec_delete");
        assert!(!app.tree.rects.map.contains_key("spec_delete_yes"));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
    }

    #[test]
    fn search_is_debounced() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        click(&mut app, &ctx, "search");
        for text in ["a", "b", "c"] {
            type_text(&mut app, &ctx, text);
        }
        assert_eq!(sent_since(&app, before), Vec::<&str>::new(), "still typing");
        // egui's clock advances 1/60 s a frame
        for _ in 0..12 {
            frame(&mut app, &ctx, vec![]);
        }
        assert_eq!(
            sent_since(&app, before),
            ["SetTreeSearch { text: \"abc\" }"]
        );
    }

    #[test]
    fn queries_do_not_mark_the_app_behind() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        click(&mut app, &ctx, "link_menu");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "link_copy");
        // the query is out, yet the app waits for nothing
        assert!(!app.behind());
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        pump_until(&mut app, &ctx, "link", |a| a.tree.link.is_some());
    }

    #[test]
    fn heatmap_failure_shows_retry() {
        let (mut app, ctx) = tree_app_with(|| Fake {
            fail_power: true,
            ..Fake::default()
        });
        let before = app.sent_full.len();
        click(&mut app, &ctx, "heat_stat");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "heat:Life");
        assert_eq!(
            sent_since(&app, before),
            ["SetHeatmap { stat: Some(\"Life\"), max_depth: None }"]
        );
        pump_until(&mut app, &ctx, "retry", |a| {
            a.tree.rects.map.contains_key("heat_retry")
        });
        click(&mut app, &ctx, "heat_retry");
        assert_eq!(
            sent_since(&app, before),
            [
                "SetHeatmap { stat: Some(\"Life\"), max_depth: None }",
                "SetHeatmap { stat: Some(\"Life\"), max_depth: None }"
            ]
        );
    }

    #[test]
    fn power_report_row_click_centres_node() {
        let (mut app, ctx) = tree_app();
        click(&mut app, &ctx, "report_toggle");
        let row = |id, name: &str| crate::engine::PowerReportRow {
            id,
            name: name.into(),
            kind: "Notable".into(),
            power: 1.0,
            power_text: "1".into(),
            path_power: 2.0,
            path_power_text: "2".into(),
            allocated: false,
            path_dist: 1,
        };
        app.on_event(Event::Snapshot {
            seq: 0,
            snapshot: Snapshot::PowerReport(Arc::new(crate::engine::PowerReport {
                stat: "Life".into(),
                rows: vec![row(1, "First"), row(2, "Second")],
                listed: vec![0, 1],
                filter: 0,
                masteries: true,
            })),
        });
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "report_row:1");
        assert_eq!(app.tree.camera.center, egui::Vec2::new(600.0, 0.0));
        assert!(matches!(app.tree.pulse, Some((2, _))));
    }

    #[test]
    fn copy_link_puts_link_on_clipboard() {
        let (mut app, ctx) = tree_app();
        click(&mut app, &ctx, "link_menu");
        frame(&mut app, &ctx, vec![]);
        let pos = rect_of(&app, "link_copy").center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = Vec::new();
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
        ] {
            let output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            copied.extend(output.platform_output.commands);
        }
        let copied_later = copied_within(&mut app, &ctx, 100);
        let all: Vec<String> = copied
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .chain(copied_later)
            .collect();
        assert_eq!(all, ["https://example.test/tree"]);
    }

    #[test]
    fn second_build_still_draws_the_tree_when_the_layout_is_not_resent() {
        let (mut app, ctx) = tree_app();
        app.tree.rects.map.clear();
        app.request(Pending::New);
        pump_until(&mut app, &ctx, "second build", |a| {
            a.screen == Screen::Build
                && a.entering.is_none()
                && a.tree.rects.map.contains_key("reset")
        });
    }

    #[test]
    fn background_failure_after_a_restart_shows_retry() {
        let (mut app, ctx) = tree_app_with(|| Fake {
            fail_power: true,
            ..Fake::default()
        });
        click(&mut app, &ctx, "heat_stat");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "heat:Life");
        pump_until(&mut app, &ctx, "first failure", |a| {
            a.tree.rects.map.contains_key("heat_retry")
        });
        // the tree changes, the calculation restarts, and fails again
        app.tree.rects.map.clear();
        app.send(Command::ResetTree);
        pump_until(&mut app, &ctx, "second failure", |a| {
            a.tree.rects.map.contains_key("heat_retry")
        });
        assert!(!app.notices.is_empty());
    }

    #[test]
    fn gem_dps_events_leave_the_tree_alone() {
        let (mut app, ctx) = tree_app();
        app.on_event(Event::Progress {
            task: BackgroundTask::NodePower,
            percent: 40,
        });
        app.on_event(Event::Progress {
            task: BackgroundTask::GemDps,
            percent: 90,
        });
        assert_eq!(app.tree.progress, Some(40), "gem progress reached the tree");
        app.on_event(Event::BackgroundFailed {
            task: BackgroundTask::GemDps,
            message: "dps broke".into(),
        });
        assert!(app.notices.is_empty(), "the picker shows gem failures");
        // power_failed would have cleared the progress and offered a retry
        assert_eq!(app.tree.progress, Some(40));
        frame(&mut app, &ctx, vec![]);
        assert!(!app.tree.rects.map.contains_key("heat_retry"));
    }

    #[test]
    fn power_events_of_the_previous_build_are_dropped_when_another_opens() {
        let (mut app, ctx) = tree_app();
        app.request(Pending::New);
        frame(&mut app, &ctx, vec![]);
        assert!(app.entering.is_some());
        // events the old build's calculation left in the queue, ahead of the new build's
        app.on_event(Event::Progress {
            task: crate::engine::BackgroundTask::NodePower,
            percent: 37,
        });
        app.on_event(Event::Snapshot {
            seq: 0,
            snapshot: Snapshot::PowerReport(Arc::new(crate::engine::PowerReport {
                stat: "Life".into(),
                rows: vec![],
                listed: vec![],
                filter: 0,
                masteries: true,
            })),
        });
        app.on_event(Event::BackgroundFailed {
            task: crate::engine::BackgroundTask::NodePower,
            message: "power calculation failed".into(),
        });
        pump_until(&mut app, &ctx, "new build", |a| {
            a.screen == Screen::Build
                && a.entering.is_none()
                && a.tree.rects.map.contains_key("reset")
        });
        assert_eq!(app.tree.progress, None, "no progress bar");
        assert!(app.tree.report.is_none(), "no report");
        assert!(!app.tree.rects.map.contains_key("heat_retry"), "no failure");
        assert!(app.notices.is_empty(), "{:?}", app.notices);
    }

    #[test]
    fn going_back_to_the_list_stops_the_heatmap() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        // nothing to stop yet
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before), ["ListBuilds { folder: \"\" }"]);

        let (mut app, ctx) = tree_app();
        click(&mut app, &ctx, "heat_stat");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "heat:Life");
        pump_until(&mut app, &ctx, "heat map on", |a| a.tree.heatmap_on());
        let before = app.sent_full.len();
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            sent_since(&app, before),
            [
                "SetHeatmap { stat: None, max_depth: None }",
                "ListBuilds { folder: \"\" }"
            ]
        );
    }

    #[test]
    fn a_command_error_while_the_heatmap_runs_is_not_a_power_failure() {
        let (mut app, ctx) = tree_app();
        click(&mut app, &ctx, "heat_stat");
        frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, "heat:Life");
        app.send(Command::SetTreeSearch {
            text: "boom".into(),
        });
        pump_until(&mut app, &ctx, "notice", |a| !a.notices.is_empty());
        assert_eq!(app.notices, ["search broke"]);
        assert!(!app.tree.rects.map.contains_key("heat_retry"));
    }

    #[test]
    fn leaving_the_tree_tab_closes_its_dialog_and_frees_the_shortcuts() {
        let (mut app, ctx) = tree_app();
        click(&mut app, &ctx, "reset");
        assert!(app.tree.has_dialog());
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
        assert!(!app.tree.has_dialog());
        frame(&mut app, &ctx, ctrl(egui::Key::S));
        assert!(app.save_as.is_some(), "Ctrl+S reached the save command");
    }

    #[test]
    fn shortcuts_wait_while_a_tree_dialog_is_open() {
        let (mut app, ctx) = tree_app();
        let before = app.sent_full.len();
        click(&mut app, &ctx, "reset");
        frame(&mut app, &ctx, ctrl(egui::Key::Z));
        frame(&mut app, &ctx, ctrl(egui::Key::S));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
    }

    fn notes_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = open_new_build();
        app.build = Some(Arc::new(BuildView {
            path: Some("/b/x.xml".into()),
            ..(**app.build.as_ref().unwrap()).clone()
        }));
        app.tab = BuildTab::Notes;
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    fn stages_app() -> (AutoPobApp, egui::Context, egui::Pos2) {
        let (mut app, ctx) = notes_app();
        app.stats = Some(Arc::new(crate::engine::views::StatsView {
            main_skill: crate::engine::views::MainSkillSelector {
                stages: Some("1".into()),
                ..Default::default()
            },
            ..Default::default()
        }));
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let rect: egui::Rect = ctx
            .data(|d| d.get_temp(egui::Id::NULL.with(("sidebar_rect", "stages".to_owned()))))
            .expect("the stages box is drawn");
        let pos = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![egui::Event::Text("2".into())],
        ] {
            frame(&mut app, &ctx, events);
        }
        (app, ctx, pos)
    }

    #[test]
    fn a_stage_count_being_typed_counts_as_unsaved_and_flushes_before_save() {
        let (mut app, ctx, _) = stages_app();
        assert!(app.sidebar.has_pending());
        assert!(app.current_unsaved());
        let before = app.sent_full.len();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(
            sent_since(&app, before),
            [
                "SetMainSkillCount { control: Stages, text: \"12\" }",
                "Save"
            ]
        );
        // leaving the box doesn't send it again
        let before = app.sent_full.len();
        let away = egui::Pos2::new(900.0, 600.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: away,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(away)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(&mut app, &ctx, events);
        }
        let sent = sent_since(&app, before);
        assert!(
            !sent.iter().any(|c| c.contains("SetMainSkillCount")),
            "{sent:?}"
        );
    }

    /// Where the top bar drew `key` last frame
    fn top_rect(ctx: &egui::Context, key: &str) -> egui::Rect {
        ctx.data(|d| d.get_temp(egui::Id::NULL.with(("top_bar_rect", key.to_owned()))))
            .unwrap_or_else(|| panic!("the top bar drew no {key}"))
    }

    fn click_at(app: &mut AutoPobApp, ctx: &egui::Context, pos: egui::Pos2) {
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(app, ctx, events);
        }
    }

    /// Clicks the level box and types `text` over what it shows
    fn type_level(app: &mut AutoPobApp, ctx: &egui::Context, text: &str) {
        let pos = top_rect(ctx, "level").center();
        click_at(app, ctx, pos);
        frame(app, ctx, key_with(egui::Key::A, egui::Modifiers::COMMAND));
        frame(app, ctx, vec![egui::Event::Text(text.into())]);
    }

    #[test]
    fn a_level_being_typed_counts_as_unsaved_and_flushes_before_save() {
        let (mut app, ctx) = notes_app();
        frame(&mut app, &ctx, vec![]);
        type_level(&mut app, &ctx, "75");
        assert!(app.top_bar.has_pending());
        assert!(app.current_unsaved());
        let before = app.sent_full.len();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(
            sent_since(&app, before),
            ["SetCharacterLevel { level: 75 }", "Save"]
        );
        assert!(!app.top_bar.has_pending());
    }

    #[test]
    fn dont_save_drops_a_level_being_typed() {
        let (mut app, ctx) = notes_app();
        frame(&mut app, &ctx, vec![]);
        type_level(&mut app, &ctx, "75");
        assert!(app.top_bar.has_pending());
        let before = app.sent_full.len();
        app.answer_prompt(Pending::BackToList, PromptChoice::Discard);
        assert!(!app.top_bar.has_pending());
        let sent = sent_since(&app, before);
        assert!(
            !sent.iter().any(|c| c.contains("SetCharacterLevel")),
            "{sent:?}"
        );
    }

    #[test]
    fn the_new_loadout_window_holds_the_shortcuts() {
        let (mut app, ctx) = notes_app();
        let mut build = (**app.build.as_ref().unwrap()).clone();
        build.loadouts = [
            "^7^7Loadouts:",
            "Default",
            "^7^7-----",
            "^7^7New Loadout",
            "^7^7Sync",
            "^7^7Help >>",
        ]
        .map(String::from)
        .to_vec();
        build.loadout_selected = Some(1);
        app.build = Some(Arc::new(build));
        frame(&mut app, &ctx, vec![]);
        let pos = top_rect(&ctx, "loadouts").center();
        click_at(&mut app, &ctx, pos);
        frame(&mut app, &ctx, vec![]);
        let pos = top_rect(&ctx, "loadout_3").center();
        click_at(&mut app, &ctx, pos);
        assert!(app.top_bar.has_dialog());
        let before = app.sent_full.len();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
    }

    #[test]
    fn calcs_and_sidebar_counts_do_not_share_ids() {
        let (app, _ctx) = notes_app();
        assert_ne!(app.calcs.count_salt(), app.sidebar.count_salt());
    }

    #[test]
    fn calcs_count_typed_then_save_is_flushed() {
        let (mut app, ctx) = notes_app();
        let mut view = crate::engine::views::CalcsView::default();
        view.selector.stages = Some("1".into());
        app.calcs.on_snapshot(&Snapshot::Calcs(Arc::new(view)));
        app.tab = BuildTab::Calcs;
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let pos = app.calcs.rects.map["stages"].center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![egui::Event::Text("3".into())],
        ] {
            frame(&mut app, &ctx, events);
        }
        let before = app.sent_full.len();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(
            sent_since(&app, before),
            ["SetCalcsCount { control: Stages, text: \"13\" }", "Save"]
        );
        // leaving the box doesn't send it again
        let before = app.sent_full.len();
        let away = egui::Pos2::new(900.0, 600.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: away,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(away)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(&mut app, &ctx, events);
        }
        let sent = sent_since(&app, before);
        assert!(
            !sent.iter().any(|c| c.contains("SetCalcsCount")),
            "{sent:?}"
        );
    }

    #[test]
    fn a_build_switch_clears_a_pending_stage_count() {
        let (mut app, _ctx, _) = stages_app();
        assert!(app.sidebar.has_pending());
        app.entering = Some(5);
        app.on_event(Event::Snapshot {
            seq: 5,
            snapshot: Snapshot::Skills(Arc::new(crate::ui::skills::tests::sample())),
        });
        assert!(!app.sidebar.has_pending());
    }

    #[test]
    fn pending_notes_flush_before_save_and_leave() {
        // Save, within the debounce time
        let (mut app, ctx) = notes_app();
        let before = app.sent_log.len();
        app.notes.type_for_test("my notes", 0.0);
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        assert_eq!(
            sent_since(&app, before),
            ["SetNotes { text: \"my notes\" }", "Save"]
        );

        // Back
        let (mut app, ctx) = notes_app();
        let before = app.sent_log.len();
        app.notes.type_for_test("my notes", 0.0);
        app.run_action(&ctx, Pending::BackToList);
        assert_eq!(
            sent_since(&app, before),
            [
                "SetNotes { text: \"my notes\" }",
                "ListBuilds { folder: \"\" }"
            ]
        );

        // opening another build
        let (mut app, ctx) = notes_app();
        let before = app.sent_log.len();
        app.notes.type_for_test("my notes", 0.0);
        app.run_action(&ctx, Pending::Open("/b/y.xml".into()));
        assert_eq!(
            sent_since(&app, before),
            [
                "SetNotes { text: \"my notes\" }",
                "OpenBuild { path: \"/b/y.xml\" }"
            ]
        );

        // leaving the tab
        let (mut app, ctx) = notes_app();
        let before = app.sent_log.len();
        app.notes.type_for_test("my notes", 0.0);
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            sent_since(&app, before),
            ["SetNotes { text: \"my notes\" }"]
        );
    }

    #[test]
    fn ctrl_z_on_notes_sends_no_engine_undo() {
        let (mut app, ctx) = notes_app();
        let before = app.sent_log.len();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Z, egui::Modifiers::COMMAND),
        );
        frame(&mut app, &ctx, ctrl(egui::Key::Y));
        assert!(sent_since(&app, before).is_empty());
    }

    #[test]
    fn back_or_close_with_a_pending_note_asks_first() {
        let (mut app, ctx) = notes_app();
        app.notes.type_for_test("my notes", 0.0);
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.prompt, Some(Pending::BackToList));
        assert_eq!(app.screen, Screen::Build);

        app.prompt = None;
        ctx.run_ui(
            egui::RawInput {
                viewports: std::iter::once((
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        events: vec![egui::ViewportEvent::Close],
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.prompt, Some(Pending::Close));
        assert!(!app.allow_close);
    }

    #[test]
    fn dont_save_drops_the_pending_note_and_save_sends_it() {
        let (mut app, ctx) = notes_app();
        let before = app.sent_log.len();
        app.notes.type_for_test("my notes", 0.0);
        app.prompt = Some(Pending::BackToList);
        // "Don't save"
        app.notes.discard();
        app.ready_actions.push(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before), ["ListBuilds { folder: \"\" }"]);

        // "Save" flushes first, then saves
        let (mut app, _ctx) = notes_app();
        let before = app.sent_log.len();
        app.notes.type_for_test("my notes", 0.0);
        app.save(Some(Pending::BackToList));
        assert_eq!(
            sent_since(&app, before),
            ["SetNotes { text: \"my notes\" }", "Save"]
        );
    }

    // ---- the crafting popup window ------------------------------------------------------

    fn corrupt_display() -> crate::engine::DisplayItemView {
        crate::engine::DisplayItemView {
            popups: vec![crate::engine::PopupKind::Corrupt],
            ..display_with_quality()
        }
    }

    fn popup_view(kind: crate::engine::PopupKind) -> crate::engine::PopupView {
        crate::engine::PopupView {
            kind,
            title: "Corrupt Item".into(),
            enter: None,
            controls: vec![],
        }
    }

    /// The items tab with the display item offering "Corrupt...", and the popup open as the
    /// engine would report it after the button was pressed.
    fn pending_popup_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = items_app();
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(corrupt_display()))),
        });
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        click_item_key(&mut app, &ctx, "open_Corrupt...");
        assert_eq!(app.sent_log.last().map(String::as_str), Some("OpenPopup"));
        (app, ctx)
    }

    fn popup_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = pending_popup_app();
        app.on_event(Event::Snapshot {
            seq: 92,
            snapshot: Snapshot::Popup(Some(Arc::new(popup_view(
                crate::engine::PopupKind::Corrupt,
            )))),
        });
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    #[test]
    fn popup_window_closes_on_tab_leave() {
        let (mut app, ctx) = popup_app();
        assert!(app.popup.is_open());
        assert_eq!(app.popup_tab, Some(BuildTab::Items));
        let before = app.sent_full.len();
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before), ["ClosePopup"]);
        assert!(!app.popup.is_open());
        // a view still on its way from before the close doesn't bring it back
        app.on_event(Event::Snapshot {
            seq: app.last_sent,
            snapshot: Snapshot::Popup(Some(Arc::new(popup_view(
                crate::engine::PopupKind::Corrupt,
            )))),
        });
        assert!(!app.popup.is_open());
    }

    #[test]
    fn a_failed_import_that_replaced_the_build_shows_the_list_with_pobs_message() {
        // PoB's Init fails, PoB closes the build and shows its list with the message; the
        // engine puts a new build in its place, which the app never shows
        let message = "Unknown Passive Tree Version: the tree version of this build isn't known";
        let fail = |app: &mut AutoPobApp, generation: u64| {
            let seq = app.send(Command::ImportFromUrl {
                url: "eNrtWk1z".into(),
                mode: ImportMode::ThisBuild,
            });
            app.on_event(Event::Error {
                seq,
                message: message.into(),
            });
            let mut view = (**app.build.as_ref().unwrap()).clone();
            view.generation = generation;
            view.name = "Unnamed build".into();
            app.on_event(Event::Snapshot {
                seq,
                snapshot: Snapshot::Build(Arc::new(view)),
            });
        };
        let (mut app, ctx) = open_new_build();
        app.tab = BuildTab::Import;
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        let generation = app.build.as_ref().unwrap().generation;
        fail(&mut app, generation + 1);
        assert_eq!(app.screen, Screen::List);
        assert!(
            app.notices.iter().any(|n| n == message),
            "{:?}",
            app.notices
        );
        assert!(
            sent_since(&app, before)
                .iter()
                .any(|c| c.starts_with("ListBuilds")),
        );
        // an import refused before PoB's Init (a bad code) leaves the build as it was
        let (mut app, ctx) = open_new_build();
        app.tab = BuildTab::Import;
        frame(&mut app, &ctx, vec![]);
        let generation = app.build.as_ref().unwrap().generation;
        fail(&mut app, generation);
        assert_eq!(app.screen, Screen::Build);
        assert_eq!(app.tab, BuildTab::Import);
    }

    #[test]
    fn the_app_starts_on_the_build_pob_reopened() {
        // PoB starts on the build its last exit recorded, and on its list otherwise
        for (open, screen) in [(true, Screen::Build), (false, Screen::List)] {
            let (mut app, ctx) = started();
            app.screen = Screen::List;
            app.on_event(Event::Snapshot {
                seq: 0,
                snapshot: Snapshot::BuildList(Arc::new(BuildListView {
                    build_open: open,
                    ..(**app.list.as_ref().unwrap()).clone()
                })),
            });
            frame(&mut app, &ctx, vec![]);
            assert_eq!(app.screen, screen, "build_open {open}");
        }
        // only the start's listing: a later one never takes the user off the list
        let (mut app, ctx) = started();
        app.on_event(Event::Snapshot {
            seq: app.last_sent.max(1),
            snapshot: Snapshot::BuildList(Arc::new(BuildListView {
                build_open: true,
                ..(**app.list.as_ref().unwrap()).clone()
            })),
        });
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.screen, Screen::List);
    }

    #[test]
    fn a_failure_with_no_build_loaded_doesnt_bounce_the_next_open() {
        // no build loaded: the failed code's error comes with no Build view, so its check is
        // never answered; the next good open must not take it for its own
        let (mut app, ctx) = started();
        app.build = None;
        app.enter_build(Command::ImportShareCode { code: "bad".into() });
        let bad = app.entering.expect("an import");
        app.on_event(Event::Error {
            seq: bad,
            message: "that isn't a valid build code".into(),
        });
        assert_eq!(app.screen, Screen::List);
        app.enter_build(Command::OpenBuild {
            path: "/b/x.xml".into(),
        });
        let open = app.entering.expect("an open");
        let view = BuildView {
            name: "x".into(),
            path: Some("/b/x.xml".into()),
            generation: 1,
            ..BuildView::default()
        };
        app.on_event(Event::Snapshot {
            seq: open,
            snapshot: Snapshot::Build(Arc::new(view)),
        });
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            app.screen,
            Screen::Build,
            "the good open bounced to the list"
        );
    }

    #[test]
    fn a_failed_link_import_that_replaced_the_build_shows_the_list() {
        let (mut app, ctx) = open_new_build();
        frame(&mut app, &ctx, vec![]);
        app.on_requests(&RequestsView {
            pending: vec![],
            finished: vec![crate::engine::views::FinishedRequest {
                id: 1,
                label: "Import".into(),
                error: Some("Unknown Passive Tree Version".into()),
                cancelled: false,
                new_build: true,
            }],
        });
        assert_eq!(app.screen, Screen::List);
        assert!(
            app.notices
                .iter()
                .any(|n| n.contains("Unknown Passive Tree Version"))
        );
    }

    /// PoB's Game Version window as the engine reports it after an open that needs it
    fn game_version_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = open_new_build();
        let button = |name: &str, x: f32| crate::engine::views::PopupControl {
            name: name.into(),
            kind: crate::engine::views::PopupControlKind::Button,
            label: name.into(),
            pos: [x, 40.0],
            size: [80.0, 20.0],
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
        };
        // an open whose build needs it: the engine's window comes with the open's views, the
        // first of them here, before the build screen's reset
        app.run_action(&ctx, Pending::Open("/b/old.xml".into()));
        let seq = app.entering.expect("an open");
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::Popup(Some(Arc::new(crate::engine::PopupView {
                kind: crate::engine::PopupKind::GameVersion,
                title: "Game Version".into(),
                enter: None,
                controls: vec![button("convert", 0.0), button("cancel", 100.0)],
            }))),
        });
        pump_until(&mut app, &ctx, "the open's views", |a| a.entering.is_none());
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }

    fn click_popup(app: &mut AutoPobApp, ctx: &egui::Context, key: &str) {
        let pos = app.popup.rects.map[key].center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(app, ctx, events);
        }
    }

    #[test]
    fn game_version_stays_up_and_its_cancel_goes_to_the_list() {
        // PoB's window is main's: it stays up over the build, whatever the tab
        // (Build.lua:1323-1350)
        let (mut app, ctx) = game_version_app();
        assert!(app.popup.is_open(), "the window went at once");
        app.tab = BuildTab::Items;
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert!(app.popup.is_open(), "a tab switch ended it");
        assert!(!app.sent_full.iter().any(|c| c == "ClosePopup"));
        // Cancel goes back to the build list (Build.lua:1344-1347)
        let before = app.sent_full.len();
        click_popup(&mut app, &ctx, "btn_cancel");
        assert_eq!(app.screen, Screen::List);
        let sent = sent_since(&app, before);
        assert!(
            sent.first()
                .is_some_and(|c| c.starts_with("PressPopupButton { name: \"cancel\"")),
            "{sent:?}"
        );
        assert!(sent.iter().any(|c| c.starts_with("ListBuilds")), "{sent:?}");

        // Esc is Cancel too
        let (mut app, ctx) = game_version_app();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Escape, egui::Modifiers::NONE),
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.screen, Screen::List);

        // Convert stays on the build screen
        let (mut app, ctx) = game_version_app();
        let before = app.sent_full.len();
        click_popup(&mut app, &ctx, "btn_convert");
        assert_eq!(app.screen, Screen::Build);
        assert!(
            sent_since(&app, before)
                .iter()
                .any(|c| c.starts_with("PressPopupButton { name: \"convert\"")),
        );
    }

    #[test]
    fn popup_window_closes_on_engine_restart() {
        let (mut app, _ctx) = popup_app();
        assert!(app.popup.is_open());
        app.restart();
        assert!(!app.popup.is_open());
        assert_eq!(app.popup_tab, None);
    }

    #[test]
    fn popup_window_resets_for_another_build() {
        let (mut app, ctx) = popup_app();
        app.entering = Some(100);
        app.on_event(Event::Snapshot {
            seq: 100,
            snapshot: Snapshot::Items(Arc::new(crate::ui::items::tests::sample())),
        });
        assert!(!app.popup.is_open());
        frame(&mut app, &ctx, vec![]);
    }

    #[test]
    fn open_popup_flushes_pending_edits_first() {
        let (mut app, ctx) = items_app();
        app.stats = Some(Arc::new(crate::engine::views::StatsView {
            main_skill: crate::engine::views::MainSkillSelector {
                stages: Some("1".into()),
                ..Default::default()
            },
            ..Default::default()
        }));
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(corrupt_display()))),
        });
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        let stages: egui::Rect = ctx
            .data(|d| d.get_temp(egui::Id::NULL.with(("sidebar_rect", "stages".to_owned()))))
            .expect("the stages box is drawn");
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(stages.center())],
            vec![press(stages.center(), true)],
            vec![press(stages.center(), false)],
            vec![egui::Event::Text("2".into())],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(app.sidebar.has_pending());
        // whatever opens the window (the button, the tree's right click), the box still holds
        // its focus and its text when the command goes out
        let before = app.sent_full.len();
        app.send(Command::OpenPopup(crate::engine::PopupKind::Corrupt));
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            sent_since(&app, before),
            [
                "SetMainSkillCount { control: Stages, text: \"12\" }",
                "OpenPopup(Corrupt)"
            ]
        );
    }

    #[test]
    fn an_open_popup_blocks_shortcuts_and_the_tab_behind() {
        let (mut app, ctx) = popup_app();
        let before = app.sent_full.len();
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::S, egui::Modifiers::COMMAND),
        );
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Z, egui::Modifiers::COMMAND),
        );
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        // a click on the panel behind the window would end the session
        let pos = app.items.rect("cancel").center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        assert!(app.popup.is_open());
    }

    /// Clicks over a grid of points, so every widget in the region gets pressed.
    fn click_sweep(
        app: &mut AutoPobApp,
        ctx: &egui::Context,
        xs: std::ops::Range<u32>,
        ys: std::ops::Range<u32>,
        step: usize,
    ) {
        for y in ys.step_by(step) {
            for x in xs.clone().step_by(step) {
                let pos = egui::Pos2::new(x as f32, y as f32);
                let press = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                for events in [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![press(true)],
                    vec![press(false)],
                ] {
                    frame(app, ctx, events);
                }
            }
        }
    }

    #[test]
    fn an_open_popup_blocks_the_top_bar_the_tabs_and_the_sidebar() {
        let (mut app, ctx) = popup_app();
        let before = app.sent_full.len();
        // top bar (Back, Save, Save as, undo, redo), the tab strip and the sidebar
        click_sweep(&mut app, &ctx, 0..700, 0..120, 10);
        click_sweep(&mut app, &ctx, 0..300, 120..400, 30);
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        assert_eq!(app.tab, BuildTab::Items);
        assert_eq!(app.screen, Screen::Build);
        assert!(app.prompt.is_none() && app.save_as.is_none());
        assert!(app.popup.is_open());
    }

    #[test]
    fn the_ui_is_blocked_from_the_opener_press_until_the_view_or_an_error() {
        let (mut app, ctx) = items_app();
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(corrupt_display()))),
        });
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        click_item_key(&mut app, &ctx, "open_Corrupt...");
        frame(&mut app, &ctx, vec![]);
        assert!(!app.popup.is_open());
        assert!(app.popup.is_blocking(), "OpenPopup is pending");
        // Cancel, behind the pending window, would end the session
        let before = app.sent_full.len();
        click_item_key(&mut app, &ctx, "cancel");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        // an error for the open frees the UI
        let open_seq = app.last_sent;
        app.on_event(Event::Error {
            seq: open_seq,
            message: "refused".into(),
        });
        assert!(!app.popup.is_blocking());
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        click_item_key(&mut app, &ctx, "cancel");
        assert_eq!(
            sent_since(&app, before),
            ["CancelDisplayItem { if_id: None }"]
        );
    }

    #[test]
    fn the_view_frees_a_pending_open() {
        let (mut app, ctx) = items_app();
        app.on_event(Event::Snapshot {
            seq: 91,
            snapshot: Snapshot::DisplayItem(Some(Arc::new(corrupt_display()))),
        });
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        click_item_key(&mut app, &ctx, "open_Corrupt...");
        // a view older than the open doesn't count
        app.on_event(Event::Snapshot {
            seq: 1,
            snapshot: Snapshot::Popup(None),
        });
        assert!(app.popup.is_blocking());
        app.on_event(Event::Snapshot {
            seq: app.last_sent,
            snapshot: Snapshot::Popup(Some(Arc::new(popup_view(
                crate::engine::PopupKind::Corrupt,
            )))),
        });
        assert!(app.popup.is_open());
    }

    #[test]
    fn closing_the_window_with_the_popup_open_closes_it_and_shows_the_prompt() {
        let (mut app, ctx) = popup_app();
        app.notes.type_for_test("my notes", 0.0);
        let before = app.sent_full.len();
        ctx.run_ui(
            egui::RawInput {
                viewports: std::iter::once((
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        events: vec![egui::ViewportEvent::Close],
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.prompt, Some(Pending::Close));
        assert!(
            !app.popup.is_open(),
            "the prompt must not hide under the modal"
        );
        assert!(sent_since(&app, before).contains(&"ClosePopup"));
    }

    #[test]
    fn closing_the_window_while_the_open_is_pending_closes_the_popup_too() {
        let (mut app, ctx) = pending_popup_app();
        assert!(app.popup.is_blocking() && !app.popup.is_open());
        app.notes.type_for_test("my notes", 0.0);
        let open_seq = app.last_sent;
        let before = app.sent_full.len();
        ctx.run_ui(
            egui::RawInput {
                viewports: std::iter::once((
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        events: vec![egui::ViewportEvent::Close],
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.prompt, Some(Pending::Close));
        assert!(sent_since(&app, before).contains(&"ClosePopup"));
        // the window the engine opens before it hears of the close never shows
        app.on_event(Event::Snapshot {
            seq: open_seq,
            snapshot: Snapshot::Popup(Some(Arc::new(popup_view(
                crate::engine::PopupKind::Corrupt,
            )))),
        });
        assert!(!app.popup.is_open());
        assert!(!app.popup.is_blocking());
    }

    #[test]
    fn a_popup_error_is_a_notice_and_keeps_the_window() {
        let (mut app, ctx) = popup_app();
        app.on_event(Event::Error {
            seq: 93,
            message: "the list changed; pick again".into(),
        });
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.notices, ["the list changed; pick again"]);
        assert!(app.popup.is_open());
    }

    // ---- the database panel and the tree's right click ----------------------------------------

    /// The tree tab with an allocated jewel socket (node 26725), and the items view whose
    /// "Jewel 5" slot belongs to it. `enabled`: PoB lets the slot be used.
    fn socket_app(enabled: bool) -> (AutoPobApp, egui::Context) {
        use crate::engine::{NodeArt, TreeLayout, TreeNode, TreeState};
        let (mut app, ctx) = tree_app();
        let layout = TreeLayout {
            version: "3_28".into(),
            nodes: vec![TreeNode {
                id: 26725,
                pos: [0.0, 0.0],
                kind: "Socket".into(),
                name: "Jewel Socket".into(),
                stats: vec![],
                orbit: None,
                group_center: None,
                art: NodeArt::default(),
                ascendancy: None,
                replaceable: false,
                mastery_effects: false,
            }],
            edges: vec![],
            sheets: vec![],
            key: "socket".into(),
        };
        let state: TreeState = serde_json::from_value(serde_json::json!({
            "version": "3_28", "version_label": "3.28", "class_id": 0, "ascendancy_id": 0,
            "points": { "used": 1, "available": 122, "ascendancy_used": 0,
                        "ascendancy_available": 8 },
            "allocated": [26725],
            "active_spec": 0, "can_undo": false, "can_redo": false,
        }))
        .unwrap();
        app.tree
            .on_snapshot(&Snapshot::TreeLayout(Arc::new(layout)));
        app.tree.on_snapshot(&Snapshot::Tree(Arc::new(state)));
        let mut items = crate::ui::items::tests::sample();
        items.slots[9].node = Some(26725);
        items.slots[9].enabled = enabled;
        app.on_event(Event::Snapshot {
            seq: 90,
            snapshot: Snapshot::Items(Arc::new(items)),
        });
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }

    fn right_click_socket(app: &mut AutoPobApp, ctx: &egui::Context) {
        let rect = app.tree.rect.expect("drawn");
        let pos = app.tree.camera.to_screen(rect, [0.0, 0.0]);
        let secondary = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![secondary(true)],
            vec![secondary(false)],
        ] {
            frame(app, ctx, events);
        }
    }

    #[test]
    fn jump_to_socket_switches_to_items_and_focuses_slot() {
        let (mut app, ctx) = socket_app(true);
        assert_eq!(app.tab, BuildTab::Tree);
        let before = app.sent_full.len();
        right_click_socket(&mut app, &ctx);
        assert_eq!(app.tab, BuildTab::Items);
        assert_eq!(app.items.focused_slot(), Some("Jewel 5"));
        // the slot's dropdown is drawn open on the Items tab; the jump sends nothing
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.items.focused_slot(), None);
        assert!(
            egui::Popup::is_any_open(&ctx),
            "the slot's dropdown is open"
        );
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
    }

    #[test]
    fn a_socket_whose_slot_is_not_enabled_does_not_jump() {
        let (mut app, ctx) = socket_app(false);
        right_click_socket(&mut app, &ctx);
        assert_eq!(app.tab, BuildTab::Tree);
        assert_eq!(app.items.focused_slot(), None);
    }

    /// The items tab with the database panel open.
    fn db_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = items_app();
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        click_item_key(&mut app, &ctx, "db_header");
        assert_eq!(app.sent_log.last().map(String::as_str), Some("SetItemDb"));
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }

    #[test]
    fn db_closed_on_tab_leave() {
        let (mut app, ctx) = db_app();
        let before = app.sent_full.len();
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before), ["CloseItemDb"]);
        // back on the tab, the open panel asks for its database again
        app.tab = BuildTab::Items;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before)[1..].len(), 1);
        assert!(sent_since(&app, before)[1].starts_with("SetItemDb("));
    }

    #[test]
    fn db_progress_and_failure_events_reach_the_panel() {
        let (mut app, ctx) = db_app();
        let mut sorting = crate::ui::items::tests::db_view_for_app();
        sorting.sorting = true;
        sorting.rows.clear();
        app.on_event(Event::Snapshot {
            seq: 95,
            snapshot: Snapshot::ItemDb(Some(Arc::new(sorting))),
        });
        app.on_event(Event::Progress {
            task: BackgroundTask::ItemDb,
            percent: 40,
        });
        frame(&mut app, &ctx, vec![]);
        assert!(app.items.rect("db_progress").width() > 0.0);
        app.on_event(Event::BackgroundFailed {
            task: BackgroundTask::ItemDb,
            message: "boom".into(),
        });
        frame(&mut app, &ctx, vec![]);
        assert!(app.items.rect("db_retry").width() > 0.0);
        assert!(app.notices.is_empty(), "the panel shows its own error");
    }

    /// A pending DB search goes out before `OpenPopup` (M10, R4).
    #[test]
    fn open_popup_flushes_the_db_search_first() {
        let (mut app, ctx) = db_app();
        click_item_key(&mut app, &ctx, "db_search");
        frame(&mut app, &ctx, vec![egui::Event::Text("ring".into())]);
        let before = app.sent_full.len();
        app.send(Command::OpenPopup(crate::engine::PopupKind::Corrupt));
        let sent = sent_since(&app, before);
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(sent[0].starts_with("SetItemDb(") && sent[0].contains("\"ring\""));
        assert_eq!(sent[1], "OpenPopup(Corrupt)");
        // once: the box has nothing left to send
        frame(&mut app, &ctx, vec![]);
        assert_eq!(sent_since(&app, before).len(), 2);
    }

    /// Keyboard focus a widget under the popup's modal held must not let typing through (M5).
    #[test]
    fn typing_while_the_popup_opens_does_not_reach_the_boxes_behind() {
        let (mut app, ctx) = items_app_typing();
        assert!(ctx.text_edit_focused(), "the quality box holds the focus");
        assert_eq!(app.items.quality_text(), "25");
        let before = app.sent_full.len();
        app.send(Command::OpenPopup(crate::engine::PopupKind::Corrupt));
        // the edit that was being typed went out first
        assert_eq!(sent_since(&app, before), [QUALITY, "OpenPopup(Corrupt)"]);
        frame(&mut app, &ctx, vec![]);
        app.on_event(Event::Snapshot {
            seq: 99,
            snapshot: Snapshot::Popup(Some(Arc::new(popup_view(
                crate::engine::PopupKind::Corrupt,
            )))),
        });
        frame(&mut app, &ctx, vec![]);
        assert!(app.popup.is_open());
        // egui's modal takes the focus the box behind it held, so keys can't reach it
        assert!(!ctx.text_edit_focused(), "the focus is gone");
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Backspace, egui::Modifiers::NONE),
        );
        frame(&mut app, &ctx, vec![egui::Event::Text("7".into())]);
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Enter, egui::Modifiers::NONE),
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            sent_since(&app, before),
            [QUALITY, "OpenPopup(Corrupt)"],
            "typing under the window sent nothing"
        );
    }

    /// The frames while `OpenPopup` is pending are typed into too (the spinner's modal).
    #[test]
    fn typing_while_the_popup_is_pending_does_not_reach_the_boxes_behind() {
        let (mut app, ctx) = items_app_typing();
        assert!(ctx.text_edit_focused(), "the quality box holds the focus");
        let before = app.sent_full.len();
        app.send(Command::OpenPopup(crate::engine::PopupKind::Corrupt));
        assert!(app.popup.is_blocking() && !app.popup.is_open());
        for _ in 0..2 {
            frame(&mut app, &ctx, vec![]);
        }
        // the box takes two characters, so a key must free one for the 7 to land
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Backspace, egui::Modifiers::NONE),
        );
        frame(&mut app, &ctx, vec![egui::Event::Text("7".into())]);
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Enter, egui::Modifiers::NONE),
        );
        frame(&mut app, &ctx, vec![]);
        assert!(
            !app.items.quality_text().contains('7'),
            "the box behind took a key: {}",
            app.items.quality_text()
        );
        assert_eq!(
            sent_since(&app, before),
            [QUALITY, "OpenPopup(Corrupt)"],
            "typing while the open is pending sent nothing"
        );
    }

    #[test]
    fn anoint_sort_events_reach_the_popup() {
        let (mut app, _ctx) = popup_app();
        app.on_event(Event::Progress {
            task: BackgroundTask::AnointSort,
            percent: 55,
        });
        assert_eq!(app.popup.sort_line(), (Some(55), None));
        app.on_event(Event::BackgroundFailed {
            task: BackgroundTask::AnointSort,
            message: "boom".into(),
        });
        assert_eq!(app.popup.sort_line(), (None, Some("boom")));
        assert!(
            app.notices.is_empty(),
            "the window shows it: {:?}",
            app.notices
        );
    }

    // ---- the Import/Export tab and the pending line ------------------------------------

    use crate::engine::views::{
        AccountView, FinishedRequest, ImportCodeView, OAuthView, PendingRequest, ShareView, SiteRef,
    };
    use crate::ui::import::marked;

    fn deliver(app: &mut AutoPobApp, snapshot: Snapshot) -> u64 {
        let seq = app.last_sent.max(app.last_snapshot) + 1;
        app.on_event(Event::Snapshot { seq, snapshot });
        seq
    }

    fn requests_snapshot(pending: &[(u64, &str)], finished: Vec<FinishedRequest>) -> Snapshot {
        Snapshot::Requests(Arc::new(RequestsView {
            pending: pending
                .iter()
                .map(|(id, label)| PendingRequest {
                    id: *id,
                    label: (*label).into(),
                })
                .collect(),
            finished,
        }))
    }

    fn finished(id: u64, error: Option<&str>, cancelled: bool, new_build: bool) -> FinishedRequest {
        FinishedRequest {
            id,
            label: "Fetching characters".into(),
            error: error.map(str::to_owned),
            cancelled,
            new_build,
        }
    }

    fn click_mark(app: &mut AutoPobApp, ctx: &egui::Context, key: &str) {
        crate::ui::import::clear_marks();
        frame(app, ctx, vec![]);
        let pos = marked(key)
            .unwrap_or_else(|| panic!("no widget {key}"))
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, ctx, vec![egui::Event::PointerMoved(pos)]);
        frame(app, ctx, vec![button(true)]);
        frame(app, ctx, vec![button(false)]);
        frame(app, ctx, vec![]);
    }

    fn import_app() -> (AutoPobApp, egui::Context) {
        let (mut app, ctx) = open_new_build();
        app.build = Some(Arc::new(BuildView {
            path: Some("/b/x.xml".into()),
            ..(**app.build.as_ref().unwrap()).clone()
        }));
        app.tab = BuildTab::Import;
        deliver(
            &mut app,
            Snapshot::Account(Arc::new(AccountView::default())),
        );
        deliver(
            &mut app,
            Snapshot::Share(Arc::new(ShareView {
                sites: vec![SiteRef {
                    id: "POBBin".into(),
                    label: "pobb.in".into(),
                }],
                import: ImportCodeView {
                    can_import_to_this_build: true,
                    ..ImportCodeView::default()
                },
                ..ShareView::default()
            })),
        );
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    /// Runs frames and collects what egui was asked to do besides drawing.
    fn outputs_within(
        app: &mut AutoPobApp,
        ctx: &egui::Context,
        frames: usize,
    ) -> (Vec<String>, Vec<String>, bool) {
        let (mut opened, mut copied, mut focus) = (Vec::new(), Vec::new(), false);
        for _ in 0..frames {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| app.show(ui));
            for command in &output.platform_output.commands {
                match command {
                    egui::OutputCommand::OpenUrl(open) => opened.push(open.url.clone()),
                    egui::OutputCommand::CopyText(text) => copied.push(text.clone()),
                    _ => {}
                }
            }
            focus |= output.viewport_output.values().any(|v| {
                v.commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::Focus))
            });
            output.drop_without_applying_deltas();
        }
        (opened, copied, focus)
    }

    fn waiting_account(url: &str) -> AccountView {
        AccountView {
            oauth: OAuthView {
                signing_in: true,
                auth_url: Some(url.into()),
                login_deadline: Some(i64::MAX),
                ..OAuthView::default()
            },
            ..AccountView::default()
        }
    }

    const AUTH: &str = "https://www.pathofexile.com/oauth/authorize?client_id=x";

    #[test]
    fn the_app_opens_the_auth_url_once_and_focuses_when_signed_in() {
        let (mut app, ctx) = import_app();
        click_mark(&mut app, &ctx, "sign_in");
        assert_eq!(app.sent_log.last().map(String::as_str), Some("SignIn"));
        let seq = app.last_sent;
        for i in 0..3 {
            app.on_event(Event::Snapshot {
                seq: seq + i,
                snapshot: Snapshot::Account(Arc::new(waiting_account(AUTH))),
            });
        }
        let (opened, _, focus) = outputs_within(&mut app, &ctx, 3);
        assert_eq!(opened, [AUTH]);
        assert!(!focus, "still waiting for the browser");
        let account = AccountView {
            oauth: OAuthView {
                signed_in: true,
                ..OAuthView::default()
            },
            ..AccountView::default()
        };
        app.on_event(Event::Snapshot {
            seq: seq + 5,
            snapshot: Snapshot::Account(Arc::new(account)),
        });
        let (opened, _, focus) = outputs_within(&mut app, &ctx, 3);
        assert!(opened.is_empty(), "{opened:?}");
        assert!(focus, "the window comes back once the sign-in completes");
        let (_, _, again) = outputs_within(&mut app, &ctx, 3);
        assert!(!again);
    }

    #[test]
    fn share_copy_and_ctrl_c_use_the_ui_clipboard_not_the_engine() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            Snapshot::Share(Arc::new(ShareView {
                sites: vec![SiteRef {
                    id: "POBBin".into(),
                    label: "pobb.in".into(),
                }],
                link: Some("https://pobb.in/abc".into()),
                import: ImportCodeView::default(),
                ..ShareView::default()
            })),
        );
        frame(&mut app, &ctx, vec![]);
        // the link only reaches the clipboard when the user presses Copy
        let (_, copied, _) = outputs_within(&mut app, &ctx, 3);
        assert!(copied.is_empty(), "{copied:?}");
        click_mark(&mut app, &ctx, "copy_link_result");
        // Ctrl+C on the tab asks for the code, then copies the answer
        let before = app.sent_full.len();
        frame(&mut app, &ctx, ctrl(egui::Key::C));
        assert_eq!(sent_since(&app, before), ["ExportShareCode"]);
        deliver(&mut app, Snapshot::ShareCode(Arc::new("eNrt".into())));
        let (_, copied, _) = outputs_within(&mut app, &ctx, 3);
        assert_eq!(copied, ["eNrt"]);
        assert!(
            app.sent_log.iter().all(|c| !c.contains("Copy")),
            "the engine never copies: {:?}",
            app.sent_log
        );
    }

    #[test]
    fn the_link_copy_button_copies_the_link() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            Snapshot::Share(Arc::new(ShareView {
                link: Some("https://pobb.in/abc".into()),
                ..ShareView::default()
            })),
        );
        crate::ui::import::clear_marks();
        frame(&mut app, &ctx, vec![]);
        let pos = marked("copy_link_result").unwrap().center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]);
        frame(&mut app, &ctx, vec![button(true)]);
        let mut copied = Vec::new();
        for events in [vec![button(false)], vec![]] {
            let output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            copied.extend(
                output
                    .platform_output
                    .commands
                    .iter()
                    .filter_map(|c| match c {
                        egui::OutputCommand::CopyText(t) => Some(t.clone()),
                        _ => None,
                    }),
            );
            output.drop_without_applying_deltas();
        }
        assert_eq!(copied, ["https://pobb.in/abc"]);
    }

    #[test]
    fn pending_line_cancels_by_id() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            requests_snapshot(
                &[(3, "Sharing on pobb.in"), (8, "Resolving PoEURL link")],
                vec![],
            ),
        );
        frame(&mut app, &ctx, vec![]);
        assert!(marked("cancel:3").is_some() && marked("cancel:8").is_some());
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "cancel:8");
        assert_eq!(sent_since(&app, before), ["CancelRequest { id: 8 }"]);
    }

    #[test]
    fn the_pending_line_shows_on_the_build_list_too() {
        let (mut app, ctx) = started();
        assert_eq!(app.screen, Screen::List);
        deliver(
            &mut app,
            requests_snapshot(&[(4, "Fetching build from pobb.in")], vec![]),
        );
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "cancel:4");
        assert_eq!(sent_since(&app, before), ["CancelRequest { id: 4 }"]);
        // and it goes when the request is over
        deliver(&mut app, requests_snapshot(&[], vec![]));
        crate::ui::import::clear_marks();
        frame(&mut app, &ctx, vec![]);
        assert!(marked("cancel:4").is_none());
    }

    #[test]
    fn cancelling_an_oauth_fetch_asks_first() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            Snapshot::Account(Arc::new(AccountView {
                oauth: OAuthView {
                    signed_in: true,
                    busy: true,
                    ..OAuthView::default()
                },
                ..AccountView::default()
            })),
        );
        deliver(
            &mut app,
            requests_snapshot(&[(5, "Importing passive tree")], vec![]),
        );
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "cancel:5");
        assert!(app.confirm.as_ref().is_some_and(
            |c| c.message == "Cancelling now may sign you out of Path of Exile in both apps."
        ));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        // keeping the request sends nothing
        click_mark(&mut app, &ctx, "confirm_no");
        assert!(app.confirm.is_none());
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        // confirming cancels it
        click_mark(&mut app, &ctx, "cancel:5");
        click_mark(&mut app, &ctx, "confirm_yes");
        assert_eq!(sent_since(&app, before), ["CancelRequest { id: 5 }"]);
    }

    #[test]
    fn cancelling_something_else_needs_no_question() {
        let (mut app, ctx) = import_app();
        deliver(&mut app, requests_snapshot(&[(6, "Signing in")], vec![]));
        frame(&mut app, &ctx, vec![]);
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "cancel:6");
        assert!(app.confirm.is_none());
        assert_eq!(sent_since(&app, before), ["CancelRequest { id: 6 }"]);
    }

    #[test]
    fn cancelled_request_raises_no_notice() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            requests_snapshot(&[], vec![finished(1, Some("Cancelled"), true, false)]),
        );
        frame(&mut app, &ctx, vec![]);
        assert!(app.notices.is_empty(), "{:?}", app.notices);
        // a real failure does
        deliver(
            &mut app,
            requests_snapshot(
                &[],
                vec![finished(2, Some("Timeout was reached"), false, false)],
            ),
        );
        assert_eq!(app.notices, ["Fetching characters: Timeout was reached"]);
        // and a success says nothing
        deliver(
            &mut app,
            requests_snapshot(&[], vec![finished(3, None, false, false)]),
        );
        assert_eq!(app.notices.len(), 1);
    }

    #[test]
    fn finished_new_build_enters_the_build_screen() {
        let (mut app, ctx) = started();
        assert_eq!(app.screen, Screen::List);
        app.tab = BuildTab::Notes;
        // one step: the new build's views, then the requests
        let seq = app.last_sent.max(app.last_snapshot) + 1;
        let build = BuildView {
            name: "Imported".into(),
            path: None,
            class: "Witch".into(),
            ascendancy: None,
            level: 1,
            unsaved: false,
            can_undo: false,
            can_redo: false,
            ..BuildView::default()
        };
        let items = crate::engine::ItemsView {
            can_undo: true,
            ..Default::default()
        };
        for snapshot in [
            requests_snapshot(&[], vec![finished(1, None, false, true)]),
            Snapshot::Build(Arc::new(build)),
            Snapshot::Items(Arc::new(items)),
        ] {
            app.on_event(Event::Snapshot { seq, snapshot });
        }
        assert_eq!(app.screen, Screen::Build);
        assert_eq!(app.tab, BuildTab::Tree, "the build screen starts over");
        assert!(app.entering.is_none());
        assert_eq!(
            app.build.as_ref().map(|b| b.name.as_str()),
            Some("Imported")
        );
        // the views that came before the requests survive the reset
        assert_eq!(app.items.history(), (true, false));
        // a finished request without a new build leaves the screen alone
        app.tab = BuildTab::Notes;
        let seq = deliver(
            &mut app,
            requests_snapshot(&[], vec![finished(2, None, false, false)]),
        );
        assert!(seq > 0);
        assert_eq!(app.tab, BuildTab::Notes);
        frame(&mut app, &ctx, vec![]);
    }

    #[test]
    fn a_new_build_resets_the_tab_state_of_the_old_one() {
        let (mut app, _ctx) = import_app();
        let old = crate::engine::ItemsView {
            can_undo: true,
            ..Default::default()
        };
        deliver(&mut app, Snapshot::Items(Arc::new(old)));
        assert_eq!(app.items.history(), (true, false));
        let seq = app.last_sent.max(app.last_snapshot) + 1;
        for snapshot in [
            requests_snapshot(&[], vec![finished(1, None, false, true)]),
            Snapshot::Build(Arc::new((**app.build.as_ref().unwrap()).clone())),
        ] {
            app.on_event(Event::Snapshot { seq, snapshot });
        }
        assert_eq!(
            app.items.history(),
            (false, false),
            "the old items are gone"
        );
        assert_eq!(app.tab, BuildTab::Tree);
    }

    #[test]
    fn restart_clears_pending() {
        let (mut app, ctx) = started();
        deliver(&mut app, requests_snapshot(&[(4, "Signing in")], vec![]));
        frame(&mut app, &ctx, vec![]);
        assert!(marked("cancel:4").is_some());
        app.restart();
        assert!(app.requests.pending.is_empty());
        crate::ui::import::clear_marks();
        frame(&mut app, &ctx, vec![]);
        assert!(marked("cancel:4").is_none());
    }

    fn valid_share() -> Snapshot {
        Snapshot::Share(Arc::new(ShareView {
            import: ImportCodeView {
                text: "https://pobb.in/abc".into(),
                valid: true,
                detail: "URL is valid (pobb.in)".into(),
                can_import_to_this_build: true,
                ..ImportCodeView::default()
            },
            ..ShareView::default()
        }))
    }

    #[test]
    fn new_build_import_prompts_for_unsaved_changes() {
        let (mut app, ctx) = import_app();
        deliver(&mut app, valid_share());
        frame(&mut app, &ctx, vec![]);
        app.build = Some(Arc::new(BuildView {
            unsaved: true,
            ..(**app.build.as_ref().unwrap()).clone()
        }));
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "import_code");
        assert_eq!(
            app.prompt,
            Some(Pending::ImportNew("https://pobb.in/abc".into()))
        );
        assert!(
            sent_since(&app, before)
                .iter()
                .all(|c| !c.contains("ImportFromUrl")),
            "nothing is imported before the answer"
        );
        // once the build is saved it goes at once
        app.prompt = None;
        app.build = Some(Arc::new(BuildView {
            unsaved: false,
            ..(**app.build.as_ref().unwrap()).clone()
        }));
        click_mark(&mut app, &ctx, "import_code");
        assert!(app.prompt.is_none());
        assert_eq!(
            sent_since(&app, before),
            ["ImportFromUrl { url: \"https://pobb.in/abc\", mode: NewBuild }"]
        );
    }

    #[test]
    fn this_build_import_asks_then_replaces_the_build_screen() {
        let (mut app, ctx) = import_app();
        deliver(&mut app, valid_share());
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "mode:this");
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "import_code");
        assert!(app.confirm.as_ref().is_some_and(|c| c.message
            == "Importing to the current build will erase ALL existing data for this build."));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click_mark(&mut app, &ctx, "confirm_yes");
        assert_eq!(
            sent_since(&app, before),
            ["ImportFromUrl { url: \"https://pobb.in/abc\", mode: ThisBuild }"]
        );
        // a code imports at once: its Build view, at the command's seq, starts the screen over
        assert_eq!(app.tab, BuildTab::Import);
        let seq = app.last_sent;
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::Build(Arc::new((**app.build.as_ref().unwrap()).clone())),
        });
        assert_eq!(app.tab, BuildTab::Tree);
        // not again for the next view
        app.tab = BuildTab::Import;
        app.on_event(Event::Snapshot {
            seq: seq + 1,
            snapshot: Snapshot::Build(Arc::new((**app.build.as_ref().unwrap()).clone())),
        });
        assert_eq!(app.tab, BuildTab::Import);
    }

    #[test]
    fn a_character_import_into_a_built_tree_asks_first() {
        let (mut app, ctx) = import_app();
        let account = AccountView {
            oauth: OAuthView {
                signed_in: true,
                fetched: true,
                leagues: vec!["Standard".into(), "Any".into()],
                characters: vec![crate::engine::views::CharacterEntry {
                    name: "Alice".into(),
                    class: "Witch".into(),
                    level: 90,
                    league: "Standard".into(),
                }],
                ..OAuthView::default()
            },
            ..AccountView::default()
        };
        deliver(&mut app, Snapshot::Account(Arc::new(account)));
        deliver(&mut app, crate::ui::import::tests::tree_snapshot(3, 0));
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "char:Alice");
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "import_character");
        assert!(app.confirm.as_ref().is_some_and(
            |c| c.message == "Importing the passive tree will overwrite your current tree."
        ));
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click_mark(&mut app, &ctx, "confirm_yes");
        let sent = sent_since(&app, before);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("Tree { delete_jewels: true }"), "{sent:?}");
    }

    #[test]
    fn a_new_build_does_not_bring_back_the_old_builds_power_views() {
        // a heat map calculation of the old build sent at the same seq as the import's step
        let (mut app, _ctx) = import_app();
        let seq = app.last_sent.max(app.last_snapshot) + 1;
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::NodePower(Arc::new(crate::engine::NodePower {
                stat: Some("Life".into()),
                nodes: vec![],
                max_single: 1.0,
                max_offence: 1.0,
                max_defence: 1.0,
                finished: true,
            })),
        });
        app.on_event(Event::Snapshot {
            seq,
            snapshot: requests_snapshot(&[], vec![finished(1, None, false, true)]),
        });
        assert!(!app.tree.heatmap_on(), "the old build's heat map is gone");
    }

    #[test]
    fn leaving_asks_while_an_oauth_import_runs() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            Snapshot::Account(Arc::new(AccountView {
                oauth: OAuthView {
                    signed_in: true,
                    busy: true,
                    ..OAuthView::default()
                },
                ..AccountView::default()
            })),
        );
        deliver(
            &mut app,
            requests_snapshot(&[(5, "Fetching characters")], vec![]),
        );
        let before = app.sent_full.len();
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert!(
            app.confirm
                .as_ref()
                .is_some_and(|c| c.message.contains("sign you out"))
        );
        assert_eq!(app.screen, Screen::Build, "nothing happened yet");
        click_mark(&mut app, &ctx, "confirm_no");
        assert_eq!(app.screen, Screen::Build);
        assert!(sent_since(&app, before).is_empty());
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "confirm_yes");
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.screen, Screen::List);
        // without such a request it goes straight away
        let (mut app, ctx) = import_app();
        app.request(Pending::BackToList);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.screen, Screen::List);
    }

    #[test]
    fn blurring_the_account_name_box_in_the_app_sends_no_request() {
        let (mut app, ctx) = import_app();
        click_mark(&mut app, &ctx, "mode:name");
        click_mark(&mut app, &ctx, "account_name");
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Someone#1234".into())],
        );
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "mode:oauth");
        assert!(
            sent_since(&app, before)
                .iter()
                .all(|c| !c.contains("ListPublic")),
            "{:?}",
            sent_since(&app, before)
        );
    }

    #[test]
    fn the_sign_in_link_opens_even_when_the_tab_was_left() {
        let (mut app, ctx) = import_app();
        click_mark(&mut app, &ctx, "sign_in");
        app.tab = BuildTab::Notes;
        let seq = app.last_sent;
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::Account(Arc::new(waiting_account(AUTH))),
        });
        let (opened, _, _) = outputs_within(&mut app, &ctx, 3);
        assert_eq!(opened, [AUTH]);
    }

    // ---- the list's link import, the startup link and the release banner ----------------

    #[test]
    fn startup_link_asks_before_importing() {
        let (mut app, ctx) = fake_app();
        app = app.with_startup_link(Some("pob://pobbin/abc".into()));
        pump_until(&mut app, &ctx, "list", |a| a.ready && a.list.is_some());
        crate::ui::import::clear_marks();
        frame(&mut app, &ctx, vec![]);
        assert!(app.confirm.as_ref().is_some_and(|c| {
            c.message == "Import the build from this link?\npob://pobbin/abc"
        }));
        assert!(
            app.sent_full.iter().all(|c| !c.contains("ImportFromUrl")),
            "nothing is imported before the answer"
        );
        click_mark(&mut app, &ctx, "confirm_yes");
        assert!(app.confirm.is_none());
        assert_eq!(
            app.sent_full
                .iter()
                .filter(|c| c.contains("ImportFromUrl"))
                .collect::<Vec<_>>(),
            [r#"ImportFromUrl { url: "pob://pobbin/abc", mode: NewBuild }"#]
        );
    }

    #[test]
    fn startup_link_yes_asks_about_unsaved_changes_first() {
        // the dialog stays up while the user opens a build and edits it
        let (mut app, ctx) = import_app();
        app = app.with_startup_link(Some("pob://pobbin/abc".into()));
        frame(&mut app, &ctx, vec![]);
        assert!(app.confirm.is_some());
        app.build = Some(Arc::new(BuildView {
            unsaved: true,
            ..(**app.build.as_ref().unwrap()).clone()
        }));
        let before = app.sent_full.len();
        click_mark(&mut app, &ctx, "confirm_yes");
        assert_eq!(
            app.prompt,
            Some(Pending::ImportNew("pob://pobbin/abc".into()))
        );
        assert!(
            sent_since(&app, before)
                .iter()
                .all(|c| !c.contains("ImportFromUrl")),
            "the edited build was replaced with no question: {:?}",
            sent_since(&app, before)
        );
    }

    #[test]
    fn the_confirm_dialog_blocks_the_screen_behind_it() {
        let (mut app, ctx) = fake_app();
        app = app.with_startup_link(Some("pob://pobbin/abc".into()));
        pump_until(&mut app, &ctx, "list", |a| a.ready && a.list.is_some());
        app.import = Some("https://pobb.in/abc".into());
        frame(&mut app, &ctx, vec![]);
        assert!(app.confirm.is_some());
        let before = app.sent_full.len();
        // the list's own button, under the dialog's backdrop
        click_mark(&mut app, &ctx, "list_import");
        assert!(
            sent_since(&app, before).is_empty(),
            "a click went through the dialog: {:?}",
            sent_since(&app, before)
        );
        assert!(app.confirm.is_some(), "the click closed the question");
    }

    #[test]
    fn a_cancel_question_goes_when_its_request_ended() {
        let (mut app, ctx) = import_app();
        deliver(
            &mut app,
            Snapshot::Account(Arc::new(AccountView {
                oauth: OAuthView {
                    signed_in: true,
                    busy: true,
                    ..OAuthView::default()
                },
                ..AccountView::default()
            })),
        );
        deliver(
            &mut app,
            requests_snapshot(&[(5, "Importing passive tree")], vec![]),
        );
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "cancel:5");
        assert!(app.confirm.is_some());
        // the request ends while the question is up
        deliver(&mut app, requests_snapshot(&[], vec![]));
        let before = app.sent_full.len();
        frame(&mut app, &ctx, vec![]);
        assert!(app.confirm.is_none());
        assert!(sent_since(&app, before).is_empty());
    }

    #[test]
    fn a_links_request_ends_the_import_reset_but_a_cancelled_code_import_does_not() {
        // a link: its answer is a Requests view with the request pending
        let (mut app, ctx) = import_app();
        deliver(&mut app, valid_share());
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "mode:this");
        click_mark(&mut app, &ctx, "import_code");
        click_mark(&mut app, &ctx, "confirm_yes");
        let seq = app.last_sent;
        assert_eq!(app.import_seq, Some(seq));
        app.on_event(Event::Snapshot {
            seq,
            snapshot: requests_snapshot(&[(7, "Fetching build from pobb.in")], vec![]),
        });
        assert_eq!(app.import_seq, None);
        // a later view of the same seq (an edit another request made) is no new build
        app.tab = BuildTab::Import;
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::Build(Arc::new((**app.build.as_ref().unwrap()).clone())),
        });
        assert_eq!(app.tab, BuildTab::Import);

        // a code: the cancelled requests come first, with nothing pending
        let (mut app, ctx) = import_app();
        deliver(&mut app, valid_share());
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "mode:this");
        click_mark(&mut app, &ctx, "import_code");
        click_mark(&mut app, &ctx, "confirm_yes");
        let seq = app.last_sent;
        app.on_event(Event::Snapshot {
            seq,
            snapshot: requests_snapshot(&[], vec![]),
        });
        assert_eq!(app.import_seq, Some(seq));
        app.on_event(Event::Snapshot {
            seq,
            snapshot: Snapshot::Build(Arc::new((**app.build.as_ref().unwrap()).clone())),
        });
        assert_eq!(app.tab, BuildTab::Tree);
    }

    #[test]
    fn pending_edits_flush_before_a_confirmed_import() {
        // a stage count still being typed (the sidebar shows on every tab)
        let (mut app, _ctx, _) = stages_app();
        assert!(app.sidebar.has_pending());
        app.confirm = Some(AppConfirm {
            message: "Importing to the current build will erase ALL existing data for this build."
                .into(),
            yes: "Import",
            no: "Cancel",
            then: ConfirmThen::Send(Command::ImportFromUrl {
                url: "https://pobb.in/abc".into(),
                mode: ImportMode::ThisBuild,
            }),
        });
        let before = app.sent_full.len();
        app.answer_confirm(true);
        let sent = sent_since(&app, before);
        let edit = sent.iter().position(|c| c.contains("SetMainSkillCount"));
        let import = sent.iter().position(|c| c.contains("ImportFromUrl"));
        assert!(
            edit.is_some() && import.is_some() && edit < import,
            "{sent:?}"
        );
    }

    #[test]
    fn startup_link_no_drops_it() {
        let (mut app, ctx) = fake_app();
        app = app.with_startup_link(Some("pob://pobbin/abc".into()));
        pump_until(&mut app, &ctx, "list", |a| a.ready && a.list.is_some());
        click_mark(&mut app, &ctx, "confirm_no");
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert!(app.confirm.is_none());
        assert!(app.startup_link.is_none());
        assert!(app.sent_full.iter().all(|c| !c.contains("ImportFromUrl")));
    }

    #[test]
    fn list_import_of_a_link_stays_on_the_list_until_new_build() {
        let (mut app, ctx) = started();
        for text in ["https://pobb.in/abc", "  POB://pobbin/abc "] {
            let before = app.sent_full.len();
            app.import = Some(text.into());
            frame(&mut app, &ctx, vec![]);
            click_mark(&mut app, &ctx, "list_import");
            assert_eq!(
                sent_since(&app, before),
                [format!(
                    "ImportFromUrl {{ url: {:?}, mode: NewBuild }}",
                    text.trim()
                )]
            );
            assert_eq!(app.screen, Screen::List);
            assert!(app.entering.is_none());
        }
        deliver(
            &mut app,
            requests_snapshot(&[(7, "Importing a build")], vec![]),
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.screen, Screen::List, "still waiting");
        deliver(
            &mut app,
            requests_snapshot(&[], vec![finished(7, None, false, true)]),
        );
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.screen, Screen::Build);
        // other text stays a share code
        let (mut app, ctx) = started();
        let before = app.sent_full.len();
        app.import = Some("eNrtvQ".into());
        frame(&mut app, &ctx, vec![]);
        click_mark(&mut app, &ctx, "list_import");
        assert!(sent_since(&app, before)[0].starts_with("ImportShareCode"));
    }

    #[test]
    fn banner_dismisses() {
        let (mut app, ctx) = started();
        let (tx, rx) = std::sync::mpsc::channel();
        app = app.with_release_check(rx);
        crate::ui::import::clear_marks();
        frame(&mut app, &ctx, vec![]);
        assert!(marked("release_dismiss").is_none());
        tx.send(Release {
            version: "9.9.9".into(),
            url: "https://github.com/cyprx/auto-pob/releases/tag/v9.9.9".into(),
        })
        .unwrap();
        click_mark(&mut app, &ctx, "release_link");
        let (opened, _, _) = outputs_within(&mut app, &ctx, 1);
        assert!(opened.is_empty());
        assert!(app.release.is_some());
        click_mark(&mut app, &ctx, "release_dismiss");
        assert!(app.release.is_none());
        crate::ui::import::clear_marks();
        frame(&mut app, &ctx, vec![]);
        assert!(
            marked("release_dismiss").is_none(),
            "hidden for the session"
        );
    }

    /// The Items tab's "Manage..." under its tree dropdown opens PoB's Manage Passive Trees
    /// window, whose controls are the Tree tab's spec controls
    #[test]
    fn items_manage_opens_the_tree_manager() {
        let (mut app, ctx) = tree_app();
        let mut view = crate::ui::items::tests::sample();
        view.specs = Some(crate::engine::views::DropList {
            labels: vec!["Main".into()],
            selected: 0,
            enabled: false,
        });
        app.on_event(Event::Snapshot {
            seq: 90,
            snapshot: Snapshot::Items(Arc::new(view)),
        });
        app.tab = BuildTab::Items;
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        click_item_key(&mut app, &ctx, "spec_manage");
        assert_eq!(app.tab, BuildTab::Items, "the tab stays");
        assert!(app.tree.spec_manager_open());
        // the window reaches its place over a few frames
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        assert!(
            app.tree.rects.map.contains_key("spec_manager_done"),
            "the window is not drawn"
        );
        // its New is the Tree tab's New tree, with the same dialog
        let pos = app.tree.rects.map["spec_manager_new"].center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ] {
            frame(&mut app, &ctx, events);
        }
        assert!(app.tree.has_dialog(), "no New tree dialog");
        // leaving the tab closes the window
        app.tab = BuildTab::Config;
        frame(&mut app, &ctx, vec![]);
        assert!(!app.tree.spec_manager_open());
    }

    /// "Add Mod" on the Config tab opens the Mod Browser there; leaving the tab ends it (R28)
    #[test]
    fn add_mod_opens_the_mod_browser_on_the_config_tab() {
        let (mut app, ctx) = config_app(sample_config());
        let before = app.sent_full.len();
        click(&mut app, &ctx, "mod_add:0");
        assert_eq!(
            sent_since(&app, before),
            ["OpenPopup(ModBrowser { group: 0 })"]
        );
        assert_eq!(app.popup_tab, Some(BuildTab::Config));
        app.on_event(Event::Snapshot {
            seq: app.last_sent,
            snapshot: Snapshot::Popup(Some(Arc::new(popup_view(
                crate::engine::PopupKind::ModBrowser { group: 0 },
            )))),
        });
        frame(&mut app, &ctx, vec![]);
        assert!(app.popup.is_open());
        let before = app.sent_full.len();
        app.tab = BuildTab::Items;
        frame(&mut app, &ctx, vec![]);
        assert!(sent_since(&app, before).contains(&"ClosePopup"));
    }

    // the shortcuts' tests (R25), with this module's helpers
    mod shortcut_tests {
        include!("app_shortcut_tests.rs");
    }
}
