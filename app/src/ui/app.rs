//! The application: a screen state machine over the engine's events.

use std::sync::Arc;

use crate::engine::views::{BuildListView, BuildView, ConfigView, StatsView};
use crate::engine::{BackgroundTask, Command, Engine, Event, Snapshot, Tab};
use crate::game::Game;
use crate::pob_files::DownloadProgress;
use crate::theme;
use crate::ui::build::{self, BuildAction, BuildTab};
use crate::ui::build_list::{BuildListUi, ListScreenAction};
use crate::ui::calcs::{CalcsOutput, CalcsUi};
use crate::ui::config::ConfigUi;
use crate::ui::items::{ItemsOutput, ItemsUi};
use crate::ui::notes::NotesUi;
use crate::ui::skills::{SkillsOutput, SkillsUi};
use crate::ui::tree::{TreeOutput, TreeTab};
use crate::ui::unsaved::{Pending, PromptChoice, Resolution, needs_prompt, resolve};
use crate::ui::{sidebar, startup};
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    List,
    Build,
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
    /// The seq of the raw item text last sent, to keep its window open on an error
    text_seq: Option<u64>,
    sidebar: sidebar::SidebarState,
    notes: NotesUi,
    config_ui: ConfigUi,
    list_ui: BuildListUi,

    notices: Vec<String>,
    pending_save: Option<PendingSave>,
    save_as: Option<SaveAsDialog>,
    prompt: Option<Pending>,
    import: Option<String>,
    /// Actions waiting for the next frame (they need the egui context).
    ready_actions: Vec<Pending>,
    allow_close: bool,
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
            text_seq: None,
            sidebar: sidebar::SidebarState::default(),
            notes: NotesUi::default(),
            config_ui: ConfigUi::default(),
            list_ui: BuildListUi::default(),
            notices: Vec::new(),
            pending_save: None,
            save_as: None,
            prompt: None,
            import: None,
            ready_actions: Vec::new(),
            allow_close: false,
            #[cfg(test)]
            sent_log: Vec::new(),
            #[cfg(test)]
            sent_full: Vec::new(),
        }
    }

    fn restart(&mut self) {
        let spawn = std::mem::replace(&mut self.spawn, Box::new(|| unreachable!()));
        *self = AutoPobApp::new(spawn);
    }

    /// Sends a command whose snapshots the UI waits for.
    fn send(&mut self, command: Command) -> u64 {
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
        let is_notes = matches!(command, Command::SetNotes { .. });
        let seq = self.engine.send(command);
        self.last_sent = seq;
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
            Event::Ready => self.ready = true,
            Event::StartupFailed { message } => self.startup_error = Some(message),
            Event::Snapshot { seq, snapshot } => {
                self.last_snapshot = self.last_snapshot.max(seq);
                // The events are in order, so everything from the previous build has arrived
                // by the new build's first snapshot. Only now is the tree tab's state for the
                // old build (and any late power events) dropped.
                if self.entering.is_some_and(|e| seq >= e) {
                    self.entering = None;
                    self.tree.reset();
                    self.calcs.reset();
                    self.skills.reset();
                    self.items.reset();
                    self.sidebar = sidebar::SidebarState::default();
                    self.notes.reset();
                }
                match snapshot {
                    Snapshot::Build(view) => {
                        self.build = Some(view);
                        self.last_unsaved = self.current_unsaved();
                        if self.pending_save.as_ref().is_some_and(|p| seq >= p.seq) {
                            self.save_succeeded();
                        }
                    }
                    Snapshot::BuildList(view) => self.list = Some(view),
                    Snapshot::Stats(view) => {
                        self.stats = Some(view);
                    }
                    Snapshot::Config(view) => self.config = Some(view),
                    Snapshot::Calcs(_) => self.calcs.on_snapshot(&snapshot),
                    Snapshot::Skills(_)
                    | Snapshot::SocketGroupText(_)
                    | Snapshot::GroupTooltip(_)
                    | Snapshot::GemSearch(_)
                    | Snapshot::GemTooltip(_) => self.skills.on_snapshot(&snapshot),
                    Snapshot::Items(_)
                    | Snapshot::DisplayItem(_)
                    | Snapshot::ItemTooltip(_)
                    | Snapshot::ItemText(_) => self.items.on_snapshot_at(&snapshot, seq),
                    Snapshot::Breakdown(_) => self.calcs.on_snapshot(&snapshot),
                    // the popup window arrives with the crafting UI
                    Snapshot::Popup(_) | Snapshot::ControlTooltip(_) => {}
                    Snapshot::Notes(_) => self.notes.on_snapshot(&snapshot, seq),
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
            Event::Progress { percent, .. } => self.tree.on_progress(percent),
            Event::BackgroundFailed { message, .. } => {
                self.tree.power_failed();
                self.notices.push(message);
            }
        }
    }

    /// Shows a passive node on the tree tab.
    fn show_node(&mut self, id: u32, now: f64) {
        self.tab = BuildTab::Tree;
        self.tree.focus(id, now);
    }

    fn on_error(&mut self, seq: u64, message: String) {
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
        if let Some(command) = self.calcs.flush() {
            self.send(command);
        }
        if let Some(command) = self.skills.flush() {
            self.send(command);
        }
        for command in self.items.flush() {
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
            Pending::BackToList => {
                self.screen = Screen::List;
                // the heat map would go on calculating for a build nobody is looking at
                if let Some(stop) = self.tree.stop_heatmap() {
                    self.send(stop);
                }
                let folder = self.list_folder();
                self.send(Command::ListBuilds { folder });
            }
            Pending::Open(path) => self.enter_build(Command::OpenBuild { path }),
            Pending::New => self.enter_build(Command::NewBuild),
        }
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
        let name = self
            .build
            .as_ref()
            .map(|b| b.name.clone())
            .unwrap_or_default();
        self.save_as = Some(SaveAsDialog {
            folder: self.list_folder(),
            name,
            then,
            confirm_overwrite: false,
            waiting: false,
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
            name: dialog.name.trim().to_owned(),
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

    fn undo_redo(&mut self, redo: bool) {
        let tab = match self.tab {
            BuildTab::Config => Tab::Config,
            BuildTab::Tree => Tab::Tree,
            BuildTab::Skills => Tab::Skills,
            BuildTab::Items => Tab::Items,
            BuildTab::Calcs => Tab::Calcs,
            // the notes editor keeps its own history
            BuildTab::Notes => return,
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

        // closing the window with unsaved changes asks first
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.engine.is_alive()
            && self.current_unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.prompt = Some(Pending::Close);
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

        self.notices_panel(ui);
        match self.screen {
            Screen::List => self.list_screen(ui),
            Screen::Build => self.build_screen(ui),
        }
        self.dialogs(&ctx);
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

    fn list_screen(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        egui::CentralPanel::default().show(ui, |ui| match self.list.clone() {
            Some(view) => action = self.list_ui.show(ui, &view),
            None => {
                ui.spinner();
            }
        });
        match action {
            Some(ListScreenAction::OpenFolder(folder)) => {
                self.send(Command::ListBuilds { folder });
            }
            Some(ListScreenAction::OpenBuild(path)) => self.request(Pending::Open(path)),
            Some(ListScreenAction::New) => self.request(Pending::New),
            Some(ListScreenAction::Import) => self.import = Some(String::new()),
            None => {}
        }
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
        let modal = self.prompt.is_some()
            || self.save_as.is_some()
            || self.import.is_some()
            || self.tree.has_dialog()
            || self.skills.has_dialog()
            || self.items.has_dialog();
        // a focused button or row doesn't take Ctrl+Z from the tab
        let typing = ui.ctx().text_edit_focused();
        let mut action = None;
        if !modal {
            let command = egui::Modifiers::COMMAND;
            let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            ui.input_mut(|i| {
                if i.consume_key(command, egui::Key::S) {
                    action = Some(BuildAction::Save);
                }
                if !typing && self.tab.has_undo() {
                    // the Shift variant first: `consume_key` ignores extra Shift
                    if i.consume_key(shift, egui::Key::Z) || i.consume_key(command, egui::Key::Y) {
                        action = action.or(Some(BuildAction::Redo));
                    } else if i.consume_key(command, egui::Key::Z) {
                        action = action.or(Some(BuildAction::Undo));
                    }
                }
            });
        }

        egui::Panel::top("top_bar").show(ui, |ui| {
            let history = match self.tab {
                BuildTab::Tree => self.tree.history(),
                BuildTab::Skills => self.skills.history(),
                BuildTab::Items => self.items.history(),
                BuildTab::Calcs => self.calcs.history(),
                _ => (view.can_undo, view.can_redo),
            };
            action = build::top_bar(ui, &view, self.tab, history).or(action);
        });
        let behind = self.behind();
        self.skills.set_modal(modal);
        self.items.set_modal(modal);
        let mut sidebar_commands = Vec::new();
        egui::Panel::left("stats_sidebar")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                sidebar_commands =
                    sidebar::show(ui, self.stats.as_deref(), behind, &mut self.sidebar);
            });
        for command in sidebar_commands {
            self.send(command);
        }

        let mut config_commands = Vec::new();
        let mut tree_out = TreeOutput::default();
        let mut calcs_out = CalcsOutput::default();
        let mut skills_out = SkillsOutput::default();
        let mut items_out = ItemsOutput::default();
        let mut notes_command = None;
        let config = self.config.clone();
        egui::CentralPanel::default().show(ui, |ui| {
            build::tab_strip(ui, &mut self.tab);
            ui.separator();
            match self.tab {
                BuildTab::Tree => tree_out = self.tree.show(ui, behind),
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
            self.send(command);
        }
        if let Some(command) = notes_command {
            self.send(command);
        } else if self.tab != BuildTab::Notes {
            // leaving the tab
            self.flush_notes();
        }
        if self.tab != BuildTab::Tree {
            self.tree.close_dialog();
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
        // questions are answered with snapshots but never leave the app waiting
        for query in tree_out.queries {
            self.engine.send(query);
        }

        for command in calcs_out.commands {
            self.send(command);
        }
        // only the latest question matters; the others are about cells the pointer has left
        if let Some(query) = calcs_out.queries.pop() {
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
            None => {}
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.prompt_dialog(ctx);
        self.save_as_dialog(ctx);
        self.import_dialog(ctx);
    }

    fn prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.prompt.clone() else {
            return;
        };
        let mut choice = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let name = self
                    .build
                    .as_ref()
                    .map_or("this build", |b| b.name.as_str());
                ui.label(format!("Save changes to {name}?"));
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("Save")).clicked() {
                        choice = Some(PromptChoice::Save);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(PromptChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(PromptChoice::Cancel);
                    }
                });
            });
        let Some(choice) = choice else {
            return;
        };
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
        let mut cancel = false;
        egui::Window::new("Save as")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_enabled_ui(!dialog.waiting, |ui| {
                    ui.label(format!(
                        "Folder: Builds/{}",
                        dialog.folder.trim_end_matches('/')
                    ));
                    let name = ui
                        .add(egui::TextEdit::singleline(&mut dialog.name).hint_text("Build name"));
                    if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
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
                            let valid = !dialog.name.trim().is_empty();
                            if ui
                                .add_enabled(valid, theme::primary_button("Save"))
                                .clicked()
                            {
                                submit = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    }
                });
            });
        if cancel {
            self.save_as = None;
        } else if overwrite {
            self.send_save_as(true);
        } else if submit && !dialog.name.trim().is_empty() {
            self.send_save_as(false);
        }
    }

    fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(text) = &mut self.import else {
            return;
        };
        let mut import = false;
        let mut cancel = false;
        egui::Window::new("Import code")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Paste a PoB share code:");
                ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(4)
                        .desired_width(400.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!text.trim().is_empty(), theme::primary_button("Import"))
                        .clicked()
                    {
                        import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if import {
            let code = std::mem::take(text);
            self.import = None;
            self.enter_build(Command::ImportShareCode { code });
        } else if cancel {
            self.import = None;
        }
    }
}

