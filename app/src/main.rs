//! auto-pob: an egui front-end driving Path of Building's calc engine headlessly.
//!
//! Usage: `auto-pob [POB_DIR]`. `POB_DIR` defaults to the PoB1 install created by
//! rusty-path-of-building (`~/.local/share/RustyPathOfBuilding1` on Linux), so run the main
//! app once first.

use auto_pob::{engine::lab_engine::Engine, theme, ui::lab::LabApp};
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
            let engine = Engine::spawn(pob_dir, move || ctx.request_repaint());
            Ok(Box::new(LabApp::new(engine)))
        }),
    )
}
