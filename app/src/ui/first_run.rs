//! The first-run screen (R15): shown before the engine starts when there is no
//! `auto-pob.toml`, no `AUTO_POB_USER_DIR`, and the classic app's folder exists. The answer is
//! written to the settings file; nothing else on disk is touched.

use super::app::AutoPobApp;
use crate::settings::{AppSettings, FirstRun, FirstRunChoice, settle_first_run};
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

/// The question and the user's draft answer.
pub struct FirstRunScreen {
    classic: PathBuf,
    other: bool,
    text: String,
}

impl FirstRunScreen {
    pub fn new(classic: PathBuf) -> Self {
        Self {
            classic,
            other: false,
            text: String::new(),
        }
    }

    /// Draws the screen; returns the answer once the user gives it.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<FirstRunChoice> {
        let mut choice = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.2);
                theme::heading(ui, "Welcome to auto-pob");
                ui.add_space(8.0);
                ui.label("Which folder should hold your builds and settings?");
                ui.label(
                    "The classic Path of Building app already has one. Using it shares your \
                     builds, and Settings.xml, with that app.",
                );
                ui.add_space(12.0);
                let classic = ui.add(theme::primary_button(&format!(
                    "Use the classic app's builds ({})",
                    self.classic.display()
                )));
                crate::ui::import::mark(ui, "first_run_classic", &classic);
                if classic.clicked() {
                    choice = Some(FirstRunChoice::Classic);
                }
                ui.add_space(6.0);
                let other = ui.button("Use another folder");
                crate::ui::import::mark(ui, "first_run_other", &other);
                if other.clicked() {
                    self.other = true;
                }
                if self.other {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        // the box is centred by the layout; its width keeps the row readable
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.text)
                                .hint_text("/path/to/a/folder")
                                .desired_width(360.0),
                        );
                        crate::ui::import::mark(ui, "first_run_path", &edit);
                        let enter =
                            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let usable = !self.text.trim().is_empty();
                        let go = ui.add_enabled(usable, egui::Button::new("Use this folder"));
                        crate::ui::import::mark(ui, "first_run_go", &go);
                        if (go.clicked() || enter) && usable {
                            let dir = PathBuf::from(self.text.trim());
                            choice = Some(FirstRunChoice::Folder(
                                std::path::absolute(&dir).unwrap_or(dir),
                            ));
                        }
                    });
                    ui.label("The folder is made if it isn't there.");
                }
                ui.add_space(12.0);
                ui.label("Options changes it later.");
            });
        });
        choice
    }
}

/// Writes the answer. A failed write is told to the user and the answer is used for this run.
pub fn answer(
    classic: &Path,
    choice: FirstRunChoice,
    config: &Path,
) -> (AppSettings, Option<String>) {
    let run = FirstRun::Ask {
        classic: classic.to_path_buf(),
    };
    match settle_first_run(&run, Some(choice.clone()), config) {
        Ok(settings) => (settings, None),
        Err(e) => {
            let settings = AppSettings {
                user_dir: Some(match choice {
                    FirstRunChoice::Classic => classic.to_path_buf(),
                    FirstRunChoice::Folder(dir) => dir,
                }),
                ..AppSettings::default()
            };
            (
                settings,
                Some(format!(
                    "Couldn't save your choice to {}: {e:#}",
                    config.display()
                )),
            )
        }
    }
}

type Start = Box<dyn FnOnce(&egui::Context, AppSettings, Option<String>) -> AutoPobApp>;

/// Asks the first-run question, then becomes the app (the engine starts only then).
pub struct FirstRunGate {
    screen: FirstRunScreen,
    classic: PathBuf,
    config: PathBuf,
    start: Option<Start>,
    app: Option<AutoPobApp>,
}

impl FirstRunGate {
    pub fn new(classic: PathBuf, config: PathBuf, start: Start) -> Self {
        Self {
            screen: FirstRunScreen::new(classic.clone()),
            classic,
            config,
            start: Some(start),
            app: None,
        }
    }
}

impl eframe::App for FirstRunGate {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(app) = &mut self.app {
            app.show(ui);
            return;
        }
        if let Some(choice) = self.screen.show(ui)
            && let Some(start) = self.start.take()
        {
            let (settings, notice) = answer(&self.classic, choice, &self.config);
            let mut app = start(ui.ctx(), settings, notice);
            app.show(ui);
            self.app = Some(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::import::{clear_marks, marked};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    struct Harness {
        screen: FirstRunScreen,
        ctx: egui::Context,
        choice: Option<FirstRunChoice>,
    }

    impl Harness {
        fn new() -> Self {
            let mut h = Self {
                screen: FirstRunScreen::new(PathBuf::from("/classic/userdata")),
                ctx: egui::Context::default(),
                choice: None,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            clear_marks();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                events,
                ..Default::default()
            };
            let mut got = None;
            let out = self
                .ctx
                .run_ui(input, |ui| got = got.take().or(self.screen.show(ui)));
            out.drop_without_applying_deltas();
            self.choice = self.choice.take().or(got);
        }

        fn click(&mut self, key: &str) {
            let pos = marked(key).unwrap_or_else(|| panic!("no {key}")).center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                self.frame(events);
            }
        }
    }

    #[test]
    fn classic_button_chooses_the_classic_folder() {
        let mut h = Harness::new();
        assert_eq!(h.choice, None);
        h.click("first_run_classic");
        assert_eq!(h.choice, Some(FirstRunChoice::Classic));
    }

    #[test]
    fn another_folder_needs_a_path() {
        let mut h = Harness::new();
        assert!(marked("first_run_path").is_none());
        h.click("first_run_other");
        h.click("first_run_go");
        assert_eq!(h.choice, None, "an empty path chooses nothing");
        h.click("first_run_path");
        h.frame(vec![Event::Text("/somewhere/else".into())]);
        h.click("first_run_go");
        assert_eq!(
            h.choice,
            Some(FirstRunChoice::Folder(PathBuf::from("/somewhere/else")))
        );
    }

    #[test]
    fn answering_writes_only_the_given_file() {
        let dir = std::env::temp_dir().join(format!("auto-pob-firstrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("auto-pob.toml");
        let (settings, notice) = answer(
            Path::new("/classic/userdata"),
            FirstRunChoice::Classic,
            &config,
        );
        assert_eq!(notice, None);
        assert_eq!(settings.user_dir, Some(PathBuf::from("/classic/userdata")));
        assert_eq!(crate::settings::load(&config).unwrap().unwrap(), settings);
        // the answer names a folder; nothing is made there
        assert!(!Path::new("/classic").exists());
        // a file in the way of the config folder: the answer still holds, with a notice
        let blocked = dir.join("auto-pob.toml").join("inside.toml");
        let (settings, notice) = answer(
            Path::new("/classic/userdata"),
            FirstRunChoice::Folder("/x".into()),
            &blocked,
        );
        assert_eq!(settings.user_dir, Some(PathBuf::from("/x")));
        assert!(notice.unwrap().contains("Couldn't save"));
    }
}
