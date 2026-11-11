//! The engine thread: a command loop over a [`Backend`], and the [`Engine`] handle the UI holds.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crate::pob_files::DownloadProgress;

use super::commands::{Command, GemTooltipQuery, ItemTooltipQuery};
use super::views::{
    BreakdownView, BuildListView, BuildView, CalcsView, CellRef, ConfigView, GemSearchView,
    GemTooltipView, GroupTooltipView, ItemText, ItemTooltipView, ItemsView, NodePower, NodeTooltip,
    NotesView, PowerReport, SkillsView, SocketGroupText, StatsView, TreeLayout, TreeState,
};

#[derive(Debug, Clone)]
pub enum Snapshot {
    Build(Arc<BuildView>),
    BuildList(Arc<BuildListView>),
    Stats(Arc<StatsView>),
    Config(Arc<ConfigView>),
    TreeLayout(Arc<TreeLayout>),
    Tree(Arc<TreeState>),
    NodePower(Arc<NodePower>),
    PowerReport(Arc<PowerReport>),
    NodeTooltip(Arc<NodeTooltip>),
    Breakdown(Arc<BreakdownView>),
    TreeLink(Arc<String>),
    Calcs(Arc<CalcsView>),
    Skills(Arc<SkillsView>),
    Items(Arc<ItemsView>),
    SocketGroupText(Arc<SocketGroupText>),
    GroupTooltip(Arc<GroupTooltipView>),
    GemSearch(Arc<GemSearchView>),
    GemTooltip(Arc<GemTooltipView>),
    ItemTooltip(Arc<ItemTooltipView>),
    ItemText(Arc<ItemText>),
    Notes(Arc<NotesView>),
}

/// Long-running work the engine does between commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTask {
    /// The gem picker's DPS sort; it goes first whenever it has work (R12)
    GemDps,
    NodePower,
}

#[derive(Debug, Clone)]
pub enum Event {
    Downloading(DownloadProgress),
    Ready,
    StartupFailed {
        message: String,
    },
    Snapshot {
        seq: u64,
        snapshot: Snapshot,
    },
    Error {
        seq: u64,
        message: String,
    },
    Progress {
        task: BackgroundTask,
        percent: u32,
    },
    /// A background step failed; its work stops until something restarts it.
    BackgroundFailed {
        task: BackgroundTask,
        message: String,
    },
}

/// What a command changed, so the loop knows which snapshots to send.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Affected {
    pub build: bool,
    pub list: bool,
    pub stats: bool,
    pub config: bool,
    /// The tree's geometry changed: a new version, or a different cluster jewel subgraph
    pub tree_layout: bool,
    /// The build's state on the tree changed
    pub tree: bool,
    pub recalc: bool,
    /// The Calcs tab's sections changed
    pub calcs: bool,
    /// The Skills tab changed
    pub skills: bool,
    /// The Items tab changed
    pub items: bool,
    /// The build's notes changed
    pub notes: bool,
    /// Node power changed (a background step ran)
    pub power: bool,
    /// The gem search changed: it opened, its text changed or a DPS step ran
    pub gem_search: bool,
    /// The power report is ready
    pub report: bool,
    /// A tooltip was requested for this node; the latest request wins
    pub preview: Option<u32>,
    /// The breakdown of this Calcs cell was requested; the latest request wins
    pub breakdown: Option<CellRef>,
    /// The text PoB copies for this socket group was requested; the latest request wins
    pub group_text: Option<usize>,
    /// The tooltip of this socket group was requested; the latest request wins
    pub group_tooltip: Option<usize>,
    /// This gem tooltip was requested; the latest request wins
    pub gem_tooltip: Option<GemTooltipQuery>,
    /// This item tooltip was requested; the latest request wins
    pub item_tooltip: Option<ItemTooltipQuery>,
    /// The text PoB copies for this item was requested; the latest request wins
    pub item_text: Option<u32>,
    /// The tree link was requested
    pub link: bool,
}

impl Affected {
    pub(crate) const ALL_VIEWS: Affected = Affected {
        build: true,
        list: true,
        stats: true,
        config: true,
        tree_layout: false,
        tree: true,
        recalc: false,
        calcs: true,
        skills: true,
        items: true,
        notes: true,
        power: false,
        gem_search: false,
        report: false,
        preview: None,
        breakdown: None,
        group_text: None,
        group_tooltip: None,
        gem_tooltip: None,
        item_tooltip: None,
        item_text: None,
        link: false,
    };

    fn merge(&mut self, other: Affected) {
        self.build |= other.build;
        self.list |= other.list;
        self.stats |= other.stats;
        self.config |= other.config;
        self.tree_layout |= other.tree_layout;
        self.tree |= other.tree;
        self.recalc |= other.recalc;
        self.calcs |= other.calcs;
        self.skills |= other.skills;
        self.items |= other.items;
        self.notes |= other.notes;
        self.power |= other.power;
        self.gem_search |= other.gem_search;
        self.report |= other.report;
        self.preview = other.preview.or(self.preview);
        if other.breakdown.is_some() {
            self.breakdown = other.breakdown;
        }
        self.group_text = other.group_text.or(self.group_text);
        self.group_tooltip = other.group_tooltip.or(self.group_tooltip);
        if other.gem_tooltip.is_some() {
            self.gem_tooltip = other.gem_tooltip;
        }
        if other.item_tooltip.is_some() {
            self.item_tooltip = other.item_tooltip;
        }
        self.item_text = other.item_text.or(self.item_text);
        self.link |= other.link;
    }
}

