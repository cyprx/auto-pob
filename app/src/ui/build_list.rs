//! The build list: the current folder's entries, folders first, plus New and Import code.

use std::path::PathBuf;

use crate::engine::views::{BuildListEntry, BuildListView};
use crate::theme;
use crate::widgets::{ListAction, ListColumn, list, list_sort};
use eframe::egui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListScreenAction {
    /// Show this Builds sub-folder ("" is the root).
    OpenFolder(String),
    OpenBuild(PathBuf),
    New,
    Import,
    Options,
}

pub struct BuildListUi {
    id: egui::Id,
    selected: Option<usize>,
    folder: String,
}

impl Default for BuildListUi {
    fn default() -> Self {
        Self {
            id: egui::Id::unique("build_list"),
            selected: None,
            folder: String::new(),
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

/// `..` first, then folders, then builds; each group ordered by the chosen column.
/// Columns: 0 name, 1 class, 2 level.
fn ordered(view: &BuildListView, sort: Option<(usize, bool)>) -> Vec<Row<'_>> {
    let mut entries: Vec<&BuildListEntry> = view.entries.iter().collect();
    let (column, ascending) = sort.unwrap_or((0, true));
    entries.sort_by(|a, b| {
        b.is_folder.cmp(&a.is_folder).then_with(|| {
            let ord = match column {
                1 => a
                    .ascendancy
                    .as_deref()
                    .or(a.class.as_deref())
                    .cmp(&b.ascendancy.as_deref().or(b.class.as_deref())),
                2 => a.level.cmp(&b.level),
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            };
            if ascending { ord } else { ord.reverse() }
        })
    });
    let mut rows = Vec::new();
    if !view.folder.is_empty() {
        rows.push(Row::Up);
    }
    rows.extend(entries.into_iter().map(Row::Entry));
    rows
}

impl BuildListUi {
    pub fn show(&mut self, ui: &mut egui::Ui, view: &BuildListView) -> Option<ListScreenAction> {
        let mut action = None;
        if self.folder != view.folder {
            self.folder.clone_from(&view.folder);
            self.selected = None;
        }
        ui.horizontal(|ui| {
            theme::heading(
                ui,
                &format!(
                    "Builds{}",
                    if view.folder.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " / {}",
                            view.folder.trim_end_matches('/').replace('/', " / ")
                        )
                    }
                ),
            );
            if ui.add(theme::primary_button("New")).clicked() {
                action = Some(ListScreenAction::New);
            }
            if ui.button("Import code").clicked() {
                action = Some(ListScreenAction::Import);
            }
            if ui.button("Options").clicked() {
                action = Some(ListScreenAction::Options);
            }
        });
        ui.separator();

        let id = self.id;
        let sort = list_sort(ui.ctx(), id).map(|s| (s.column, s.ascending));
        let rows = ordered(view, sort);
        // the rows can shrink when the folder changes or the list refreshes
        if let Some(s) = self.selected
            && s >= rows.len()
        {
            self.selected = rows.len().checked_sub(1);
        }
        if rows.is_empty() {
            ui.label("No builds here yet. Use New or Import code.");
            return action;
        }
        let columns = [
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
        let result = list(
            ui,
            id,
            &columns,
            rows.len(),
            |i, row| match &rows[i] {
                Row::Up => {
                    row.col(|ui| {
                        ui.label("..");
                    });
                    row.col(|_| {});
                    row.col(|_| {});
                }
                Row::Entry(e) if e.is_folder => {
                    row.col(|ui| {
                        ui.label(format!("{}/", e.name));
                    });
                    row.col(|_| {});
                    row.col(|_| {});
                }
                Row::Entry(e) => {
                    row.col(|ui| {
                        ui.label(&e.name);
                    });
                    row.col(|ui| {
                        let class = e.ascendancy.as_deref().or(e.class.as_deref());
                        ui.label(class.unwrap_or(""));
                    });
                    row.col(|ui| {
                        if let Some(level) = e.level {
                            ui.label(level.to_string());
                        }
                    });
                }
            },
            &mut self.selected,
        );
        if let Some(ListAction::Activate(i)) = result {
            action = match rows.get(i) {
                Some(Row::Up) => Some(ListScreenAction::OpenFolder(parent_folder(&view.folder))),
                Some(Row::Entry(e)) if e.is_folder => Some(ListScreenAction::OpenFolder(format!(
                    "{}{}/",
                    view.folder, e.name
                ))),
                Some(Row::Entry(e)) => Some(ListScreenAction::OpenBuild(e.path.clone())),
                None => None,
            };
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, folder: bool, level: Option<u32>) -> BuildListEntry {
        BuildListEntry {
            name: name.into(),
            is_folder: folder,
            path: format!("/b/{name}.xml").into(),
            class: Some("Witch".into()),
            ascendancy: None,
            level,
        }
    }

    #[test]
    fn parent_folders() {
        assert_eq!(parent_folder("a/b/"), "a/");
        assert_eq!(parent_folder("a/"), "");
        assert_eq!(parent_folder(""), "");
    }

    #[test]
    fn folders_first_and_sorted() {
        let view = BuildListView {
            folder: "x/".into(),
            entries: vec![
                entry("zed", false, Some(90)),
                entry("Beta", true, None),
                entry("alpha", false, Some(10)),
            ],
        };
        let names = |sort| {
            ordered(&view, sort)
                .iter()
                .map(|r| match r {
                    Row::Up => "..".to_owned(),
                    Row::Entry(e) => e.name.clone(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(None), ["..", "Beta", "alpha", "zed"]);
        assert_eq!(names(Some((2, false))), ["..", "Beta", "zed", "alpha"]);
    }
}
