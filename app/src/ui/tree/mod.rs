//! The passive tree tab: the canvas plus the state it needs between frames. It takes engine
//! snapshots and returns commands; it knows nothing else about PoB.

pub mod canvas;
pub mod geometry;
mod keys;
pub mod overlays;
pub mod power_report;
pub mod toolbar;

use crate::engine::{
    Command, NodePower, NodeTooltip, PopupKind, PowerReport, Snapshot, TreeLayout, TreeState,
};
use canvas::CanvasAction;
use eframe::egui::{self, TextureHandle};
use geometry::{Camera, HitGrid};
use power_report::ReportPanel;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use toolbar::{PowerStatus, Toolbar};

/// A hover must rest this long on a node before its tooltip is requested.
const PREVIEW_DELAY: f64 = 0.1;

/// Where widgets were drawn last frame, so tests can click them. Does nothing outside tests.
#[derive(Default)]
pub struct Rects {
    #[cfg(test)]
    pub(crate) map: HashMap<String, egui::Rect>,
}

impl Rects {
    #[cfg(test)]
    pub(super) fn note(&mut self, response: &egui::Response, key: impl FnOnce() -> String) {
        self.map.insert(key(), response.rect);
    }

    #[cfg(not(test))]
    pub(super) fn note(&mut self, _response: &egui::Response, _key: impl FnOnce() -> String) {}
    /// A rect that isn't a widget's (a drop area)
    #[cfg(test)]
    pub(crate) fn note_rect(&mut self, rect: egui::Rect, key: &str) {
        self.map.insert(key.to_owned(), rect);
    }

    #[cfg(not(test))]
    pub(crate) fn note_rect(&mut self, _rect: egui::Rect, _key: &str) {}
}

/// How long a node pulses after a power report row was clicked (seconds).
pub(super) const PULSE_SECONDS: f64 = 1.5;

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct TreeOutput {
    pub commands: Vec<Command>,
    /// Questions about the hovered node; the app may send only the latest of them.
    pub queries: Vec<Command>,
    /// The user right-clicked an allocated jewel socket: show its slot on the Items tab
    pub jump_to_socket: Option<u32>,
}

/// Lookups over [`TreeState`], built once per snapshot rather than once per frame.
#[derive(Debug, Default)]
pub(super) struct Derived {
    pub allocated: HashSet<u32>,
    pub paths: HashMap<u32, Vec<u32>>,
    pub depends: HashMap<u32, Vec<u32>>,
    pub matches: HashSet<u32>,
    pub compare: HashMap<u32, [f32; 3]>,
    /// The id (not the label) nodes of the build's ascendancy carry
    pub current_ascendancy: Option<String>,
}

impl Derived {
    fn new(state: &TreeState) -> Self {
        let current_ascendancy = state
            .classes
            .iter()
            .find(|c| c.id == state.class_id)
            .and_then(|c| c.ascendancies.iter().find(|a| a.id == state.ascendancy_id))
            .filter(|a| a.id != 0)
            .map(|a| a.ascend_id.clone());
        Self {
            allocated: state.allocated.iter().copied().collect(),
            paths: state
                .paths
                .iter()
                .map(|p| (p.node, p.path.clone()))
                .collect(),
            depends: state
                .depends
                .iter()
                .map(|p| (p.node, p.path.clone()))
                .collect(),
            matches: state.search_matches.iter().copied().collect(),
            compare: state.compare.iter().map(|c| (c.node, c.rgb)).collect(),
            current_ascendancy,
        }
    }
}

#[derive(Default)]
pub struct TreeTab {
    pub(super) camera: Camera,
    /// Fit the whole tree into the view on the next frame
    pub(super) needs_fit: bool,
    /// Where the canvas was drawn last
    pub(super) rect: Option<egui::Rect>,
    layout: Option<Arc<TreeLayout>>,
    state: Option<Arc<TreeState>>,
    power: Option<Arc<NodePower>>,
    pub(super) grid: HitGrid,
    /// Node id to index in the layout
    pub(super) index: HashMap<u32, usize>,
    pub(super) derived: Derived,
    /// Node id to index in `power.nodes`
    pub(super) power_index: HashMap<u32, usize>,
    /// Uploaded sprite sheets, by file
    pub(super) textures: HashMap<PathBuf, TextureHandle>,
    pub(super) hover: Option<u32>,
    /// `ui.input(|i| i.time)` when the hover started
    hover_since: f64,
    /// The node whose tooltip was asked for since the hover started
    previewed: Option<u32>,
    pub(super) tooltip: Option<Arc<NodeTooltip>>,
    /// The mastery whose effect picker is open
    picker: Option<u32>,
    /// The jewel sockets whose item slot PoB lets the user use (from the Items tab)
    pub(super) sockets: HashSet<u32>,
    /// Node power progress, while it is being calculated
    pub(super) progress: Option<u32>,
    pub(super) busy: bool,
    /// The latest power report and tree link, for the panels around the canvas
    pub report: Option<Arc<PowerReport>>,
    pub link: Option<Arc<String>>,
    /// The node power calculation failed, until the next progress or result
    failed: bool,
    toolbar: Toolbar,
    report_panel: ReportPanel,
    /// The node being pointed out, and when the pointing started
    pub(super) pulse: Option<(u32, f64)>,
    pub(crate) rects: Rects,
    /// The path traced with Shift held, from the tree to its end (PoB's `tracePath`)
    pub(super) trace: Vec<u32>,
}

impl TreeTab {
    /// Forgets the build's state, as for another build. Uploaded textures stay.
    ///
    /// The layout stays: the engine resends it only when its content changes, and a new
    /// snapshot replaces it.
    pub fn reset(&mut self) {
        let kept = Self {
            textures: std::mem::take(&mut self.textures),
            layout: self.layout.take(),
            grid: std::mem::take(&mut self.grid),
            index: std::mem::take(&mut self.index),
            camera: self.camera,
            ..Self::default()
        };
        *self = kept;
    }

    /// Drops a pending confirmation; its dialog only draws on this tab. The top bar's class
    /// question stays.
    pub fn close_dialog(&mut self) {
        self.toolbar.close_dialog();
    }

    /// Off the Tree tab its questions close; on the Compare tab's tree view the Class Change
    /// question stays, as that view asks it too (CompareTab.lua:3401, PassiveTreeView.lua:474)
    pub fn close_dialog_off_tab(&mut self, compare_tree: bool) {
        if compare_tree {
            self.toolbar.close_dialog_but_class_change();
        } else {
            self.toolbar.close_dialog();
        }
    }

    /// The top bar's point display (PoB's `pointDisplay`), with PoB's progress tooltip
    pub fn top_bar_points(&self, ui: &mut egui::Ui, progress: &str) {
        if let Some(state) = &self.state {
            self.toolbar.points(ui, state, progress);
        }
    }

    /// The top bar's class and ascendancy dropdowns, on every tab as PoB has them, and the
    /// question a class change can ask. Returns what the user chose.
    pub fn top_bar_class(&mut self, ui: &mut egui::Ui) -> Vec<Command> {
        let mut commands = Vec::new();
        if let Some(state) = self.state.clone() {
            self.toolbar
                .class(ui, &state, &mut self.rects, &mut commands);
        }
        self.toolbar
            .class_dialog(ui.ctx(), &mut self.rects, &mut commands);
        commands
    }

