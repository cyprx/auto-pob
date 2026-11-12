//! The Compare tab (R4-R8): PoB's comparison builds, their entry list and selectors, and the
//! Summary, Tree, Items, Skills and Config views. Every view is the engine's `CompareView`;
//! every change goes out as a command. The tab has no undo of its own: its copies go through
//! the target tab's (R8).

mod config;
mod items;
mod skills;
mod summary;
#[cfg(test)]
pub(crate) mod tests;

use super::popup::PopupWindow;
use super::tree::Rects;
use crate::engine::views::{BuildListView, CompareView, ConfigView, ItemTooltipView, PopupControl};
use crate::engine::{Command, CompareSource, ImportMode, ItemTooltipQuery, PopupInput, Snapshot};
use crate::theme;
use crate::widgets::item_tooltip_ui;
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;

/// An item must be hovered this long before its tooltip is asked for.
const HOVER_DELAY: f64 = 0.1;

/// The views under PoB's sub-tabs; the Calcs view is out (R9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubTab {
    #[default]
    Summary,
    Tree,
    Items,
    Skills,
    Config,
}

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct CompareOutput {
    pub commands: Vec<Command>,
    /// Questions (item tooltips); the app sends them without waiting for the answer
    pub queries: Vec<Command>,
    /// The Tree view is up: the app draws our tree below, coloured by the overlay (R7)
    pub tree: bool,
}

/// The "Load build…" menu's windows (PoB's "Import Comparison Build" popup)
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dialog {
    /// A build from the build list
    Saved,
    Code(String),
    Link(String),
}

/// PoB's top-bar selectors, with their captions (`compare*Label`, `cmpSkillLabel` and so on),
/// in two rows as PoB draws them.
const SET_SELECTORS: [(&str, &str); 4] = [
    ("compareSpecSelect", "Tree set:"),
    ("compareSkillSetSelect", "Skill set:"),
    ("compareItemSetSelect", "Item set:"),
    ("compareConfigSetSelect", "Config set:"),
];
const SKILL_SELECTORS: [(&str, &str); 7] = [
    ("cmpSocketGroup", "Skill:"),
    ("cmpMainSkill", ""),
    ("cmpSkillPart", ""),
    ("cmpStageCount", "Stages:"),
    ("cmpMineCount", "Mines:"),
    ("cmpMinion", ""),
    ("cmpMinionSkill", ""),
];
/// The Items view's own selectors
const ITEM_SELECTORS: [(&str, &str); 2] = [
    ("primaryItemSetSelect", "Item set:"),
    ("compareItemSetSelect2", "Their item set:"),
];
/// PoB's "Overlay comparison" box: side by side is not drawn here (R7 fallback)
const OVERLAY_CHECK: &str = "treeOverlayCheck";

/// Asks for an item's tooltip once the pointer has rested on it, and keeps the answer to the
/// latest question only.
#[derive(Default)]
struct ItemTip {
    hover: Option<(ItemTooltipQuery, f64)>,
    asked: Option<ItemTooltipQuery>,
    tooltip: Option<Arc<ItemTooltipView>>,
    /// What the pointer is over this frame
    now: Option<ItemTooltipQuery>,
}

impl ItemTip {
    fn note(&mut self, response: &egui::Response, query: ItemTooltipQuery) {
        if response.hovered() {
            self.now = Some(query.clone());
        }
        if let Some(t) = self.tooltip.clone()
            && t.query == query
        {
            response.clone().on_hover_ui(|ui| {
                item_tooltip_ui(ui, &t.tooltip);
            });
        }
    }

    fn end_frame(&mut self, ctx: &egui::Context, now: f64) -> Option<Command> {
        match (self.now.take(), &self.hover) {
            (Some(q), Some((p, _))) if q == *p => {}
            (Some(q), _) => {
                self.hover = Some((q, now));
                self.asked = None;
                self.tooltip = None;
            }
            (None, _) => *self = Self::default(),
        }
        let (query, since) = self.hover.as_ref()?;
        if self.asked.as_ref() == Some(query) {
            return None;
        }
        if now - since >= HOVER_DELAY {
            self.asked = Some(query.clone());
            Some(Command::ItemTooltip(query.clone()))
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            None
        }
    }

    fn on_reply(&mut self, view: &Arc<ItemTooltipView>) {
        if self.asked.as_ref() == Some(&view.query) {
            self.tooltip = Some(view.clone());
        }
    }
}

