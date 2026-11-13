//! The Compare tab's windows: PoB's "Import Comparison Build" sources (a saved build, a code, a
//! link) and its "Convert to Version" popup for the Tree view's version dropdowns.

use super::{CompareOutput, CompareUi, Dialog};
use crate::engine::{Command, CompareSource, ImportMode};
use crate::theme;
use eframe::egui;

/// PoB's warning in its "Convert to Version" popup (TreeTab.lua:651-652)
const CONVERT_WARNING: &str = "Warning: some or all of the passives may be de-allocated due to \
                               changes in the tree.\n\nConvert will replace your current tree.\n\
                               Copy + Convert will backup your current tree.";

/// The folders directly in `folder` ("" or "a/b/"), from the build list's folder tree
fn subfolders<'a>(tree: &'a [crate::engine::views::FolderNode], folder: &str) -> Vec<&'a str> {
    let mut out: Vec<&str> = tree
        .iter()
        .filter_map(|f| f.path.strip_prefix(folder))
        .filter(|rest| !rest.is_empty() && rest.matches('/').count() == 1)
        .map(|rest| rest.trim_end_matches('/'))
        .collect();
    out.sort_by_key(|name| name.to_lowercase());
    out
}

impl CompareUi {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, out: &mut CompareOutput) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        // Esc closes it once its box is left, as every PoB popup (PopupDialog.lua:72-77)
        let window = egui::Id::unique("compare_dialog");
        let escaped = crate::widgets::escape::escape(ctx, window);
        let mut keep = true;
        let title = match &dialog {
            Dialog::Saved { .. } => "Compare with a saved build".to_owned(),
            Dialog::Code(_) => "Compare with a build code".to_owned(),
            Dialog::Link(_) => "Compare with a build link".to_owned(),
            Dialog::Convert { label, .. } => format!("Convert to Version {label}"),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| match &mut dialog {
                Dialog::Saved { folder } => keep = self.saved(ui, folder, out),
                Dialog::Code(text) => keep = self.text(ui, text, true, out),
                Dialog::Link(text) => keep = self.text(ui, text, false, out),
                Dialog::Convert {
                    theirs,
                    index,
                    label,
                } => {
                    ui.label(CONVERT_WARNING);
                    ui.horizontal(|ui| {
                        for (keep_copy, text, key) in [
                            (false, "Convert", "convert"),
                            (true, "Copy + Convert", "convert_copy"),
                        ] {
                            let r = ui.add_enabled(!self.busy, egui::Button::new(text));
                            self.rects.note(&r, || key.into());
                            if r.clicked() {
                                out.commands.push(Command::ConvertCompareTree {
                                    theirs: *theirs,
                                    index: *index,
                                    label: label.clone(),
                                    keep_copy,
                                });
                                keep = false;
                            }
                        }
                        let r = ui.button("Cancel");
                        self.rects.note(&r, || "convert_cancel".into());
                        if r.clicked() {
                            keep = false;
                        }
                    });
                }
            });
        crate::widgets::escape::note(ctx, window);
        keep &= !escaped;
        // a window opened while this one drew (none can) would win
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    /// The saved-build picker: the Builds folder tree from the build list, the builds of
    /// `folder` from the engine (`BrowseBuilds`). Returns whether it stays open.
    fn saved(&mut self, ui: &mut egui::Ui, folder: &mut String, out: &mut CompareOutput) -> bool {
        let mut go = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Folder:");
            let r = ui.link("Builds");
            self.rects.note(&r, || "crumb:0".into());
            if r.clicked() {
                go = Some(String::new());
            }
            let mut target = String::new();
            for (n, segment) in folder.split('/').filter(|s| !s.is_empty()).enumerate() {
                ui.label("/");
                target.push_str(segment);
                target.push('/');
                let r = ui.link(segment);
                self.rects.note(&r, || format!("crumb:{}", n + 1));
                if r.clicked() {
                    go = Some(target.clone());
                }
            }
        });
        let mut load = None;
        // the folders, then the builds or a line saying why there are none; only the rows
        // in view are laid out, so a folder of thousands of builds costs a screenful a frame
        let list = self.list.clone();
        let folders = list
            .as_deref()
            .map(|l| subfolders(&l.tree, folder))
            .unwrap_or_default();
        let browse = self.browse.clone().filter(|b| b.folder == *folder);
        let builds = browse.as_ref().map_or(0, |b| b.entries.len());
        let total = folders.len() + builds.max(1);
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .id_salt("compare_saved_builds")
            .max_height(320.0)
            .show_rows(ui, row_height, total, |ui, shown| {
                ui.set_min_width(360.0);
                for row in shown {
                    if let Some(name) = folders.get(row) {
                        let r = ui.add(
                            egui::Button::selectable(false, format!("\u{1f4c1} {name}")).truncate(),
                        );
                        self.rects.note(&r, || format!("folder:{name}"));
                        if r.clicked() {
                            go = Some(format!("{folder}{name}/"));
                        }
                        continue;
                    }
                    let i = row - folders.len();
                    match &browse {
                        Some(view) if i < view.entries.len() => {
                            let entry = &view.entries[i];
                            let r = ui.add(
                                egui::Button::selectable(false, entry.name.as_str()).truncate(),
                            );
                            self.rects.note(&r, || format!("saved:{i}"));
                            if r.clicked() {
                                load = Some(Command::LoadCompare {
                                    source: CompareSource::File(entry.path.clone()),
                                });
                            }
                        }
                        Some(view) => match &view.error {
                            Some(error) => {
                                let r = ui.colored_label(
                                    theme::NEGATIVE,
                                    format!("Couldn't list this folder: {error}"),
                                );
                                self.rects.note(&r, || "browse_error".into());
                            }
                            None => {
                                ui.weak("No builds here.");
                            }
                        },
                        None => {
                            ui.spinner();
                        }
                    }
                }
            });
        if let Some(go) = go {
            out.queries
                .push(Command::BrowseBuilds { folder: go.clone() });
            *folder = go;
        }
        if ui.button("Cancel").clicked() {
            return false;
        }
        match load {
            Some(command) => {
                out.commands.push(command);
                false
            }
            None => true,
        }
    }

    /// A code or a link box with Load; returns whether it stays open.
    fn text(
        &mut self,
        ui: &mut egui::Ui,
        text: &mut String,
        is_code: bool,
        out: &mut CompareOutput,
    ) -> bool {
        if is_code {
            ui.label("Paste a PoB share code:");
            let r = ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(4)
                    .desired_width(400.0),
            );
            self.rects.note(&r, || "dialog_text".into());
        } else {
            ui.label("Paste a build link:");
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(400.0));
            self.rects.note(&r, || "dialog_text".into());
        }
        let mut keep = true;
        ui.horizontal(|ui| {
            let go = ui.add_enabled(!text.trim().is_empty(), theme::primary_button("Load"));
            self.rects.note(&go, || "dialog_load".into());
            if go.clicked() {
                let text = text.trim().to_owned();
                out.commands.push(if is_code {
                    Command::LoadCompare {
                        source: CompareSource::Code(text),
                    }
                } else {
                    Command::ImportFromUrl {
                        url: text,
                        mode: ImportMode::Comparison,
                    }
                });
                keep = false;
            }
            if ui.button("Cancel").clicked() {
                keep = false;
            }
        });
        keep
    }
}