    /// Opens PoB's "Manage Passive Trees" window (the Items tab's "Manage...",
    /// ItemsTab.lua:203-205): the Tree tab's own manager, with its questions.
    pub fn open_spec_manager(&mut self) {
        if let Some(state) = &self.state {
            self.toolbar.open_manager(state);
        }
    }

    pub fn spec_manager_open(&self) -> bool {
        self.toolbar.manager_open()
    }

    pub fn close_spec_manager(&mut self) {
        self.toolbar.close_manager();
    }

    /// Draws the "Manage Passive Trees" window off the Tree tab while it is open; returns what
    /// the user chose
    pub fn spec_manager(&mut self, ctx: &egui::Context) -> Vec<Command> {
        let mut commands = Vec::new();
        if let Some(state) = self.state.clone() {
            self.toolbar
                .manager_and_dialogs(ctx, &state, &mut self.rects, &mut commands);
        }
        commands
    }

    /// A toolbar dialog is open, so shortcuts must wait.
    pub fn has_dialog(&self) -> bool {
        self.toolbar.has_dialog()
    }

    /// One of the tree's questions (New, Rename, Delete, ...) is up
    #[cfg(test)]
    pub(crate) fn has_question(&self) -> bool {
        self.toolbar.has_question()
    }

    /// PoB's Class Change question is up
    #[cfg(test)]
    pub(crate) fn class_question(&self) -> bool {
        self.toolbar.class_question()
    }

    /// The heat map is on in the latest tree state.
    pub fn heatmap_on(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.heatmap.is_some())
    }

    /// The command that turns the heat map off, when it is on.
    pub fn stop_heatmap(&self) -> Option<Command> {
        let state = self.state.as_ref()?;
        state.heatmap.as_ref()?;
        Some(Command::SetHeatmap {
            stat: None,
            max_depth: state.max_depth,
        })
    }

    /// The jewel sockets whose slot the Items tab lets the user use.
    pub fn set_enabled_sockets(&mut self, sockets: impl IntoIterator<Item = u32>) {
        self.sockets = sockets.into_iter().collect();
    }

    /// The undo and redo buttons' enablement.
    pub fn history(&self) -> (bool, bool) {
        self.state
            .as_ref()
            .map_or((false, false), |s| (s.can_undo, s.can_redo))
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot) {
        match snapshot {
            Snapshot::TreeLayout(layout) => {
                // Always rebuilt: a layout with another key can order its nodes differently,
                // and the grid and index hold positions in that order.
                self.grid = HitGrid::new(layout);
                self.index = layout
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.id, i))
                    .collect();
                // the view is only refitted for another tree; a change in node text keeps it
                if self
                    .layout
                    .as_ref()
                    .is_none_or(|l| l.version != layout.version)
                {
                    self.needs_fit = true;
                }
                self.hover = None;
                self.previewed = None;
                self.layout = Some(layout.clone());
            }
            Snapshot::Tree(state) => {
                self.derived = Derived::new(state);
                // the hovered node's tooltip may read differently now
                self.previewed = None;
                if self.picker.is_some_and(|n| !has_options(state, n)) {
                    self.picker = None;
                }
                self.state = Some(state.clone());
            }
            Snapshot::NodePower(power) => {
                self.power_index = power
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.id, i))
                    .collect();
                if power.finished {
                    self.progress = None;
                }
                self.failed = false;
                self.power = Some(power.clone());
            }
            Snapshot::PowerReport(report) => self.report = Some(report.clone()),
            Snapshot::NodeTooltip(tooltip) => self.tooltip = Some(tooltip.clone()),
            Snapshot::TreeLink(link) => {
                self.link = Some(link.clone());
                self.toolbar.link_arrived();
            }
            _ => {}
        }
    }

    pub fn on_progress(&mut self, percent: u32) {
        self.failed = false;
        self.progress = Some(percent.min(100));
    }

    /// The node power calculation stopped with an error.
    pub fn power_failed(&mut self) {
        self.progress = None;
        self.failed = true;
    }

    /// Shows a node from outside the tab (the Calcs tab's node links).
    pub fn focus(&mut self, id: u32, now: f64) {
        self.focus_node(id, now);
    }

    /// Centres the view on a node and makes it pulse.
    pub(super) fn focus_node(&mut self, id: u32, now: f64) {
        let (Some(layout), Some(&i)) = (&self.layout, self.index.get(&id)) else {
            return;
        };
        self.camera.center = layout.nodes[i].pos.into();
        self.pulse = Some((id, now));
    }

    /// `busy`: the engine is working, which the legend says.
    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool) -> TreeOutput {
        let mut out = TreeOutput::default();
        let (Some(layout), Some(state)) = (self.layout.clone(), self.state.clone()) else {
            ui.centered_and_justified(|ui| ui.label("No passive tree loaded"));
            return out;
        };
        self.busy = busy;
        let status = PowerStatus {
            progress: self.progress,
            failed: self.failed,
        };
        let link = self.link.clone();
        egui::Panel::top("tree_toolbar").show(ui, |ui| {
            self.toolbar.show(
                ui,
                &state,
                &status,
                link.as_deref().map(String::as_str),
                &mut self.rects,
                &mut out,
            );
        });
        if self.toolbar.report_open {
            let report = self.report.clone();
            let mut clicked = None;
            egui::Panel::right("tree_power_report")
                .resizable(true)
                .default_size(420.0)
                .show(ui, |ui| {
                    clicked = self.report_panel.show(
                        ui,
                        report.as_deref(),
                        &mut self.rects,
                        &mut out.commands,
                    );
                });
            if let Some(id) = clicked {
                let now = ui.input(|i| i.time);
                self.focus_node(id, now);
            }
        }
        self.canvas(ui, &layout, &state, true, &mut out);
        out
    }

    /// The Compare tab's Tree view (R7's overlay): our tree with the comparison's colours, under
    /// PoB's header for it, our tree set and the overlay search (`leftSpecSelect`,
    /// `overlayTreeSearch`). The Tree tab's toolbar and power report are not drawn.
    pub fn show_compare(&mut self, ui: &mut egui::Ui, busy: bool) -> TreeOutput {
        let mut out = TreeOutput::default();
        let (Some(layout), Some(state)) = (self.layout.clone(), self.state.clone()) else {
            ui.centered_and_justified(|ui| ui.label("No passive tree loaded"));
            return out;
        };
        self.busy = busy;
        ui.horizontal_wrapped(|ui| {
            ui.label("Tree set:");
            self.toolbar
                .specs(ui, &state, &mut self.rects, &mut out.commands, false);
            ui.separator();
            self.toolbar
                .search(ui, &state, &mut self.rects, &mut out.commands);
        });
        // PoB's overlay draws the build's own tree view (CompareTab.lua:3401), which asks
        // the Class Change question (PassiveTreeView.lua:474); it asks nothing else here
        self.canvas(ui, &layout, &state, true, &mut out);
        self.toolbar
            .compare_dialogs(ui.ctx(), &state, &mut self.rects, &mut out.commands);
        out
    }

    /// The canvas, its hover tooltips and the mastery picker. `class_change`: a click on
    /// another class's node asks PoB's Class Change question; otherwise it does nothing.
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        layout: &Arc<TreeLayout>,
        state: &Arc<TreeState>,
        class_change: bool,
        out: &mut TreeOutput,
    ) {
        let (layout, state) = (layout.clone(), state.clone());
        let power = self.power.clone();
        let actions = canvas::show(ui, self, &layout, &state, power.as_deref());
        let now = ui.input(|i| i.time);
        for action in actions {
            match action {
                CanvasAction::Alloc(id) => match class_change_for(&layout, &state, &self.index, id)
                {
                    // the class's tree doesn't connect: PoB asks first (PassiveTreeView.lua:474)
                    Some(class) if class_change => self.toolbar.ask_class_change(id, class),
                    Some(_) => {}
                    None => out.commands.push(Command::AllocNode { id }),
                },
                CanvasAction::AllocTrace(id, path) => {
                    out.commands.push(Command::AllocTracePath { id, path })
                }
                CanvasAction::Dealloc(id) => out.commands.push(Command::DeallocNode { id }),
                CanvasAction::PickMastery(id) => self.picker = Some(id),
                CanvasAction::ReplaceNode(node) => out
                    .commands
                    .push(Command::OpenPopup(PopupKind::NodeReplacement { node })),
                CanvasAction::JumpToSocket(node) => out.jump_to_socket = Some(node),
                CanvasAction::Hover(id) => {
                    self.hover = id;
                    self.hover_since = now;
                    self.previewed = None;
                }
            }
        }

        if let Some(id) = self.hover.filter(|&id| self.previewed != Some(id)) {
            let waited = now - self.hover_since;
            if waited >= PREVIEW_DELAY {
                out.queries.push(if self.trace.is_empty() {
                    Command::PreviewNode { id }
                } else {
                    Command::PreviewTrace {
                        id,
                        path: self.trace.clone(),
                    }
                });
                self.previewed = Some(id);
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(PREVIEW_DELAY - waited));
            }
        }

        self.show_picker(ui.ctx(), &layout, &state, out);
    }

    /// The effect list of a mastery. A choice sends `SetMastery`, which allocates the mastery
    /// too when needed (`TreeTabClass:SaveMasteryPopup`).
    fn show_picker(
        &mut self,
        ctx: &egui::Context,
        layout: &TreeLayout,
        state: &TreeState,
        out: &mut TreeOutput,
    ) {
        let Some(node) = self.picker else {
            return;
        };
        let Some(options) = state.mastery_options.iter().find(|m| m.node == node) else {
            self.picker = None;
            return;
        };
        let name = self
            .index
            .get(&node)
            .map_or("Mastery", |&i| layout.nodes[i].name.as_str());
        let chosen = state
            .masteries
            .iter()
            .find(|m| m.node == node)
            .map(|m| m.effect);
        // PoB's mastery popup closes on Esc (TreeTab.lua:1034, PopupDialog.lua:72-77)
        let window = egui::Id::unique("tree_mastery_picker");
        let mut open = !crate::widgets::escape::escape(ctx, window);
        let mut picked = None;
        egui::Window::new(name)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                for option in &options.options {
                    let button = egui::Button::selectable(chosen == Some(option.id), &option.text);
                    let response = ui.add(button);
                    #[cfg(test)]
                    self.rects
                        .map
                        .insert(format!("option:{}", option.id), response.rect);
                    if response.clicked() {
                        picked = Some(option.id);
                    }
                }
            });
        crate::widgets::escape::note(ctx, window);
        if let Some(effect) = picked {
            out.commands.push(Command::SetMastery { node, effect });
        }
        if !open || picked.is_some() {
            self.picker = None;
        }
    }
}

