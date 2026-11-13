//! The build list: PoB's own order and filter, with its New, New Folder, Open, Copy, Rename and
//! Delete, Ctrl+X/Ctrl+V and dragging to move, and the sort and search boxes (R16, R25).

use std::path::PathBuf;

use crate::engine::commands::DeleteKind;
use crate::engine::views::{BuildListEntry, BuildListView, BuildSort};
use crate::theme;
use crate::ui::import::mark;
use crate::ui::name_box::{self, Filter};
use crate::widgets::{ListAction, ListColumn, list_with};
use eframe::egui::{self, Key, PointerButton};

/// How long the search box waits after the last key before the filter is sent.
const FILTER_DELAY: f64 = 0.2;
/// PoB's name boxes take 100 characters and strip these (`\/:*?"<>|` and control characters).
const SEARCH_LIMIT: usize = 100;
/// The pointer has to travel this far before a press on a row is a drag.
const DRAG_DISTANCE: f32 = 6.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListScreenAction {
    /// Show this Builds sub-folder ("" is the root).
    OpenFolder(String),
    OpenBuild(PathBuf),
    New,
    Import,
    Options,
    NewFolder(String),
    Rename {
        path: PathBuf,
        name: String,
    },
    Copy {
        path: PathBuf,
        name: String,
    },
    Delete {
        path: PathBuf,
        kind: DeleteKind,
    },
    Move {
        path: PathBuf,
        folder: String,
    },
    CopyTo {
        path: PathBuf,
        folder: String,
    },
    SetSort(BuildSort),
    SetFilter(String),
    /// PoB's About button (Main.lua:209-211)
    About,
}

/// A question the list is waiting on.
#[derive(Debug, Clone, PartialEq)]
enum Dialog {
    /// PoB's Rename or Copy name box
    Name {
        copy: bool,
        path: PathBuf,
        is_folder: bool,
        original: String,
        text: String,
        focus: bool,
        /// typed into: PoB's Save waits for it (BuildListControl.lua:93-115)
        edited: bool,
    },
    NewFolder {
        text: String,
        focus: bool,
    },
    Delete {
        path: PathBuf,
        kind: DeleteKind,
        prompt: String,
    },
}

/// The row menu, opened on a right click.
#[derive(Debug, Clone)]
struct Menu {
    entry: BuildListEntry,
    pos: egui::Pos2,
    /// The "Move to…" list is unfolded
    move_open: bool,
    /// Opened this frame: the click that opened it must not close it
    fresh: bool,
}

/// What Ctrl+X or Ctrl+C marked
#[derive(Debug, Clone, PartialEq)]
struct Mark {
    entry: BuildListEntry,
    /// Ctrl+C (the entry is copied), not Ctrl+X
    copy: bool,
}

/// The folder (below Builds, with a closing slash) an entry stands for when it is a folder
fn own_path(e: &BuildListEntry) -> String {
    format!("{}{}/", e.folder, e.name)
}

/// A press on a row that may become a drag.
#[derive(Debug, Clone)]
struct Drag {
    path: PathBuf,
    origin: egui::Pos2,
}

pub struct BuildListUi {
    id: egui::Id,
    selected: Option<usize>,
    selected_path: Option<PathBuf>,
    folder: String,
    /// The search box's text, and when it was last typed in (None: nothing waits to be sent)
    filter_text: String,
    filter_edit: Option<f64>,
    /// The entry Ctrl+X marked, and the folder it was marked in
    cut: Option<Mark>,
    /// The mark a Move was sent for, kept until the engine says whether it worked
    spent: Option<Mark>,
    /// A Ctrl+V press was seen (as an event or a key), so its release is not a second paste
    paste_seen: bool,
    dialog: Option<Dialog>,
    menu: Option<Menu>,
    drag: Option<Drag>,
    /// Folders visited before this one and after it, for Mouse4 and Mouse5
    back: Vec<String>,
    forward: Vec<String>,
    /// The folder a Mouse4/Mouse5 asked for, so that its arrival isn't new history
    navigating: Option<String>,
    /// Where each visible row was drawn this frame
    row_rects: Vec<(usize, egui::Rect)>,
    search_id: egui::Id,
    menu_id: egui::Id,
    dialog_id: egui::Id,
    name_id: egui::Id,
}

impl Default for BuildListUi {
    fn default() -> Self {
        Self {
            id: egui::Id::unique("build_list"),
            selected: None,
            selected_path: None,
            folder: String::new(),
            filter_text: String::new(),
            filter_edit: None,
            cut: None,
            spent: None,
            paste_seen: false,
            dialog: None,
            menu: None,
            drag: None,
            back: Vec::new(),
            forward: Vec::new(),
            navigating: None,
            row_rects: Vec::new(),
            search_id: egui::Id::unique("build_search"),
            menu_id: egui::Id::unique("build_row_menu"),
            dialog_id: egui::Id::unique("build_list_dialog"),
            name_id: egui::Id::unique("build_list_name"),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Row<'a> {
    Up,
    Entry(&'a BuildListEntry),
}

/// The parent of a Builds sub-folder ("a/b/" -> "a/"; "a/" -> "").
pub fn parent_folder(folder: &str) -> String {
    let trimmed = folder.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => trimmed[..=i].to_owned(),
        None => String::new(),
    }
}

/// `..` first, then PoB's order (its sort mode and filter are applied by the engine).
fn ordered(view: &BuildListView) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    if !view.folder.is_empty() {
        rows.push(Row::Up);
    }
    rows.extend(view.entries.iter().map(Row::Entry));
    rows
}

/// PoB's four sorts, with its labels.
const SORTS: [(BuildSort, &str); 4] = [
    (BuildSort::Name, "Sort by Name"),
    (BuildSort::Class, "Sort by Class"),
    (BuildSort::Edited, "Sort by Last Edited"),
    (BuildSort::Level, "Sort by Level"),
];

fn sort_label(sort: BuildSort) -> &'static str {
    SORTS
        .iter()
        .find(|(s, _)| *s == sort)
        .map_or("Sort by Name", |(_, l)| l)
}

/// What PoB's search box keeps.
fn clean_search(text: &mut String) {
    text.retain(|c| !c.is_control() && c != '(' && c != ')');
    truncate_chars(text, SEARCH_LIMIT);
}

fn truncate_chars(text: &mut String, limit: usize) {
    if let Some((i, _)) = text.char_indices().nth(limit) {
        text.truncate(i);
    }
}

/// The folders an entry can be moved to or copied to: every folder of the Builds tree except
/// the one it is in and, for a folder, itself and what is inside it.
fn move_targets(view: &BuildListView, entry: &BuildListEntry) -> Vec<(String, String)> {
    let inside = entry.is_folder.then(|| own_path(entry));
    let mut folders = vec![String::new()];
    folders.extend(view.tree.iter().map(|f| f.path.clone()));
    folders
        .into_iter()
        .filter(|f| *f != entry.folder)
        .filter(|f| {
            inside
                .as_ref()
                .is_none_or(|own| !f.starts_with(own.as_str()))
        })
        .map(|f| {
            let label = format!("Builds/{f}");
            (f, label)
        })
        .collect()
}

fn dialog_for(entry: &BuildListEntry, copy: bool) -> Dialog {
    Dialog::Name {
        copy,
        path: entry.path.clone(),
        is_folder: entry.is_folder,
        original: entry.name.clone(),
        text: entry.name.clone(),
        focus: true,
        edited: false,
    }
}

