//! The application: a screen state machine over the engine's events.

use std::sync::Arc;

use crate::engine::views::{BuildListView, BuildView, ConfigView, StatsView};
use crate::engine::{Command, Engine, Event, Snapshot, Tab};
use crate::game::Game;
use crate::pob_files::DownloadProgress;
use crate::theme;
use crate::ui::build::{self, BuildAction, BuildTab};
use crate::ui::build_list::{BuildListUi, ListScreenAction};
use crate::ui::config::ConfigUi;
use crate::ui::lab::LabTab;
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
    /// Seqs of recent Lab commands, so only their errors reset the Lab tab.
    lab_seqs: Vec<u64>,
    tab: BuildTab,
    lab: LabTab,
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
            lab_seqs: Vec::new(),
            tab: BuildTab::default(),
            lab: LabTab::default(),
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
        let seq = self.engine.send(command);
        self.last_sent = seq;
        seq
    }

    /// True while the engine hasn't caught up with what we sent.
    fn behind(&self) -> bool {
        self.last_sent > self.last_snapshot
    }

    fn current_unsaved(&self) -> bool {
        self.screen == Screen::Build
            && self.entering.is_none()
            && self.build.as_ref().is_some_and(|b| b.unsaved)
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
                match snapshot {
                    Snapshot::Build(view) => {
                        self.last_unsaved = view.unsaved;
                        self.build = Some(view);
                        if self.pending_save.as_ref().is_some_and(|p| seq >= p.seq) {
                            self.save_succeeded();
                        }
                    }
                    Snapshot::BuildList(view) => self.list = Some(view),
                    Snapshot::Stats(view) => {
                        if self.stats.as_deref() != Some(&*view) {
                            self.lab.stats_changed();
                        }
                        self.stats = Some(view);
                    }
                    Snapshot::Config(view) => self.config = Some(view),
                }
                if self.entering.is_some_and(|e| seq >= e) {
                    self.entering = None;
                }
            }
            Event::Error { seq, message } => self.on_error(seq, message),
            Event::Lab(event) => self.lab.on_event(event),
        }
    }

    fn on_error(&mut self, seq: u64, message: String) {
        if let Some(at) = self.lab_seqs.iter().position(|s| *s == seq) {
            self.lab_seqs.remove(at);
            self.lab.failed();
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

    fn run_action(&mut self, ctx: &egui::Context, action: Pending) {
        match action {
            Pending::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::BackToList => {
                self.screen = Screen::List;
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
        self.lab.build_changed();
    }

    fn save(&mut self, then: Option<Pending>) {
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
        if !self.tab.has_undo() {
            return;
        }
        let tab = Tab::Config;
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
        let modal = self.prompt.is_some() || self.save_as.is_some() || self.import.is_some();
        let typing = ui.ctx().egui_wants_keyboard_input();
        let mut action = None;
        if !modal {
            let command = egui::Modifiers::COMMAND;
            let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            ui.input_mut(|i| {
                if i.consume_key(command, egui::Key::S) {
                    action = Some(BuildAction::Save);
                }
                if !typing {
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
            action = build::top_bar(ui, &view, self.tab).or(action);
        });
        let behind = self.behind();
        egui::Panel::left("stats_sidebar")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| sidebar::show(ui, self.stats.as_deref(), behind));

        let mut lab_commands = Vec::new();
        let mut config_commands = Vec::new();
        let config = self.config.clone();
        egui::CentralPanel::default().show(ui, |ui| {
            build::tab_strip(ui, &mut self.tab);
            ui.separator();
            match self.tab {
                BuildTab::Config => match &config {
                    Some(config) => {
                        config_commands = self.config_ui.show(ui, config, behind);
                    }
                    None => {
                        ui.spinner();
                    }
                },
                BuildTab::Lab => lab_commands = self.lab.show(ui, behind),
            }
        });
        for command in config_commands {
            self.send(command);
        }
        for command in lab_commands {
            // lab commands answer with events, not snapshots, so they don't count as "behind"
            let seq = self.engine.send(Command::Lab(command));
            self.lab_seqs.push(seq);
            if self.lab_seqs.len() > 32 {
                self.lab_seqs.remove(0);
            }
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
            Resolution::Proceed => self.ready_actions.push(pending),
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
    use crate::engine::{Affected, Backend, ConfigValue, LabCommand, LabEvent, NodeScore, Tree};
    use std::collections::HashSet;
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
    }

    impl Backend for Fake {
        fn apply(&mut self, command: Command, send: &dyn Fn(Event)) -> anyhow::Result<Affected> {
            let all = Affected {
                build: true,
                list: true,
                stats: true,
                config: true,
                recalc: false,
            };
            match command {
                Command::NewBuild => {
                    self.path = None;
                    self.unsaved = false;
                    Ok(all)
                }
                Command::OpenBuild { path } => {
                    if path.to_string_lossy().contains("bad") {
                        anyhow::bail!("couldn't open that build");
                    }
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
                Command::Lab(LabCommand::Tree) => {
                    send(Event::Lab(LabEvent::Tree(Tree {
                        nodes: vec![],
                        edges: vec![],
                        sheets: vec![],
                        allocated: HashSet::new(),
                    })));
                    Ok(Affected::default())
                }
                Command::Lab(LabCommand::NodePower { .. }) => {
                    send(Event::Lab(LabEvent::Progress { done: 1, total: 2 }));
                    send(Event::Lab(LabEvent::NodePower(vec![NodeScore {
                        id: 1,
                        name: "Node".into(),
                        kind: "Notable".into(),
                        distance: 2,
                        gain: 1.0,
                        path_gain: 2.0,
                        per_point: 1.0,
                    }])));
                    Ok(Affected::default())
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
            };
            Ok(StatsView {
                stats: vec![row("Life")],
                minion: vec![row("Minion Life")],
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
        app.tab = BuildTab::Lab;
        pump_until(&mut app, &ctx, "lab tree", |a| a.lab_has_tree());
        // ranking streams progress then results, and never leaves the sidebar spinning
        app.engine.send(Command::Lab(LabCommand::NodePower {
            metric: "Life",
            max_distance: 3,
        }));
        for _ in 0..50 {
            frame(&mut app, &ctx, vec![]);
            std::thread::sleep(Duration::from_millis(2));
        }
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

    impl AutoPobApp {
        fn lab_has_tree(&self) -> bool {
            self.lab.has_tree()
        }
    }
}