pub trait Backend {
    fn apply(&mut self, command: Command, send: &dyn Fn(Event)) -> anyhow::Result<Affected>;
    fn recalculate(&mut self) -> anyhow::Result<()>;
    fn build_view(&mut self) -> anyhow::Result<BuildView>;
    fn build_list_view(&mut self) -> anyhow::Result<BuildListView>;
    fn stats_view(&mut self) -> anyhow::Result<StatsView>;
    fn config_view(&mut self) -> anyhow::Result<ConfigView>;
    fn tree_layout_view(&mut self) -> anyhow::Result<TreeLayout>;
    fn tree_state_view(&mut self) -> anyhow::Result<TreeState>;
    fn calcs_view(&mut self) -> anyhow::Result<CalcsView>;
    fn skills_view(&mut self) -> anyhow::Result<SkillsView>;
    fn items_view(&mut self) -> anyhow::Result<ItemsView>;
    fn notes_view(&mut self) -> anyhow::Result<NotesView>;
    /// What copying this socket group writes; an error for a group that doesn't exist or can't be copied.
    fn socket_group_text(&mut self, group: usize) -> anyhow::Result<SocketGroupText>;
    /// What hovering this socket group shows. A failure is answered with empty lines (R15).
    fn group_tooltip_view(&mut self, group: usize) -> anyhow::Result<GroupTooltipView>;
    /// The background job to run a step of next, by priority (R12); `None` when no job has
    /// steps left. After a job's step failed it must not be offered again until restarted.
    fn next_background(&self) -> Option<BackgroundTask>;
    /// Runs one slice of `task`; may `send` Progress. Returns what to snapshot.
    fn background_step(
        &mut self,
        task: BackgroundTask,
        send: &dyn Fn(Event),
    ) -> anyhow::Result<Affected>;
    /// The open gem search as it stands; `None` when no search is open.
    fn gem_search_view(&mut self) -> anyhow::Result<Option<GemSearchView>>;
    /// A gem tooltip. A failure is answered with empty lines (R15).
    fn gem_tooltip_view(&mut self, query: &GemTooltipQuery) -> anyhow::Result<GemTooltipView>;
    /// `None`: the item or slot is gone
    fn item_tooltip_view(
        &mut self,
        query: &ItemTooltipQuery,
    ) -> anyhow::Result<Option<ItemTooltipView>>;
    fn item_text(&mut self, id: u32) -> anyhow::Result<ItemText>;
    fn node_power_view(&mut self) -> anyhow::Result<NodePower>;
    fn power_report_view(&mut self) -> anyhow::Result<PowerReport>;
    /// `Ok(None)` when the node no longer exists: nothing is sent.
    fn node_tooltip_view(&mut self, id: u32) -> anyhow::Result<Option<NodeTooltip>>;
    /// Empty sections when the cell is gone or has no breakdown.
    fn breakdown_view(&mut self, cell: &CellRef) -> anyhow::Result<BreakdownView>;
    fn tree_link_view(&mut self) -> anyhow::Result<String>;
}