impl eframe::App for AutoPobApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
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
                _ => Ok(Affected::default()),
            }
        }

        fn recalculate(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn build_view(&mut self) -> anyhow::Result<BuildView> {
            Ok(BuildView {
                name: "Fake build".into(),
                path: self.path.clone(),
                class: "Witch".into(),
                ascendancy: Some("Necromancer".into()),
                level: 90,
                unsaved: self.unsaved,
                can_undo: true,
                can_redo: false,
            })
        }

        fn build_loaded(&mut self) -> anyhow::Result<bool> {
            Ok(true)
        }

        fn build_list_view(&mut self) -> anyhow::Result<BuildListView> {
            let entry = |name: &str, is_folder| BuildListEntry {
                name: name.into(),
                is_folder,
                path: format!("/b/{}{name}.xml", self.folder).into(),
                class: Some("Witch".into()),
                ascendancy: None,
                level: Some(90),
            };
            Ok(BuildListView {
                folder: self.folder.clone(),
                entries: vec![entry("sub", true), entry("one", false), entry("bad", false)],
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
    fn ctrl_shift_z_redoes_and_ctrl_z_undoes() {
        let (mut app, ctx) = open_new_build();
        let before = app.last_sent;
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
        assert_eq!(app.last_sent, before + 1);
        // the sent command was Redo: check via the engine's record
        assert_eq!(app.sent_log.last().map(String::as_str), Some("Redo"));
        frame(
            &mut app,
            &ctx,
            key_with(egui::Key::Z, egui::Modifiers::COMMAND),
        );
        assert_eq!(app.sent_log.last().map(String::as_str), Some("Undo"));
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
                },
                ConfigSet {
                    id: 2,
                    title: "Mapping".into(),
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
        // an invalid keystroke is kept in the field but never sent
        let before = app.sent_full.len();
        type_text(&mut app, &ctx, "x");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        assert_eq!(app.config_ui.draft("overrideCount"), Some("72x"));
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

        // removing asks first
        let before = app.sent_full.len();
        click(&mut app, &ctx, "mod_remove:0");
        assert_eq!(sent_since(&app, before), Vec::<&str>::new());
        click(&mut app, &ctx, "mod_remove_yes:0");
        assert_eq!(
            sent_since(&app, before),
            ["RemoveCustomModGroup { group: 0 }"]
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
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
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
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
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
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
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
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
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
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
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
        frame(
            &mut app,
            &ctx,
            key_with(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
        );
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
}
