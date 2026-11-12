//! PoB's About window (Main.lua:1336-1440): its help, section by section, and its version
//! history. The engine reads both off PoB's own window (`Command::Help`), once per run.

use std::sync::Arc;

use crate::engine::{Command, HelpSection, HelpView};
use crate::widgets::pob_text::pob_text;
use eframe::egui;

/// Where the window opens, as PoB's `OpenAboutPopup(helpSectionIndex)` is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpStart {
    /// A section by its number, from 1 ("Help >>" asks for 7, Build.lua:312)
    Section(usize),
    /// F1's "<tab> tab" (Main.lua:441-443): the section of that title, else the first
    Tab(String),
    /// The About button: the version history (Main.lua:210)
    VersionHistory,
}

/// The 0-based help section `start` names, as `OpenAboutPopup` picks it (Main.lua:1407-1416):
/// a section number that exists, else the section whose lowercased title is the name, else
/// the first. PoB compares the name as given, so "Build List tab" never matches.
pub fn start_section(sections: &[HelpSection], start: &HelpStart) -> usize {
    match start {
        HelpStart::Section(n) if *n >= 1 && *n <= sections.len() => n - 1,
        HelpStart::Tab(name) => sections
            .iter()
            .position(|s| s.title.to_lowercase() == *name)
            .unwrap_or(0),
        _ => 0,
    }
}

/// The name F1 gives PoB for where the user is (Main.lua:441): the view mode, lowercased,
/// or "Build List" on the list.
pub fn tab_name(tab: Option<crate::ui::build::BuildTab>) -> String {
    use crate::ui::build::BuildTab;
    let mode = match tab {
        None => return "Build List tab".into(),
        Some(BuildTab::Tree) => "tree",
        Some(BuildTab::Skills) => "skills",
        Some(BuildTab::Items) => "items",
        Some(BuildTab::Calcs) => "calcs",
        Some(BuildTab::Config) => "config",
        Some(BuildTab::Notes) => "notes",
        Some(BuildTab::Compare) => "compare",
        Some(BuildTab::Import) => "import",
    };
    format!("{mode} tab")
}

#[derive(Default)]
pub struct HelpUi {
    open: bool,
    view: Option<Arc<HelpView>>,
    /// The seq of the `Help` sent, while its answer is awaited
    asked: Option<u64>,
    start: Option<HelpStart>,
    /// The version history is shown instead of the help
    history: bool,
    /// The row to scroll to on the next draw: (history, section)
    scroll_to: Option<(bool, usize)>,
    /// Where the window last opened, for tests
    #[cfg(test)]
    placed: Option<(bool, usize)>,
    error: Option<String>,
}

impl HelpUi {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the window at `start`. Returns the query to send when PoB's help hasn't been
    /// read yet; the caller gives back its seq with [`HelpUi::asked`].
    pub fn open(&mut self, start: HelpStart) -> Option<Command> {
        self.open = true;
        self.error = None;
        self.history = start == HelpStart::VersionHistory;
        self.start = Some(start);
        match &self.view {
            Some(_) => {
                self.place();
                None
            }
            None if self.asked.is_some() => None,
            None => Some(Command::Help),
        }
    }