fn has_options(state: &TreeState, node: u32) -> bool {
    state
        .mastery_options
        .iter()
        .any(|m| m.node == node && !m.options.is_empty())
}

/// The class a click on node `id` would switch to with PoB's "Class Change" question: a node of
/// another class's ascendancy while the tree has points that don't connect to that class
/// (`ClassChoice::needs_confirm`, PoB's own test).
fn class_change_for(
    layout: &TreeLayout,
    state: &TreeState,
    index: &HashMap<u32, usize>,
    id: u32,
) -> Option<String> {
    let ascendancy = layout.nodes.get(*index.get(&id)?)?.ascendancy.as_ref()?;
    let class = state.classes.iter().find(|c| {
        c.ascendancies
            .iter()
            .any(|a| a.id != 0 && &a.ascend_id == ascendancy)
    })?;
    (class.id != state.class_id && class.needs_confirm).then(|| class.name.clone())
}

#[cfg(test)]
mod tests {
    use super::geometry::tests::{layout, node};
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};

    /// 1 is allocated; 2 is reachable through 1; 3 is a mastery with two effects; 4 can't be
    /// reached.
    fn sample() -> (Arc<TreeLayout>, Arc<TreeState>) {
        let mut mastery = node(3, [0.0, 600.0], "Mastery");
        mastery.mastery_effects = true;
        let layout = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                node(2, [600.0, 0.0], "Normal"),
                mastery,
                node(4, [600.0, 600.0], "Normal"),
            ],
            vec![(0, 1)],
        );
        let state: TreeState = serde_json::from_value(serde_json::json!({
            "version": "3_28", "version_label": "3.28", "class_id": 0, "ascendancy_id": 0,
            "points": { "used": 1, "available": 122, "ascendancy_used": 0,
                        "ascendancy_available": 8 },
            "allocated": [1],
            "paths": [ { "node": 2, "path": [2] } ],
            "mastery_options": [ { "node": 3, "options": [
                { "id": 30, "text": "first effect" }, { "id": 31, "text": "second effect" } ] } ],
            "active_spec": 0, "can_undo": false, "can_redo": false,
        }))
        .unwrap();
        (Arc::new(layout), Arc::new(state))
    }

    struct Harness {
        tab: TreeTab,
        ctx: egui::Context,
        time: f64,
        /// Draw the Compare tab's tree view instead of the Tree tab
        compare: bool,
    }

    impl Harness {
        fn new(layout: Arc<TreeLayout>, state: Arc<TreeState>) -> Self {
            let mut tab = TreeTab::default();
            tab.on_snapshot(&Snapshot::TreeLayout(layout));
            tab.on_snapshot(&Snapshot::Tree(state));
            let mut h = Self {
                tab,
                ctx: egui::Context::default(),
                time: 1.0,
                compare: false,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> TreeOutput {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = TreeOutput::default();
            self.ctx
                .run_ui(input, |ui| {
                    let o = if self.compare {
                        self.tab.show_compare(ui, false)
                    } else {
                        self.tab.show(ui, false)
                    };
                    out.commands.extend(o.commands);
                    out.queries.extend(o.queries);
                    out.jump_to_socket = out.jump_to_socket.or(o.jump_to_socket);
                })
                .drop_without_applying_deltas();
            out
        }

        fn screen_of(&self, id: u32) -> Pos2 {
            let i = self.tab.index[&id];
            let pos = self.tab.layout.as_ref().unwrap().nodes[i].pos;
            self.tab.camera.to_screen(self.tab.rect_for_test(), pos)
        }

        /// Moves to `pos`, presses and releases; returns everything the frames sent.
        fn click_at(&mut self, pos: Pos2) -> TreeOutput {
            self.click_with(pos, PointerButton::Primary)
        }

        fn click_with(&mut self, pos: Pos2, which: PointerButton) -> TreeOutput {
            let button = |pressed| Event::PointerButton {
                pos,
                button: which,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut all = TreeOutput::default();
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                let o = self.frame(events);
                all.commands.extend(o.commands);
                all.queries.extend(o.queries);
                all.jump_to_socket = all.jump_to_socket.or(o.jump_to_socket);
            }
            all
        }
    }

    impl TreeTab {
        fn rect_for_test(&self) -> Rect {
            self.rect.expect("drawn")
        }
    }

    fn debug(commands: &[Command]) -> Vec<String> {
        commands.iter().map(|c| format!("{c:?}")).collect()
    }

    #[test]
    fn click_unallocated_sends_alloc() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        let out = h.click_at(h.screen_of(2));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 2 }"]);
        // a node without a path does nothing
        let out = h.click_at(h.screen_of(4));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
    }

    #[test]
    fn click_allocated_sends_dealloc() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        let out = h.click_at(h.screen_of(1));
        assert_eq!(debug(&out.commands), ["DeallocNode { id: 1 }"]);
    }

    #[test]
    fn click_mastery_opens_picker_then_sends_set_mastery() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        let out = h.click_at(h.screen_of(3));
        assert!(out.commands.is_empty());
        assert_eq!(h.tab.picker, Some(3));
        h.frame(vec![]);
        let pos = h.tab.rects.map["option:31"].center();
        let out = h.click_at(pos);
        assert_eq!(debug(&out.commands), ["SetMastery { node: 3, effect: 31 }"]);
        assert_eq!(h.tab.picker, None);
    }

    /// The sample with the mastery 3 allocated with the first effect.
    fn allocated_mastery() -> Harness {
        let (l, s) = sample();
        let mut state = (*s).clone();
        state.allocated.push(3);
        Harness::new(l, Arc::new(state))
    }

    #[test]
    fn left_click_on_an_allocated_mastery_deallocates_it() {
        let mut h = allocated_mastery();
        let out = h.click_at(h.screen_of(3));
        assert_eq!(debug(&out.commands), ["DeallocNode { id: 3 }"]);
        assert_eq!(h.tab.picker, None);
    }

    #[test]
    fn right_click_on_a_mastery_opens_the_picker() {
        // R19: only an allocated mastery opens the picker; an unallocated one is node
        // replacement (`right_click_routes_like_pob`)
        let mut h = allocated_mastery();
        let out = h.click_with(h.screen_of(3), PointerButton::Secondary);
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert_eq!(h.tab.picker, Some(3));
    }

    #[test]
    fn esc_closes_the_mastery_picker() {
        // PoB's mastery popup (TreeTab.lua:1034) closes on Esc (PopupDialog.lua:72-77)
        let mut h = allocated_mastery();
        h.click_with(h.screen_of(3), PointerButton::Secondary);
        assert_eq!(h.tab.picker, Some(3));
        let esc = Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let out = h.frame(vec![esc]);
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert_eq!(h.tab.picker, None, "Esc left it open");
    }

    /// 1 allocated plain passive, 2 a replaceable small attribute, 3 an unallocated mastery
    /// with effects, 4 an allocated socket, 5 an unallocated socket, 6 an allocated socket
    /// whose slot PoB doesn't enable, 7 an allocated mastery.
    fn routing() -> Harness {
        let mut replaceable = node(2, [600.0, 0.0], "Normal");
        replaceable.replaceable = true;
        let mut mastery = node(3, [0.0, 600.0], "Mastery");
        mastery.mastery_effects = true;
        let mut allocated_mastery = node(7, [1200.0, 600.0], "Mastery");
        allocated_mastery.mastery_effects = true;
        let l = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                replaceable,
                mastery,
                node(4, [600.0, 600.0], "Socket"),
                node(5, [1200.0, 0.0], "Socket"),
                node(6, [0.0, 1200.0], "Socket"),
                allocated_mastery,
            ],
            vec![],
        );
        let (_, s) = sample();
        let mut state = (*s).clone();
        state.allocated = vec![1, 4, 6, 7];
        state.mastery_options = serde_json::from_value(serde_json::json!([
            { "node": 3, "options": [ { "id": 30, "text": "first" } ] },
            { "node": 7, "options": [ { "id": 70, "text": "seventh" } ] },
        ]))
        .unwrap();
        let mut h = Harness::new(Arc::new(l), Arc::new(state));
        h.tab.set_enabled_sockets([4, 5]);
        h
    }

    #[test]
    fn right_click_routes_like_pob() {
        let right = |h: &mut Harness, id| {
            h.tab.picker = None;
            let out = h.click_with(h.screen_of(id), PointerButton::Secondary);
            (debug(&out.commands), out.jump_to_socket, h.tab.picker)
        };
        let mut h = routing();
        // a replaceable node: node replacement
        assert_eq!(
            right(&mut h, 2),
            (
                vec!["OpenPopup(NodeReplacement { node: 2 })".into()],
                None,
                None
            )
        );
        // an unallocated mastery with effects: node replacement too, not the picker
        assert_eq!(
            right(&mut h, 3),
            (
                vec!["OpenPopup(NodeReplacement { node: 3 })".into()],
                None,
                None
            )
        );
        // an allocated mastery: the picker
        assert_eq!(right(&mut h, 7), (vec![], None, Some(7)));
        // an allocated socket whose slot is enabled: jump to it
        assert_eq!(right(&mut h, 4), (vec![], Some(4), None));
        // an unallocated socket, a socket whose slot isn't enabled, a plain passive: nothing
        assert_eq!(right(&mut h, 5), (vec![], None, None));
        assert_eq!(right(&mut h, 6), (vec![], None, None));
        assert_eq!(right(&mut h, 1), (vec![], None, None));
        // a mastery without effects has nothing to offer either way
        let mut bare = routing();
        let mut layout = (**bare.tab.layout.as_ref().unwrap()).clone();
        layout.nodes[2].mastery_effects = false;
        bare.tab
            .on_snapshot(&Snapshot::TreeLayout(Arc::new(layout)));
        assert_eq!(right(&mut bare, 3), (vec![], None, None));
    }

    fn click_key(h: &mut Harness, key: &str) -> TreeOutput {
        let pos = h.tab.rects.map[key].center();
        h.click_at(pos)
    }

    #[test]
    fn timeless_button_opens_the_session() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        let out = click_key(&mut h, "find_timeless");
        assert_eq!(debug(&out.commands), ["OpenPopup(TimelessJewel)"]);
    }

    /// PoB can't undo "Remove All Tattoos", so the first click only asks (Review Focus 5).
    #[test]
    fn remove_all_tattoos_asks_first() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        let out = click_key(&mut h, "remove_tattoos");
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert!(h.tab.has_dialog());
        // Cancel sends nothing and closes the question
        let out = click_key(&mut h, "remove_tattoos_no");
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert!(!h.tab.has_dialog());
        // asking again and confirming sends the command, once
        click_key(&mut h, "remove_tattoos");
        let out = click_key(&mut h, "remove_tattoos_yes");
        assert_eq!(debug(&out.commands), ["RemoveAllTattoos"]);
        assert!(!h.tab.has_dialog());
    }

    #[test]
    fn clicking_another_ascendancy_sends_alloc() {
        let (mut l, s) = sample();
        let mut layout = (*l).clone();
        layout.nodes[3].ascendancy = Some("Juggernaut".into());
        layout.nodes[1].ascendancy = Some("Berserker".into());
        l = Arc::new(layout);
        let mut state = (*s).clone();
        // the build is a Berserker
        state.classes = serde_json::from_value(serde_json::json!([
            { "id": 0, "name": "Marauder", "needs_confirm": false,
              "ascendancies": [ { "id": 1, "name": "Berserker", "ascend_id": "Berserker" } ] }
        ]))
        .unwrap();
        state.ascendancy_id = 1;
        let mut h = Harness::new(l, Arc::new(state));
        // node 4 is Juggernaut and has no path; node 2 is the build's own and has one
        let out = h.click_at(h.screen_of(4));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 4 }"]);
        let out = h.click_at(h.screen_of(2));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 2 }"]);
    }

    #[test]
    fn own_ascendancy_node_without_a_path_does_nothing() {
        let (l, s) = sample();
        let mut layout = (*l).clone();
        // nodes carry the id; the dropdown label differs from it (TreeData 3_29)
        layout.nodes[3].ascendancy = Some("Raider".into());
        let mut state = (*s).clone();
        state.classes = serde_json::from_value(serde_json::json!([
            { "id": 0, "name": "Marauder", "needs_confirm": false,
              "ascendancies": [ { "id": 1, "name": "Warden", "ascend_id": "Raider" } ] }
        ]))
        .unwrap();
        state.ascendancy_id = 1;
        let mut h = Harness::new(Arc::new(layout), Arc::new(state));
        // the nodes count as the build's own, so they aren't faded either
        assert_eq!(h.tab.derived.current_ascendancy.as_deref(), Some("Raider"));
        let out = h.click_at(h.screen_of(4));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
    }

    #[test]
    fn a_resent_layout_rebuilds_the_grid_and_keeps_the_camera() {
        let (l, s) = sample();
        let mut h = Harness::new(l.clone(), s);
        let camera = h.tab.camera;
        h.tab.needs_fit = false;
        // same key, nodes in another order
        let mut reordered = (*l).clone();
        reordered.nodes.reverse();
        reordered.edges.clear();
        h.tab
            .on_snapshot(&Snapshot::TreeLayout(Arc::new(reordered)));
        assert_eq!(h.tab.index[&1], 3);
        assert!(!h.tab.needs_fit, "same version, no refit");
        assert_eq!(h.tab.camera, camera);
        let out = h.click_at(h.screen_of(2));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 2 }"]);
    }

    #[test]
    fn hover_sends_preview_after_100ms_once() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        let pos = h.screen_of(2);
        h.time = 2.0;
        let out = h.frame(vec![Event::PointerMoved(pos)]);
        assert!(out.queries.is_empty());
        assert_eq!(h.tab.hover, Some(2));
        h.time = 2.05;
        assert!(h.frame(vec![]).queries.is_empty());
        h.time = 2.11;
        let out = h.frame(vec![]);
        assert_eq!(debug(&out.queries), ["PreviewNode { id: 2 }"]);
        h.time = 3.0;
        assert!(h.frame(vec![]).queries.is_empty());
        // a tooltip for the hovered node is kept, and drawn with the node's own text
        h.tab
            .on_snapshot(&Snapshot::NodeTooltip(Arc::new(NodeTooltip {
                id: 2,
                trace: vec![],
                lines: vec![],
            })));
        h.frame(vec![]);
    }

    #[test]
    fn hovering_another_node_restarts_the_wait() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        h.time = 2.0;
        h.frame(vec![Event::PointerMoved(h.screen_of(2))]);
        h.time = 2.2;
        assert_eq!(h.frame(vec![]).queries.len(), 1);
        h.frame(vec![Event::PointerMoved(h.screen_of(1))]);
        assert!(h.frame(vec![]).queries.is_empty());
        h.time = 2.4;
        assert_eq!(debug(&h.frame(vec![]).queries), ["PreviewNode { id: 1 }"]);
    }

    #[test]
    fn empty_and_single_node_layouts_draw_and_hover() {
        let (_, state) = sample();
        for nodes in [vec![], vec![node(1, [0.0, 0.0], "Normal")]] {
            let mut h = Harness::new(Arc::new(layout(nodes, vec![])), state.clone());
            assert!(h.tab.camera.zoom > 0.0);
            h.frame(vec![Event::PointerMoved(Pos2::new(400.0, 300.0))]);
            h.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                source: egui::MouseWheelSource::Wheel,
                delta: Vec2::new(0.0, 50.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        // a camera that was never fitted must not hang or divide by zero either
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        h.tab.camera.zoom = 0.0;
        h.tab.needs_fit = false;
        h.frame(vec![Event::PointerMoved(Pos2::new(400.0, 300.0))]);
        assert_eq!(h.tab.hover, None);
    }

    #[test]
    fn unknown_ids_are_skipped() {
        let (layout, _) = sample();
        let state: TreeState = serde_json::from_value(serde_json::json!({
            "version": "3_28", "version_label": "3.28", "class_id": 0, "ascendancy_id": 0,
            "points": { "used": 1, "available": 122, "ascendancy_used": 0,
                        "ascendancy_available": 8 },
            "allocated": [1, 999],
            "paths": [ { "node": 998, "path": [998, 997] } ],
            "depends": [ { "node": 999, "path": [999] } ],
            "mastery_options": [ { "node": 996, "options": [ { "id": 1, "text": "x" } ] } ],
            "search_matches": [995], "heatmap": "Life",
            "compare": [ { "node": 994, "rgb": [1.0, 0.0, 0.0] } ],
            "jewels": [ { "node": 993, "name": "J", "ring": [0.0, 960.0] },
                        { "node": 1, "name": "J2", "ring": [0.0, 960.0] } ],
            "active_spec": 0, "can_undo": false, "can_redo": false,
        }))
        .unwrap();
        let mut h = Harness::new(layout, Arc::new(state));
        h.tab.on_snapshot(&Snapshot::NodePower(Arc::new(NodePower {
            stat: Some("Life".into()),
            nodes: vec![crate::engine::NodePowerEntry {
                id: 992,
                single: 5.0,
                path: None,
                offence: None,
                defence: None,
            }],
            max_single: 5.0,
            max_offence: 0.0,
            max_defence: 0.0,
            finished: false,
        })));
        h.tab.on_progress(40);
        h.frame(vec![]);
        h.frame(vec![Event::PointerMoved(h.screen_of(1))]);
    }

    #[test]
    fn ctrl_clicks_zoom_instead_of_acting_on_the_node() {
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        h.tab.camera.zoom = 0.2;
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::COMMAND)]);
        let start = h.tab.camera.zoom;
        let out = h.click_at(h.screen_of(2));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        let zoomed = h.tab.camera.zoom;
        assert!((zoomed / start - 1.44).abs() < 1e-3, "{start} -> {zoomed}");
        // the right click zooms out and opens nothing
        let out = h.click_with(h.screen_of(2), PointerButton::Secondary);
        assert!(out.commands.is_empty() && out.jump_to_socket.is_none());
        assert!((h.tab.camera.zoom - start).abs() < 1e-4);
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::NONE)]);
        let out = h.click_at(h.screen_of(2));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 2 }"]);
    }

    /// 1 is allocated; 2 and 6 hang off it; 5 links 2 and 6, its own path runs through 6
    fn trace_sample() -> (Arc<TreeLayout>, Arc<TreeState>) {
        let layout = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                node(2, [600.0, 0.0], "Normal"),
                node(5, [600.0, 600.0], "Normal"),
                node(6, [0.0, 600.0], "Normal"),
            ],
            vec![(0, 1), (1, 2), (2, 3), (3, 0)],
        );
        let state: TreeState = serde_json::from_value(serde_json::json!({
            "version": "3_28", "version_label": "3.28", "class_id": 0, "ascendancy_id": 0,
            "points": { "used": 1, "available": 122, "ascendancy_used": 0,
                        "ascendancy_available": 8 },
            "allocated": [1],
            "paths": [ { "node": 2, "path": [2] }, { "node": 5, "path": [5, 6] },
                       { "node": 6, "path": [6] } ],
            "active_spec": 0, "can_undo": false, "can_redo": false,
        }))
        .unwrap();
        (Arc::new(layout), Arc::new(state))
    }

    #[test]
    fn shift_traces_a_path_and_a_click_on_its_end_allocates_it() {
        let (l, s) = trace_sample();
        let mut h = Harness::new(l, s);
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::SHIFT)]);
        h.frame(vec![Event::PointerMoved(h.screen_of(2))]);
        assert_eq!(h.tab.trace_for_test(), [2]);
        // 5 links to the trace's end: it is added, though its own path goes through 6
        h.frame(vec![Event::PointerMoved(h.screen_of(5))]);
        assert_eq!(h.tab.trace_for_test(), [2, 5]);
        // the tooltip is asked with the trace, after the hover's wait
        h.time += 1.0;
        let out = h.frame(vec![Event::PointerMoved(h.screen_of(5))]);
        assert_eq!(
            debug(&out.queries),
            ["PreviewTrace { id: 5, path: [2, 5] }"]
        );
        let out = h.click_at(h.screen_of(5));
        assert_eq!(
            debug(&out.commands),
            ["AllocTracePath { id: 5, path: [2, 5] }"]
        );
        // without Shift the trace is gone and a click allocates the node's own path
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::NONE)]);
        assert!(h.tab.trace_for_test().is_empty());
        let out = h.click_at(h.screen_of(5));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 5 }"]);
    }

    #[test]
    fn under_shift_an_allocated_node_stays_hovered_and_a_click_deallocates_it() {
        let (l, s) = trace_sample();
        let mut h = Harness::new(l, s);
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::SHIFT)]);
        h.frame(vec![Event::PointerMoved(h.screen_of(1))]);
        // its path is empty, not missing (PassiveSpec.lua:898): the trace stays empty
        assert_eq!(h.tab.hover, Some(1));
        assert!(h.tab.trace_for_test().is_empty());
        let out = h.click_at(h.screen_of(1));
        assert_eq!(debug(&out.commands), ["DeallocNode { id: 1 }"]);
    }

    #[test]
    fn a_trace_passes_through_an_allocated_node() {
        let (l, s) = trace_sample();
        let mut h = Harness::new(l, s);
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::SHIFT)]);
        for id in [2, 1, 6] {
            h.frame(vec![Event::PointerMoved(h.screen_of(id))]);
        }
        assert_eq!(h.tab.trace_for_test(), [2, 1, 6]);
        let out = h.click_at(h.screen_of(6));
        assert_eq!(
            debug(&out.commands),
            ["AllocTracePath { id: 6, path: [2, 1, 6] }"]
        );
    }

    #[test]
    fn a_node_the_trace_cant_take_isnt_hovered() {
        let (l, s) = trace_sample();
        let mut h = Harness::new(l, s);
        h.frame(vec![Event::ModifiersChanged(egui::Modifiers::SHIFT)]);
        h.frame(vec![Event::PointerMoved(h.screen_of(2))]);
        // 6 isn't linked to 2: no hover, and the trace stays
        h.frame(vec![Event::PointerMoved(h.screen_of(6))]);
        assert_eq!(h.tab.trace_for_test(), [2]);
        assert_eq!(h.tab.hover, None);
        let out = h.click_at(h.screen_of(6));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
    }

    // ---- Task 13: tree Copy and reorder (NF-3) ----

    fn two_specs() -> Harness {
        let (l, s) = sample();
        let mut state = (*s).clone();
        state.specs = serde_json::from_value(serde_json::json!([
            { "title": "Main", "version": "3_28", "points": 1 },
            { "title": "Bare", "version": "3_28", "points": 0 },
        ]))
        .unwrap();
        Harness::new(l, Arc::new(state))
    }

    /// The New, Rename and Copy Tree boxes are PoB's (PassiveSpecListControl.lua:46-69): the
    /// filter drops every non-ASCII byte, Save needs a visible character after an edit, and the
    /// name is saved as it stands
    #[test]
    fn tree_names_follow_pobs_box() {
        let select_all = |h: &mut Harness| {
            h.frame(vec![Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }]);
        };
        // New
        let mut h = two_specs();
        let pos = h.tab.rects.map["spec_new"].center();
        h.click_at(pos);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_new_name"].center();
        h.click_at(pos);
        h.frame(vec![Event::Text("\u{65e5}\u{672c}".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_new_ok"].center();
        assert!(h.click_at(pos).commands.is_empty());
        let pos = h.tab.rects.map["spec_new_name"].center();
        h.click_at(pos);
        h.frame(vec![Event::Text("\u{65e5} a ".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_new_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["NewSpec { title: \" a \" }"]
        );
        // Rename: untouched, Save is off; nothing PoB keeps, Save stays off; 100 bytes at most
        let mut h = two_specs();
        let pos = h.tab.rects.map["spec_rename"].center();
        h.click_at(pos);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        assert!(h.click_at(pos).commands.is_empty());
        select_all(&mut h);
        h.frame(vec![Event::Text("\u{3a9}".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        assert!(h.click_at(pos).commands.is_empty());
        let pos = h.tab.rects.map["spec_rename_name"].center();
        h.click_at(pos);
        select_all(&mut h);
        h.frame(vec![Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        // past the limit, an insertion is refused whole
        h.frame(vec![Event::Text("x".repeat(130))]);
        h.frame(vec![Event::Text("x".repeat(100))]);
        h.frame(vec![Event::Text("y".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        let sent = debug(&h.click_at(pos).commands);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(
            sent[0],
            format!("RenameSpec {{ index: 0, title: \"{}\" }}", "x".repeat(100))
        );
        // Copy drops the same characters; a key PoB's box refuses is no edit, so Save stays off
        let mut h = two_specs();
        let pos = h.tab.rects.map["spec_copy"].center();
        h.click_at(pos);
        h.frame(vec![]);
        h.frame(vec![Event::Text("\u{65e5}".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_copy_ok"].center();
        assert!(h.click_at(pos).commands.is_empty());
    }

    /// PoB opens the rename box on the tree's own title: a tree with none opens it empty
    #[test]
    fn renaming_an_untitled_tree_starts_empty() {
        let (l, s) = sample();
        let mut state = (*s).clone();
        state.specs = serde_json::from_value(serde_json::json!([
            { "title": "Default", "untitled": true, "version": "3_28", "points": 1 },
        ]))
        .unwrap();
        let mut h = Harness::new(l, Arc::new(state));
        let pos = h.tab.rects.map["spec_rename"].center();
        h.click_at(pos);
        h.frame(vec![]);
        h.frame(vec![Event::Text("x".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["RenameSpec { index: 0, title: \"x\" }"]
        );
    }

    #[test]
    fn spec_copy_asks_for_a_name_and_sends_copy_spec() {
        let mut h = two_specs();
        let pos = h.tab.rects.map["spec_copy"].center();
        assert!(h.click_at(pos).commands.is_empty());
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_copy_ok"].center();
        // PoB's Save waits for the name to be edited
        assert!(h.click_at(pos).commands.is_empty());
        h.frame(vec![Event::Text("2".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_copy_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["CopySpec { index: 0, title: \"Main2\" }"]
        );
        // the manager's Copy copies its selected row (PassiveSpecListControl.lua:13-21); a
        // click on a row only selects it
        let pos = h.tab.rects.map["spec_order"].center();
        h.click_at(pos);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_order_row:1"].center();
        assert!(h.click_at(pos).commands.is_empty());
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_manager_copy"].center();
        h.click_at(pos);
        h.frame(vec![]);
        h.frame(vec![Event::Text("!".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_copy_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["CopySpec { index: 1, title: \"Bare!\" }"]
        );
    }

    fn key(h: &mut Harness, key: egui::Key) -> TreeOutput {
        h.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }])
    }

    /// F2 in "Manage Passive Trees" renames its selected row, not the active tree
    /// (PassiveSpecListControl.lua's OnSelKeyDown); the arrows pick the row
    #[test]
    fn f2_in_the_tree_manager_renames_the_selected_tree() {
        let mut h = two_specs();
        let pos = h.tab.rects.map["spec_order"].center();
        h.click_at(pos);
        h.frame(vec![]);
        assert!(h.tab.has_dialog(), "the manager counts as a dialog");
        let pos = h.tab.rects.map["spec_order_row:1"].center();
        h.click_at(pos);
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        // PoB's Save waits for an edit, even on the name it opened with
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        assert!(h.click_at(pos).commands.is_empty());
        h.frame(vec![Event::Text("!".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["RenameSpec { index: 1, title: \"Bare!\" }"]
        );
        // the manager is still open; a typed Copy Tree name box leaves it the keyboard
        h.frame(vec![]);
        assert!(h.tab.spec_manager_open());
        let pos = h.tab.rects.map["spec_manager_copy"].center();
        h.click_at(pos);
        h.frame(vec![]);
        h.frame(vec![Event::Text("!".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_copy_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["CopySpec { index: 1, title: \"Bare!\" }"]
        );
        h.frame(vec![]);
        // an untyped Copy Tree box holds the keyboard until Cancel takes it away
        let pos = h.tab.rects.map["spec_manager_copy"].center();
        h.click_at(pos);
        h.frame(vec![]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_copy_cancel"].center();
        assert!(h.click_at(pos).commands.is_empty());
        h.frame(vec![]);
        h.frame(vec![]);
        // Up then F2 renames the first tree
        key(&mut h, egui::Key::ArrowUp);
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        h.frame(vec![Event::Text("!".into())]);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_rename_ok"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["RenameSpec { index: 0, title: \"Main!\" }"]
        );
    }

    /// Delete in the manager asks about the selected tree; F2 on the tab itself does nothing,
    /// as PoB's F2 belongs to the manager's list
    #[test]
    fn delete_in_the_tree_manager_asks_and_f2_off_it_does_nothing() {
        let mut h = two_specs();
        key(&mut h, egui::Key::F2);
        h.frame(vec![]);
        assert!(!h.tab.has_dialog());
        let pos = h.tab.rects.map["spec_order"].center();
        h.click_at(pos);
        h.frame(vec![]);
        key(&mut h, egui::Key::ArrowDown);
        key(&mut h, egui::Key::Delete);
        h.frame(vec![]);
        let pos = h.tab.rects.map["spec_delete_yes"].center();
        assert_eq!(
            debug(&h.click_at(pos).commands),
            ["DeleteSpec { index: 1 }"]
        );
    }

    /// The arrows over the Version dropdown step it as a pick: PoB's Convert question asks
    /// (DropDownControl.lua:476-516, TreeTab.lua:652)
    #[test]
    fn the_arrows_over_the_version_dropdown_ask_to_convert() {
        let (l, s) = sample();
        let mut state = (*s).clone();
        state.versions = serde_json::from_value(serde_json::json!([
            { "value": "3_27", "label": "3.27" },
            { "value": "3_28", "label": "3.28" },
        ]))
        .unwrap();
        let mut h = Harness::new(l, Arc::new(state));
        let pos = h.tab.rects.map["version"].center();
        h.frame(vec![Event::PointerMoved(pos)]);
        h.frame(vec![]);
        assert!(!h.tab.has_dialog());
        let out = key(&mut h, egui::Key::ArrowUp);
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        h.frame(vec![]);
        assert!(h.tab.has_dialog(), "no Convert question");
        assert!(h.tab.rects.map.contains_key("version_convert"));
    }

    #[test]
    fn dragging_a_tree_sends_move_spec() {
        let mut h = two_specs();
        let pos = h.tab.rects.map["spec_order"].center();
        h.click_at(pos);
        h.frame(vec![]);
        let from = h.tab.rects.map["spec_order_row:0"].center();
        let r = h.tab.rects.map["spec_order_row:1"];
        let to = Pos2::new(r.center().x, r.bottom() - 2.0);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut all = vec![];
        for events in [
            vec![Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![Event::PointerMoved(Pos2::new(from.x + 30.0, from.y + 6.0))],
            vec![Event::PointerMoved(to)],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
            vec![],
        ] {
            all.extend(h.frame(events).commands);
        }
        assert_eq!(debug(&all), ["MoveSpec { from: 0, to: 1 }"]);
    }

    // ---- Task 13: heat map Custom depth (NF-6), Version (NF-7), report filter (NF-5) ----

    fn heat_harness() -> Harness {
        let (l, s) = sample();
        let mut state = (*s).clone();
        state.heatmap = Some("Life".into());
        state.power_stats = vec!["Life".into()];
        state.versions = serde_json::from_value(serde_json::json!([
            { "label": "3.28", "value": "3_28" },
            { "label": "3.29", "value": "3_29" },
        ]))
        .unwrap();
        let mut h = Harness::new(l, Arc::new(state));
        h.frame(vec![]);
        h
    }

    /// `click_key`, then a frame for what the click opened
    fn press(h: &mut Harness, key: &str) -> TreeOutput {
        let out = click_key(h, key);
        h.frame(vec![]);
        out
    }

    #[test]
    fn custom_depth_box_sends_its_depth() {
        let mut h = heat_harness();
        press(&mut h, "heat_depth");
        press(&mut h, "depth:custom");
        press(&mut h, "depth_custom");
        h.frame(vec![Event::Text("7a".into())]);
        let enter = Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let out = h.frame(vec![enter]);
        assert_eq!(
            debug(&out.commands),
            ["SetHeatmap { stat: Some(\"Life\"), max_depth: Some(7) }"]
        );
    }

    fn wheel(y: f32) -> Event {
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, y),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// The hand-merged depth step (13a's Custom entry with Task 14's wheel): All, 5, 10, 15,
    /// then Custom, which opens its box with the depth; off Custom the box goes and the depth
    /// is sent
    #[test]
    fn the_wheel_steps_the_heat_depth_onto_and_off_custom() {
        let mut h = heat_harness();
        let mut state = (*h.tab.state.clone().unwrap()).clone();
        state.max_depth = Some(15);
        h.tab.on_snapshot(&Snapshot::Tree(Arc::new(state)));
        h.frame(vec![]);
        let depth = h.tab.rects.map["heat_depth"].center();
        h.frame(vec![Event::PointerMoved(depth)]);
        // from 15 down: Custom, its box holding 15, nothing sent yet
        let out = h.frame(vec![wheel(-1.0)]);
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        h.frame(vec![]);
        assert_eq!(h.tab.toolbar.custom_depth_for_test(), Some("15"));
        // back up: the box goes and 15 is sent again only if it changed (it didn't)
        let out = h.frame(vec![wheel(1.0)]);
        assert!(h.tab.toolbar.custom_depth_for_test().is_none());
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        // and up again: 10
        h.frame(vec![]);
        let out = h.frame(vec![wheel(1.0)]);
        assert_eq!(
            debug(&out.commands),
            ["SetHeatmap { stat: Some(\"Life\"), max_depth: Some(10) }"]
        );
    }

    /// The Custom depth box is a %D box: its + steps the depth and sends it at once
    /// (EditControl.lua:64-75, 680-700)
    #[test]
    fn the_custom_depth_box_steps_on_its_plus_button() {
        let mut h = heat_harness();
        let mut state = (*h.tab.state.clone().unwrap()).clone();
        state.max_depth = Some(15);
        h.tab.on_snapshot(&Snapshot::Tree(Arc::new(state)));
        h.frame(vec![]);
        let depth = h.tab.rects.map["heat_depth"].center();
        h.frame(vec![Event::PointerMoved(depth)]);
        h.frame(vec![wheel(-1.0)]);
        h.frame(vec![]);
        assert_eq!(h.tab.toolbar.custom_depth_for_test(), Some("15"));
        let out = press(&mut h, "depth_custom_plus");
        assert_eq!(
            debug(&out.commands),
            ["SetHeatmap { stat: Some(\"Life\"), max_depth: Some(16) }"]
        );
    }

    #[test]
    fn version_pick_asks_and_converts() {
        let mut h = heat_harness();
        press(&mut h, "version");
        assert!(press(&mut h, "version:3_29").commands.is_empty());
        let out = press(&mut h, "version_convert_copy");
        assert_eq!(
            debug(&out.commands),
            ["ConvertSpec { version: \"3_29\", keep_copy: true }"]
        );
        // the tree's own version isn't offered as a conversion
        press(&mut h, "version");
        assert!(press(&mut h, "version:3_28").commands.is_empty());
        assert!(!h.tab.toolbar.has_dialog());
    }

    #[test]
    fn report_filter_and_masteries_send_pobs_choice() {
        let mut h = heat_harness();
        h.tab
            .on_snapshot(&Snapshot::PowerReport(Arc::new(PowerReport {
                stat: "Life".into(),
                rows: vec![],
                listed: vec![],
                filter: 0,
                masteries: true,
            })));
        press(&mut h, "report_toggle");
        let out = press(&mut h, "report_masteries");
        assert_eq!(
            debug(&out.commands),
            ["SetPowerReportFilter { filter: 0, masteries: false }"]
        );
        press(&mut h, "report_filter");
        let out = press(&mut h, "report_filter:2");
        assert_eq!(
            debug(&out.commands),
            ["SetPowerReportFilter { filter: 2, masteries: true }"]
        );
    }

    /// 13a's power report filter steps with the wheel (merge follow-up)
    #[test]
    fn the_wheel_steps_the_power_report_filter() {
        let mut h = heat_harness();
        h.tab
            .on_snapshot(&Snapshot::PowerReport(Arc::new(PowerReport {
                stat: "Life".into(),
                rows: vec![],
                listed: vec![],
                filter: 0,
                masteries: true,
            })));
        press(&mut h, "report_toggle");
        let filter = h.tab.rects.map["report_filter"].center();
        h.frame(vec![Event::PointerMoved(filter)]);
        let out = h.frame(vec![wheel(-1.0)]);
        assert_eq!(
            debug(&out.commands),
            ["SetPowerReportFilter { filter: 1, masteries: true }"]
        );
    }

    #[test]
    fn an_older_tree_offers_pobs_convert_button() {
        let (l, s) = sample();
        let mut state = (*s).clone();
        state.convertible_to = Some("3_29".into());
        let mut h = Harness::new(l, Arc::new(state));
        h.frame(vec![]);
        let out = press(&mut h, "spec_convert_latest");
        assert_eq!(debug(&out.commands), ["ConvertSpecLatest"]);
        // the newest tree has no such button
        let mut h = heat_harness();
        h.tab.rects.map.clear();
        h.frame(vec![]);
        assert!(!h.tab.rects.map.contains_key("spec_convert_latest"));
    }

    #[test]
    fn the_compare_view_has_no_tree_buttons() {
        // CompareTab.lua:840-900: the tree set dropdown, but no New, Rename, Delete, Copy or
        // Manage
        let (l, s) = sample();
        let mut h = Harness::new(l, s);
        assert!(h.tab.rects.map.contains_key("spec_new"));
        h.tab.rects.map.clear();
        h.compare = true;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.tab.rects.map.contains_key("spec_combo"));
        for key in [
            "spec_new",
            "spec_rename",
            "spec_delete",
            "spec_copy",
            "spec_order",
        ] {
            assert!(!h.tab.rects.map.contains_key(key), "{key} drawn");
        }
    }

    #[test]
    fn another_class_ascendancy_click_asks_pobs_class_change() {
        let mut necro = node(4, [600.0, 600.0], "Normal");
        necro.ascendancy = Some("Necromancer".into());
        let layout = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                node(2, [600.0, 0.0], "Normal"),
                necro,
            ],
            vec![(0, 1)],
        );
        let (_, s) = sample();
        let mut state = (*s).clone();
        state.classes = serde_json::from_value(serde_json::json!([
            { "id": 0, "name": "Scion", "needs_confirm": false, "ascendancies": [] },
            { "id": 3, "name": "Witch", "needs_confirm": true, "ascendancies": [
                { "id": 0, "name": "None", "ascend_id": "" },
                { "id": 3, "name": "Necromancer", "ascend_id": "Necromancer" } ] },
        ]))
        .unwrap();
        // the Compare tab's tree view asks it too: PoB's overlay is the build's own tree
        // view (CompareTab.lua:3401, PassiveTreeView.lua:474)
        let mut h = Harness::new(Arc::new(layout.clone()), Arc::new(state.clone()));
        h.compare = true;
        h.frame(vec![]);
        h.frame(vec![]);
        let out = h.click_at(h.screen_of(4));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert!(h.tab.has_question(), "the compare view didn't ask");
        // the app keeps it on that view, and drops any other question there
        h.tab.close_dialog_off_tab(true);
        assert!(
            h.tab.has_question(),
            "the app dropped the compare view's question"
        );
        h.frame(vec![]);
        let out = press(&mut h, "node_class_connect");
        assert_eq!(
            debug(&out.commands),
            ["AllocNodeChangingClass { id: 4, connect: true }"]
        );
        h.tab.close_dialog_off_tab(false);
        assert!(!h.tab.has_question());
        let mut h = Harness::new(Arc::new(layout.clone()), Arc::new(state.clone()));
        let out = h.click_at(h.screen_of(4));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        h.frame(vec![]);
        let out = press(&mut h, "node_class_connect");
        assert_eq!(
            debug(&out.commands),
            ["AllocNodeChangingClass { id: 4, connect: true }"]
        );
        // a tree that connects to the class switches with the click, as PoB does
        state.classes[1].needs_confirm = false;
        let mut h = Harness::new(Arc::new(layout), Arc::new(state));
        let out = h.click_at(h.screen_of(4));
        assert_eq!(debug(&out.commands), ["AllocNode { id: 4 }"]);
    }
}