fn delete_dialog(entry: &BuildListEntry) -> Dialog {
    let prompt = if entry.delete_prompt.is_empty() {
        // PoB deletes an empty folder without asking; this app still asks
        format!("Delete empty folder {}?", entry.name)
    } else {
        entry.delete_prompt.clone()
    };
    Dialog::Delete {
        path: entry.path.clone(),
        kind: entry.delete_kind(),
        prompt,
    }
}

impl BuildListUi {
    /// The search box's text
    #[cfg(test)]
    pub(crate) fn filter_for_test(&self) -> &str {
        &self.filter_text
    }

    /// `blocked`: a question of the app's own is open, so the list's keys and buttons wait.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        view: &BuildListView,
        blocked: bool,
    ) -> Option<ListScreenAction> {
        let mut action = None;
        let now = ui.input(|i| i.time);
        self.track_folder(view);

        action = self.top_rows(ui, view, blocked, now).or(action);
        ui.separator();

        let rows = ordered(view);
        // the selection stays on its entry when the list refreshes
        if let Some(path) = &self.selected_path
            && !matches!(self.selected.and_then(|s| rows.get(s)),
                Some(Row::Entry(e)) if &e.path == path)
            && let Some(i) = rows
                .iter()
                .position(|r| matches!(r, Row::Entry(e) if &e.path == path))
        {
            self.selected = Some(i);
        }
        if let Some(s) = self.selected
            && s >= rows.len()
        {
            self.selected = rows.len().checked_sub(1);
        }

        let modal = blocked || self.dialog.is_some();
        if !modal {
            action = self.keys(ui, view, &rows).or(action);
        }

        if rows.is_empty() {
            ui.label(if view.filter.is_empty() {
                "No builds here yet. Use New or Import code."
            } else {
                "No build matches the search."
            });
        } else {
            action = self.table(ui, view, &rows).or(action);
        }
        self.selected_path = match self.selected.and_then(|s| rows.get(s)) {
            Some(Row::Entry(e)) => Some(e.path.clone()),
            _ => None,
        };

        if !modal {
            action = self.pointer(ui, view, &rows).or(action);
        } else {
            self.drag = None;
        }
        action = self.menu(ui.ctx(), view).or(action);
        action = self.dialog(ui.ctx()).or(action);
        // only the paste's move can be given its mark back; a drag or menu move is not it
        if let Some(ListScreenAction::Move { path, .. }) = &action
            && self.spent.as_ref().map(|m| &m.entry.path) != Some(path)
        {
            self.spent = None;
        }
        // a folder asked for any other way than Mouse4/5 is not part of the history walk
        if let Some(ListScreenAction::OpenFolder(f)) = &action
            && self.navigating.as_deref() != Some(f.as_str())
        {
            self.navigating = None;
        }
        action
    }

    /// Keeps the Mouse4/Mouse5 history and the selection in step with the folder shown.
    fn track_folder(&mut self, view: &BuildListView) {
        if self.folder == view.folder {
            return;
        }
        let from = std::mem::replace(&mut self.folder, view.folder.clone());
        if self.navigating.take().as_deref() != Some(view.folder.as_str()) {
            self.back.push(from);
            self.forward.clear();
        }
        self.selected = None;
        self.selected_path = None;
        self.menu = None;
    }

    fn top_rows(
        &mut self,
        ui: &mut egui::Ui,
        view: &BuildListView,
        blocked: bool,
        now: f64,
    ) -> Option<ListScreenAction> {
        let mut action = None;
        let mut path_action = None;
        // the path: every segment goes back to that folder
        ui.horizontal(|ui| {
            let segments: Vec<&str> = view.folder.split('/').filter(|s| !s.is_empty()).collect();
            let mut target = String::new();
            let crumb = |ui: &mut egui::Ui, key: &str, text: &str, target: &str, last: bool| {
                if last {
                    theme::heading(ui, text);
                    None
                } else {
                    let r = ui.link(text);
                    mark(ui, key, &r);
                    r.clicked()
                        .then(|| ListScreenAction::OpenFolder(target.to_owned()))
                }
            };
            path_action = crumb(ui, "crumb:0", "Builds", "", segments.is_empty());
            for (n, segment) in segments.iter().enumerate() {
                ui.label("/");
                target.push_str(segment);
                target.push('/');
                let clicked = crumb(
                    ui,
                    &format!("crumb:{}", n + 1),
                    segment,
                    &target,
                    n + 1 == segments.len(),
                );
                path_action = clicked.or(path_action.take());
            }
        });
        action = path_action.or(action);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!blocked && self.dialog.is_none(), |ui| {
                let new = ui.add(theme::primary_button("New"));
                mark(ui, "list_new", &new);
                if new.clicked() {
                    action = Some(ListScreenAction::New);
                }
                let folder = ui.button("New Folder");
                mark(ui, "list_new_folder", &folder);
                if folder.clicked() {
                    self.dialog = Some(Dialog::NewFolder {
                        text: String::new(),
                        focus: true,
                    });
                }
                let import = ui.button("Import code");
                mark(ui, "list_import", &import);
                if import.clicked() {
                    action = Some(ListScreenAction::Import);
                }
                if ui.button("Options").clicked() {
                    action = Some(ListScreenAction::Options);
                }
                let about = ui.button("About");
                mark(ui, "list_about", &about);
                if about.clicked() {
                    action = Some(ListScreenAction::About);
                }
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt("build_sort")
                    .selected_text(sort_label(view.sort))
                    .show_ui(ui, |ui| {
                        for (sort, label) in SORTS {
                            let item = crate::widgets::dropdown::item(ui, view.sort == sort, label);
                            mark(ui, &format!("sort:{}", sort.id()), &item);
                            if item.clicked() && sort != view.sort {
                                action = Some(ListScreenAction::SetSort(sort));
                            }
                        }
                    });
                mark(ui, "sort_combo", &combo.response);
                // the wheel and the arrows step it (DropDownControl.lua:476-516)
                let at = SORTS.iter().position(|(s, _)| *s == view.sort).unwrap_or(0);
                if let Some(i) =
                    crate::widgets::scroll_step::combo_step(ui, &combo.response, SORTS.len(), at)
                {
                    action = Some(ListScreenAction::SetSort(SORTS[i].0));
                }
            });
        });
        action = self.search_box(ui, view, now).or(action);
        action
    }

    /// The search box; the text goes to PoB 200 ms after the last key.
    fn search_box(
        &mut self,
        ui: &mut egui::Ui,
        view: &BuildListView,
        now: f64,
    ) -> Option<ListScreenAction> {
        // the engine's text replaces ours only when nothing of ours is on its way
        if self.filter_edit.is_none()
            && self.filter_text != view.filter
            && !ui.memory(|m| m.has_focus(self.search_id))
        {
            self.filter_text.clone_from(&view.filter);
        }
        let mut clear = false;
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.filter_text)
                    .id(self.search_id)
                    .hint_text("(e.g. class:assassin myfilename)")
                    .desired_width(480.0),
            );
            mark(ui, "search", &edit);
            if edit.changed() {
                clean_search(&mut self.filter_text);
                self.filter_edit = Some(now);
            }
            if !self.filter_text.is_empty() {
                let x = ui.small_button("x");
                mark(ui, "search_clear", &x);
                clear = x.clicked();
            }
        });
        if clear {
            self.filter_text.clear();
            self.filter_edit = Some(now - FILTER_DELAY);
        }
        let edited = self.filter_edit?;
        let wait = edited + FILTER_DELAY - now;
        if wait > 0.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return None;
        }
        self.filter_edit = None;
        (self.filter_text != view.filter)
            .then(|| ListScreenAction::SetFilter(self.filter_text.clone()))
    }

    /// The path Ctrl+X or Ctrl+C marked
    #[cfg(test)]
    pub(crate) fn marked_path(&self) -> Option<&std::path::Path> {
        self.cut.as_ref().map(|m| m.entry.path.as_path())
    }

    #[cfg(test)]
    pub(crate) fn mark_for_test(&mut self, entry: BuildListEntry) {
        self.cut = Some(Mark { entry, copy: false });
    }

    #[cfg(test)]
    pub(crate) fn spend_for_test(&mut self) {
        self.spent = self.cut.take();
    }

    /// The engine refused the move the last paste sent: the mark comes back
    pub fn move_refused(&mut self) {
        if self.cut.is_none() {
            self.cut = self.spent.take();
        }
    }

    fn selected_entry<'a>(&self, rows: &[Row<'a>]) -> Option<&'a BuildListEntry> {
        match self.selected.and_then(|s| rows.get(s)) {
            Some(Row::Entry(e)) => Some(*e),
            _ => None,
        }
    }

    /// Ctrl+X, Ctrl+V, F2, Delete, Ctrl+N and the mouse's back and forward buttons.
    fn keys(
        &mut self,
        ui: &mut egui::Ui,
        view: &BuildListView,
        rows: &[Row<'_>],
    ) -> Option<ListScreenAction> {
        let mut action = None;
        let typing = ui.ctx().text_edit_focused();
        let list_focused = ui.memory(|m| m.has_focus(self.id));
        let (new, back, forward) = ui.input_mut(|i| {
            (
                // a text box keeps its Ctrl+N
                !typing && i.consume_key(egui::Modifiers::COMMAND, Key::N),
                i.pointer.button_pressed(PointerButton::Extra1),
                i.pointer.button_pressed(PointerButton::Extra2),
            )
        });
        if new {
            return Some(ListScreenAction::New);
        }
        if back && let Some(prev) = self.back.pop() {
            self.forward.push(view.folder.clone());
            self.navigating = Some(prev.clone());
            action = Some(ListScreenAction::OpenFolder(prev));
        } else if forward && let Some(next) = self.forward.pop() {
            self.back.push(view.folder.clone());
            self.navigating = Some(next.clone());
            action = Some(ListScreenAction::OpenFolder(next));
        }
        if !list_focused || typing {
            return action;
        }
        // Up, Down, Home and End move the selection (ListControl.lua:375-382)
        crate::ui::shortcuts::keep_arrows(ui.ctx(), self.id);
        if let Some(to) =
            ui.input_mut(|i| crate::ui::shortcuts::list_key(i, rows.len(), self.selected))
        {
            self.selected = Some(to);
        }
        // egui-winit turns Ctrl+X and Ctrl+C into events, and Ctrl+V into one only when the
        // clipboard holds text; with nothing on it the press is dropped and only the release
        // arrives as a key. So a paste is the event, the key press, or a release whose press
        // was not seen.
        let command = egui::Modifiers::COMMAND;
        let (cut, copy, paste_down, paste_up, f2, delete) = ui.input_mut(|i| {
            let cut = i.events.iter().any(|e| matches!(e, egui::Event::Cut))
                || i.consume_key(command, Key::X);
            let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                || i.consume_key(command, Key::C);
            let paste_down = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
                || i.consume_key(command, Key::V);
            let paste_up = i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: Key::V, pressed: false, modifiers, .. }
                    if modifiers.command)
            });
            (
                cut,
                copy,
                paste_down,
                paste_up,
                i.consume_key(egui::Modifiers::NONE, Key::F2),
                // PoB's list deletes on Delete and Backspace (ListControl.lua:392)
                i.consume_key(egui::Modifiers::NONE, Key::Delete)
                    || i.consume_key(egui::Modifiers::NONE, Key::Backspace),
            )
        });
        let paste = if paste_down {
            self.paste_seen = true;
            true
        } else if paste_up {
            !std::mem::take(&mut self.paste_seen)
        } else {
            false
        };
        let entry = self.selected_entry(rows);
        if (cut || copy)
            && let Some(e) = entry
        {
            self.cut = Some(Mark {
                entry: e.clone(),
                copy: copy && !cut,
            });
        }
        if paste && let Some(mark) = self.cut.clone() {
            let from = &mark.entry.folder;
            if mark.copy && *from == view.folder {
                // in its own folder PoB asks for a name
                self.cut = None;
                self.dialog = Some(dialog_for(&mark.entry, true));
            } else if mark.copy {
                // PoB clears the mark after every paste (BuildList.lua:153)
                self.cut = None;
                return Some(ListScreenAction::CopyTo {
                    path: mark.entry.path,
                    folder: view.folder.clone(),
                });
            } else if *from != view.folder {
                // the mark waits for the engine's answer
                self.spent = self.cut.take();
                return Some(ListScreenAction::Move {
                    path: mark.entry.path,
                    folder: view.folder.clone(),
                });
            } else {
                // PoB clears it in the folder it was cut in, too (BuildList.lua:167)
                self.cut = None;
            }
        }
        if f2 && let Some(e) = entry {
            self.dialog = Some(dialog_for(e, false));
        }
        if delete && let Some(e) = entry {
            self.dialog = Some(delete_dialog(e));
        }
        action
    }

    fn table(
        &mut self,
        ui: &mut egui::Ui,
        view: &BuildListView,
        rows: &[Row<'_>],
    ) -> Option<ListScreenAction> {
        // a search lists matches from every folder, so each says where it is
        let searching = !view.filter.is_empty();
        let mut columns = vec![
            ListColumn {
                title: "Name",
                width: 360.0,
            },
            ListColumn {
                title: "Class",
                width: 220.0,
            },
            ListColumn {
                title: "Level",
                width: 60.0,
            },
        ];
        if searching {
            columns.push(ListColumn {
                title: "Folder",
                width: 200.0,
            });
        }
        let total: f32 = columns.iter().map(|c| c.width).sum();
        let cut = self.cut.as_ref().map(|m| m.entry.path.clone());
        let mut rects = Vec::new();
        let result = list_with(
            ui,
            self.id,
            &columns,
            rows.len(),
            |i, row| {
                let mut first = true;
                let mut cell = |row: &mut egui_extras::TableRow, f: &dyn Fn(&mut egui::Ui)| {
                    let (rect, _) = row.col(|ui| f(ui));
                    if first {
                        first = false;
                        rects.push((
                            i,
                            egui::Rect::from_min_max(
                                rect.min,
                                egui::pos2(rect.min.x + total, rect.max.y),
                            ),
                        ));
                    }
                };
                match &rows[i] {
                    Row::Up => {
                        cell(row, &|ui| {
                            ui.label("..");
                        });
                        cell(row, &|_| {});
                        cell(row, &|_| {});
                        if searching {
                            cell(row, &|_| {});
                        }
                    }
                    Row::Entry(e) => {
                        let is_cut = cut.as_ref() == Some(&e.path);
                        cell(row, &|ui| {
                            let name = if e.is_folder {
                                format!("{}/", e.name)
                            } else {
                                e.name.clone()
                            };
                            let text = egui::RichText::new(name);
                            ui.label(if is_cut { text.weak() } else { text });
                        });
                        cell(row, &|ui| {
                            if !e.is_folder {
                                let class = e.ascendancy.as_deref().or(e.class.as_deref());
                                ui.label(class.unwrap_or(""));
                            }
                        });
                        cell(row, &|ui| {
                            if let Some(level) = e.level.filter(|_| !e.is_folder) {
                                ui.label(level.to_string());
                            }
                        });
                        if searching {
                            cell(row, &|ui| {
                                let r = ui.label(format!("Builds/{}", e.folder));
                                mark(ui, &format!("folder_cell:{i}"), &r);
                            });
                        }
                    }
                }
            },
            &mut self.selected,
            false,
        );
        self.row_rects = rects;
        if let Some(ListAction::Activate(i)) = result {
            return match rows.get(i) {
                Some(Row::Up) => Some(ListScreenAction::OpenFolder(parent_folder(&view.folder))),
                Some(Row::Entry(e)) if e.is_folder => {
                    Some(ListScreenAction::OpenFolder(own_path(e)))
                }
                Some(Row::Entry(e)) => Some(ListScreenAction::OpenBuild(e.path.clone())),
                None => None,
            };
        }
        None
    }

    fn row_at(&self, pos: egui::Pos2) -> Option<usize> {
        self.row_rects
            .iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(i, _)| *i)
    }

    /// Right click opens the row menu; dragging a row onto a folder or `..` moves it.
    fn pointer(
        &mut self,
        ui: &mut egui::Ui,
        view: &BuildListView,
        rows: &[Row<'_>],
    ) -> Option<ListScreenAction> {
        let (pos, pressed, released, right_click) = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.button_released(PointerButton::Primary),
                i.pointer.button_clicked(PointerButton::Secondary),
            )
        });
        let pos = pos?;
        let row = self.row_at(pos);
        if right_click
            && let Some(i) = row
            && let Row::Entry(e) = &rows[i]
        {
            self.selected = Some(i);
            ui.memory_mut(|m| m.request_focus(self.id));
            self.menu = Some(Menu {
                entry: (*e).clone(),
                pos,
                move_open: false,
                fresh: true,
            });
        }
        if pressed {
            self.drag = row.and_then(|i| match &rows[i] {
                Row::Entry(e) => Some(Drag {
                    path: e.path.clone(),
                    origin: pos,
                }),
                Row::Up => None,
            });
        }
        let drag = self.drag.clone()?;
        if pos.distance(drag.origin) >= DRAG_DISTANCE {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if !released {
            return None;
        }
        self.drag = None;
        if pos.distance(drag.origin) < DRAG_DISTANCE {
            return None;
        }
        let folder = match row.map(|i| &rows[i]) {
            Some(Row::Up) => parent_folder(&view.folder),
            Some(Row::Entry(e)) if e.is_folder && e.path != drag.path => own_path(e),
            _ => return None,
        };
        Some(ListScreenAction::Move {
            path: drag.path,
            folder,
        })
    }

    fn menu(&mut self, ctx: &egui::Context, view: &BuildListView) -> Option<ListScreenAction> {
        let menu = self.menu.as_mut()?;
        let mut action = None;
        let mut close = false;
        let mut dialog = None;
        let entry = menu.entry.clone();
        let targets = move_targets(view, &entry);
        let shown = egui::Area::new(self.menu_id)
            .order(egui::Order::Foreground)
            .fixed_pos(menu.pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(140.0);
                    let item = |ui: &mut egui::Ui, key: &str, text: &str| {
                        let r = ui.add(egui::Button::new(text).frame(false));
                        mark(ui, key, &r);
                        r.clicked()
                    };
                    if item(ui, "menu:open", "Open") {
                        action = Some(if entry.is_folder {
                            ListScreenAction::OpenFolder(own_path(&entry))
                        } else {
                            ListScreenAction::OpenBuild(entry.path.clone())
                        });
                        close = true;
                    }
                    if item(ui, "menu:copy", "Copy…") {
                        dialog = Some(dialog_for(&entry, true));
                        close = true;
                    }
                    if item(ui, "menu:rename", "Rename…") {
                        dialog = Some(dialog_for(&entry, false));
                        close = true;
                    }
                    if item(ui, "menu:delete", "Delete…") {
                        dialog = Some(delete_dialog(&entry));
                        close = true;
                    }
                    if item(ui, "menu:move", "Move to…") {
                        menu.move_open = !menu.move_open;
                    }
                    if menu.move_open {
                        for (folder, label) in &targets {
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                if item(ui, &format!("menu:to:{folder}"), label) {
                                    action = Some(ListScreenAction::Move {
                                        path: entry.path.clone(),
                                        folder: folder.clone(),
                                    });
                                    close = true;
                                }
                            });
                        }
                    }
                });
            });
        let outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !shown.response.rect.contains(p))
        }) && !menu.fresh;
        let escape = ctx.input(|i| i.key_pressed(Key::Escape));
        menu.fresh = false;
        if close || outside || escape {
            self.menu = None;
        }
        if dialog.is_some() {
            self.dialog = dialog;
        }
        action
    }

    fn dialog(&mut self, ctx: &egui::Context) -> Option<ListScreenAction> {
        let (dialog_id, name_id) = (self.dialog_id, self.name_id);
        let dialog = self.dialog.as_mut()?;
        let mut done = None;
        // PoB's popup closes on Esc once its name box is left, and never on an outside click
        // (PopupDialog.lua:68-77)
        let mut cancel = crate::widgets::escape::escape(ctx, dialog_id);
        egui::Modal::new(dialog_id).show(ctx, |ui| {
            ui.set_width(380.0);
            match dialog {
                Dialog::Name {
                    copy,
                    is_folder,
                    original,
                    text,
                    focus,
                    edited,
                    path,
                } => {
                    let what = if *is_folder { "Folder" } else { "Build" };
                    ui.heading(format!("{} {what}", if *copy { "Copy" } else { "Rename" }));
                    ui.label(format!(
                        "Enter the new name for this {}:",
                        what.to_lowercase()
                    ));
                    let ok = name_edit(ui, name_id, text, focus, edited);
                    // a folder's Save needs only a visible name; a build's must differ from
                    // the build's own (BuildListControl.lua:93-115)
                    let valid =
                        *edited && name_box::has_visible(text) && (*is_folder || text != original);
                    let (save, cancelled) = buttons(ui, "Save", valid);
                    cancel |= cancelled;
                    if (save || ok) && valid {
                        let (path, name) = (path.clone(), text.clone());
                        done = Some(if *copy {
                            ListScreenAction::Copy { path, name }
                        } else {
                            ListScreenAction::Rename { path, name }
                        });
                    }
                }
                Dialog::NewFolder { text, focus } => {
                    ui.heading("New Folder");
                    ui.label("Enter folder name:");
                    let ok = name_edit(ui, name_id, text, focus, &mut true);
                    let valid = name_box::has_visible(text);
                    let (create, cancelled) = buttons(ui, "Create", valid);
                    cancel |= cancelled;
                    if (create || ok) && valid {
                        done = Some(ListScreenAction::NewFolder(text.clone()));
                    }
                }
                Dialog::Delete { path, kind, prompt } => {
                    ui.heading("Delete");
                    ui.label(prompt.as_str());
                    let (yes, cancelled) = buttons_keyed(ui, "Delete", "delete_yes", "delete_no");
                    cancel |= cancelled;
                    if yes {
                        done = Some(ListScreenAction::Delete {
                            path: path.clone(),
                            kind: *kind,
                        });
                    }
                }
            }
        });
        crate::widgets::escape::note(ctx, dialog_id);
        if done.is_some() || cancel {
            self.dialog = None;
        }
        done
    }
}

