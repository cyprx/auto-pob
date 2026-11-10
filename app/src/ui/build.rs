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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildTab {
    #[default]
    Config,
    Lab,
}

impl BuildTab {
    /// Only the Config tab has undo history.
    pub fn has_undo(self) -> bool {
        self == BuildTab::Config
    }
}

pub fn top_bar(ui: &mut egui::Ui, view: &BuildView, tab: BuildTab) -> Option<BuildAction> {
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
            .add_enabled(undo && view.can_undo, egui::Button::new("Undo"))
            .clicked()
        {
            action = Some(BuildAction::Undo);
        }
        if ui
            .add_enabled(undo && view.can_redo, egui::Button::new("Redo"))
            .clicked()
        {
            action = Some(BuildAction::Redo);
        }
        ui.separator();
        let class = view.ascendancy.as_deref().unwrap_or(&view.class);
        ui.label(format!("Level {} {class}", view.level));
    });
    action
}

pub fn tab_strip(ui: &mut egui::Ui, tab: &mut BuildTab) {
    ui.horizontal(|ui| {
        ui.selectable_value(tab, BuildTab::Config, "Config");
        ui.selectable_value(tab, BuildTab::Lab, "Lab");
    });
}
