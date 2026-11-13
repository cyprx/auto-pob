//! The folder list of PoB's Save As box (`FolderListControl`): the Builds tree one level at a
//! time, a path to walk back up, New Folder, PoB's two sorts and a delete for empty folders.
//! It only reads the engine's folder tree; every change goes out as a command.

use crate::engine::views::{BuildListView, FolderNode};
use crate::theme;
use crate::ui::import::mark;
use crate::ui::name_box::{self, Filter};
use eframe::egui;

/// How the folders are ordered (PoB's `saveAsSortMode`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FolderSort {
    #[default]
    Name,
    Edited,
}

impl FolderSort {
    fn label(self) -> &'static str {
        match self {
            FolderSort::Name => "Sort By Name",
            FolderSort::Edited => "Sort By Last Edited",
        }
    }
}

/// What the picker wants done
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutput {
    /// PoB's New Folder, in `folder`
    NewFolder { folder: String, name: String },
    /// Delete this empty folder (a path below Builds, "a/b/")
    DeleteFolder(String),
    /// PoB's popup text, for a folder that can't be deleted
    Notice(String),
}

#[derive(Debug, Default)]
pub struct FolderPicker {
    sort: FolderSort,
    selected: Option<String>,
    /// The New Folder name being typed
    naming: Option<String>,
    focus_name: bool,
    /// The empty folder that is about to be deleted, if the user says so
    deleting: Option<String>,
}

/// The folders directly in `folder`, in the chosen order
fn children<'a>(tree: &'a [FolderNode], folder: &str, sort: FolderSort) -> Vec<&'a FolderNode> {
    let mut out: Vec<&FolderNode> = tree
        .iter()
        .filter(|f| {
            f.path
                .strip_prefix(folder)
                .is_some_and(|rest| !rest.is_empty() && rest.matches('/').count() == 1)
        })
        .collect();
    out.sort_by(|a, b| match sort {
        FolderSort::Edited => b
            .modified
            .cmp(&a.modified)
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase())),
        FolderSort::Name => a.path.to_lowercase().cmp(&b.path.to_lowercase()),
    });
    out
}