/// A name box with PoB's character filter. True when Enter was pressed in it.
fn name_edit(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &mut String,
    focus: &mut bool,
    edited: &mut bool,
) -> bool {
    let before = text.clone();
    let edit = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .id(id),
    );
    mark(ui, "name_edit", &edit);
    if *focus {
        edit.request_focus();
        *focus = false;
    }
    if edit.changed() {
        *text = name_box::spliced(&before, text, Filter::Path, name_box::NAME_LIMIT);
    }
    // PoB's change function runs only when a key changed the text
    *edited |= *text != before;
    edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
}

/// The OK and Cancel buttons of a name box: (ok pressed, cancel pressed)
fn buttons(ui: &mut egui::Ui, ok: &str, enabled: bool) -> (bool, bool) {
    ui.horizontal(|ui| {
        let save = ui.add_enabled(enabled, theme::primary_button(ok));
        mark(ui, "name_ok", &save);
        let cancel = ui.button("Cancel");
        mark(ui, "name_cancel", &cancel);
        (save.clicked(), cancel.clicked())
    })
    .inner
}

fn buttons_keyed(ui: &mut egui::Ui, ok: &str, yes_key: &str, no_key: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        let yes = ui.add(theme::primary_button(ok));
        mark(ui, yes_key, &yes);
        let no = ui.button("Cancel");
        mark(ui, no_key, &no);
        (yes.clicked(), no.clicked())
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::views::FolderNode;
    use crate::ui::import::{clear_marks, marked};
    use eframe::egui::{Event, Modifiers, Pos2, RawInput, Rect, Vec2};

    fn entry(name: &str, folder: bool, level: Option<u32>) -> BuildListEntry {
        BuildListEntry {
            name: name.into(),
            is_folder: folder,
            folder: String::new(),
            path: if folder {
                format!("/b/{name}").into()
            } else {
                format!("/b/{name}.xml").into()
            },
            class: Some("Witch".into()),
            ascendancy: None,
            level,
            delete_prompt: if folder {
                String::new()
            } else {
                format!("Are you sure you want to delete build:\n{name}\nThis cannot be undone.")
            },
        }
    }

    fn view(folder: &str) -> BuildListView {
        let mut entries = vec![
            entry("sub", true, None),
            entry("one", false, Some(90)),
            entry("two", false, Some(10)),
        ];
        for e in &mut entries {
            e.folder = folder.into();
        }
        BuildListView {
            folder: folder.into(),
            tree: ["sub/", "other/", "other/inner/", "a/", "a/sub/"]
                .iter()
                .map(|p| FolderNode {
                    path: (*p).into(),
                    ..FolderNode::default()
                })
                .collect(),
            entries,
            sort: BuildSort::Name,
            filter: String::new(),
            ..Default::default()
        }
    }

    /// Drives the list headlessly, one frame at a time.
    struct H {
        ctx: egui::Context,
        ui: BuildListUi,
        view: BuildListView,
        time: f64,
        focus: bool,
        blocked: bool,
    }

    impl H {
        fn new(view: BuildListView) -> Self {
            let mut h = H {
                ctx: egui::Context::default(),
                ui: BuildListUi::default(),
                view,
                time: 1.0,
                focus: false,
                blocked: false,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<ListScreenAction> {
            clear_marks();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = None;
            let (focus, blocked) = (self.focus, self.blocked);
            let (ui_state, view) = (&mut self.ui, &self.view);
            self.ctx
                .clone()
                .run_ui(input, |ui| {
                    if focus {
                        let id = ui_state.id;
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                    out = ui_state.show(ui, view, blocked);
                })
                .drop_without_applying_deltas();
            out
        }

        /// Selects row `i` and gives the list the keyboard.
        fn select(&mut self, i: usize) {
            self.ui.selected = Some(i);
            self.ui.selected_path = None;
            self.focus = true;
            self.frame(vec![]);
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) -> Option<ListScreenAction> {
            self.frame(vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }])
        }

        fn click_at(&mut self, pos: Pos2, button: PointerButton) -> Option<ListScreenAction> {
            let b = |pressed| Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut out = None;
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![b(true)],
                vec![b(false)],
                vec![],
            ] {
                out = self.frame(events).or(out);
            }
            out
        }

        fn click(&mut self, key: &str) -> Option<ListScreenAction> {
            self.frame(vec![]);
            let pos = marked(key)
                .unwrap_or_else(|| panic!("no widget {key}"))
                .center();
            self.click_at(pos, PointerButton::Primary)
        }

        fn row_center(&self, i: usize) -> Pos2 {
            self.ui
                .row_rects
                .iter()
                .find(|(n, _)| *n == i)
                .unwrap_or_else(|| panic!("row {i} not drawn: {:?}", self.ui.row_rects))
                .1
                .center()
        }

        fn drag(&mut self, from: Pos2, to: Pos2) -> Option<ListScreenAction> {
            let b = |pos, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut out = None;
            for events in [
                vec![Event::PointerMoved(from)],
                vec![b(from, true)],
                vec![Event::PointerMoved(Pos2::new(
                    from.x,
                    (from.y + to.y) / 2.0,
                ))],
                vec![Event::PointerMoved(to)],
                vec![b(to, false)],
                vec![],
            ] {
                out = self.frame(events).or(out);
            }
            out
        }

        fn text(&mut self, text: &str) {
            assert_eq!(self.frame(vec![Event::Text(text.into())]), None);
        }
    }

    #[test]
    fn parent_folders() {
        assert_eq!(parent_folder("a/b/"), "a/");
        assert_eq!(parent_folder("a/"), "");
        assert_eq!(parent_folder(""), "");
    }

    #[test]
    fn shows_pobs_order_not_its_own() {
        let mut v = view("x/");
        v.entries = vec![
            entry("zed", false, Some(90)),
            entry("Beta", true, None),
            entry("alpha", false, Some(10)),
        ];
        let names: Vec<String> = ordered(&v)
            .iter()
            .map(|r| match r {
                Row::Up => "..".to_owned(),
                Row::Entry(e) => e.name.clone(),
            })
            .collect();
        assert_eq!(names, ["..", "zed", "Beta", "alpha"]);
    }

    #[test]
    fn arrow_keys_home_and_end_move_the_selection() {
        let mut h = H::new(view(""));
        h.select(1);
        let rows = ordered(&h.view).len();
        h.key(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(h.ui.selected, Some(2));
        h.key(Key::Home, Modifiers::NONE);
        assert_eq!(h.ui.selected, Some(0));
        // Up from the top wraps round, as PoB's list does
        h.key(Key::ArrowUp, Modifiers::NONE);
        assert_eq!(h.ui.selected, Some(rows - 1));
        h.key(Key::End, Modifiers::NONE);
        assert_eq!(h.ui.selected, Some(rows - 1));
        // Backspace asks to delete, as Delete does
        h.select(1);
        h.key(Key::Backspace, Modifiers::NONE);
        assert!(matches!(h.ui.dialog, Some(Dialog::Delete { .. })));
    }

    #[test]
    fn delete_asks_with_pobs_text_then_sends() {
        let mut h = H::new(view(""));
        h.select(1);
        // nothing is sent by the key itself
        assert_eq!(h.key(Key::Delete, Modifiers::NONE), None);
        assert_eq!(h.frame(vec![]), None);
        let Some(Dialog::Delete { prompt, kind, .. }) = &h.ui.dialog else {
            panic!("no question: {:?}", h.ui.dialog);
        };
        assert_eq!(
            prompt,
            "Are you sure you want to delete build:\none\nThis cannot be undone."
        );
        assert_eq!(*kind, DeleteKind::Build);
        // Cancel sends nothing and closes it
        h.focus = false;
        assert_eq!(h.click("delete_no"), None);
        assert_eq!(h.ui.dialog, None);
        // asked again, and this time the answer is yes
        h.select(1);
        h.key(Key::Delete, Modifiers::NONE);
        h.focus = false;
        assert_eq!(
            h.click("delete_yes"),
            Some(ListScreenAction::Delete {
                path: "/b/one.xml".into(),
                kind: DeleteKind::Build,
            })
        );
        assert_eq!(h.ui.dialog, None);
    }

    #[test]
    fn an_empty_folder_is_asked_about_too() {
        let mut h = H::new(view(""));
        h.select(0);
        h.key(Key::Delete, Modifiers::NONE);
        h.focus = false;
        h.frame(vec![]);
        let Some(Dialog::Delete { prompt, .. }) = &h.ui.dialog else {
            panic!("an empty folder was deleted without asking");
        };
        assert_eq!(prompt, "Delete empty folder sub?");
        assert_eq!(
            h.click("delete_yes"),
            Some(ListScreenAction::Delete {
                path: "/b/sub".into(),
                kind: DeleteKind::EmptyFolder,
            })
        );
        // a folder with things in it keeps PoB's text and its kind
        let mut full = view("");
        full.entries[0].delete_prompt = "The folder is not empty.\nAre you sure?".into();
        let mut h = H::new(full);
        h.select(0);
        h.key(Key::Delete, Modifiers::NONE);
        h.focus = false;
        h.frame(vec![]);
        assert_eq!(
            h.click("delete_yes"),
            Some(ListScreenAction::Delete {
                path: "/b/sub".into(),
                kind: DeleteKind::Folder,
            })
        );
    }

    #[test]
    fn rename_dialog_filters_characters() {
        let mut h = H::new(view(""));
        h.select(1);
        assert_eq!(h.key(Key::F2, Modifiers::NONE), None);
        h.focus = false;
        h.frame(vec![]);
        h.frame(vec![]);
        // an unchanged name can't be saved
        assert_eq!(h.click("name_ok"), None);
        assert!(h.ui.dialog.is_some());
        h.click("name_edit");
        h.key(Key::A, Modifiers::COMMAND);
        h.text("a/b:c*d?e\"f<g>h|i\\j");
        let Some(Dialog::Name { text, .. }) = &h.ui.dialog else {
            panic!("no name box");
        };
        assert_eq!(text, "abcdefghij");
        // 100 characters at most
        h.key(Key::A, Modifiers::COMMAND);
        h.text(&"x".repeat(130));
        let Some(Dialog::Name { text, .. }) = &h.ui.dialog else {
            panic!("no name box");
        };
        assert_eq!(text.len(), 100);
        h.key(Key::A, Modifiers::COMMAND);
        h.text("new name");
        assert_eq!(
            h.click("name_ok"),
            Some(ListScreenAction::Rename {
                path: "/b/one.xml".into(),
                name: "new name".into(),
            })
        );
        assert_eq!(h.ui.dialog, None);
    }

    #[test]
    fn the_row_menu_copies_renames_and_moves() {
        let mut h = H::new(view(""));
        let right = |h: &mut H, row| {
            let pos = h.row_center(row);
            h.click_at(pos, PointerButton::Secondary)
        };
        // Copy…
        assert_eq!(right(&mut h, 1), None);
        assert_eq!(h.click("menu:copy"), None);
        h.frame(vec![]);
        h.key(Key::A, Modifiers::COMMAND);
        h.text("one copy");
        assert_eq!(
            h.click("name_ok"),
            Some(ListScreenAction::Copy {
                path: "/b/one.xml".into(),
                name: "one copy".into(),
            })
        );
        // Open
        right(&mut h, 2);
        assert_eq!(
            h.click("menu:open"),
            Some(ListScreenAction::OpenBuild("/b/two.xml".into()))
        );
        // Move to… a folder listed here, and the Builds folder is not offered from it
        right(&mut h, 1);
        assert_eq!(h.click("menu:move"), None);
        h.frame(vec![]);
        // every folder of the tree, not only the ones listed here
        assert!(
            marked("menu:to:other/inner/").is_some(),
            "a deep folder is missing"
        );
        assert!(
            marked("menu:to:").is_none(),
            "the folder it is in is offered"
        );
        assert_eq!(
            h.click("menu:to:sub/"),
            Some(ListScreenAction::Move {
                path: "/b/one.xml".into(),
                folder: "sub/".into(),
            })
        );
        assert!(h.ui.menu.is_none());
        // a click elsewhere closes it
        right(&mut h, 1);
        assert!(h.ui.menu.is_some());
        h.click_at(Pos2::new(900.0, 650.0), PointerButton::Primary);
        assert!(h.ui.menu.is_none());
    }

    #[test]
    fn new_folder_asks_for_a_name() {
        let mut h = H::new(view(""));
        assert_eq!(h.click("list_new_folder"), None);
        h.frame(vec![]);
        // empty: nothing to create
        assert_eq!(h.click("name_ok"), None);
        h.click("name_edit");
        h.text("my/fol:der");
        assert_eq!(
            h.click("name_ok"),
            Some(ListScreenAction::NewFolder("myfolder".into()))
        );
        assert_eq!(h.ui.dialog, None);
    }

    /// PoB's name boxes keep every byte but `\/:*?"<>|` and control characters, count the limit
    /// in bytes, and test the name with `%S` (BuildListControl.lua:90, Main.lua:1696)
    #[test]
    fn name_boxes_keep_non_ascii_and_count_bytes() {
        let mut h = H::new(view(""));
        h.click("list_new_folder");
        h.frame(vec![]);
        h.click("name_edit");
        h.text(&"\u{65e5}".repeat(40));
        let Some(Dialog::NewFolder { text, .. }) = &h.ui.dialog else {
            panic!("no name box");
        };
        // 33 three-byte characters fit in 100 bytes
        assert_eq!(text.chars().count(), 33);
        // a Unicode space is no Lua space, so it is a visible name
        h.key(Key::A, Modifiers::COMMAND);
        h.text("\u{3000}");
        assert_eq!(
            h.click("name_ok"),
            Some(ListScreenAction::NewFolder("\u{3000}".into()))
        );
        // an ASCII space is not
        h.click("list_new_folder");
        h.frame(vec![]);
        h.click("name_edit");
        h.text("  ");
        assert_eq!(h.click("name_ok"), None);
    }

    /// A folder's Save asks only for an edit and a visible name; a build's must also differ
    /// from the build's own name (BuildListControl.lua:93-115)
    #[test]
    fn a_folder_saves_after_an_edit_back_to_its_own_name() {
        let mut h = H::new(view(""));
        h.select(0);
        h.key(Key::F2, Modifiers::NONE);
        h.focus = false;
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.click("name_ok"), None);
        h.click("name_edit");
        h.text("x");
        h.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(
            h.click("name_ok"),
            Some(ListScreenAction::Rename {
                path: "/b/sub".into(),
                name: "sub".into(),
            })
        );
        // a build edited back to its name can't be saved
        h.select(1);
        h.key(Key::F2, Modifiers::NONE);
        h.focus = false;
        h.frame(vec![]);
        h.frame(vec![]);
        h.click("name_edit");
        h.text("x");
        h.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(h.click("name_ok"), None);
        assert!(h.ui.dialog.is_some());
    }

    #[test]
    fn a_name_dialog_leaves_its_box_on_esc_first_and_ignores_outside_clicks() {
        // PopupDialog.lua:68-77: an outside click does nothing, and the selected box takes
        // the first Esc (EditControl.lua:505)
        let mut h = H::new(view(""));
        h.click("list_new_folder");
        h.frame(vec![]);
        h.click("name_edit");
        h.text("typed");
        h.click_at(Pos2::new(2.0, 2.0), PointerButton::Primary);
        assert!(h.ui.dialog.is_some(), "an outside click closed it");
        h.click("name_edit");
        h.key(Key::Escape, Modifiers::NONE);
        let Some(Dialog::NewFolder { text, .. }) = &h.ui.dialog else {
            panic!("the first Esc closed it");
        };
        assert_eq!(text, "typed");
        h.key(Key::Escape, Modifiers::NONE);
        assert_eq!(h.ui.dialog, None, "the second Esc left it open");
    }

    #[test]
    fn cut_paste_sends_move() {
        let mut h = H::new(view(""));
        h.select(1);
        assert_eq!(h.frame(vec![Event::Cut]), None);
        // pasting where it was cut changes nothing, and PoB clears the mark
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), None);
        assert!(h.ui.marked_path().is_none());
        h.frame(vec![Event::Cut]);
        // another folder
        h.view = view("sub/");
        h.frame(vec![]);
        h.focus = true;
        assert_eq!(
            h.frame(vec![Event::Paste(String::new())]),
            Some(ListScreenAction::Move {
                path: "/b/one.xml".into(),
                folder: "sub/".into(),
            })
        );
        // the mark is spent
        h.view = view("");
        h.frame(vec![]);
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), None);
        // paste with nothing cut
        let mut fresh = H::new(view("sub/"));
        fresh.select(1);
        assert_eq!(fresh.frame(vec![Event::Paste(String::new())]), None);
        // the key chord does the same as the event
        let mut h = H::new(view(""));
        h.select(2);
        h.key(Key::X, Modifiers::COMMAND);
        h.view = view("sub/");
        h.frame(vec![]);
        assert_eq!(
            h.key(Key::V, Modifiers::COMMAND),
            Some(ListScreenAction::Move {
                path: "/b/two.xml".into(),
                folder: "sub/".into(),
            })
        );
    }

    #[test]
    fn cut_and_paste_leave_the_search_box_alone() {
        let mut h = H::new(view(""));
        h.select(1);
        h.frame(vec![Event::Cut]);
        h.view = view("sub/");
        h.focus = false;
        h.click("search");
        assert_eq!(h.frame(vec![Event::Paste("abc".into())]), None);
        assert!(h.ui.cut.is_some(), "the search box's paste used the mark");
    }

    #[test]
    fn drag_to_folder_sends_move() {
        let mut h = H::new(view("a/"));
        // rows: .., sub/, one, two
        let (up, sub, one) = (h.row_center(0), h.row_center(1), h.row_center(2));
        assert_eq!(
            h.drag(one, sub),
            Some(ListScreenAction::Move {
                path: "/b/one.xml".into(),
                folder: "a/sub/".into(),
            })
        );
        assert_eq!(
            h.drag(one, up),
            Some(ListScreenAction::Move {
                path: "/b/one.xml".into(),
                folder: String::new(),
            })
        );
        // onto a build, onto itself, or a short wobble: nothing
        let two = h.row_center(3);
        assert_eq!(h.drag(one, two), None);
        assert_eq!(h.drag(one, one + Vec2::new(0.0, 2.0)), None);
        // the `..` row can't be dragged
        assert_eq!(h.drag(up, sub), None);
    }

    /// The sort is one of PoB's searchable dropdowns (SearchHost)
    #[test]
    fn the_sort_dropdown_filters_as_typed() {
        let mut h = H::new(view(""));
        h.click("sort_combo");
        h.frame(vec![]);
        h.frame(vec![Event::Text("level".into())]);
        h.frame(vec![]);
        assert_eq!(
            marked("sort:NAME").unwrap().height(),
            0.0,
            "Name wasn't filtered out"
        );
        assert_eq!(
            h.click("sort:LEVEL"),
            Some(ListScreenAction::SetSort(BuildSort::Level))
        );
    }

    #[test]
    fn sort_combo_sends_set_build_sort() {
        let mut h = H::new(view(""));
        assert_eq!(h.click("sort_combo"), None);
        h.frame(vec![]);
        // PoB's labels
        assert!(marked("sort:NAME").is_some() && marked("sort:EDITED").is_some());
        assert_eq!(sort_label(BuildSort::Edited), "Sort by Last Edited");
        // the sort already chosen sends nothing
        assert_eq!(h.click("sort:NAME"), None);
        h.click("sort_combo");
        h.frame(vec![]);
        assert_eq!(
            h.click("sort:LEVEL"),
            Some(ListScreenAction::SetSort(BuildSort::Level))
        );
        // the view's sort is what the box shows
        h.view.sort = BuildSort::Class;
        h.frame(vec![]);
        h.click("sort_combo");
        h.frame(vec![]);
        assert_eq!(h.click("sort:CLASS"), None);
    }

    #[test]
    fn filter_debounces_200ms() {
        let mut h = H::new(view(""));
        h.click("search");
        h.time = 1.0;
        h.text("cl(a)ss");
        h.time = 1.1;
        assert_eq!(h.frame(vec![]), None);
        // typing again starts the wait over
        h.text("x");
        h.time = 1.25;
        assert_eq!(h.frame(vec![]), None);
        h.time = 1.31;
        assert_eq!(
            h.frame(vec![]),
            Some(ListScreenAction::SetFilter("classx".into()))
        );
        // sent once
        h.view.filter = "classx".into();
        h.time = 2.0;
        assert_eq!(h.frame(vec![]), None);
        // the clear button sends at once
        h.time = 3.0;
        assert_eq!(
            h.click("search_clear"),
            Some(ListScreenAction::SetFilter(String::new()))
        );
    }

    #[test]
    fn ctrl_n_starts_a_new_build() {
        let mut h = H::new(view(""));
        assert_eq!(
            h.key(Key::N, Modifiers::COMMAND),
            Some(ListScreenAction::New)
        );
        // not under a question of the app's own
        h.blocked = true;
        assert_eq!(h.key(Key::N, Modifiers::COMMAND), None);
    }

    #[test]
    fn mouse4_goes_up_a_folder() {
        let mut h = H::new(view(""));
        let press = |button| Event::PointerButton {
            pos: Pos2::new(100.0, 100.0),
            button,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        // nowhere to go back to yet
        assert_eq!(h.frame(vec![press(PointerButton::Extra1)]), None);
        h.view = view("a/");
        h.frame(vec![]);
        h.view = view("a/b/");
        h.frame(vec![]);
        assert_eq!(
            h.frame(vec![press(PointerButton::Extra1)]),
            Some(ListScreenAction::OpenFolder("a/".into()))
        );
        h.view = view("a/");
        h.frame(vec![]);
        // and forward again with Mouse5
        assert_eq!(
            h.frame(vec![press(PointerButton::Extra2)]),
            Some(ListScreenAction::OpenFolder("a/b/".into()))
        );
        h.view = view("a/b/");
        h.frame(vec![]);
        assert_eq!(h.frame(vec![press(PointerButton::Extra2)]), None);
        // a new folder ends the forward history
        h.frame(vec![press(PointerButton::Extra1)]);
        h.view = view("a/");
        h.frame(vec![]);
        h.view = view("a/c/");
        h.frame(vec![]);
        assert_eq!(h.frame(vec![press(PointerButton::Extra2)]), None);
        assert_eq!(
            h.frame(vec![press(PointerButton::Extra1)]),
            Some(ListScreenAction::OpenFolder("a/".into()))
        );
    }

    #[test]
    fn the_path_goes_back_to_any_folder() {
        let mut h = H::new(view("a/b/"));
        assert_eq!(
            h.click("crumb:1"),
            Some(ListScreenAction::OpenFolder("a/".into()))
        );
        assert_eq!(
            h.click("crumb:0"),
            Some(ListScreenAction::OpenFolder(String::new()))
        );
        // the folder shown is not a link
        h.frame(vec![]);
        assert!(marked("crumb:2").is_none());
    }

    #[test]
    fn enter_opens_and_double_click_opens() {
        let mut h = H::new(view(""));
        h.select(1);
        assert_eq!(
            h.key(Key::Enter, Modifiers::NONE),
            Some(ListScreenAction::OpenBuild("/b/one.xml".into()))
        );
    }

    #[test]
    fn the_selection_follows_its_entry_when_the_list_changes() {
        let mut h = H::new(view(""));
        h.select(1);
        h.frame(vec![]);
        // a new entry sorts in before it
        h.view.entries.insert(0, entry("aaa", false, Some(1)));
        h.frame(vec![]);
        assert_eq!(h.ui.selected, Some(2));
    }

    fn event_key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn ctrl_n_does_nothing_while_a_text_box_has_focus() {
        let mut h = H::new(view(""));
        h.click("search");
        assert_eq!(h.key(Key::N, Modifiers::COMMAND), None);
        // with the focus off the box, it starts a build
        h.click_at(Pos2::new(900.0, 650.0), PointerButton::Primary);
        assert_eq!(
            h.key(Key::N, Modifiers::COMMAND),
            Some(ListScreenAction::New)
        );
    }

    #[test]
    fn paste_works_on_an_empty_clipboard() {
        // egui-winit sends no Paste and no press for Ctrl+V when the clipboard is empty: only
        // the release arrives
        let mut h = H::new(view(""));
        h.select(1);
        h.frame(vec![Event::Cut]);
        h.view = view("sub/");
        h.frame(vec![]);
        assert_eq!(
            h.frame(vec![event_key(Key::V, false, Modifiers::COMMAND)]),
            Some(ListScreenAction::Move {
                path: "/b/one.xml".into(),
                folder: "sub/".into(),
            })
        );
        // with text on the clipboard the Paste event comes first and the release is not a
        // second paste (a copy would be made twice)
        let mut h = H::new(view(""));
        h.select(1);
        h.frame(vec![Event::Copy]);
        h.view = view("sub/");
        h.frame(vec![]);
        assert!(matches!(
            h.frame(vec![Event::Paste("x".into())]),
            Some(ListScreenAction::CopyTo { .. })
        ));
        assert_eq!(
            h.frame(vec![event_key(Key::V, false, Modifiers::COMMAND)]),
            None
        );
        // the next press and release is a paste again, of a fresh mark
        h.view = view("");
        h.frame(vec![]);
        h.select(1);
        h.frame(vec![Event::Copy]);
        h.view = view("sub/");
        h.frame(vec![]);
        assert!(matches!(
            h.frame(vec![Event::Paste("x".into())]),
            Some(ListScreenAction::CopyTo { .. })
        ));
    }

    #[test]
    fn copy_paste_copies_into_another_folder() {
        let mut h = H::new(view(""));
        h.select(1);
        assert_eq!(h.frame(vec![Event::Copy]), None);
        // in its own folder PoB asks for a name: that is the Copy box
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), None);
        assert!(matches!(h.ui.dialog, Some(Dialog::Name { copy: true, .. })));
        h.ui.dialog = None;
        // PoB clears the mark with that paste
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), None);
        assert!(h.ui.dialog.is_none());
        h.select(1);
        h.frame(vec![Event::Copy]);
        h.view = view("sub/");
        h.frame(vec![]);
        let copy = Some(ListScreenAction::CopyTo {
            path: "/b/one.xml".into(),
            folder: "sub/".into(),
        });
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), copy);
        // and after every paste (a second Ctrl+V copies nothing)
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), None);
        // Ctrl+X after Ctrl+C replaces the mark
        h.select(2);
        h.frame(vec![Event::Cut]);
        h.view = view("");
        h.frame(vec![]);
        assert!(matches!(
            h.frame(vec![Event::Paste(String::new())]),
            Some(ListScreenAction::Move { .. })
        ));
    }

    #[test]
    fn a_refused_move_keeps_the_cut_mark() {
        let mut h = H::new(view(""));
        h.select(1);
        h.frame(vec![Event::Cut]);
        h.view = view("sub/");
        h.frame(vec![]);
        assert!(h.frame(vec![Event::Paste(String::new())]).is_some());
        assert_eq!(h.frame(vec![Event::Paste(String::new())]), None);
        // the engine said no
        h.ui.move_refused();
        assert!(matches!(
            h.frame(vec![Event::Paste(String::new())]),
            Some(ListScreenAction::Move { .. })
        ));
    }

    /// A search shows matches from every folder: the entry knows where it is
    fn search_view() -> BuildListView {
        let mut v = view("");
        v.filter = "x".into();
        v.entries = vec![entry("deep", false, Some(1)), entry("dir", true, None)];
        v.entries[0].folder = "x/y/".into();
        v.entries[0].path = "/b/x/y/deep.xml".into();
        v.entries[1].folder = "x/".into();
        v.entries[1].path = "/b/x/dir".into();
        v
    }

    #[test]
    fn a_search_result_acts_from_its_own_folder() {
        let mut h = H::new(search_view());
        // each result says where it is
        assert!(marked("folder_cell:0").is_some() && marked("folder_cell:1").is_some());
        // Ctrl+X on a match from x/y/, Ctrl+V in the Builds folder moves it
        h.select(0);
        h.frame(vec![Event::Cut]);
        assert_eq!(
            h.frame(vec![Event::Paste(String::new())]),
            Some(ListScreenAction::Move {
                path: "/b/x/y/deep.xml".into(),
                folder: String::new(),
            })
        );
        // a folder found by the search opens where it is, and takes a drag to it
        h.select(1);
        assert_eq!(
            h.key(Key::Enter, Modifiers::NONE),
            Some(ListScreenAction::OpenFolder("x/dir/".into()))
        );
        let (deep, dir) = (h.row_center(0), h.row_center(1));
        assert_eq!(
            h.drag(deep, dir),
            Some(ListScreenAction::Move {
                path: "/b/x/y/deep.xml".into(),
                folder: "x/dir/".into(),
            })
        );
        // and the row menu's Move to… never offers the folder it is in
        assert_eq!(h.click_at(deep, PointerButton::Secondary), None);
        h.click("menu:move");
        h.frame(vec![]);
        assert!(marked("menu:to:x/y/").is_none());
        assert!(marked("menu:to:").is_some());
    }

    #[test]
    fn move_to_lists_the_tree_but_not_a_folder_or_what_is_inside_it() {
        let v = view("");
        let names = |e: &BuildListEntry| -> Vec<String> {
            move_targets(&v, e).into_iter().map(|(f, _)| f).collect()
        };
        // a build in the Builds folder: every folder but the Builds folder itself
        assert_eq!(
            names(&v.entries[1]),
            ["sub/", "other/", "other/inner/", "a/", "a/sub/"]
        );
        // a folder: not itself, not below itself
        let other = entry("other", true, None);
        assert_eq!(names(&other), ["sub/", "a/", "a/sub/"]);
        // an entry deeper down can go to the top, too
        let mut deep = entry("b", false, Some(1));
        deep.folder = "other/".into();
        assert_eq!(names(&deep), ["", "sub/", "other/inner/", "a/", "a/sub/"]);
    }

    #[test]
    fn only_the_pastes_move_gets_its_mark_back() {
        let mut h = H::new(view(""));
        h.select(1);
        h.frame(vec![Event::Cut]);
        h.view = view("sub/");
        h.frame(vec![]);
        assert!(h.frame(vec![Event::Paste(String::new())]).is_some());
        // that move worked; later a drag is refused: the old mark must not return
        h.view = view("a/");
        h.frame(vec![]);
        let (two, sub) = (h.row_center(3), h.row_center(1));
        assert!(h.drag(two, sub).is_some());
        h.ui.move_refused();
        assert!(h.ui.marked_path().is_none());
    }
}
