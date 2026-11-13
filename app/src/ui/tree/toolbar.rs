//! The Tree tab's toolbar and the dialogs it opens: trees, reset, search,
//! the timeless jewel search, heat map, the power report toggle and the link menu.

use super::{Rects, TreeOutput};
use crate::engine::{Command, DropdownOwner, HoverTarget, PopupKind, TreeState};
use crate::theme;
use crate::ui::items::list::reorder_rows;
use crate::ui::name_box::{self, Filter};
use crate::widgets::pob_text;
use eframe::egui;

/// A search edit is sent once typing has paused this long (seconds).
pub(super) const SEARCH_DEBOUNCE: f64 = 0.15;

/// The depths the heat map can be limited to, as the dropdown lists them.
const DEPTHS: [u32; 3] = [5, 10, 15];

/// What the tab knows about the node power calculation.
pub(super) struct PowerStatus {
    pub progress: Option<u32>,
    pub failed: bool,
}

/// A question the toolbar is waiting on.
#[derive(Debug, PartialEq)]
enum Dialog {
    NewSpec(String),
    RenameSpec {
        index: usize,
        title: String,
        /// typed into: PoB's Save waits for it (PassiveSpecListControl.lua:50-52)
        edited: bool,
    },
    DeleteSpec(usize),
    /// PoB's "Reset Tree/Tattoos": Reset Tree or Remove All Tattoos, which can't be undone
    Reset,
    /// PoB's "Export Tree" popup (TreeTab.lua:812-839): the link, Shrink with PoEURL, Copy
    Export {
        /// The link shown, once it is here
        link: String,
        shrink: Shrink,
    },
    Import {
        title: String,
        url: String,
    },
    /// The tree manager's Copy and its "Copy Tree" name popup (NF-3)
    CopySpec {
        index: usize,
        title: String,
        /// typed into: PoB's Save waits for it (PassiveSpecListControl.lua:44-46)
        edited: bool,
    },
    /// PoB's "Convert to Version" popup for a Version dropdown pick (NF-7)
    ConvertVersion {
        version: String,
        label: String,
    },
    /// PoB's "Class Change" for a click on another class's ascendancy node
    /// (PassiveTreeView.lua:474)
    ClassChange {
        node: u32,
        class: String,
    },
}

/// The search box's tooltip (TreeTab.lua:214)
const SEARCH_TIP: &str = "Uses Lua pattern matching for complex searches.\nPrefix your search with \"oil:\" to search by anoint recipe.\nTo search for multiple terms: (increased.fire.damage|increased.area.of.effect|etc)";

/// The depth dropdown's tooltip (TreeTab.lua:263)
const DEPTH_TIP: &str = "Limit of Node distance to search (lower = faster)";

/// The Export popup's "Shrink with PoEURL" button (TreeTab.lua:818-831)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shrink {
    /// Not pressed yet
    Idle,
    /// Pressed; its request isn't listed yet
    Asked,
    /// Its request is running
    Running,
    /// The request ended, with the short link or PoB's failure message
    Done,
}

/// The label of the shrink request, as the engine lists it
const SHRINK_REQUEST: &str = "Shrinking tree link";

/// What a dragged tree row carries: its position
struct SpecDrag(usize);

#[derive(Default)]
pub(super) struct Toolbar {
    search: String,
    /// The engine's search text last frame, to notice when it changes by itself (undo)
    search_seen: String,
    /// When the search text was last edited, while that edit hasn't been sent
    search_edit: Option<f64>,
    pub(super) report_open: bool,
    dialog: Option<Dialog>,
    /// Choosing this class would drop points that don't connect to it: asked by the top bar's
    /// class dropdown, on every tab (id, name)
    class_dialog: Option<(u32, String)>,
    /// "Copy link" was clicked and the link has not arrived yet
    copy_pending: bool,
    link_fresh: bool,
    /// The tree selector was pointed at or open when drawn last: Up and Down leave the trees
    /// alone (TreeTab.lua:367-371)
    pub(super) spec_busy: bool,
    /// Ctrl+F: the search box takes the keyboard when next drawn
    pub(super) focus_search: bool,
    /// The heat map depth's Custom box, while chosen (NF-6)
    custom_depth: Option<String>,
    /// PoB's "Manage Passive Trees" window (TreeTab.lua:630), while open: its list's selected
    /// row, which Copy, Rename, Delete and F2 act on (NF-3)
    manager: Option<usize>,
    /// The tree selector's tooltip (TreeTab.lua:51-104)
    pub(super) spec_tip: crate::ui::hover::HoverTip,
    /// PoB's "Node Power colours", which the heat map's tooltip names
    pub(super) power_theme: super::overlays::PowerTheme,
    /// Its questions, moved to the output when the toolbar is done
    tip_queries: Vec<Command>,
}

impl Toolbar {
    /// The search box's text, for tests
    #[cfg(test)]
    pub(super) fn search_for_test(&self) -> &str {
        &self.search
    }

    /// The Custom depth box's text, for tests
    #[cfg(test)]
    pub(super) fn custom_depth_for_test(&self) -> Option<&str> {
        self.custom_depth.as_deref()
    }

    /// A click on another class's ascendancy node whose class needs the question: PoB asks
    /// "Class Change" before anything changes.
    pub(super) fn ask_class_change(&mut self, node: u32, class: String) {
        self.dialog = Some(Dialog::ClassChange { node, class });
    }

    #[cfg(test)]
    pub(super) fn has_question(&self) -> bool {
        self.dialog.is_some()
    }

    #[cfg(test)]
    pub(super) fn class_question(&self) -> bool {
        self.class_dialog.is_some()
    }