#[derive(Default)]
pub struct CompareUi {
    view: Option<Arc<CompareView>>,
    sub: SubTab,
    /// SP5b's control renderers, drawn inline (never opened as a window)
    renderer: PopupWindow,
    dialog: Option<Dialog>,
    /// The build list, for "From a saved build"
    list: Option<Arc<BuildListView>>,
    /// Our items' names by id, with PoB's rarity colour, for the Items view
    item_names: HashMap<u32, String>,
    /// Our config sets, for the Config view's own selector
    config: Option<Arc<ConfigView>>,
    /// The Config view's search text (PoB's `configSearchEdit`)
    config_filter: String,
    /// The open build has a file: PoB offers a link comparison only then
    has_file: bool,
    tip: ItemTip,
    /// PoB switched to its Compare tab and the app hasn't followed yet
    switched: bool,
    pub(crate) rects: Rects,
}

impl CompareUi {
    /// Forgets the build's comparisons, as for another build.
    pub fn reset(&mut self) {
        let (list, has_file) = (self.list.take(), self.has_file);
        *self = Self::default();
        self.list = list;
        self.has_file = has_file;
    }

    pub fn sub_tab(&self) -> SubTab {
        self.sub
    }

    /// The Tree view is up with a comparison loaded: the overlay should be on (R7).
    pub fn tree_shown(&self) -> bool {
        self.view.is_some() && self.sub == SubTab::Tree
    }

    /// A load window is open.
    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// PoB switched to its Compare tab (an Import-tab comparison landed), once.
    pub fn take_switched(&mut self) -> bool {
        std::mem::take(&mut self.switched)
    }

    /// A count typed into a selector box and not sent yet.
    pub fn has_pending(&self) -> bool {
        self.renderer.has_typed()
    }

    /// The typed count, to go out before whatever comes next (leaving the tab, Save).
    pub fn flush(&mut self) -> Option<Command> {
        let (name, text) = self.renderer.take_typed()?;
        Some(Command::SetCompareControl {
            name,
            value: PopupInput::Text(text),
        })
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        match snapshot {
            Snapshot::Compare(Some(view)) => {
                if view.switched {
                    self.switched = true;
                }
                if self.view.as_deref() != Some(&**view) {
                    // what the engine says about an item may have changed
                    self.tip = ItemTip::default();
                }
                self.view = Some(view.clone());
            }
            Snapshot::Compare(None) => {
                // the last comparison went: the tab starts over
                self.view = None;
                self.sub = SubTab::Summary;
                self.tip = ItemTip::default();
                self.renderer.reset();
                self.config_filter.clear();
            }
            Snapshot::Build(view) => self.has_file = view.path.is_some(),
            Snapshot::BuildList(view) => self.list = Some(view.clone()),
            Snapshot::Config(view) => self.config = Some(view.clone()),
            Snapshot::Items(view) => {
                self.item_names = view
                    .items
                    .iter()
                    .map(|e| (e.id, format!("{}{}", e.colour, e.name)))
                    .collect();
            }
            Snapshot::ItemTooltip(view) => self.tip.on_reply(view),
            _ => {}
        }
    }

