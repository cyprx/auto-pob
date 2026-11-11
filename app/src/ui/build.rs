//! The build screen's top bar and tab strip.

use crate::engine::views::BuildView;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildAction {
    Back,
    Save,
    SaveAs,
    Undo,
    Redo,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildTab {
    #[default]
    Tree,
    Skills,
    Items,
    Calcs,
    Config,
    Notes,
    Import,
}

impl BuildTab {
    /// The Config, Tree, Skills, Items and Calcs tabs have undo history; the notes editor has its
    /// own and the Import/Export tab has none.
    pub fn has_undo(self) -> bool {
        matches!(
            self,
            BuildTab::Config
                | BuildTab::Tree
                | BuildTab::Skills
                | BuildTab::Items
                | BuildTab::Calcs
        )
    }
}

/// `history`: whether the tab's undo and redo have anything to do.
pub fn top_bar(
    ui: &mut egui::Ui,
    view: &BuildView,
    tab: BuildTab,
    history: (bool, bool),
) -> Option<BuildAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui.button("< Builds").clicked() {
            action = Some(BuildAction::Back);
        }
        ui.separator();
        let marker = if view.unsaved { " *" } else { "" };
        ui.strong(format!("{}{marker}", view.name));
        ui.separator();
        if ui.button("Save").clicked() {
            action = Some(BuildAction::Save);
        }
        if ui.button("Save as…").clicked() {
            action = Some(BuildAction::SaveAs);
        }
        ui.separator();
        let undo = tab.has_undo();
        if ui
            .add_enabled(undo && history.0, egui::Button::new("Undo"))
            .clicked()
        {
            action = Some(BuildAction::Undo);
        }
        if ui
            .add_enabled(undo && history.1, egui::Button::new("Redo"))
            .clicked()
        {
            action = Some(BuildAction::Redo);
        }
        ui.separator();
        if ui.button("Options").clicked() {
            action = Some(BuildAction::Options);
        }
        ui.separator();
        let class = view.ascendancy.as_deref().unwrap_or(&view.class);
        ui.label(format!("Level {} {class}", view.level));
    });
    action
}

pub fn tab_strip(ui: &mut egui::Ui, tab: &mut BuildTab) {
    ui.horizontal(|ui| {
        ui.selectable_value(tab, BuildTab::Tree, "Tree");
        ui.selectable_value(tab, BuildTab::Skills, "Skills");
        ui.selectable_value(tab, BuildTab::Items, "Items");
        ui.selectable_value(tab, BuildTab::Calcs, "Calcs");
        ui.selectable_value(tab, BuildTab::Config, "Config");
        ui.selectable_value(tab, BuildTab::Notes, "Notes");
        ui.selectable_value(tab, BuildTab::Import, "Import/Export");
    });
}