    pub fn asked(&mut self, seq: u64) {
        self.asked = Some(seq);
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn on_view(&mut self, view: Arc<HelpView>) {
        self.asked = None;
        self.view = Some(view);
        if self.open {
            self.place();
        }
    }

    /// The query failed (PoB's help.txt is missing): the window says so. Returns whether
    /// the error was the window's.
    pub fn on_error(&mut self, seq: u64, message: &str) -> bool {
        if self.asked != Some(seq) {
            return false;
        }
        self.asked = None;
        self.error = Some(message.to_owned());
        true
    }

    fn place(&mut self) {
        let (Some(view), Some(start)) = (&self.view, &self.start) else {
            return;
        };
        self.scroll_to = Some(if self.history {
            (true, 0)
        } else {
            (false, start_section(&view.sections, start))
        });
        #[cfg(test)]
        {
            self.placed = self.scroll_to;
        }
    }

    /// Where the window last opened: (version history, section)
    #[cfg(test)]
    pub fn target(&self) -> Option<(bool, usize)> {
        self.placed
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        let default = ctx.global_style().visuals.text_color();
        egui::Window::new("About")
            .id(egui::Id::unique("help_window"))
            .collapsible(false)
            .default_size([810.0, 628.0])
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(view) = self.view.clone() else {
                    match &self.error {
                        Some(error) => {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                        None => {
                            ui.spinner();
                        }
                    }
                    return;
                };
                for line in &view.header {
                    let job = pob_text(line, 14.0, default);
                    match line.find("https://") {
                        Some(at) => {
                            let url = line[at..].trim().to_owned();
                            ui.hyperlink_to(job, url);
                        }
                        None => {
                            ui.label(job);
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.history, "Version history")
                        .clicked()
                    {
                        self.history = true;
                        self.scroll_to = Some((true, 0));
                    }
                    if ui.selectable_label(!self.history, "Help").clicked() {
                        self.history = false;
                        self.scroll_to = Some((false, 0));
                    }
                });
                ui.separator();
                let sections = if self.history {
                    &view.changelog
                } else {
                    &view.sections
                };
                let target = self.scroll_to.take().filter(|t| t.0 == self.history);
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if sections.is_empty() && self.history {
                            ui.label("PoB's version history isn't installed.");
                        }
                        for (i, section) in sections.iter().enumerate() {
                            let title = if self.history {
                                section.title.clone()
                            } else {
                                format!("{} ({})", section.title, i + 1)
                            };
                            let heading = ui.label(pob_text(&title, 20.0, default));
                            if target.is_some_and(|t| t.1 == i) {
                                heading.scroll_to_me(Some(egui::Align::TOP));
                            }
                            for line in &section.lines {
                                help_line(ui, line, default);
                            }
                            ui.add_space(8.0);
                        }
                    });
            });
        if !open {
            self.open = false;
        }
    }
}

/// A row: one cell, or a key and what it does in PoB's two columns (x = 1 and 135).
fn help_line(ui: &mut egui::Ui, line: &str, default: egui::Color32) {
    match line.split_once('\t') {
        Some((key, text)) => {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [134.0, 16.0],
                    egui::Label::new(pob_text(key, 14.0, default)).truncate(),
                );
                ui.add(egui::Label::new(pob_text(text, 14.0, default)).wrap());
            });
        }
        None => {
            ui.add(egui::Label::new(pob_text(line, 14.0, default)).wrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections(titles: &[&str]) -> Vec<HelpSection> {
        titles
            .iter()
            .map(|t| HelpSection {
                title: (*t).into(),
                lines: vec![],
            })
            .collect()
    }

    #[test]
    fn the_start_section_is_picked_as_pob_picks_it() {
        let s = sections(&["Intro", "Contents", "Skills Tab", "Items Tab"]);
        assert_eq!(start_section(&s, &HelpStart::Section(3)), 2);
        // a number past the end falls back to the first
        assert_eq!(start_section(&s, &HelpStart::Section(9)), 0);
        assert_eq!(start_section(&s, &HelpStart::Tab("items tab".into())), 3);
        assert_eq!(start_section(&s, &HelpStart::Tab("tree tab".into())), 0);
        assert_eq!(start_section(&s, &HelpStart::Tab(tab_name(None))), 0);
        assert_eq!(
            tab_name(Some(crate::ui::build::BuildTab::Skills)),
            "skills tab"
        );
    }

    #[test]
    fn opening_asks_once_and_places_the_section_when_the_view_arrives() {
        let mut help = HelpUi::default();
        assert!(matches!(
            help.open(HelpStart::Tab("skills tab".into())),
            Some(Command::Help)
        ));
        help.asked(4);
        // asked again while waiting: no second query
        assert!(help.open(HelpStart::Tab("skills tab".into())).is_none());
        help.on_view(Arc::new(HelpView {
            sections: sections(&["Intro", "Skills Tab"]),
            ..HelpView::default()
        }));
        assert_eq!(help.target(), Some((false, 1)));
        // the view is kept: another opening asks nothing
        assert!(help.open(HelpStart::Section(1)).is_none());
        assert_eq!(help.target(), Some((false, 0)));
        assert!(help.open(HelpStart::VersionHistory).is_none());
        assert_eq!(help.target(), Some((true, 0)));
    }

    #[test]
    fn a_failed_query_is_the_windows_own_error() {
        let mut help = HelpUi::default();
        help.open(HelpStart::Section(1));
        help.asked(7);
        assert!(!help.on_error(6, "other"));
        assert!(help.on_error(7, "PoB's help file (help.txt) isn't installed"));
        assert!(help.is_open());
        // the next opening asks again
        assert!(matches!(
            help.open(HelpStart::Section(1)),
            Some(Command::Help)
        ));
    }
}