fn send_snapshots(backend: &mut dyn Backend, seq: u64, flags: Affected, send: &dyn Fn(Event)) {
    fn emit<T>(
        seq: u64,
        send: &dyn Fn(Event),
        result: anyhow::Result<T>,
        wrap: impl FnOnce(Arc<T>) -> Snapshot,
    ) {
        send(match result {
            Ok(view) => Event::Snapshot {
                seq,
                snapshot: wrap(Arc::new(view)),
            },
            Err(e) => Event::Error {
                seq,
                message: format!("{e:#}"),
            },
        });
    }
    if flags.build {
        emit(seq, send, backend.build_view(), Snapshot::Build);
    }
    if flags.list {
        emit(seq, send, backend.build_list_view(), Snapshot::BuildList);
    }
    if flags.stats {
        emit(seq, send, backend.stats_view(), Snapshot::Stats);
    }
    if flags.config {
        emit(seq, send, backend.config_view(), Snapshot::Config);
    }
    // the state refers to the layout, so the layout goes first
    if flags.tree_layout {
        emit(seq, send, backend.tree_layout_view(), Snapshot::TreeLayout);
    }
    if flags.tree {
        emit(seq, send, backend.tree_state_view(), Snapshot::Tree);
    }
    if flags.calcs {
        emit(seq, send, backend.calcs_view(), Snapshot::Calcs);
    }
    if flags.skills {
        emit(seq, send, backend.skills_view(), Snapshot::Skills);
    }
    if flags.items {
        emit(seq, send, backend.items_view(), Snapshot::Items);
    }
    if flags.notes {
        emit(seq, send, backend.notes_view(), Snapshot::Notes);
    }
    if flags.power {
        emit(seq, send, backend.node_power_view(), Snapshot::NodePower);
    }
    if flags.report {
        emit(
            seq,
            send,
            backend.power_report_view(),
            Snapshot::PowerReport,
        );
    }
    if let Some(id) = flags.preview {
        match backend.node_tooltip_view(id) {
            Ok(Some(tooltip)) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::NodeTooltip(Arc::new(tooltip)),
            }),
            Ok(None) => {}
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
    }
    if let Some(cell) = &flags.breakdown {
        // A failure is the answer to this question (the panel says so), not a notice, and it
        // stays attached to its cell however many other errors the batch has.
        let view = backend
            .breakdown_view(cell)
            .unwrap_or_else(|e| BreakdownView {
                cell: cell.clone(),
                sections: vec![],
                error: Some(format!("{e:#}")),
            });
        send(Event::Snapshot {
            seq,
            snapshot: Snapshot::Breakdown(Arc::new(view)),
        });
    }
    if let Some(group) = flags.group_text {
        emit(
            seq,
            send,
            backend.socket_group_text(group),
            Snapshot::SocketGroupText,
        );
    }
    if let Some(group) = flags.group_tooltip {
        let view = backend
            .group_tooltip_view(group)
            .unwrap_or_else(|_| GroupTooltipView {
                group,
                lines: vec![],
            });
        send(Event::Snapshot {
            seq,
            snapshot: Snapshot::GroupTooltip(Arc::new(view)),
        });
    }
    if flags.gem_search {
        match backend.gem_search_view() {
            Ok(Some(view)) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::GemSearch(Arc::new(view)),
            }),
            Ok(None) => {}
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
    }
    if let Some(query) = &flags.gem_tooltip {
        let view = backend
            .gem_tooltip_view(query)
            .unwrap_or_else(|_| GemTooltipView {
                query: query.clone(),
                lines: vec![],
            });
        send(Event::Snapshot {
            seq,
            snapshot: Snapshot::GemTooltip(Arc::new(view)),
        });
    }
    if let Some(query) = &flags.item_tooltip {
        match backend.item_tooltip_view(query) {
            Ok(Some(view)) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::ItemTooltip(Arc::new(view)),
            }),
            // a vanished item or slot sends nothing
            Ok(None) => {}
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
    }
    if let Some(id) = flags.item_text {
        emit(seq, send, backend.item_text(id), Snapshot::ItemText);
    }
    if flags.link {
        emit(seq, send, backend.tree_link_view(), Snapshot::TreeLink);
    }
}

