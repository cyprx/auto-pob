//! The first-run screen (R15): shown before the engine starts when there is no
//! `auto-pob.toml`, no `AUTO_POB_USER_DIR`, and the classic app is installed with its folder.
//! The answer is written to the settings file; nothing else on disk is touched.

use super::app::AutoPobApp;
use crate::settings::{
    AppSettings, FirstRun, FirstRunChoice, expand_folder, prepare_folder, settle_first_run,
};
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};

/// The question and the user's draft answer.
pub struct FirstRunScreen {
    classic: PathBuf,
    other: bool,
    text: String,
    /// What `~` means
    home: Option<PathBuf>,
    /// Why the last answer was refused
    error: Option<String>,
}

impl FirstRunScreen {
    pub fn new(classic: PathBuf) -> Self {
        Self {
            classic,
            other: false,
            text: String::new(),
            home: directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()),
            error: None,
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
                    match prepare_folder(&self.classic) {
                        Ok(()) => choice = Some(FirstRunChoice::Classic),
                        Err(e) => self.error = Some(format!("{e:#}")),
                    }
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
                            // confirming is what makes the folder
                            let dir = expand_folder(&self.text, self.home.as_deref());
                            match prepare_folder(&dir) {
                                Ok(()) => choice = Some(FirstRunChoice::Folder(dir)),
                                Err(e) => self.error = Some(format!("{e:#}")),
                            }
                        }
                    });
                    ui.label("The folder is made if it isn't there.");
                }
                if let Some(error) = &self.error {
                    let label = ui.colored_label(ui.visuals().error_fg_color, error);
                    crate::ui::import::mark(ui, "first_run_error", &label);
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
            Self::with(PathBuf::from("/classic/userdata"), None)
        }

        fn with(classic: PathBuf, home: Option<PathBuf>) -> Self {
            let mut screen = FirstRunScreen::new(classic);
            screen.home = home;
            let mut h = Self {
                screen,
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
        let dir = std::env::temp_dir().join(format!("auto-pob-fr-classic-{}", std::process::id()));
        let mut h = Harness::with(dir.clone(), None);
        assert_eq!(h.choice, None);
        h.click("first_run_classic");
        assert_eq!(h.choice, Some(FirstRunChoice::Classic));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn type_path(h: &mut Harness, text: &str) {
        h.click("first_run_path");
        h.frame(vec![Event::Text(text.into())]);
        h.click("first_run_go");
    }

    #[test]
    fn a_folder_that_cannot_be_used_stays_on_the_screen_with_the_reason() {
        let dir = std::env::temp_dir().join(format!("auto-pob-fr-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file");
        std::fs::write(&file, "x").unwrap();
        let mut h = Harness::with(PathBuf::from("/classic"), Some(dir.join("home")));
        h.click("first_run_other");
        type_path(&mut h, file.to_str().unwrap());
        assert_eq!(h.choice, None);
        assert!(marked("first_run_error").is_some());
        assert!(marked("first_run_path").is_some(), "the question stays");
        assert!(file.is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_folder_is_made_only_on_confirm_and_tilde_means_home() {
        let dir = std::env::temp_dir().join(format!("auto-pob-fr-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut h = Harness::with(PathBuf::from("/classic"), Some(dir.clone()));
        h.click("first_run_other");
        h.click("first_run_path");
        h.frame(vec![Event::Text("~/builds/new".into())]);
        assert!(!dir.join("builds").exists(), "typing makes nothing");
        h.click("first_run_go");
        assert_eq!(
            h.choice,
            Some(FirstRunChoice::Folder(dir.join("builds").join("new")))
        );
        assert!(dir.join("builds").join("new").is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn another_folder_needs_a_path() {
        let mut h = Harness::new();
        assert!(marked("first_run_path").is_none());
        h.click("first_run_other");
        h.click("first_run_go");
        assert_eq!(h.choice, None, "an empty path chooses nothing");
        let dir = std::env::temp_dir().join(format!("auto-pob-fr-else-{}", std::process::id()));
        type_path(&mut h, dir.to_str().unwrap());
        assert_eq!(h.choice, Some(FirstRunChoice::Folder(dir.clone())));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn answering_writes_only_the_given_file() {
        let dir = crate::test_dir::TestDir::reserved("firstrun");
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

    /// Every path below `dir`, relative to it, sorted
    fn files(dir: &Path) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                out.push(path.strip_prefix(base).unwrap().display().to_string());
                if path.is_dir() {
                    walk(base, &path, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn each_choice_leaves_only_its_files() {
        for other in [false, true] {
            let root = crate::test_dir::TestDir::new(&format!("fr-files-{other}"));
            // the classic app installed, with its user folder and a build
            let data = root.join("data");
            let classic = crate::game::Game::Poe1.default_user_dir(&data);
            std::fs::create_dir_all(classic.join("Path of Building/Builds")).unwrap();
            std::fs::write(classic.parent().unwrap().join("Launch.lua"), "").unwrap();
            std::fs::write(classic.join("Path of Building/Builds/a.xml"), "a").unwrap();
            let config = root.join("cfg").join("auto-pob.toml");
            assert_eq!(
                crate::settings::first_run(None, &data, false),
                Some(FirstRun::Ask {
                    classic: classic.clone()
                })
            );
            let before = files(&root);

            let mut h = Harness::with(classic.clone(), Some(root.to_path_buf()));
            let mut made = vec!["cfg".to_string(), "cfg/auto-pob.toml".to_string()];
            let folder = if other {
                h.click("first_run_other");
                type_path(&mut h, "~/mine/builds");
                made.extend(["mine".into(), "mine/builds".into()]);
                root.join("mine/builds")
            } else {
                h.click("first_run_classic");
                classic.clone()
            };
            let choice = h.choice.clone().expect("an answer");
            let (settings, notice) = answer(&classic, choice, &config);
            assert_eq!(notice, None);
            assert_eq!(
                settings,
                AppSettings {
                    user_dir: Some(folder),
                    ..AppSettings::default()
                }
            );
            assert_eq!(crate::settings::load(&config).unwrap().unwrap(), settings);
            // the answer and the folder it names; no probe or temporary file stays, and the
            // classic folder is as it was
            let mut want = before.clone();
            want.extend(made);
            want.sort();
            assert_eq!(files(&root), want, "other folder: {other}");
            assert_eq!(
                std::fs::read(classic.join("Path of Building/Builds/a.xml")).unwrap(),
                b"a"
            );
            // the next start asks nothing
            assert_eq!(
                crate::settings::first_run(Some(&settings), &data, false),
                None
            );
        }
    }
}