fn last_segment(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

impl FolderPicker {
    /// Draws the picker over `folder` (the folder the build will be saved in).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        folder: &mut String,
        view: &BuildListView,
    ) -> Option<PickerOutput> {
        let mut out = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Folder:");
            let segments: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
            let mut go = None;
            let r = ui.link("Builds");
            mark(ui, "picker_crumb:0", &r);
            if r.clicked() {
                go = Some(String::new());
            }
            let mut target = String::new();
            for (n, segment) in segments.iter().enumerate() {
                ui.label("/");
                target.push_str(segment);
                target.push('/');
                let r = ui.link(*segment);
                mark(ui, &format!("picker_crumb:{}", n + 1), &r);
                if r.clicked() {
                    go = Some(target.clone());
                }
            }
            if let Some(go) = go {
                *folder = go;
                self.selected = None;
            }
        });
        ui.horizontal(|ui| {
            let new = ui.button("New Folder...");
            mark(ui, "picker_new", &new);
            if new.clicked() {
                self.naming = Some(String::new());
                self.focus_name = true;
            }
            let selected = self.selected.clone();
            let delete = ui.add_enabled(selected.is_some(), egui::Button::new("Delete folder"));
            mark(ui, "picker_delete", &delete);
            if delete.clicked()
                && let Some(path) = selected
            {
                out = self.ask_delete(&path, view);
            }
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("picker_sort")
                .selected_text(self.sort.label())
                .show_ui(ui, |ui| {
                    for sort in [FolderSort::Name, FolderSort::Edited] {
                        let item =
                            crate::widgets::dropdown::value(ui, &mut self.sort, sort, sort.label());
                        mark(ui, &format!("picker_sort:{sort:?}"), &item);
                    }
                });
            mark(ui, "picker_sort", &combo.response);
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            let sorts = [FolderSort::Name, FolderSort::Edited];
            let at = sorts.iter().position(|s| *s == self.sort).unwrap_or(0);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, sorts.len(), at)
            {
                self.sort = sorts[i];
            }
        });
        if let Some(text) = &mut self.naming {
            let mut create = false;
            let mut cancel = false;
            ui.horizontal(|ui| {
                let (edit, _) = name_box::edit(ui, text, Filter::Path, name_box::NAME_LIMIT, |t| {
                    t.hint_text("Folder name")
                });
                mark(ui, "picker_name", &edit);
                if self.focus_name {
                    edit.request_focus();
                    self.focus_name = false;
                }
                // PoB's Create asks for `%S` (Main.lua:1697)
                let ok = name_box::has_visible(text);
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let go = ui.add_enabled(ok, theme::primary_button("Create"));
                mark(ui, "picker_create", &go);
                create = (go.clicked() || enter) && ok;
                cancel = ui.button("Cancel").clicked();
            });
            if create {
                out = Some(PickerOutput::NewFolder {
                    folder: folder.clone(),
                    name: std::mem::take(text),
                });
                self.naming = None;
            } else if cancel {
                self.naming = None;
            }
        }
        if let Some(path) = self.deleting.clone() {
            let mut answer = None;
            ui.horizontal(|ui| {
                ui.label(format!("Delete empty folder {}?", last_segment(&path)));
                let yes = ui.add(theme::primary_button("Delete"));
                mark(ui, "picker_delete_yes", &yes);
                let no = ui.button("Cancel");
                answer = if yes.clicked() {
                    Some(true)
                } else {
                    no.clicked().then_some(false)
                };
            });
            if let Some(yes) = answer {
                self.deleting = None;
                if yes {
                    out = Some(PickerOutput::DeleteFolder(path));
                    self.selected = None;
                }
            }
        }
        let list = children(&view.tree, folder, self.sort);
        let mut enter = None;
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .id_salt("picker_list")
            .show(ui, |ui| {
                ui.set_min_width(420.0);
                if list.is_empty() {
                    ui.weak("No folders here.");
                }
                for f in &list {
                    let on = self.selected.as_deref() == Some(f.path.as_str());
                    let r = ui.selectable_label(on, format!("{}/", last_segment(&f.path)));
                    mark(ui, &format!("picker_row:{}", f.path), &r);
                    if r.clicked() {
                        self.selected = Some(f.path.clone());
                    }
                    if r.double_clicked() {
                        enter = Some(f.path.clone());
                    }
                }
            });
        if let Some(path) = enter {
            *folder = path;
            self.selected = None;
            self.deleting = None;
        }
        out
    }

    /// PoB's `OnSelDelete`: a folder with anything in it is refused with a message; an empty
    /// one goes after a question (PoB asks none here, this app always asks before deleting).
    fn ask_delete(&mut self, path: &str, view: &BuildListView) -> Option<PickerOutput> {
        let node = view.tree.iter().find(|f| f.path == path)?;
        if node.empty {
            self.deleting = Some(path.to_owned());
            None
        } else {
            Some(PickerOutput::Notice("The folder is not empty.".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::import::{clear_marks, marked};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    fn node(path: &str, modified: u64, empty: bool) -> FolderNode {
        FolderNode {
            path: path.into(),
            modified,
            empty,
        }
    }

    fn view() -> BuildListView {
        BuildListView {
            tree: vec![
                node("beta/", 30, false),
                node("alpha/", 10, true),
                node("alpha/deep/", 5, true),
                node("zeta/", 20, true),
            ],
            ..BuildListView::default()
        }
    }

    struct H {
        ctx: egui::Context,
        picker: FolderPicker,
        folder: String,
        view: BuildListView,
    }

    impl H {
        fn new() -> H {
            let mut h = H {
                ctx: egui::Context::default(),
                picker: FolderPicker::default(),
                folder: String::new(),
                view: view(),
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<PickerOutput> {
            clear_marks();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(1.0),
                events,
                ..Default::default()
            };
            let mut out = None;
            let (picker, folder, view) = (&mut self.picker, &mut self.folder, &self.view);
            self.ctx
                .clone()
                .run_ui(input, |ui| out = picker.show(ui, folder, view))
                .drop_without_applying_deltas();
            out
        }

        fn click_at(&mut self, pos: Pos2, times: usize) -> Option<PickerOutput> {
            let b = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut out = self.frame(vec![Event::PointerMoved(pos)]);
            for _ in 0..times {
                out = self.frame(vec![b(true)]).or(out);
                out = self.frame(vec![b(false)]).or(out);
            }
            self.frame(vec![]).or(out)
        }

        fn click(&mut self, key: &str) -> Option<PickerOutput> {
            self.frame(vec![]);
            let pos = marked(key)
                .unwrap_or_else(|| panic!("no widget {key}"))
                .center();
            self.click_at(pos, 1)
        }

        fn rows(&mut self) -> Vec<String> {
            self.frame(vec![]);
            let mut rows: Vec<(f32, String)> = ["alpha/", "beta/", "zeta/", "alpha/deep/"]
                .iter()
                .filter_map(|p| {
                    marked(&format!("picker_row:{p}")).map(|r| (r.min.y, (*p).to_owned()))
                })
                .collect();
            rows.sort_by(|a, b| a.0.total_cmp(&b.0));
            rows.into_iter().map(|(_, p)| p).collect()
        }
    }

    #[test]
    fn it_lists_one_level_and_walks_in_and_out() {
        let mut h = H::new();
        assert_eq!(h.rows(), ["alpha/", "beta/", "zeta/"]);
        // a double click goes in; only that folder's children show
        h.frame(vec![]);
        let pos = marked("picker_row:alpha/").unwrap().center();
        h.click_at(pos, 2);
        assert_eq!(h.folder, "alpha/");
        assert_eq!(h.rows(), ["alpha/deep/"]);
        // the path goes back
        h.click("picker_crumb:0");
        assert_eq!(h.folder, "");
    }

    #[test]
    fn it_sorts_by_name_or_last_edited() {
        let mut h = H::new();
        assert_eq!(h.rows(), ["alpha/", "beta/", "zeta/"]);
        h.click("picker_sort");
        h.frame(vec![]);
        h.click("picker_sort:Edited");
        assert_eq!(h.rows(), ["beta/", "zeta/", "alpha/"]);
        h.click("picker_sort");
        h.frame(vec![]);
        h.click("picker_sort:Name");
        assert_eq!(h.rows(), ["alpha/", "beta/", "zeta/"]);
    }

    #[test]
    fn new_folder_names_it_with_pobs_filter() {
        let mut h = H::new();
        h.folder = "alpha/".into();
        assert_eq!(h.click("picker_new"), None);
        h.click("picker_name");
        assert_eq!(h.frame(vec![Event::Text("n/e:w".into())]), None);
        assert_eq!(
            h.click("picker_create"),
            Some(PickerOutput::NewFolder {
                folder: "alpha/".into(),
                name: "new".into(),
            })
        );
        assert!(h.picker.naming.is_none());
    }

    /// PoB's New Folder box keeps non-ASCII text, counts 100 bytes, and Create asks for `%S`
    /// (Main.lua:1693-1718)
    #[test]
    fn new_folder_keeps_non_ascii_names_and_counts_bytes() {
        let mut h = H::new();
        h.folder = "alpha/".into();
        h.click("picker_new");
        h.click("picker_name");
        h.frame(vec![Event::Text("\u{65e5}".repeat(33))]);
        h.frame(vec![Event::Text("\u{65e5}".into())]);
        assert_eq!(
            h.picker.naming.as_deref().map(|t| t.chars().count()),
            Some(33)
        );
        assert_eq!(
            h.click("picker_create"),
            Some(PickerOutput::NewFolder {
                folder: "alpha/".into(),
                name: "\u{65e5}".repeat(33),
            })
        );
    }

    #[test]
    fn a_full_folder_is_refused_and_an_empty_one_asked_about() {
        let mut h = H::new();
        h.click("picker_row:beta/");
        assert_eq!(
            h.click("picker_delete"),
            Some(PickerOutput::Notice("The folder is not empty.".into()))
        );
        h.click("picker_row:zeta/");
        // asks, and Cancel deletes nothing
        assert_eq!(h.click("picker_delete"), None);
        assert!(h.picker.deleting.is_some());
        h.frame(vec![]);
        h.picker.deleting = None;
        assert_eq!(h.click("picker_delete"), None);
        assert_eq!(
            h.click("picker_delete_yes"),
            Some(PickerOutput::DeleteFolder("zeta/".into()))
        );
    }
}