    /// `busy`: the engine hasn't caught up with what was sent.
    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool) -> CompareOutput {
        let mut out = CompareOutput::default();
        let now = ui.input(|i| i.time);
        self.header(ui, &mut out);
        if let Some(view) = self.view.clone() {
            self.selectors(ui, &view, busy, &mut out);
            ui.separator();
            self.sub_tabs(ui);
            ui.separator();
            match self.sub {
                SubTab::Summary => summary::show(ui, &view),
                SubTab::Tree => self.tree_header(ui, &view, &mut out),
                SubTab::Items => {
                    self.item_selectors(ui, &view, busy, &mut out);
                    items::show(ui, self, &view, &mut out);
                }
                SubTab::Skills => skills::show(ui, &view),
                SubTab::Config => config::show(ui, self, &view, &mut out),
            }
        } else {
            ui.add_space(8.0);
            ui.label("No comparison build loaded. Load one with \"Load build…\" above.");
        }
        // a count box no longer drawn sends what was typed into it
        for (name, text) in self.renderer.end_inline_frame() {
            out.commands.push(Command::SetCompareControl {
                name,
                value: PopupInput::Text(text),
            });
        }
        self.dialogs(ui.ctx(), &mut out);
        if let Some(query) = self.tip.end_frame(ui.ctx(), now) {
            out.queries.push(query);
        }
        out
    }

    /// PoB's "Compare with:" row: the entries, "Load build…" (PoB's "Import...") and Remove.
    fn header(&mut self, ui: &mut egui::Ui, out: &mut CompareOutput) {
        ui.horizontal_wrapped(|ui| {
            if let Some(view) = self.view.clone() {
                ui.label("Compare with:");
                let active = view.entries.get(view.active).map_or("", String::as_str);
                let rects = &mut self.rects;
                let combo = egui::ComboBox::from_id_salt("compare_entries")
                    .selected_text(plain(active))
                    .width(250.0)
                    .show_ui(ui, |ui| {
                        for (i, entry) in view.entries.iter().enumerate() {
                            let item = ui.selectable_label(i == view.active, plain(entry));
                            rects.note(&item, || format!("entry:{i}"));
                            if item.clicked() && i != view.active {
                                out.commands.push(Command::SelectCompare { index: i });
                            }
                        }
                    });
                self.rects.note(&combo.response, || "entries".into());
            }
            let has_file = self.has_file;
            let rects = &mut self.rects;
            let mut open = None;
            let menu = ui.menu_button("Load build…", |ui| {
                let saved = ui.button("From a saved build…");
                rects.note(&saved, || "load:saved".into());
                if saved.clicked() {
                    open = Some(Dialog::Saved);
                }
                let code = ui.button("From a code…");
                rects.note(&code, || "load:code".into());
                if code.clicked() {
                    open = Some(Dialog::Code(String::new()));
                }
                let link = ui
                    .add_enabled(has_file, egui::Button::new("From a link…"))
                    .on_disabled_hover_text(
                        "PoB imports a comparison from a link only into a build saved to a file",
                    );
                rects.note(&link, || "load:link".into());
                if link.clicked() {
                    open = Some(Dialog::Link(String::new()));
                }
            });
            self.rects.note(&menu.response, || "load".into());
            if let Some(dialog) = open {
                self.dialog = Some(dialog);
                ui.close();
            }
            if let Some(view) = &self.view {
                let remove = ui.button("Remove");
                self.rects.note(&remove, || "remove".into());
                if remove.clicked() {
                    out.commands
                        .push(Command::RemoveCompare { index: view.active });
                }
            }
        });
    }

    /// The selector row above the views, drawn from `controls` with SP5b's renderers.
    fn selectors(
        &mut self,
        ui: &mut egui::Ui,
        view: &CompareView,
        busy: bool,
        out: &mut CompareOutput,
    ) {
        for row in [&SET_SELECTORS[..], &SKILL_SELECTORS[..]] {
            ui.horizontal_wrapped(|ui| {
                self.draw_controls(ui, view, row, busy, out);
            });
        }
    }

    fn item_selectors(
        &mut self,
        ui: &mut egui::Ui,
        view: &CompareView,
        busy: bool,
        out: &mut CompareOutput,
    ) {
        ui.horizontal_wrapped(|ui| {
            self.draw_controls(ui, view, &ITEM_SELECTORS, busy, out);
        });
    }

    /// Draws the named controls PoB shows, each after its caption.
    fn draw_controls(
        &mut self,
        ui: &mut egui::Ui,
        view: &CompareView,
        names: &[(&str, &str)],
        busy: bool,
        out: &mut CompareOutput,
    ) {
        for (name, caption) in names {
            let Some(control) = control(view, name).filter(|c| c.shown) else {
                continue;
            };
            if !caption.is_empty() {
                ui.label(*caption);
            }
            let commands = self.renderer.inline_controls(ui, &[control], busy);
            out.commands.extend(commands.into_iter().map(readdress));
        }
    }

    fn sub_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label, key) in [
                (SubTab::Summary, "Summary", "sub:summary"),
                (SubTab::Tree, "Tree", "sub:tree"),
                (SubTab::Items, "Items", "sub:items"),
                (SubTab::Skills, "Skills", "sub:skills"),
                (SubTab::Config, "Config", "sub:config"),
            ] {
                let r = ui.selectable_value(&mut self.sub, tab, label);
                self.rects.note(&r, || key.into());
            }
        });
    }

    /// The Tree view's header: PoB's copy buttons and its overlay box, which stays on (R7
    /// fallback: there is no side-by-side view). The app draws the tree under it.
    fn tree_header(&mut self, ui: &mut egui::Ui, view: &CompareView, out: &mut CompareOutput) {
        ui.horizontal_wrapped(|ui| {
            if let Some(check) = control(view, OVERLAY_CHECK).filter(|c| c.shown) {
                let mut on = check.checked;
                let r = ui
                    .add_enabled(false, egui::Checkbox::new(&mut on, "Overlay comparison"))
                    .on_disabled_hover_text(
                        "PoB's side-by-side view isn't available here: the tree always shows the \
                         comparison as an overlay",
                    );
                self.rects.note(&r, || "overlay".into());
                ui.separator();
            }
            for (and_use, label, key) in [
                (false, "Copy tree", "copy_tree"),
                (true, "Copy and use", "copy_tree_use"),
            ] {
                let r = ui.button(label).on_hover_text(
                    "Adds the comparison's tree to this build's trees, as \"<title> (Compared)\"",
                );
                self.rects.note(&r, || key.into());
                if r.clicked() {
                    out.commands.push(Command::CopyCompareTree { and_use });
                }
            }
        });
        out.tree = true;
    }

    fn dialogs(&mut self, ctx: &egui::Context, out: &mut CompareOutput) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut close = false;
        let mut load = None;
        let rects = &mut self.rects;
        let list = self.list.clone();
        let title = match dialog {
            Dialog::Saved => "Compare with a saved build",
            Dialog::Code(_) => "Compare with a build code",
            Dialog::Link(_) => "Compare with a build link",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| match dialog {
                Dialog::Saved => {
                    let Some(list) = &list else {
                        ui.spinner();
                        return;
                    };
                    let folder = if list.folder.is_empty() {
                        "Builds".to_owned()
                    } else {
                        format!("Builds/{}", list.folder)
                    };
                    ui.label(format!("Builds in {folder}:"));
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (i, entry) in list
                                .entries
                                .iter()
                                .enumerate()
                                .filter(|(_, e)| !e.is_folder)
                            {
                                let r = ui.selectable_label(false, &entry.name);
                                rects.note(&r, || format!("saved:{i}"));
                                if r.clicked() {
                                    load = Some(Command::LoadCompare {
                                        source: CompareSource::File(entry.path.clone()),
                                    });
                                }
                            }
                        });
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
                Dialog::Code(text) | Dialog::Link(text) => {
                    let is_code = title.ends_with("code");
                    if is_code {
                        ui.label("Paste a PoB share code:");
                        let r = ui.add(
                            egui::TextEdit::multiline(text)
                                .desired_rows(4)
                                .desired_width(400.0),
                        );
                        rects.note(&r, || "dialog_text".into());
                    } else {
                        ui.label("Paste a build link:");
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(400.0));
                        rects.note(&r, || "dialog_text".into());
                    }
                    ui.horizontal(|ui| {
                        let go =
                            ui.add_enabled(!text.trim().is_empty(), theme::primary_button("Load"));
                        rects.note(&go, || "dialog_load".into());
                        if go.clicked() {
                            let text = text.trim().to_owned();
                            load = Some(if is_code {
                                Command::LoadCompare {
                                    source: CompareSource::Code(text),
                                }
                            } else {
                                Command::ImportFromUrl {
                                    url: text,
                                    mode: ImportMode::Comparison,
                                }
                            });
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            });
        if let Some(command) = load {
            out.commands.push(command);
            close = true;
        }
        if close {
            self.dialog = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn rect(&self, key: &str) -> egui::Rect {
        *self
            .rects
            .map
            .get(key)
            .or_else(|| self.renderer.rects.map.get(key))
            .unwrap_or_else(|| panic!("no widget {key}"))
    }

    #[cfg(test)]
    pub(crate) fn has_rect(&self, key: &str) -> bool {
        self.rects.map.contains_key(key) || self.renderer.rects.map.contains_key(key)
    }

    #[cfg(test)]
    pub(crate) fn forget_rects(&mut self) {
        self.rects.map.clear();
        self.renderer.rects.map.clear();
    }
}

/// The control of `view` named `name`.
fn control<'a>(view: &'a CompareView, name: &str) -> Option<&'a PopupControl> {
    view.controls.iter().find(|c| c.name == name)
}

/// The renderers speak to a popup session; these controls are the Compare tab's (R6).
fn readdress(command: Command) -> Command {
    match command {
        Command::SetPopupControl { name, value } => Command::SetCompareControl { name, value },
        other => other,
    }
}

/// PoB's text without its colour codes.
fn plain(text: &str) -> String {
    crate::widgets::pob_text(text, 14.0, egui::Color32::WHITE).text
}
