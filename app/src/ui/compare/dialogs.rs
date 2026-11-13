//! The Compare tab's windows: PoB's "Import Comparison Build" sources (a saved build, a code, a
//! link) and its "Convert to Version" popup for the Tree view's version dropdowns.

use super::{CompareOutput, CompareUi, Dialog};
use crate::engine::{Command, CompareSource};
use crate::theme;
use crate::ui::name_box::{self, Filter};
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
            Dialog::Import { .. } => "Import Comparison Build".to_owned(),
            Dialog::Convert { label, .. } => format!("Convert to Version {label}"),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| match &mut dialog {
                Dialog::Saved { folder } => keep = self.saved(ui, folder, out),
                Dialog::Import {
                    text,
                    name,
                    sent,
                    error,
                    downloading,
                } => self.import(ui, text, name, sent, error, *downloading, out, &mut keep),
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
                        let r = crate::theme::list_row(ui, |ui| {
                            ui.add(
                                egui::Button::selectable(false, format!("\u{1f4c1} {name}"))
                                    .truncate(),
                            )
                        });
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
                            let r = crate::theme::list_row(ui, |ui| {
                                ui.add(
                                    egui::Button::selectable(false, entry.name.as_str()).truncate(),
                                )
                            });
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

    /// PoB's "Import Comparison Build" popup (CompareTab.lua:1462-1525): a code or a link, an
    /// optional name (100 bytes), Import, which Enter in the first box presses too. A link is
    /// downloaded as PoB's popup does, whether or not this build has a file; the comparison is
    /// named by the name box, else "Imported from <site>" or "Imported build". The popup stays
    /// until the import is answered; PoB's failure text shows in it, in red. Clears `keep` when
    /// it is cancelled.
    #[allow(clippy::too_many_arguments)]
    fn import(
        &mut self,
        ui: &mut egui::Ui,
        text: &mut String,
        name: &mut String,
        sent: &mut Option<Option<u64>>,
        error: &mut Option<String>,
        downloading: bool,
        out: &mut CompareOutput,
        keep: &mut bool,
    ) {
        ui.label("Paste a build code or URL to import as comparison:");
        let r = ui.add(egui::TextEdit::singleline(text).desired_width(450.0));
        crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &r, text);
        self.rects.note(&r, || "dialog_text".into());
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let (field, _) = name_box::edit(ui, name, Filter::Plain, name_box::NAME_LIMIT, |t| {
            t.hint_text("Name (optional)").desired_width(450.0)
        });
        self.rects.note(&field, || "dialog_name".into());
        // PoB's state line under the Name box: WARNING while downloading, NEGATIVE on a failure
        if downloading {
            // colorCodes.WARNING (Global.lua:21)
            let r = ui.colored_label(egui::Color32::from_rgb(0xFF, 0x99, 0x22), "Downloading...");
            self.rects.note(&r, || "dialog_downloading".into());
        } else if let Some(error) = error {
            let r = ui.colored_label(theme::NEGATIVE, error.as_str());
            self.rects.note(&r, || "dialog_error".into());
        }
        ui.horizontal(|ui| {
            let idle = sent.is_none() && !downloading;
            let go = ui.add_enabled(idle, theme::primary_button("Import"));
            self.rects.note(&go, || "dialog_load".into());
            // PoB's Import does nothing for an empty box
            if (go.clicked() || (enter && idle)) && !text.is_empty() {
                out.commands.push(Command::LoadCompare {
                    source: CompareSource::Import {
                        text: text.clone(),
                        name: (!name.is_empty()).then(|| name.clone()),
                    },
                });
                *sent = Some(None);
                *error = None;
            }
            if ui.button("Cancel").clicked() {
                *keep = false;
            }
        });
    }
}