    pub(super) fn has_dialog(&self) -> bool {
        self.dialog.is_some() || self.class_dialog.is_some() || self.manager.is_some()
    }

    /// Opens PoB's "Manage Passive Trees" with the active tree selected (TreeTab.lua:630:
    /// the Manage button, Ctrl+M and the Items tab's "Manage...")
    pub(super) fn open_manager(&mut self, state: &TreeState) {
        self.manager = Some(state.active_spec);
    }

    pub(super) fn manager_open(&self) -> bool {
        self.manager.is_some()
    }

    /// Closes the manager and the question it asked, if any
    pub(super) fn close_manager(&mut self) {
        if self.manager.take().is_some() {
            self.dialog = None;
        }
    }

    /// Drops the tab's own dialog; the class question belongs to the top bar and stays.
    pub(super) fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// Closes every question but the Class Change one, which the Compare tab's tree view asks
    /// too
    pub(super) fn close_dialog_but_class_change(&mut self) {
        if !matches!(self.dialog, Some(Dialog::ClassChange { .. })) {
            self.dialog = None;
        }
    }

    /// The Compare tab's tree view: its Class Change question, the only one it asks
    pub(super) fn compare_dialogs(
        &mut self,
        ctx: &egui::Context,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        if matches!(self.dialog, Some(Dialog::ClassChange { .. })) {
            self.dialogs(ctx, state, rects, commands);
        }
    }

    /// The engine's running requests: the Export popup's shrink is done when its request
    /// has come and gone, PoB's failure message being the notice
    pub(super) fn on_requests(&mut self, pending: &[&str]) {
        if let Some(Dialog::Export { shrink, .. }) = &mut self.dialog {
            let running = pending.contains(&SHRINK_REQUEST);
            *shrink = match (*shrink, running) {
                (Shrink::Asked, true) => Shrink::Running,
                (Shrink::Running, false) => Shrink::Done,
                (other, _) => other,
            };
        }
    }

    /// A command failed: a shrink asked for but never listed as a request (PoB refused the
    /// press) ends, its button reading Done as PoB's always does (TreeTab.lua:820-826)
    pub(super) fn on_command_error(&mut self) {
        if let Some(Dialog::Export { shrink, .. }) = &mut self.dialog
            && *shrink == Shrink::Asked
        {
            *shrink = Shrink::Done;
        }
    }