/// Processes commands until `Shutdown` or until every sender is gone.
///
/// Each wake-up drains the queue into a batch, applies it, recalculates at most once, and
/// sends one snapshot per affected view, tagged with the batch's last `seq`.
///
/// While the backend has background work, the loop runs one step whenever no command is
/// waiting, so commands always go first. Background snapshots carry the last command's `seq`
/// (0 before any), so the UI's idea of "behind" never moves backwards.
pub fn run_loop(
    backend: &mut dyn Backend,
    commands: &Receiver<(u64, Command)>,
    send: &dyn Fn(Event),
) {
    let mut last_seq = 0;
    loop {
        let first = if let Some(task) = backend.next_background() {
            match commands.try_recv() {
                Ok(first) => first,
                Err(TryRecvError::Empty) => {
                    match backend.background_step(task, send) {
                        Ok(affected) => send_snapshots(backend, last_seq, affected, send),
                        Err(e) => send(Event::BackgroundFailed {
                            task,
                            message: format!("{e:#}"),
                        }),
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match commands.recv() {
                Ok(first) => first,
                Err(_) => return,
            }
        };
        let mut batch = vec![first];
        while let Ok(next) = commands.try_recv() {
            batch.push(next);
        }
        let mut affected = Affected::default();
        let mut shutdown = false;
        for (seq, command) in batch {
            last_seq = seq;
            if matches!(command, Command::Shutdown) {
                shutdown = true;
                break;
            }
            match backend.apply(command, send) {
                Ok(a) => affected.merge(a),
                Err(e) => {
                    send(Event::Error {
                        seq,
                        message: format!("{e:#}"),
                    });
                    affected.merge(Affected::ALL_VIEWS);
                }
            }
        }
        if affected.recalc {
            if let Err(e) = backend.recalculate() {
                send(Event::Error {
                    seq: last_seq,
                    message: format!("{e:#}"),
                });
            }
            // Config visibility depends on the calculation.
            affected.stats = true;
            affected.config = true;
            // Allocated points and the paths between them depend on the calculation too.
            affected.tree = true;
            affected.calcs = true;
            affected.skills = true;
            affected.items = true;
        }
        send_snapshots(backend, last_seq, affected, send);
        if shutdown {
            return;
        }
    }
}

/// Clears the flag when the engine thread ends, including by panic.
struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// UI-side handle to the engine thread. Use from the UI thread only.
pub struct Engine {
    commands: Sender<(u64, Command)>,
    next_seq: Cell<u64>,
    alive: Arc<AtomicBool>,
    pub events: Receiver<Event>,
}

impl Engine {
    /// Starts the engine thread. `start` runs on that thread and builds the backend (it may
    /// send `Downloading` events); then `Ready` and the initial snapshots (`seq` 0) go out and
    /// the command loop runs. `wake_ui` is called after every event.
    pub fn spawn_with(
        start: impl FnOnce(&dyn Fn(Event)) -> anyhow::Result<Box<dyn Backend>> + Send + 'static,
        wake_ui: impl Fn() + Send + 'static,
    ) -> Engine {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let thread_alive = alive.clone();
        std::thread::spawn(move || {
            // Declared inside the body so it drops before the captured `event_tx`: by the time
            // the UI sees the events channel disconnect, `is_alive` already reads false.
            let _guard = AliveGuard(thread_alive);
            let send = |event: Event| {
                let _ = event_tx.send(event);
                wake_ui();
            };
            let mut backend = match start(&send) {
                Ok(backend) => backend,
                Err(e) => {
                    send(Event::StartupFailed {
                        message: format!("{e:#}"),
                    });
                    return;
                }
            };
            send(Event::Ready);
            let initial = Affected {
                tree_layout: true,
                ..Affected::ALL_VIEWS
            };
            send_snapshots(backend.as_mut(), 0, initial, &send);
            run_loop(backend.as_mut(), &command_rx, &send);
        });
        Engine {
            commands: command_tx,
            next_seq: Cell::new(1),
            alive,
            events: event_rx,
        }
    }

    /// Queues a command and returns its seq (starting at 1). Never blocks or panics, even if
    /// the engine thread has exited.
    pub fn send(&self, command: Command) -> u64 {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        let _ = self.commands.send((seq, command));
        seq
    }

    /// False once the engine thread has exited.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};

    use super::*;
    use crate::engine::commands::{Command, ConfigValue};
    use crate::engine::fake::Fake;

    fn set_config() -> Command {
        Command::SetConfig {
            var: "x".into(),
            value: ConfigValue::Bool(true),
        }
    }

    /// Runs the loop over pre-queued commands (sender dropped) and returns the events.
    fn run_queued(backend: &mut Fake, commands: Vec<Command>) -> Vec<Event> {
        let (tx, rx) = mpsc::channel();
        for (i, c) in commands.into_iter().enumerate() {
            tx.send((i as u64 + 1, c)).unwrap();
        }
        drop(tx);
        let events = RefCell::new(Vec::new());
        run_loop(backend, &rx, &|e| events.borrow_mut().push(e));
        events.into_inner()
    }

    fn snapshots(events: &[Event]) -> Vec<(u64, &Snapshot)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot { seq, snapshot } => Some((*seq, snapshot)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn recalc_sends_calcs_view() {
        let (mut fake, _log) = Fake::new();
        let events = run_queued(&mut fake, vec![set_config()]);
        let calcs = snapshots(&events)
            .into_iter()
            .filter(|(_, s)| matches!(s, Snapshot::Calcs(_)))
            .count();
        assert_eq!(calcs, 1);
    }

    #[test]
    fn recalc_sends_skills_view() {
        let (mut fake, _log) = Fake::new();
        let events = run_queued(&mut fake, vec![set_config()]);
        let skills = snapshots(&events)
            .into_iter()
            .filter(|(_, s)| matches!(s, Snapshot::Skills(_)))
            .count();
        assert_eq!(skills, 1);
    }

    #[test]
    fn recalc_sends_items_view() {
        let (mut fake, _log) = Fake::new();
        let events = run_queued(&mut fake, vec![set_config()]);
        let items = snapshots(&events)
            .into_iter()
            .filter(|(_, s)| matches!(s, Snapshot::Items(_)))
            .count();
        assert_eq!(items, 1);
    }

    #[test]
    fn queued_commands_share_one_recalculation() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, (0..5).map(|_| set_config()).collect());
        assert_eq!(log.lock().unwrap().recalculations, 1);
        let snaps = snapshots(&events);
        let stats: Vec<_> = snaps
            .iter()
            .filter(|(_, s)| matches!(s, Snapshot::Stats(_)))
            .collect();
        let config: Vec<_> = snaps
            .iter()
            .filter(|(_, s)| matches!(s, Snapshot::Config(_)))
            .collect();
        assert_eq!(stats.len(), 1);
        assert_eq!(config.len(), 1);
        let pos = |wanted: fn(&Snapshot) -> bool| snaps.iter().position(|(_, s)| wanted(s));
        assert!(
            pos(|s| matches!(s, Snapshot::Stats(_))) < pos(|s| matches!(s, Snapshot::Config(_)))
        );
        assert_eq!(stats[0].0, 5);
        assert_eq!(config[0].0, 5);
    }

    #[test]
    fn failing_command_reports_error_and_resnapshots() {
        let (mut fake, _log) = Fake::new();
        fake.fail_set_config = Some("boom".into());
        let events = run_queued(&mut fake, vec![set_config()]);
        match &events[0] {
            Event::Error { seq, message } => {
                assert_eq!(*seq, 1);
                assert!(message.contains("boom"), "{message}");
            }
            other => panic!("expected Error first, got {other:?}"),
        }
        let kinds: Vec<&str> = snapshots(&events)
            .iter()
            .map(|(seq, s)| {
                assert_eq!(*seq, 1);
                match s {
                    Snapshot::Build(_) => "build",
                    Snapshot::BuildList(_) => "list",
                    Snapshot::Stats(_) => "stats",
                    Snapshot::Config(_) => "config",
                    Snapshot::TreeLayout(_) => "tree_layout",
                    Snapshot::Tree(_) => "tree",
                    Snapshot::Calcs(_) => "calcs",
                    Snapshot::Skills(_) => "skills",
                    Snapshot::Items(_) => "items",
                    Snapshot::Notes(_) => "notes",
                    other => panic!("unexpected {other:?}"),
                }
            })
            .collect();
        // a failed command resends the views but not the layout, which cannot have changed
        assert_eq!(
            kinds,
            [
                "build", "list", "stats", "config", "tree", "calcs", "skills", "items", "notes"
            ]
        );
    }

    #[test]
    fn layout_is_sent_before_state() {
        let (mut fake, _log) = Fake::new();
        // NewBuild replaces the tree, so the layout comes along with the state
        let events = run_queued(&mut fake, vec![Command::NewBuild]);
        let kinds: Vec<&str> = snapshots(&events)
            .iter()
            .map(|(_, s)| match s {
                Snapshot::TreeLayout(_) => "tree_layout",
                Snapshot::Tree(_) => "tree",
                _ => "other",
            })
            .filter(|k| *k != "other")
            .collect();
        assert_eq!(kinds, ["tree_layout", "tree"]);
    }

    #[test]
    fn shutdown_stops_the_loop() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            vec![set_config(), Command::Shutdown, set_config()],
        );
        assert_eq!(log.lock().unwrap().set_configs, 1);
        // The command before Shutdown still produced its snapshots, tagged with Shutdown's seq.
        let snaps = snapshots(&events);
        assert_eq!(snaps.len(), 6);
        assert!(matches!(snaps[0], (2, Snapshot::Stats(_))));
        assert!(matches!(snaps[1], (2, Snapshot::Config(_))));
        assert!(matches!(snaps[2], (2, Snapshot::Tree(_))));
        assert!(matches!(snaps[3], (2, Snapshot::Calcs(_))));
        assert!(matches!(snaps[4], (2, Snapshot::Skills(_))));
        assert!(matches!(snaps[5], (2, Snapshot::Items(_))));
    }

    #[test]
    fn commands_sent_before_ready_are_processed() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (fake, log) = Fake::new();
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes2 = wakes.clone();
        let engine = Engine::spawn_with(
            move |_send| {
                release_rx.recv().unwrap();
                Ok(Box::new(fake) as Box<dyn Backend>)
            },
            move || {
                wakes2.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(engine.send(set_config()), 1);
        assert_eq!(engine.send(set_config()), 2);
        // Shutdown is queued too, so all three commands form one batch (last seq 3).
        assert_eq!(engine.send(Command::Shutdown), 3);
        release_tx.send(()).unwrap();

        // Ready, then six seq-0 snapshots, then the batch's snapshots tagged seq 3.
        assert!(matches!(engine.events.recv().unwrap(), Event::Ready));
        for _ in 0..6 {
            match engine.events.recv().unwrap() {
                Event::Snapshot { seq: 0, .. } => {}
                other => panic!("expected initial snapshot, got {other:?}"),
            }
        }
        let mut stats_seq = None;
        let mut config_seq = None;
        while let Ok(event) = engine.events.recv() {
            if let Event::Snapshot { seq, snapshot } = event {
                match snapshot {
                    Snapshot::Stats(_) => stats_seq = Some(seq),
                    Snapshot::Config(_) => config_seq = Some(seq),
                    _ => {}
                }
            }
        }
        assert_eq!(stats_seq, Some(3));
        assert_eq!(config_seq, Some(3));
        assert_eq!(log.lock().unwrap().set_configs, 2);
        // Ready + 10 initial snapshots + Stats + Config + Tree + Calcs + Skills + Items; the channel disconnect means the
        // thread (and so every wake) is finished.
        assert_eq!(wakes.load(Ordering::SeqCst), 17);
    }

    #[test]
    fn startup_failure_is_reported_with_context_chain() {
        let engine = Engine::spawn_with(
            |_send| Err(anyhow::anyhow!("inner reason").context("outer context")),
            || {},
        );
        match engine.events.recv().unwrap() {
            Event::StartupFailed { message } => {
                assert!(message.contains("outer context"), "{message}");
                assert!(message.contains("inner reason"), "{message}");
            }
            other => panic!("expected StartupFailed, got {other:?}"),
        }
    }

    #[test]
    fn closed_engine_is_detected() {
        let (fake, _log) = Fake::new();
        let engine = Engine::spawn_with(move |_send| Ok(Box::new(fake) as Box<dyn Backend>), || {});
        engine.send(Command::Shutdown);
        // The events channel disconnects when the thread's sender is dropped.
        while engine.events.recv().is_ok() {}
        assert!(!engine.is_alive());
        // Sending to a dead engine must neither block nor panic.
        assert_eq!(engine.send(Command::Save), 2);
    }

    #[test]
    fn panicking_engine_thread_is_not_alive() {
        let engine = Engine::spawn_with(
            |_send| -> anyhow::Result<Box<dyn Backend>> { panic!("start exploded") },
            || {},
        );
        while engine.events.recv().is_ok() {}
        assert!(!engine.is_alive());
        engine.send(Command::Save);
    }

    fn step_affecting_power() -> Result<Affected, String> {
        Ok(Affected {
            power: true,
            ..Affected::default()
        })
    }

    fn is_power(event: &Event) -> bool {
        matches!(
            event,
            Event::Snapshot {
                snapshot: Snapshot::NodePower(_),
                ..
            }
        )
    }

    /// Runs the loop on this thread with a live sender; the fake's `on_step` ends it.
    fn run_live(backend: &mut Fake, rx: &Receiver<(u64, Command)>) -> Vec<Event> {
        let events = RefCell::new(Vec::new());
        run_loop(backend, rx, &|e| events.borrow_mut().push(e));
        events.into_inner()
    }

    #[test]
    fn background_runs_only_when_idle() {
        let (mut fake, log) = Fake::new();
        fake.background
            .lock()
            .unwrap()
            .extend((0..3).map(|_| step_affecting_power()));
        let (tx, rx) = mpsc::channel();
        let stop = tx.clone();
        fake.on_step = Some(Box::new(move |n| {
            if n == 3 {
                stop.send((9, Command::Shutdown)).unwrap();
            }
        }));
        tx.send((1, set_config())).unwrap();
        let events = run_live(&mut fake, &rx);
        assert_eq!(log.lock().unwrap().steps, 3);
        let first_power = events.iter().position(is_power).unwrap();
        let last_command_snapshot = events
            .iter()
            .rposition(|e| {
                matches!(
                    e,
                    Event::Snapshot {
                        snapshot: Snapshot::Config(_),
                        ..
                    }
                )
            })
            .unwrap();
        assert!(last_command_snapshot < first_power);
        assert_eq!(events.iter().filter(|e| is_power(e)).count(), 3);
    }

    #[test]
    fn command_mid_calculation_runs_before_next_step() {
        let (mut fake, log) = Fake::new();
        fake.background
            .lock()
            .unwrap()
            .extend((0..3).map(|_| step_affecting_power()));
        let (tx, rx) = mpsc::channel();
        let inject = tx.clone();
        fake.on_step = Some(Box::new(move |n| {
            if n == 1 {
                inject.send((5, set_config())).unwrap();
            }
            if n == 3 {
                inject.send((6, Command::Shutdown)).unwrap();
            }
        }));
        let events = run_live(&mut fake, &rx);
        assert_eq!(log.lock().unwrap().steps, 3);
        // step 1's snapshot, then the command's stats snapshot, only then step 2's
        let kinds: Vec<&str> = snapshots(&events)
            .iter()
            .map(|(_, s)| match s {
                Snapshot::NodePower(_) => "power",
                Snapshot::Stats(_) => "stats",
                _ => "other",
            })
            .filter(|k| *k != "other")
            .collect();
        assert_eq!(&kinds[..3], ["power", "stats", "power"]);
    }

    #[test]
    fn build_change_restarts_background_work() {
        let (mut fake, log) = Fake::new();
        fake.restart_on_build = (0..3).map(|_| step_affecting_power()).collect();
        fake.background
            .lock()
            .unwrap()
            .extend((0..3).map(|_| step_affecting_power()));
        let (tx, rx) = mpsc::channel();
        let inject = tx.clone();
        fake.on_step = Some(Box::new(move |n| {
            if n == 2 {
                inject.send((5, set_config())).unwrap();
            }
            if n == 5 {
                inject.send((6, Command::Shutdown)).unwrap();
            }
        }));
        run_live(&mut fake, &rx);
        // 2 steps, the config change refills the queue, so 3 more run
        assert_eq!(log.lock().unwrap().steps, 5);
        assert!(fake.background.lock().unwrap().is_empty());
    }

    #[test]
    fn only_latest_preview_runs() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            (1..=3).map(|id| Command::PreviewNode { id }).collect(),
        );
        let tooltips: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot {
                    snapshot: Snapshot::NodeTooltip(t),
                    ..
                } => Some(t.id),
                _ => None,
            })
            .collect();
        assert_eq!(tooltips, [3]);
        assert_eq!(log.lock().unwrap().previews, [3]);
    }

    #[test]
    fn only_latest_copy_runs() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            vec![
                Command::CopySocketGroup { group: 1 },
                Command::CopySocketGroup { group: 4 },
            ],
        );
        let copies: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot {
                    snapshot: Snapshot::SocketGroupText(t),
                    ..
                } => Some(t.group),
                _ => None,
            })
            .collect();
        assert_eq!(copies, [4]);
        assert_eq!(log.lock().unwrap().copies, [4]);
    }

    #[test]
    fn only_latest_group_tooltip_runs() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            (1..=3)
                .map(|group| Command::GroupTooltip { group })
                .collect(),
        );
        let tooltips: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot {
                    snapshot: Snapshot::GroupTooltip(t),
                    ..
                } => Some(t.group),
                _ => None,
            })
            .collect();
        assert_eq!(tooltips, [3]);
        assert_eq!(log.lock().unwrap().tooltips, [3]);
    }

    fn item_query(id: u32) -> crate::engine::ItemTooltipQuery {
        crate::engine::ItemTooltipQuery {
            id,
            slot: Some("Ring 1".into()),
        }
    }

    #[test]
    fn only_latest_item_tooltip_runs() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            vec![
                Command::ItemTooltip(item_query(1)),
                Command::ItemTooltip(item_query(2)),
            ],
        );
        let tooltips: Vec<_> = snapshots(&events)
            .into_iter()
            .filter_map(|(_, s)| match s {
                Snapshot::ItemTooltip(t) => Some(t.query.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(tooltips, [item_query(2)]);
        assert_eq!(log.lock().unwrap().item_tooltips, [item_query(2)]);
    }

    #[test]
    fn vanished_item_tooltip_sends_nothing() {
        let (mut fake, log) = Fake::new();
        fake.item_vanished = true;
        let events = run_queued(&mut fake, vec![Command::ItemTooltip(item_query(1))]);
        assert_eq!(log.lock().unwrap().item_tooltips.len(), 1, "it was asked");
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::Snapshot {
                    snapshot: Snapshot::ItemTooltip(_),
                    ..
                } | Event::Error { .. }
            )),
            "{events:?}"
        );
    }

    #[test]
    fn only_latest_breakdown_runs() {
        let (mut fake, log) = Fake::new();
        let cell = |row| CellRef {
            section: "Life".into(),
            sub: 0,
            row,
            cell: 0,
        };
        let events = run_queued(
            &mut fake,
            (1..=3).map(|row| Command::Breakdown(cell(row))).collect(),
        );
        let breakdowns: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot {
                    snapshot: Snapshot::Breakdown(b),
                    ..
                } => Some(b.cell.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(breakdowns, [cell(3)]);
        assert_eq!(log.lock().unwrap().breakdowns, [cell(3)]);
    }

    #[test]
    fn failed_breakdown_is_answered_not_errored() {
        let (mut fake, _log) = Fake::new();
        fake.fail_breakdown = Some("boom".into());
        let cell = CellRef {
            section: "Life".into(),
            sub: 0,
            row: 1,
            cell: 0,
        };
        let events = run_queued(&mut fake, vec![Command::Breakdown(cell.clone())]);
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
        let answers: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot {
                    snapshot: Snapshot::Breakdown(b),
                    ..
                } => Some(b.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].cell, cell);
        assert!(answers[0].sections.is_empty());
        assert!(
            answers[0]
                .error
                .as_deref()
                .is_some_and(|m| m.contains("boom"))
        );
    }

    #[test]
    fn vanished_preview_sends_nothing() {
        let (mut fake, log) = Fake::new();
        fake.tooltip_vanished = true;
        let events = run_queued(&mut fake, vec![Command::PreviewNode { id: 7 }]);
        assert_eq!(log.lock().unwrap().previews, [7]);
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn failed_background_step_stops() {
        let (mut fake, log) = Fake::new();
        {
            let mut queue = fake.background.lock().unwrap();
            queue.push_back(step_affecting_power());
            queue.push_back(Err("calc broke".into()));
            queue.push_back(step_affecting_power());
        }
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let events = RefCell::new(Vec::new());
        // With no work left the loop blocks in recv, so end it from the sender side.
        std::thread::scope(|scope| {
            scope.spawn(move || {
                done_rx.recv().unwrap();
                drop(tx);
            });
            run_loop(&mut fake, &rx, &|e| {
                if matches!(e, Event::BackgroundFailed { .. }) {
                    done_tx.send(()).unwrap();
                }
                events.borrow_mut().push(e);
            });
        });
        let events = events.into_inner();
        assert_eq!(log.lock().unwrap().steps, 2);
        let errors: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::BackgroundFailed { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("calc broke"), "{errors:?}");
    }

    fn gem_query(text: &str) -> crate::engine::GemSearchQuery {
        crate::engine::GemSearchQuery {
            group: 0,
            target: crate::engine::GemTarget::Slot(3),
            text: text.into(),
            filter: crate::engine::GemFilter::All,
        }
    }

    fn step_affecting_gems() -> Result<Affected, String> {
        Ok(Affected {
            gem_search: true,
            ..Affected::default()
        })
    }

    /// Runs the loop until a `BackgroundFailed`, or until every step has run and the
    /// loop is idle, then ends it from the sender side.
    fn run_until_idle(fake: &mut Fake, commands: Vec<Command>, total_steps: usize) -> Vec<Event> {
        let (tx, rx) = mpsc::channel();
        for (i, c) in commands.into_iter().enumerate() {
            tx.send((i as u64 + 1, c)).unwrap();
        }
        let stop = tx.clone();
        let mut stop = Some(stop);
        fake.on_step = Some(Box::new(move |n| {
            if n == total_steps {
                stop.take();
            }
        }));
        drop(tx);
        run_live(fake, &rx)
    }

    #[test]
    fn gem_dps_runs_before_power() {
        let (mut fake, log) = Fake::new();
        fake.background
            .lock()
            .unwrap()
            .extend((0..3).map(|_| step_affecting_power()));
        fake.gem_background
            .lock()
            .unwrap()
            .extend((0..3).map(|_| step_affecting_gems()));
        let events = run_until_idle(&mut fake, vec![Command::SearchGems(gem_query(""))], 6);
        use BackgroundTask::{GemDps, NodePower};
        assert_eq!(
            log.lock().unwrap().step_tasks,
            [GemDps, GemDps, GemDps, NodePower, NodePower, NodePower]
        );
        // each gem step sends the search again: once for the command, three for the steps
        let searches = snapshots(&events)
            .iter()
            .filter(|(_, s)| matches!(s, Snapshot::GemSearch(_)))
            .count();
        assert_eq!(searches, 4);
    }

    #[test]
    fn failed_gem_step_reports_gem_dps() {
        let (mut fake, log) = Fake::new();
        fake.background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_power()));
        {
            let mut gems = fake.gem_background.lock().unwrap();
            gems.push_back(step_affecting_gems());
            gems.push_back(Err("dps broke".into()));
            gems.push_back(step_affecting_gems());
        }
        let events = run_until_idle(&mut fake, vec![Command::SearchGems(gem_query(""))], 4);
        let failures: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::BackgroundFailed { task, message } => Some((*task, message.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].0, BackgroundTask::GemDps);
        assert!(failures[0].1.contains("dps broke"));
        // the failed job stops; the power job still runs to its end
        use BackgroundTask::{GemDps, NodePower};
        assert_eq!(
            log.lock().unwrap().step_tasks,
            [GemDps, GemDps, NodePower, NodePower]
        );
        assert_eq!(events.iter().filter(|e| is_power(e)).count(), 2);
    }

    #[test]
    fn only_latest_search_runs() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            ["a", "ab", "abc"]
                .into_iter()
                .map(|t| Command::SearchGems(gem_query(t)))
                .collect(),
        );
        let searches: Vec<_> = snapshots(&events)
            .into_iter()
            .filter_map(|(seq, s)| match s {
                Snapshot::GemSearch(v) => Some((seq, v.query.text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(searches, [(3, "abc".to_string())]);
        assert_eq!(log.lock().unwrap().gem_searches, ["abc"]);
    }

    #[test]
    fn only_latest_gem_tooltip_runs() {
        let (mut fake, _log) = Fake::new();
        let query = |id: &str| crate::engine::GemTooltipQuery {
            group: 0,
            target: crate::engine::GemTarget::Slot(0),
            gem_id: Some(id.into()),
        };
        let events = run_queued(
            &mut fake,
            ["a", "b"]
                .into_iter()
                .map(|id| Command::GemTooltip(query(id)))
                .collect(),
        );
        let tooltips: Vec<_> = snapshots(&events)
            .into_iter()
            .filter_map(|(_, s)| match s {
                Snapshot::GemTooltip(t) => Some(t.query.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(tooltips, [query("b")]);
    }

    #[test]
    fn recalc_restarts_gem_dps() {
        let (mut fake, log) = Fake::new();
        run_queued(&mut fake, vec![Command::SearchGems(gem_query("a"))]);
        assert_eq!(log.lock().unwrap().gem_restarts, 0, "a search alone");
        run_queued(&mut fake, vec![set_config()]);
        assert_eq!(log.lock().unwrap().gem_restarts, 1);
    }

    #[test]
    fn background_snapshots_keep_last_seq() {
        let (mut fake, _log) = Fake::new();
        fake.background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_power()));
        let (tx, rx) = mpsc::channel();
        let stop = tx.clone();
        fake.on_step = Some(Box::new(move |n| {
            if n == 2 {
                stop.send((9, Command::Shutdown)).unwrap();
            }
        }));
        tx.send((4, set_config())).unwrap();
        let events = run_live(&mut fake, &rx);
        let power: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot {
                    seq,
                    snapshot: Snapshot::NodePower(_),
                } => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(power, [4, 4]);
    }
}
