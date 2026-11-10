//! The passive tree tab: the canvas plus the state it needs between frames. It takes engine
//! snapshots and returns commands; it knows nothing else about PoB.

pub mod canvas;
pub mod geometry;
pub mod overlays;
pub mod power_report;
pub mod toolbar;

use crate::engine::{
    Command, NodePower, NodeTooltip, PowerReport, Snapshot, TreeLayout, TreeState,
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
}

/// How long a node pulses after a power report row was clicked (seconds).
pub(super) const PULSE_SECONDS: f64 = 1.5;

/// What the tab asks of the engine in one frame.
#[derive(Debug, Default)]
pub struct TreeOutput {
    pub commands: Vec<Command>,
    /// Questions about the hovered node; the app may send only the latest of them.
    pub queries: Vec<Command>,
}

/// Lookups over [`TreeState`], built once per snapshot rather than once per frame.
#[derive(Debug, Default)]
pub(super) struct Derived {
    pub allocated: HashSet<u32>,
    pub paths: HashMap<u32, Vec<u32>>,
    pub depends: HashMap<u32, Vec<u32>>,
    pub matches: HashSet<u32>,
    pub compare: HashMap<u32, [f32; 3]>,
    /// The name nodes of the build's ascendancy carry
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
            .map(|a| a.name.clone());
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
}

impl TreeTab {
    /// Forgets the build's state, as for another build. Uploaded textures stay.
    ///
    /// The layout stays: the engine resends it only when its key changes, and a new snapshot
    /// with another key replaces it.
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

    /// Drops a pending confirmation; its dialog only draws on this tab.
    pub fn close_dialog(&mut self) {
        self.toolbar.close_dialog();
    }

    /// A toolbar dialog is open, so shortcuts must wait.
    pub fn has_dialog(&self) -> bool {
        self.toolbar.has_dialog()
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
                if self.layout.as_ref().is_none_or(|l| l.key != layout.key) {
                    self.grid = HitGrid::new(layout);
                    self.index = layout
                        .nodes
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (n.id, i))
                        .collect();
                    self.needs_fit = true;
                    self.hover = None;
                    self.previewed = None;
                }
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
                    clicked = self
                        .report_panel
                        .show(ui, report.as_deref(), &mut self.rects);
                });
            if let Some(id) = clicked {
                let now = ui.input(|i| i.time);
                self.focus_node(id, now);
            }
        }
        let power = self.power.clone();
        let actions = canvas::show(ui, self, &layout, &state, power.as_deref());
        let now = ui.input(|i| i.time);
        for action in actions {
            match action {
                CanvasAction::Alloc(id) => out.commands.push(Command::AllocNode { id }),
                CanvasAction::Dealloc(id) => out.commands.push(Command::DeallocNode { id }),
                CanvasAction::PickMastery(id) => self.picker = Some(id),
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
                out.queries.push(Command::PreviewNode { id });
                self.previewed = Some(id);
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(PREVIEW_DELAY - waited));
            }
        }

        self.show_picker(ui.ctx(), &layout, &state, &mut out);
        out
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
        let mut open = true;
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

#[cfg(test)]
mod tests {
    use super::geometry::tests::{layout, node};
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};

    /// 1 is allocated; 2 is reachable through 1; 3 is a mastery with two effects; 4 can't be
    /// reached.
    fn sample() -> (Arc<TreeLayout>, Arc<TreeState>) {
        let layout = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                node(2, [600.0, 0.0], "Normal"),
                node(3, [0.0, 600.0], "Mastery"),
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
                    let o = self.tab.show(ui, false);
                    out.commands.extend(o.commands);
                    out.queries.extend(o.queries);
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
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
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
}
