//! Proof of concept: an egui front-end driving Path of Building's calc engine headlessly.
//!
//! Usage: `auto-pob-lab [POB_DIR]`. `POB_DIR` defaults to the PoB1 install created by
//! rusty-path-of-building (`~/.local/share/RustyPathOfBuilding1` on Linux), so run the main
//! app once first.

mod app;
mod engine;
mod theme;
mod tree_view;

use directories::BaseDirs;
use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let pob_dir = std::env::args_os().nth(1).map_or_else(
        || {
            BaseDirs::new()
                .expect("home directory")
                .data_dir()
                .join("RustyPathOfBuilding1")
        },
        PathBuf::from,
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("auto-pob lab")
            .with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "auto-pob lab",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let engine = engine::Engine::spawn(pob_dir, move || ctx.request_repaint());
            Ok(Box::new(app::LabApp::new(engine)))
        }),
    )
}
