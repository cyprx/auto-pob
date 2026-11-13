//! The Compare tab (R4-R8): PoB's comparison builds, their entry list and selectors, and the
//! Summary, Tree, Items, Skills and Config views. Every view is the engine's `CompareView`;
//! every change goes out as a command. The tab has no undo of its own: its copies go through
//! the target tab's (R8).

mod config;
mod dialogs;
mod items;
mod skills;
mod summary;
#[cfg(test)]
pub(crate) mod tests;

use super::popup::PopupWindow;
use super::tree::Rects;
use crate::engine::views::{
    BuildBrowseView, BuildListView, CompareView, ConfigView, ItemTooltipView, PopupControl,
};
use crate::engine::{Command, ItemTooltipQuery, PopupInput, Snapshot};
use crate::widgets::{item_tooltip_ui, search_combo_ui};
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
    /// A build file, from this folder below the Builds folder
    Saved { folder: String },
    /// PoB's "Import Comparison Build" popup (CompareTab.lua:1462-1525): a code or a link,
    /// and an optional name
    Import {
        text: String,
        name: String,
        /// Import was pressed: `Some(None)` until the app says which command carries it,
        /// then its number, until it is answered
        sent: Option<Option<u64>>,
        /// PoB's red text in the popup's state line (CompareTab.lua:1476-1479)
        error: Option<String>,
    },
    /// PoB's "Convert to Version" popup, opened by a pick in a Tree view version dropdown
    Convert {
        theirs: bool,
        index: usize,
        label: String,
    },
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
const ITEM_SELECTORS: [(&str, &str); 3] = [
    ("primaryItemSetSelect", "Item set:"),
    ("compareItemSetSelect2", "Their item set:"),
    ("itemsExpandedCheck", ""),
];
/// PoB's "Expanded mode" box: the Items view shows both items' tooltips inline
const EXPANDED_CHECK: &str = "itemsExpandedCheck";
/// The Tree view's version dropdowns, ours and theirs
const VERSION_SELECTS: [(&str, &str, bool); 2] = [
    ("leftVersionSelect", "Our tree version:", false),
    ("rightVersionSelect", "Their tree version:", true),
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
    /// The builds of the folder the saved-build picker shows
    browse: Option<Arc<BuildBrowseView>>,
    /// Our tree sets' titles and the active one (PoB's `primaryTreeSetSelect` on Items)
    our_specs: Option<(Vec<String>, usize)>,
    /// The engine hasn't caught up: buttons that act on a view wait for the next one
    busy: bool,
    tip: ItemTip,
    /// The Equip buttons' stat comparison, asked for while one is hovered
    /// (CompareTab.lua:3966-4009)
    equip_tip: crate::ui::hover::HoverTip,
    /// PoB switched to its Compare tab and the app hasn't followed yet
    switched: bool,
    pub(crate) rects: Rects,
    /// Ctrl+F: the Config view's search takes the keyboard when next drawn
    focus_search: bool,
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

    /// Ctrl+F: the Config view's search, when that view shows a comparison
    /// (CompareTab.lua:1686-1689)
    pub fn focus_search(&mut self) {
        self.focus_search = self.view.is_some() && self.sub == SubTab::Config;
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

    /// The Import popup's command went out as `seq`
    pub fn import_sent(&mut self, seq: u64) {
        if let Some(Dialog::Import { sent, .. }) = &mut self.dialog
            && *sent == Some(None)
        {
            *sent = Some(Some(seq));
        }
    }

    /// An error for command `seq`: the Import popup's own shows in it, in red, and the popup
    /// stays for another try, as PoB's does (CompareTab.lua:1507-1514). Returns whether it was.
    pub fn import_failed(&mut self, seq: u64, message: &str) -> bool {
        match &mut self.dialog {
            Some(Dialog::Import { sent, error, .. }) if *sent == Some(Some(seq)) => {
                *sent = None;
                *error = Some(message.to_owned());
                true
            }
            _ => false,
        }
    }

    /// Every snapshot up to `seq` is here: an Import with no error has done its work (a code
    /// added the comparison, a link's download is out), and the popup closes as PoB's does
    pub fn import_settled(&mut self, seq: u64) {
        if let Some(Dialog::Import {
            sent: Some(Some(at)),
            ..
        }) = &self.dialog
            && *at <= seq
        {
            self.dialog = None;
        }
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
                    self.equip_tip.forget();
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
            Snapshot::BuildBrowse(view) => self.browse = Some(view.clone()),
            Snapshot::Tree(state) => {
                let titles = state.specs.iter().map(|s| s.title.clone()).collect();
                self.our_specs = Some((titles, state.active_spec));
            }
            Snapshot::Items(view) => {
                self.item_names = view
                    .items
                    .iter()
                    .map(|e| (e.id, format!("{}{}", e.colour, e.name)))
                    .collect();
            }
            Snapshot::ItemTooltip(view) => {
                self.tip.on_reply(view);
            }
            Snapshot::HoverTooltip(view) => self.equip_tip.on_view(view),
            _ => {}
        }
    }

    /// `busy`: the engine hasn't caught up with what was sent.
    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool) -> CompareOutput {
        let mut out = CompareOutput::default();
        let now = ui.input(|i| i.time);
        self.busy = busy;
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
            // PoB draws the label and the drop-down always, the drop-down disabled while there
            // is no comparison (CompareTab.lua:228-235)
            ui.label("Compare with:");
            let view = self.view.clone();
            let entries = view.as_ref().map_or(&[][..], |v| &v.entries[..]);
            let active = view.as_ref().map_or(0, |v| v.active);
            let label = entries.get(active).map_or("", String::as_str);
            let rects = &mut self.rects;
            let busy = self.busy;
            ui.add_enabled_ui(view.is_some(), |ui| {
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt("compare_entries")
                    .selected_text(plain(label))
                    .width(250.0)
                    .show_ui(ui, |ui| {
                        for (i, entry) in entries.iter().enumerate() {
                            let item =
                                crate::widgets::dropdown::item(ui, i == active, plain(entry));
                            rects.note(&item, || format!("entry:{i}"));
                            if item.clicked() && i != active && !busy {
                                out.commands.push(Command::SelectCompare { index: i });
                            }
                        }
                    });
                rects.note(&combo.response, || "entries".into());
                // the wheel and the arrows step it (DropDownControl.lua:476-516)
                if !busy
                    && view.is_some()
                    && let Some(index) = crate::widgets::scroll_step::combo_step(
                        ui,
                        &combo.response,
                        entries.len(),
                        active,
                    )
                {
                    out.commands.push(Command::SelectCompare { index });
                }
            });
            let rects = &mut self.rects;
            let mut open = None;
            let menu = ui.menu_button("Load build…", |ui| {
                let saved = ui.button("From a saved build…");
                rects.note(&saved, || "load:saved".into());
                if saved.clicked() {
                    open = Some(Dialog::Saved {
                        folder: String::new(),
                    });
                }
                // PoB's "Import..." popup takes a code or a link, with or without a file
                let code = ui.button("From a code or link…");
                rects.note(&code, || "load:code".into());
                if code.clicked() {
                    open = Some(Dialog::Import {
                        text: String::new(),
                        name: String::new(),
                        sent: None,
                        error: None,
                    });
                }
            });
            self.rects.note(&menu.response, || "load".into());
            if let Some(dialog) = open {
                if let Dialog::Saved { folder } = &dialog {
                    out.queries.push(Command::BrowseBuilds {
                        folder: folder.clone(),
                    });
                }
                self.dialog = Some(dialog);
                ui.close();
            }
            // PoB's Remove is there always, enabled with a comparison (CompareTab.lua:274-282)
            let active = self.view.as_ref().map(|v| v.active);
            let remove =
                ui.add_enabled(active.is_some() && !self.busy, egui::Button::new("Remove"));
            self.rects.note(&remove, || "remove".into());
            if remove.clicked()
                && let Some(index) = active
            {
                out.commands.push(Command::RemoveCompare { index });
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
            // PoB's `primaryTreeSetSelect`: our tree sets, as the Tree tab's own selector
            if let Some((titles, active)) = self.our_specs.clone() {
                ui.label("Tree set:");
                let id = ui.make_persistent_id("compare_our_tree_set");
                let rects = &mut self.rects;
                let (r, picked) =
                    search_combo_ui(ui, id, &titles, active, Some(200.0), &mut |i, e| {
                        rects.note(e, || format!("our_spec:{i}"));
                    });
                self.rects.note(&r, || "our_spec".into());
                if let Some(index) = picked
                    && !busy
                {
                    out.commands.push(Command::SelectSpec { index });
                }
            }
        });
    }

    /// Expanded mode is on (PoB's `itemsExpandedCheck`)
    fn expanded_on(view: &CompareView) -> bool {
        control(view, EXPANDED_CHECK).is_some_and(|c| c.shown && c.checked)
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
                // PoB's order: Skills before Items (CompareTab.lua:197)
                (SubTab::Skills, "Skills", "sub:skills"),
                (SubTab::Items, "Items", "sub:items"),
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
            for (name, caption, theirs) in VERSION_SELECTS {
                let Some(select) = control(view, name).filter(|c| c.shown) else {
                    continue;
                };
                ui.label(caption);
                let id = ui.make_persistent_id(("compare_version", name));
                let rects = &mut self.rects;
                let mut picked = None;
                ui.add_enabled_ui(select.enabled && !self.busy, |ui| {
                    let (r, pick) = search_combo_ui(
                        ui,
                        id,
                        &select.choices,
                        select.selected,
                        Some(100.0),
                        &mut |i, e| rects.note(e, || format!("{name}_{i}")),
                    );
                    rects.note(&r, || name.to_owned());
                    picked = pick;
                });
                // PoB asks first, in its "Convert to Version" popup
                if let Some(index) = picked {
                    self.dialog = Some(Dialog::Convert {
                        theirs,
                        index,
                        label: select.choices[index].clone(),
                    });
                }
                ui.separator();
            }
            for (and_use, label, key) in [
                (false, "Copy tree", "copy_tree"),
                (true, "Copy and use", "copy_tree_use"),
            ] {
                let r = ui.add_enabled(!self.busy, egui::Button::new(label)).on_hover_text(
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
        Command::Step(inner) => Command::Step(Box::new(readdress(*inner))),
        other => other,
    }
}

/// A view's column header as PoB draws it, its colour code kept
fn header(text: &str, default: egui::Color32) -> egui::text::LayoutJob {
    crate::widgets::pob_text(text, 16.0, default)
}

/// PoB's text without its colour codes.
fn plain(text: &str) -> String {
    crate::widgets::pob_text(text, 14.0, egui::Color32::WHITE).text
}
