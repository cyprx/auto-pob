//! auto-pob: an egui front-end driving Path of Building's calc engine headlessly.
//!
//! Path of Building is downloaded on first run into the data folder. Set `AUTO_POB_POB_DIR` to
//! use an existing PoB folder instead, and `AUTO_POB_USER_DIR` to keep builds and settings
//! somewhere other than the default. auto-pob's own settings live in `auto-pob.toml` (see
//! `AUTO_POB_CONFIG`); the user folder is `AUTO_POB_USER_DIR`, else the file's `user_dir`, else
//! the default.

use auto_pob::settings::{self, AppSettings, FirstRun};
use auto_pob::ui::first_run::FirstRunGate;
use auto_pob::{
    engine::Engine, game::Game, pob_link_from_args, theme, ui::AutoPobApp, update_notice,
};
use directories::BaseDirs;
use eframe::egui;
use std::path::PathBuf;

/// Everything the app needs that is settled before its window opens.
struct Setup {
    game: Game,
    data_dir: PathBuf,
    config: PathBuf,
    /// Why the settings file may not be written in this run
    locked: Option<String>,
}

impl Setup {
    /// The app over `settings`, with the engine on the folder they (and the environment) name.
    fn app(
        &self,
        ctx: &egui::Context,
        settings: Option<AppSettings>,
        notices: Vec<String>,
    ) -> AutoPobApp {
        let (game, data_dir) = (self.game, self.data_dir.clone());
        // PoB changes the working directory, so a relative override must be made absolute first
        let env_dir = std::env::var_os("AUTO_POB_USER_DIR")
            .map(|dir| std::path::absolute(&dir).unwrap_or_else(|_| PathBuf::from(dir)));
        let user_dir = settings::resolve_user_dir(
            env_dir,
            settings.as_ref(),
            game.default_user_dir(&data_dir),
        );
        let user_dir = std::path::absolute(&user_dir).unwrap_or(user_dir);
        let release_check = update_notice::enabled(settings.as_ref());
        let ctx = ctx.clone();
        let spawn_ctx = ctx.clone();
        let spawn = move || {
            let ctx = spawn_ctx.clone();
            Engine::spawn(game, data_dir.clone(), user_dir.clone(), move || {
                ctx.request_repaint()
            })
        };
        let mut app = AutoPobApp::new(Box::new(spawn))
            .with_settings(settings.unwrap_or_default(), self.config.clone())
            .with_locked_settings(self.locked.clone())
            .with_startup_link(pob_link_from_args(std::env::args()));
        for notice in notices {
            app = app.with_notice(notice);
        }
        if release_check {
            app = app.with_release_check(update_notice::spawn_check(ctx.clone()));
        }
        // the Help window's button fetches PoB's missing help files (only on that click)
        let pob_dir = auto_pob::pob_files::pob_dir(game, &self.data_dir);
        app.with_help_download(Box::new(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let (dir, ctx) = (pob_dir.clone(), ctx.clone());
            std::thread::spawn(move || {
                let result = auto_pob::pob_files::download_help_files(game, &dir)
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
                ctx.request_repaint();
            });
            rx
        }))
    }
}

fn main() -> eframe::Result {
    env_logger::init();
    let data_dir = BaseDirs::new()
        .expect("home directory")
        .data_dir()
        .to_path_buf();
    let config = settings::config_path();
    let mut notices = Vec::new();
    let mut locked = None;
    let settings = match settings::load_checked(&config) {
        settings::Loaded::Absent => None,
        settings::Loaded::Settings(settings, warnings) => {
            if !warnings.is_empty() {
                notices.push(format!(
                    "Some settings in {} were ignored: {}",
                    config.display(),
                    warnings.join("; ")
                ));
            }
            Some(settings)
        }
        // not TOML at all: kept as a backup, never overwritten
        settings::Loaded::Invalid(error) => {
            notices.push(match settings::back_up_corrupt(&config) {
                Ok(backup) => format!(
                    "{error}. It was moved to {} and the defaults are used.",
                    backup.display()
                ),
                Err(e) => format!("{error}. It could not be moved aside either: {e:#}"),
            });
            None
        }
        // can't be read: it is left exactly as it is, and not written over in this run
        settings::Loaded::Unreadable(error) => {
            notices.push(format!(
                "{error}. The defaults are used and Options won't be saved this run."
            ));
            locked = Some(error);
            Some(AppSettings::default())
        }
    };
    let env_user_dir = std::env::var_os("AUTO_POB_USER_DIR").is_some();
    let first_run = settings::first_run(settings.as_ref(), &data_dir, env_user_dir);
    let setup = Setup {
        game: Game::Poe1,
        data_dir,
        config,
        locked,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("auto-pob")
            .with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "auto-pob",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            if let Some(FirstRun::Ask { classic }) = first_run {
                // the engine starts after the answer
                let config = setup.config.clone();
                let start = Box::new(
                    move |ctx: &egui::Context, settings: AppSettings, notice: Option<String>| {
                        let mut notices = notices;
                        notices.extend(notice);
                        setup.app(ctx, Some(settings), notices)
                    },
                );
                return Ok(Box::new(FirstRunGate::new(classic, config, start)));
            }
            // no file and no folder to ask about: the defaults, and nothing is written
            Ok(Box::new(setup.app(&cc.egui_ctx, settings, notices)))
        }),
    )
}
