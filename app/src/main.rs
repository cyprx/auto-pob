//! auto-pob: an egui front-end driving Path of Building's calc engine headlessly.
//!
//! Path of Building is downloaded on first run into the data folder. Set `AUTO_POB_POB_DIR` to
//! use an existing PoB folder instead, and `AUTO_POB_USER_DIR` to keep builds and settings
//! somewhere other than the default.

use auto_pob::{engine::Engine, game::Game, theme, ui::AutoPobApp};
use directories::BaseDirs;
use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result {
    env_logger::init();
    let game = Game::Poe1;
    let data_dir = BaseDirs::new()
        .expect("home directory")
        .data_dir()
        .to_path_buf();
    // PoB changes the working directory, so a relative override must be made absolute first
    let user_dir = std::env::var_os("AUTO_POB_USER_DIR").map_or_else(
        || game.default_user_dir(&data_dir),
        |dir| std::path::absolute(&dir).unwrap_or_else(|_| PathBuf::from(dir)),
    );

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
            let ctx = cc.egui_ctx.clone();
            let spawn = move || {
                let ctx = ctx.clone();
                Engine::spawn(game, data_dir.clone(), user_dir.clone(), move || {
                    ctx.request_repaint()
                })
            };
            Ok(Box::new(AutoPobApp::new(Box::new(spawn))))
        }),
    )
}
