//! The startup screen: download progress, then the error with Retry if it failed.

use crate::pob_files::DownloadProgress;
use crate::theme;
use eframe::egui;

pub enum StartupAction {
    Retry,
}

pub fn show(
    ui: &mut egui::Ui,
    pob_version: &str,
    progress: Option<DownloadProgress>,
    error: Option<&str>,
) -> Option<StartupAction> {
    let mut action = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            match error {
                Some(message) => {
                    theme::heading(ui, "Path of Building couldn't start");
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, message);
                    ui.add_space(8.0);
                    if ui.add(theme::primary_button("Retry")).clicked() {
                        action = Some(StartupAction::Retry);
                    }
                }
                None => {
                    theme::heading(ui, &format!("Preparing PoB {pob_version}…"));
                    ui.add_space(8.0);
                    ui.spinner();
                    match progress {
                        Some(DownloadProgress::Downloading { done, total }) => {
                            let mb = |n: u64| n as f64 / (1024.0 * 1024.0);
                            match total {
                                Some(total) if total > 0 => {
                                    ui.add(
                                        egui::ProgressBar::new(done as f32 / total as f32)
                                            .desired_width(300.0)
                                            .text(format!(
                                                "Downloading {:.1} / {:.1} MB",
                                                mb(done),
                                                mb(total)
                                            )),
                                    );
                                }
                                _ => {
                                    ui.label(format!("Downloading… {:.1} MB", mb(done)));
                                }
                            }
                        }
                        Some(DownloadProgress::Extracting) => {
                            ui.label("Extracting…");
                        }
                        None => {}
                    }
                }
            }
        });
    });
    action
}