    /// A link snapshot arrived.
    pub(super) fn link_arrived(&mut self) {
        self.link_fresh = true;
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        power: &PowerStatus,
        link: Option<&str>,
        rects: &mut Rects,
        out: &mut TreeOutput,
    ) {
        if self.copy_pending
            && self.link_fresh
            && let Some(link) = link
        {
            ui.ctx().copy_text(link.to_owned());
            self.copy_pending = false;
        }
        // the Export popup shows the link once it is here; a short one ends the shrink
        if let Some(Dialog::Export {
            link: shown,
            shrink,
        }) = &mut self.dialog
            && self.link_fresh
            && let Some(link) = link
            && shown != link
        {
            if link.starts_with("http://poeurl.com/") {
                *shrink = Shrink::Done;
            }
            *shown = link.to_owned();
        }
        let commands = &mut out.commands;
        ui.horizontal_wrapped(|ui| {
            self.specs(ui, state, rects, commands, true);
            ui.separator();
            // PoB's one button asks Reset Tree, Remove All Tattoos or Cancel (TreeTab.lua:
            // 130-154)
            let reset = ui.button("Reset Tree/Tattoos");
            rects.note(&reset, || "reset".into());
            if reset.clicked() {
                self.dialog = Some(Dialog::Reset);
            }
            // PoB's Version dropdown: any listed version converts the tree (NF-7)
            if !state.versions.is_empty() {
                ui.label("Version:");
                let combo = crate::widgets::dropdown::Dropdown::from_id_salt("tree_version")
                    .selected_text(&state.version_label)
                    .show_ui(ui, |ui| {
                        for choice in &state.versions {
                            let current = choice.value == state.version;
                            let item = crate::widgets::dropdown::item(ui, current, &choice.label);
                            rects.note(&item, || format!("version:{}", choice.value));
                            if item.clicked() && !current {
                                self.dialog = Some(Dialog::ConvertVersion {
                                    version: choice.value.clone(),
                                    label: choice.label.clone(),
                                });
                            }
                        }
                    });
                rects.note(&combo.response, || "version".into());
                // the wheel and the arrows step it as a pick: the Convert question asks
                let at = state
                    .versions
                    .iter()
                    .position(|c| c.value == state.version)
                    .unwrap_or(0);
                if let Some(i) = crate::widgets::scroll_step::combo_step(
                    ui,
                    &combo.response,
                    state.versions.len(),
                    at,
                ) && state.versions[i].value != state.version
                {
                    self.dialog = Some(Dialog::ConvertVersion {
                        version: state.versions[i].value.clone(),
                        label: state.versions[i].label.clone(),
                    });
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            self.search(ui, state, rects, commands);
            // PoB's "Find Timeless Jewel", next to the search as PoB places it (R17)
            let timeless = ui
                .button("Find timeless jewel\u{2026}")
                .on_hover_text("Search the seeds of a timeless jewel for the nodes you want");
            rects.note(&timeless, || "find_timeless".into());
            if timeless.clicked() {
                commands.push(Command::OpenPopup(PopupKind::TimelessJewel));
            }
            ui.separator();
            self.heatmap(ui, state, power, rects, commands);
            ui.separator();
            self.link_menu(ui, state, rects, commands, &mut out.queries);
        });
        self.dialogs(ui.ctx(), state, rects, commands);
        out.queries.append(&mut self.tip_queries);
    }

    /// The tree set row. `manage`: the Tree tab's own, with its New, Rename, Delete, Copy and
    /// Manage buttons; the Compare tab's tree view has none of them (CompareTab.lua:840-900)
    pub(super) fn specs(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
        manage: bool,
    ) {
        let active = state.specs.get(state.active_spec);
        let mut items = Vec::new();
        let combo = crate::widgets::dropdown::Dropdown::from_id_salt("tree_spec")
            .selected_text(active.map_or("", |s| s.title.as_str()))
            .show_ui(ui, |ui| {
                for (i, spec) in state.specs.iter().enumerate() {
                    let item = crate::widgets::dropdown::item(
                        ui,
                        i == state.active_spec,
                        format!("{} ({} points)", spec.title, spec.points),
                    );
                    rects.note(&item, || format!("spec:{i}"));
                    if item.clicked() && i != state.active_spec {
                        commands.push(Command::SelectSpec { index: i });
                    }
                    items.push((i, item));
                }
            });
        rects.note(&combo.response, || "spec_combo".into());
        // PoB's tooltip on the selector and its entries: the tree's class, points, sockets and
        // what switching to it gives (TreeTab.lua:51-104); the Compare tab's has none
        if manage {
            let now = ui.input(|i| i.time);
            let target = |index, entry| HoverTarget::Dropdown {
                owner: DropdownOwner::Tree,
                name: "specSelect".into(),
                index,
                entry,
            };
            for (i, item) in &items {
                self.spec_tip
                    .track(item, target(*i, true), now, &mut self.tip_queries);
            }
            self.spec_tip.track(
                &combo.response,
                target(state.active_spec, false),
                now,
                &mut self.tip_queries,
            );
        }
        // the wheel and the arrows step it (DropDownControl.lua:476-516)
        if let Some(index) = crate::widgets::scroll_step::combo_step(
            ui,
            &combo.response,
            state.specs.len(),
            state.active_spec,
        ) {
            commands.push(Command::SelectSpec { index });
        }
        self.spec_busy = combo.response.hovered()
            || egui::Popup::is_id_open(ui.ctx(), combo.response.id.with("popup"));
        if manage {
            let add = ui.button("+").on_hover_text("Add a tree");
            rects.note(&add, || "spec_new".into());
            if add.clicked() {
                self.dialog = Some(Dialog::NewSpec(String::new()));
            }
            let rename = ui.add_enabled(active.is_some(), egui::Button::new("Rename"));
            rects.note(&rename, || "spec_rename".into());
            if rename.clicked()
                && let Some(spec) = active
            {
                self.dialog = Some(Dialog::RenameSpec {
                    index: state.active_spec,
                    title: spec.name_box_text(),
                    edited: false,
                });
            }
            let delete = ui.add_enabled(state.specs.len() > 1, egui::Button::new("Delete"));
            rects.note(&delete, || "spec_delete".into());
            if delete.clicked() {
                self.dialog = Some(Dialog::DeleteSpec(state.active_spec));
            }
        }
        // PoB's note and button under an older tree (TreeTab.lua:321-337, TreeTab:610)
        if let Some(latest) = &state.convertible_to {
            let label = state
                .versions
                .iter()
                .find(|c| &c.value == latest)
                .map_or(latest.as_str(), |c| c.label.as_str());
            let convert = ui.button(format!("Convert to {label}")).on_hover_text(
                "This is an older tree version, which may not be fully compatible with the \
                     current game version.",
            );
            rects.note(&convert, || "spec_convert_latest".into());
            if convert.clicked() {
                commands.push(Command::ConvertSpecLatest);
            }
        }
        if manage {
            let copy = ui.add_enabled(active.is_some(), egui::Button::new("Copy"));
            rects.note(&copy, || "spec_copy".into());
            if copy.clicked()
                && let Some(spec) = active
            {
                self.dialog = Some(Dialog::CopySpec {
                    index: state.active_spec,
                    title: spec.name_box_text(),
                    edited: false,
                });
            }
            let order = ui.button("Manage\u{2026}");
            rects.note(&order, || "spec_order".into());
            if order.clicked() {
                self.open_manager(state);
            }
        }
        ui.label("Compare");
        let compare = crate::widgets::dropdown::Dropdown::from_id_salt("tree_compare")
            .selected_text(
                state
                    .compare_spec
                    .and_then(|i| state.specs.get(i))
                    .map_or("None", |s| s.title.as_str()),
            )
            .show_ui(ui, |ui| {
                let none = crate::widgets::dropdown::item(ui, state.compare_spec.is_none(), "None");
                rects.note(&none, || "compare:none".into());
                if none.clicked() && state.compare_spec.is_some() {
                    commands.push(Command::SetCompareSpec { index: None });
                }
                for (i, spec) in state.specs.iter().enumerate() {
                    if i == state.active_spec {
                        continue;
                    }
                    let item = crate::widgets::dropdown::item(
                        ui,
                        state.compare_spec == Some(i),
                        &spec.title,
                    );
                    rects.note(&item, || format!("compare:{i}"));
                    if item.clicked() && state.compare_spec != Some(i) {
                        commands.push(Command::SetCompareSpec { index: Some(i) });
                    }
                }
            });
        rects.note(&compare.response, || "compare_combo".into());
        // its entries: None, then every tree but the active one
        let entries: Vec<Option<usize>> = std::iter::once(None)
            .chain(
                (0..state.specs.len())
                    .filter(|&i| i != state.active_spec)
                    .map(Some),
            )
            .collect();
        let at = entries
            .iter()
            .position(|e| *e == state.compare_spec)
            .unwrap_or(0);
        if let Some(i) =
            crate::widgets::scroll_step::combo_step(ui, &compare.response, entries.len(), at)
        {
            commands.push(Command::SetCompareSpec { index: entries[i] });
        }
    }

    /// PoB's point display (`pointDisplay`), in the top bar; `progress` is its tooltip
    pub(super) fn points(&self, ui: &mut egui::Ui, state: &TreeState, progress: &str) {
        let p = state.points;
        let colour = |over: bool| {
            if over {
                theme::NEGATIVE
            } else {
                theme::TEXT_DIM
            }
        };
        let points = ui.label(
            egui::RichText::new(format!("Points {}/{}", p.used, p.available))
                .color(colour(p.used > p.available)),
        );
        let ascendancy = ui.label(
            egui::RichText::new(format!(
                "Ascendancy {}/{}",
                p.ascendancy_used, p.ascendancy_available
            ))
            .color(colour(p.ascendancy_used > p.ascendancy_available)),
        );
        if !progress.is_empty() {
            let default = ui.visuals().text_color();
            for response in [points, ascendancy] {
                response.on_hover_ui(|ui| {
                    ui.label(pob_text(progress, 14.0, default));
                });
            }
        }
    }

    /// PoB's class and ascendancy dropdowns (`classDrop`, `ascendDrop`), in the top bar
    pub(super) fn class(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let class = state.classes.iter().find(|c| c.id == state.class_id);
        let combo = crate::widgets::dropdown::Dropdown::from_id_salt("tree_class")
            .selected_text(class.map_or("Class", |c| c.name.as_str()))
            .show_ui(ui, |ui| {
                for c in &state.classes {
                    let item = crate::widgets::dropdown::item(ui, c.id == state.class_id, &c.name);
                    rects.note(&item, || format!("class:{}", c.id));
                    if item.clicked() && c.id != state.class_id {
                        if c.needs_confirm {
                            self.class_dialog = Some((c.id, c.name.clone()));
                        } else {
                            commands.push(Command::SelectClass {
                                id: c.id,
                                connect: false,
                            });
                        }
                    }
                }
            });
        rects.note(&combo.response, || "class_combo".into());
        let at = state
            .classes
            .iter()
            .position(|c| c.id == state.class_id)
            .unwrap_or(0);
        if let Some(i) =
            crate::widgets::scroll_step::combo_step(ui, &combo.response, state.classes.len(), at)
        {
            let c = &state.classes[i];
            if c.needs_confirm {
                self.class_dialog = Some((c.id, c.name.clone()));
            } else {
                commands.push(Command::SelectClass {
                    id: c.id,
                    connect: false,
                });
            }
        }
        let ascendancies = class.map_or(&[][..], |c| c.ascendancies.as_slice());
        let current = ascendancies.iter().find(|a| a.id == state.ascendancy_id);
        let combo = crate::widgets::dropdown::Dropdown::from_id_salt("tree_ascendancy")
            .selected_text(current.map_or("Ascendancy", |a| a.name.as_str()))
            .show_ui(ui, |ui| {
                for a in ascendancies {
                    let item =
                        crate::widgets::dropdown::item(ui, a.id == state.ascendancy_id, &a.name);
                    rects.note(&item, || format!("ascendancy:{}", a.id));
                    if item.clicked() && a.id != state.ascendancy_id {
                        commands.push(Command::SelectAscendancy { id: a.id });
                    }
                }
            });
        rects.note(&combo.response, || "ascendancy_combo".into());
        let at = ascendancies
            .iter()
            .position(|a| a.id == state.ascendancy_id)
            .unwrap_or(0);
        if let Some(i) =
            crate::widgets::scroll_step::combo_step(ui, &combo.response, ascendancies.len(), at)
        {
            commands.push(Command::SelectAscendancy {
                id: ascendancies[i].id,
            });
        }
    }

    pub(super) fn search(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let now = ui.input(|i| i.time);
        // the engine changed the search itself (an undo): show it, unless an edit is on its way
        if state.search != self.search_seen {
            self.search_seen = state.search.clone();
            if self.search_edit.is_none() {
                self.search = state.search.clone();
            }
        }
        let before = self.search.clone();
        let pasted = name_box::pasted(ui);
        let next = name_box::next_selection(ui);
        let field = ui
            .add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search nodes")
                    .desired_width(160.0),
            )
            .on_hover_text(SEARCH_TIP);
        // PoB's box drops control characters and takes 100 bytes (TreeTab.lua:211)
        if field.changed() {
            let raw = std::mem::take(&mut self.search);
            let sel = name_box::own(next, field.id);
            let keeps = |c: char| !c.is_control();
            self.search = name_box::spliced_at(&before, &raw, sel, keeps, 100, pasted);
            name_box::place_caret(ui.ctx(), field.id, &before, &raw, &self.search, sel);
        }
        crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &field, &self.search);
        rects.note(&field, || "search".into());
        if std::mem::take(&mut self.focus_search) {
            field.request_focus();
        }
        if field.changed() {
            self.search_edit = Some(now);
        }
        if !self.search.is_empty() && ui.small_button("x").clicked() {
            self.search.clear();
            self.search_edit = Some(f64::NEG_INFINITY);
        }
        if let Some(edited) = self.search_edit {
            let waited = now - edited;
            if waited >= SEARCH_DEBOUNCE {
                self.search_edit = None;
                if self.search != state.search {
                    commands.push(Command::SetTreeSearch {
                        text: self.search.clone(),
                    });
                }
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(
                        SEARCH_DEBOUNCE - waited,
                    ));
            }
        }
    }

    fn heatmap(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        power: &PowerStatus,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let set =
            |stat: Option<String>, max_depth: Option<u32>| Command::SetHeatmap { stat, max_depth };
        ui.label("Heat map");
        let combo = crate::widgets::dropdown::Dropdown::from_id_salt("tree_heat_stat")
            .selected_text(state.heatmap.as_deref().unwrap_or(if state.heatmap_shown {
                "Damage / defence"
            } else {
                "Off"
            }))
            .show_ui(ui, |ui| {
                let shown = state.heatmap.is_some() || state.heatmap_shown;
                let off = crate::widgets::dropdown::item(ui, !shown, "Off");
                rects.note(&off, || "heat:off".into());
                if off.clicked() && shown {
                    commands.push(set(None, state.max_depth));
                }
                for stat in &state.power_stats {
                    let selected = state.heatmap.as_ref() == Some(stat);
                    let item = crate::widgets::dropdown::item(ui, selected, stat);
                    rects.note(&item, || format!("heat:{stat}"));
                    if item.clicked() && !selected {
                        commands.push(set(Some(stat.clone()), state.max_depth));
                    }
                }
            });
        // PoB's tooltip on its Show Node Power box (TreeTab.lua:282-285)
        let (offence, defence) = self.power_theme.names();
        let combo_response = combo.response.clone().on_hover_text(format!(
            "When enabled, an estimate of the offensive and defensive strength of\neach \
             unallocated passive is calculated and displayed visually.\nOffensive power shows \
             as {offence}, defensive power as {defence}."
        ));
        rects.note(&combo_response, || "heat_stat".into());
        // its entries: Off, then the stats
        let at = state
            .heatmap
            .as_ref()
            .and_then(|h| state.power_stats.iter().position(|s| s == h))
            .map_or(0, |i| i + 1);
        if let Some(i) = crate::widgets::scroll_step::combo_step(
            ui,
            &combo.response,
            state.power_stats.len() + 1,
            at,
        ) {
            commands.push(set(
                i.checked_sub(1).map(|i| state.power_stats[i].clone()),
                state.max_depth,
            ));
        }
        // a depth PoB's list doesn't have was typed into its Custom box
        if self.custom_depth.is_none()
            && let Some(depth) = state.max_depth.filter(|d| !DEPTHS.contains(d))
        {
            self.custom_depth = Some(depth.to_string());
        }
        // PoB's depth dropdown is always there, the heat map on or off (TreeTab.lua:202-235);
        // only P's map with no stat chosen can't take a depth here without losing its stat
        ui.add_enabled_ui(state.heatmap.is_some() || !state.heatmap_shown, |ui| {
            let shown = match (&self.custom_depth, state.max_depth) {
                (Some(_), _) => "Custom".to_owned(),
                (None, Some(d)) => format!("Within {d}"),
                (None, None) => "All".to_owned(),
            };
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("tree_heat_depth")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    let custom = self.custom_depth.is_some();
                    let all = crate::widgets::dropdown::item(
                        ui,
                        !custom && state.max_depth.is_none(),
                        "All",
                    );
                    rects.note(&all, || "depth:all".into());
                    if all.clicked() {
                        self.custom_depth = None;
                        if state.max_depth.is_some() {
                            commands.push(set(state.heatmap.clone(), None));
                        }
                    }
                    for depth in DEPTHS {
                        let item = crate::widgets::dropdown::item(
                            ui,
                            !custom && state.max_depth == Some(depth),
                            depth.to_string(),
                        );
                        rects.note(&item, || format!("depth:{depth}"));
                        if item.clicked() {
                            self.custom_depth = None;
                            if state.max_depth != Some(depth) {
                                commands.push(set(state.heatmap.clone(), Some(depth)));
                            }
                        }
                    }
                    // PoB's "Custom" entry shows a box for any depth (NF-6)
                    let item = crate::widgets::dropdown::item(ui, custom, "Custom");
                    rects.note(&item, || "depth:custom".into());
                    if item.clicked() && !custom {
                        self.custom_depth =
                            Some(state.max_depth.map(|d| d.to_string()).unwrap_or_default());
                    }
                });
            let combo_response = combo.response.clone().on_hover_text(DEPTH_TIP);
            rects.note(&combo_response, || "heat_depth".into());
            // its entries: All, then the depths, then Custom
            let n = DEPTHS.len() + 2;
            let at = if self.custom_depth.is_some() {
                n - 1
            } else {
                state
                    .max_depth
                    .and_then(|d| DEPTHS.iter().position(|x| *x == d))
                    .map_or(0, |i| i + 1)
            };
            if let Some(i) = crate::widgets::scroll_step::combo_step(ui, &combo.response, n, at) {
                if i == n - 1 {
                    self.custom_depth =
                        Some(state.max_depth.map(|d| d.to_string()).unwrap_or_default());
                } else {
                    self.custom_depth = None;
                    let depth = i.checked_sub(1).map(|i| DEPTHS[i]);
                    if depth != state.max_depth {
                        commands.push(set(state.heatmap.clone(), depth));
                    }
                }
            }
            if let Some(text) = &mut self.custom_depth {
                let field = ui.add(
                    egui::TextEdit::singleline(text)
                        .desired_width(50.0)
                        .hint_text("depth"),
                );
                rects.note(&field, || "depth_custom".into());
                crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &field, text);
                // PoB's box takes digits only ("%D")
                text.retain(|c| c.is_ascii_digit());
                // and, as every %D box, steps on + and -, Up, Down and the wheel, sending at
                // once (EditControl.lua:64-75, 680-700)
                let mut step = crate::widgets::scroll_step::number_step(ui, &field);
                ui.spacing_mut().item_spacing.x = 1.0;
                let plus = ui.small_button("+");
                rects.note(&plus, || "depth_custom_plus".into());
                let minus = ui.small_button("-");
                step += i32::from(plus.clicked()) - i32::from(minus.clicked());
                for _ in 0..step.unsigned_abs() {
                    *text = crate::widgets::edit_box::stepped(text, step > 0, false);
                }
                if step != 0 || field.lost_focus() {
                    // an empty box is no limit, as PoB's `tonumber("")`
                    let depth = text.parse::<u32>().ok();
                    if depth != state.max_depth {
                        commands.push(set(state.heatmap.clone(), depth));
                    }
                }
            }
        });
        if power.failed {
            ui.colored_label(theme::NEGATIVE, "calculation failed");
            let retry = ui.button("Retry");
            rects.note(&retry, || "heat_retry".into());
            if retry.clicked() {
                commands.push(set(state.heatmap.clone(), state.max_depth));
            }
        } else if let Some(percent) = power.progress {
            ui.add(
                egui::ProgressBar::new(percent as f32 / 100.0)
                    .desired_width(100.0)
                    .text(format!("{percent}%")),
            );
        }
        let before = self.report_open;
        let toggle = ui.toggle_value(&mut self.report_open, "Power report");
        rects.note(&toggle, || "report_toggle".into());
        // the report is built from the heat map's calculation
        if self.report_open
            && !before
            && state.heatmap.is_none()
            && let Some(stat) = state.power_stats.first()
        {
            commands.push(set(Some(stat.clone()), state.max_depth));
        }
    }

    fn link_menu(
        &mut self,
        ui: &mut egui::Ui,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
        queries: &mut Vec<Command>,
    ) {
        let menu = ui.menu_button("Link…", |ui| {
            let import = ui.button("Import Tree…");
            rects.note(&import, || "link_import".into());
            if import.clicked() {
                self.dialog = Some(Dialog::Import {
                    title: String::new(),
                    url: String::new(),
                });
            }
            // PoB's Export Tree popup, with its PoEURL shrink
            let export = ui.button("Export Tree…");
            rects.note(&export, || "link_export".into());
            if export.clicked() {
                self.dialog = Some(Dialog::Export {
                    link: String::new(),
                    shrink: Shrink::Idle,
                });
                self.link_fresh = false;
                queries.push(Command::ExportTreeLink);
            }
            let copy = ui.button("Copy link");
            rects.note(&copy, || "link_copy".into());
            if copy.clicked() {
                self.copy_pending = true;
                self.link_fresh = false;
                queries.push(Command::ExportTreeLink);
            }
            if let Some(version) = &state.convertible_to {
                ui.separator();
                for (key, label, keep_copy) in [
                    ("link_convert", "Convert this tree", false),
                    ("link_convert_copy", "Convert this tree (keep copy)", true),
                ] {
                    let item = ui.button(label);
                    rects.note(&item, || key.into());
                    if item.clicked() {
                        commands.push(Command::ConvertSpec {
                            version: version.clone(),
                            keep_copy,
                        });
                    }
                }
                let all = ui.button("Convert all trees");
                rects.note(&all, || "link_convert_all".into());
                if all.clicked() {
                    commands.push(Command::ConvertAllSpecs {
                        version: version.clone(),
                    });
                }
            }
        });
        rects.note(&menu.response, || "link_menu".into());
    }

    /// The class question, asked from the top bar on any tab
    pub(super) fn class_dialog(
        &mut self,
        ctx: &egui::Context,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let Some((id, name)) = &self.class_dialog else {
            return;
        };
        let (id, name) = (*id, name.clone());
        // Esc cancels it, as every PoB popup (PopupDialog.lua:72-77)
        let window = egui::Id::unique("tree_class_dialog");
        let mut close = crate::widgets::escape::escape(ctx, window);
        // PoB's question (Build.lua:263)
        egui::Window::new("Class Change")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Changing class to {name} will reset your passive tree.\nThis can be \
                     avoided by connecting one of the {name} starting nodes to your tree."
                ));
                ui.horizontal(|ui| {
                    if button(ui, rects, "class_reset", "Continue", true) {
                        commands.push(Command::SelectClass { id, connect: false });
                        close = true;
                    }
                    if button(ui, rects, "class_connect", "Connect Path", true) {
                        commands.push(Command::SelectClass { id, connect: true });
                        close = true;
                    }
                    close |= ui.button("Cancel").clicked();
                });
            });
        crate::widgets::escape::note(ctx, window);
        if close {
            self.class_dialog = None;
        }
    }

    /// The manager and the questions it asks (New, Copy, Rename, Delete tree), for the Items
    /// tab's "Manage..." (ItemsTab.lua:203-205)
    pub(super) fn manager_and_dialogs(
        &mut self,
        ctx: &egui::Context,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        if self.dialog.is_none()
            || matches!(
                self.dialog,
                Some(
                    Dialog::NewSpec(_)
                        | Dialog::RenameSpec { .. }
                        | Dialog::DeleteSpec(_)
                        | Dialog::CopySpec { .. }
                )
            )
        {
            self.dialogs(ctx, state, rects, commands);
        }
    }

    /// PoB's "Manage Passive Trees" (TreeTab.lua:630, PassiveSpecListControl.lua): the trees,
    /// dragged to a new place, with Copy, Delete, Rename and New on the selected row. Its list
    /// takes the arrows, Home/End, Delete/Backspace and F2 (ListControl.lua:375-392). A
    /// question it asks draws over it and holds it.
    fn manager_window(
        &mut self,
        ctx: &egui::Context,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        let Some(mut selected) = self.manager else {
            return;
        };
        let n = state.specs.len();
        if n == 0 {
            return;
        }
        selected = selected.min(n - 1);
        let asking = self.dialog.is_some();
        // Esc closes it, once its list has the key (PopupDialog.lua:72-77); a question it
        // asked takes Esc first
        let window = egui::Id::unique("tree_manager");
        let mut open = asking || !crate::widgets::escape::escape(ctx, window);
        let mut done = false;
        let mut next = None;
        // the keys wait for a question or a text field (ControlHost.lua:39-42)
        if !asking && !ctx.text_edit_focused() {
            if let Some(i) = ctx.input_mut(|i| crate::ui::shortcuts::list_key(i, n, Some(selected)))
            {
                selected = i;
            }
            let (f2, delete) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::F2),
                    crate::ui::shortcuts::list_delete(i),
                )
            });
            if f2 {
                next = Some(Dialog::RenameSpec {
                    index: selected,
                    title: state.specs[selected].name_box_text(),
                    edited: false,
                });
            } else if delete && n > 1 {
                next = Some(Dialog::DeleteSpec(selected));
            }
        }
        egui::Window::new("Manage Passive Trees")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0])
            .show(ctx, |ui| {
                ui.add_enabled_ui(!asking, |ui| {
                    let rows = reorder_rows(
                        ui,
                        "tree_specs",
                        n,
                        true,
                        false,
                        SpecDrag,
                        |p: &SpecDrag| p.0,
                        |ui, i| {
                            let spec = &state.specs[i];
                            let label = if i == state.active_spec {
                                format!("{} (current)", spec.title)
                            } else {
                                spec.title.clone()
                            };
                            let fill = if i == selected {
                                ui.visuals().selection.bg_fill
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            egui::Frame::NONE
                                .fill(fill)
                                .inner_margin(2.0)
                                .show(ui, |ui| {
                                    ui.set_min_width(240.0);
                                    ui.add(egui::Label::new(label).selectable(false));
                                });
                        },
                    );
                    for (i, row) in rows.rows.iter().enumerate() {
                        rects.note(row, || format!("spec_order_row:{i}"));
                        if row.clicked() {
                            selected = i;
                        }
                    }
                    if let Some((from, to)) = rows.moved {
                        commands.push(Command::MoveSpec { from, to });
                        selected = to;
                    }
                    let spec = &state.specs[selected];
                    // PassiveSpecListControl.lua:13-45, in its order
                    ui.horizontal(|ui| {
                        if button(ui, rects, "spec_manager_copy", "Copy", true) {
                            next = Some(Dialog::CopySpec {
                                index: selected,
                                title: spec.name_box_text(),
                                edited: false,
                            });
                        }
                        // the last tree can't be deleted
                        if button(ui, rects, "spec_manager_delete", "Delete", n > 1) {
                            next = Some(Dialog::DeleteSpec(selected));
                        }
                        if button(ui, rects, "spec_manager_rename", "Rename", true) {
                            next = Some(Dialog::RenameSpec {
                                index: selected,
                                title: spec.name_box_text(),
                                edited: false,
                            });
                        }
                        if button(ui, rects, "spec_manager_new", "New", true) {
                            next = Some(Dialog::NewSpec(String::new()));
                        }
                    });
                    done = button(ui, rects, "spec_manager_done", "Done", true);
                });
            });
        crate::widgets::escape::note(ctx, window);
        self.manager = (open && !done).then_some(selected);
        if self.manager.is_some()
            && let Some(dialog) = next
        {
            self.dialog = Some(dialog);
        }
    }

    fn dialogs(
        &mut self,
        ctx: &egui::Context,
        state: &TreeState,
        rects: &mut Rects,
        commands: &mut Vec<Command>,
    ) {
        self.manager_window(ctx, state, rects, commands);
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        // Esc cancels the question once its box is left (PopupDialog.lua:72-77)
        let window = egui::Id::unique("tree_dialog");
        let mut close = crate::widgets::escape::escape(ctx, window);
        let title = match dialog {
            Dialog::NewSpec(_) => "New tree",
            Dialog::RenameSpec { .. } => "Rename tree",
            Dialog::DeleteSpec(_) => "Delete Tree",
            Dialog::Reset => "Reset Tree/Tattoos",
            Dialog::Import { .. } => "Import Tree",
            Dialog::Export { .. } => "Export Tree",
            Dialog::CopySpec { .. } => "Copy Tree",
            Dialog::ConvertVersion { .. } => "Convert to Version",
            Dialog::ClassChange { .. } => "Class Change",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| match dialog {
                Dialog::NewSpec(name) => {
                    let (field, _) =
                        name_box::edit(ui, name, Filter::Plain, name_box::NAME_LIMIT, |t| {
                            t.hint_text("Title")
                        });
                    rects.note(&field, || "spec_new_name".into());
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let valid = name_box::has_visible(name);
                        if (button(ui, rects, "spec_new_ok", "Add", valid) || (enter && valid))
                            && valid
                        {
                            commands.push(Command::NewSpec {
                                title: name.clone(),
                            });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::RenameSpec {
                    index,
                    title,
                    edited,
                } => {
                    let (field, changed) =
                        name_box::edit(ui, title, Filter::Plain, name_box::NAME_LIMIT, |t| t);
                    rects.note(&field, || "spec_rename_name".into());
                    // PoB's name popup opens with its box focused
                    if !*edited {
                        field.request_focus();
                    }
                    *edited |= changed;
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let valid = *edited && name_box::has_visible(title);
                        if (button(ui, rects, "spec_rename_ok", "Rename", valid)
                            || (enter && valid))
                            && valid
                        {
                            commands.push(Command::RenameSpec {
                                index: *index,
                                title: title.clone(),
                            });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::DeleteSpec(index) => {
                    let name = state.specs.get(*index).map_or("", |s| s.title.as_str());
                    // PoB's question (PassiveSpecListControl.lua:97)
                    ui.label(format!("Are you sure you want to delete '{name}'?"));
                    ui.horizontal(|ui| {
                        if button(ui, rects, "spec_delete_yes", "Delete", true) {
                            commands.push(Command::DeleteSpec { index: *index });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::Reset => {
                    // PoB's warning and buttons (TreeTab.lua:132-153)
                    ui.label(
                        "Warning: resetting your passive tree or removing all tattoos cannot be \
                         undone.",
                    );
                    ui.horizontal(|ui| {
                        if button(ui, rects, "reset_yes", "Reset Tree", true) {
                            commands.push(Command::ResetTree);
                            close = true;
                        }
                        if button(ui, rects, "remove_tattoos_yes", "Remove All Tattoos", true) {
                            commands.push(Command::RemoveAllTattoos);
                            close = true;
                        }
                        let cancel = ui.button("Cancel");
                        rects.note(&cancel, || "reset_no".into());
                        close |= cancel.clicked();
                    });
                }
                Dialog::Export { link, shrink } => {
                    ui.label("Passive tree link:");
                    let mut shown = link.as_str();
                    let field = ui.add(egui::TextEdit::singleline(&mut shown).desired_width(350.0));
                    crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &field, link);
                    ui.horizontal(|ui| {
                        let label = match shrink {
                            Shrink::Idle => "Shrink with PoEURL",
                            Shrink::Asked | Shrink::Running => "Shrinking...",
                            Shrink::Done => "Done",
                        };
                        let ready = !link.is_empty();
                        if button(
                            ui,
                            rects,
                            "export_shrink",
                            label,
                            ready && *shrink == Shrink::Idle,
                        ) {
                            *shrink = Shrink::Asked;
                            commands.push(Command::ShrinkTreeLink);
                        }
                        if button(ui, rects, "export_copy", "Copy", ready) {
                            ui.ctx().copy_text(link.clone());
                        }
                        close |= button(ui, rects, "export_done", "Done", true);
                    });
                }
                Dialog::Import { title, url } => {
                    // both boxes take EditControl's own filter (TreeTab.lua:743-752), and the
                    // title has no limit; PoB's labels
                    ui.label("Enter name for this passive tree:");
                    name_box::edit(ui, title, Filter::Plain, usize::MAX, |t| t);
                    ui.label("Enter passive tree link:");
                    name_box::edit(ui, url, Filter::Plain, usize::MAX, |t| {
                        t.desired_width(400.0)
                    });
                    ui.horizontal(|ui| {
                        let valid = name_box::has_visible(title) && name_box::has_visible(url);
                        if button(ui, rects, "import_ok", "Import", valid) {
                            commands.push(Command::ImportTreeLink {
                                title: title.clone(),
                                url: url.trim().to_owned(),
                            });
                            close = true;
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::CopySpec {
                    index,
                    title,
                    edited,
                } => {
                    ui.label("Enter name for this passive tree:");
                    let (field, changed) =
                        name_box::edit(ui, title, Filter::Plain, name_box::NAME_LIMIT, |t| t);
                    // PoB's name popup opens with its box focused
                    if !*edited {
                        field.request_focus();
                    }
                    *edited |= changed;
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let valid = *edited && name_box::has_visible(title);
                        if (button(ui, rects, "spec_copy_ok", "Save", valid) || (enter && valid))
                            && valid
                        {
                            commands.push(Command::CopySpec {
                                index: *index,
                                title: title.clone(),
                            });
                            close = true;
                        }
                        let cancel = ui.button("Cancel");
                        rects.note(&cancel, || "spec_copy_cancel".into());
                        close |= cancel.clicked();
                    });
                }
                Dialog::ClassChange { node, class } => {
                    // PoB's question (PassiveTreeView.lua:474)
                    ui.label(format!(
                        "Changing class to {class} will reset your passive tree.\nThis can be \
                         avoided by connecting one of the {class} starting nodes to your tree."
                    ));
                    ui.horizontal(|ui| {
                        for (key, text, connect) in [
                            ("node_class_continue", "Continue", false),
                            ("node_class_connect", "Connect Path", true),
                        ] {
                            if button(ui, rects, key, text, true) {
                                commands
                                    .push(Command::AllocNodeChangingClass { id: *node, connect });
                                close = true;
                            }
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
                Dialog::ConvertVersion { version, label } => {
                    // PoB's warning (TreeTab.lua:652-653)
                    ui.label(format!("Convert to Version {label}"));
                    ui.label(
                        "Warning: some or all of the passives may be de-allocated due to changes \
                         in the tree.\n\nConvert will replace your current tree.\nCopy + Convert \
                         will backup your current tree.",
                    );
                    ui.horizontal(|ui| {
                        for (key, text, keep_copy) in [
                            ("version_convert", "Convert", false),
                            ("version_convert_copy", "Copy + Convert", true),
                        ] {
                            if button(ui, rects, key, text, true) {
                                commands.push(Command::ConvertSpec {
                                    version: version.clone(),
                                    keep_copy,
                                });
                                close = true;
                            }
                        }
                        close |= ui.button("Cancel").clicked();
                    });
                }
            });
        crate::widgets::escape::note(ctx, window);
        if close {
            self.dialog = None;
        }
    }
}

/// A dialog's primary button; its rect is recorded under `key` for tests.
fn button(ui: &mut egui::Ui, rects: &mut Rects, key: &str, label: &str, enabled: bool) -> bool {
    let response = ui.add_enabled(enabled, theme::primary_button(label));
    rects.note(&response, || key.to_owned());
    response.clicked()
}
