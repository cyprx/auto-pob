//! The engine thread: a command loop over a [`Backend`], and the [`Engine`] handle the UI holds.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use crate::pob_files::DownloadProgress;

use super::commands::{Command, ControlTooltipQuery, GemTooltipQuery, ItemTooltipQuery};
use super::views::{
    AccountView, BreakdownView, BuildBrowseView, BuildListView, BuildView, CalcsView, CellRef,
    CompareView, ConfigView, ControlTooltipView, DisplayItemView, ExternalUrl, GemSearchView,
    GemTooltipView, GroupTooltipView, ItemDbView, ItemText, ItemTooltipView, ItemsView, NodePower,
    NodeTooltip, NotesView, PopupView, PowerReport, RequestsView, SettingsView, ShareView,
    SkillsView, SocketGroupText, StatsView, TreeLayout, TreeState,
};
use pob_runtime::net::Waker;

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
    /// The display item; `None` while nothing is displayed
    DisplayItem(Option<Arc<DisplayItemView>>),
    /// The popup session's window; `None` once the session is over
    Popup(Option<Arc<PopupView>>),
    ControlTooltip(Arc<ControlTooltipView>),
    SocketGroupText(Arc<SocketGroupText>),
    GroupTooltip(Arc<GroupTooltipView>),
    GemSearch(Arc<GemSearchView>),
    /// The open item database; `None` once it closed or the build changed (R21)
    ItemDb(Option<Arc<ItemDbView>>),
    GemTooltip(Arc<GemTooltipView>),
    ItemTooltip(Arc<ItemTooltipView>),
    ItemText(Arc<ItemText>),
    Notes(Arc<NotesView>),
    /// The pending and newly finished network requests; sent with or without a build (R10)
    Requests(Arc<RequestsView>),
    /// PoB's settings and the app's overrides; sent with or without a build (R12)
    Settings(Arc<SettingsView>),
    /// The build sharing and import box of the Import/Export tab (R16, R17)
    Share(Arc<ShareView>),
    /// The build's share code, answering `ExportShareCode` (R17)
    ShareCode(Arc<String>),
    /// The Import/Export tab's account import (R18, R19)
    Account(Arc<AccountView>),
    /// The Compare tab; `None` with no comparison loaded (R4, R5)
    Compare(Option<Arc<CompareView>>),
    /// The builds of one folder, answering `BrowseBuilds`; sent with or without a build
    BuildBrowse(Arc<BuildBrowseView>),
    /// A trade link a popup press made (R20), tagged with that press's own seq: the UI opens
    /// and copies it for its own click only
    ExternalUrl(Arc<ExternalUrl>),
}

/// Long-running work the engine does between commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTask {
    /// Finished subscripts and their calls; first, as its steps are short and their latency
    /// shows. Offered only while one is ready, so a pending request never spins the loop (R9)
    Network,
    /// The gem picker's DPS sort; it goes first among the calculations whenever it has work (R12)
    GemDps,
    /// The Anoint window's stat sort; it goes after the gem picker's, before node power (R20)
    AnointSort,
    /// The timeless jewel search (R17); after the Anoint sort, before the item databases (R32)
    Timeless,
    /// The item databases' load and stat sort; after the Anoint sort, before node power (R20)
    ItemDb,
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
    /// The Items tab's display item changed
    pub display_item: bool,
    /// The popup session changed or ended (R21)
    pub popup: bool,
    /// The build's notes changed
    pub notes: bool,
    /// Node power changed (a background step ran)
    pub power: bool,
    /// The gem search changed: it opened, its text changed or a DPS step ran
    pub gem_search: bool,
    /// The open item database changed, or a recalculation restarted its list (R21)
    pub item_db: bool,
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
    /// This control's tooltip was requested; the latest request wins (R6)
    pub control_tooltip: Option<ControlTooltipQuery>,
    /// The text PoB copies for this item was requested; the latest request wins
    pub item_text: Option<u32>,
    /// The tree link was requested
    pub link: bool,
    /// A PoEURL shrink finished: `short_tree_link_view` has the short link (R20)
    pub short_link: bool,
    /// The network requests changed: one began, finished or was cancelled (R10)
    pub requests: bool,
    /// The settings or the overrides changed (R12); list-safe
    pub settings: bool,
    /// The import box or the sharing state changed (R16, R17); build-only
    pub share: bool,
    /// The share code was requested (R17); build-only
    pub share_code: bool,
    /// The account import changed (R18, R19); build-only
    pub account: bool,
    /// The Compare tab changed: its comparisons, or this build's stats it is diffed against
    /// (R4); build-only
    pub compare: bool,
    /// The builds of this folder were asked for (`BrowseBuilds`); the latest request wins.
    /// List-safe
    pub browse: Option<String>,
    /// The trade link this command's press made (R20). The loop sends it at once, with the
    /// command's own seq, so it is never a view of the batch
    pub external_url: Option<ExternalUrl>,
}

impl Affected {
    pub const ALL_VIEWS: Affected = Affected {
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
        display_item: true,
        popup: true,
        notes: true,
        power: false,
        gem_search: false,
        item_db: false,
        report: false,
        preview: None,
        breakdown: None,
        group_text: None,
        group_tooltip: None,
        gem_tooltip: None,
        item_tooltip: None,
        control_tooltip: None,
        item_text: None,
        link: false,
        short_link: false,
        requests: true,
        settings: true,
        share: true,
        share_code: false,
        account: true,
        compare: true,
        browse: None,
        external_url: None,
    };

    /// Only views that need no build are asked for: the build list, the requests, the settings
    /// and the popup (a main-owned session needs no build, R19).
    fn list_safe_only(&self) -> bool {
        *self == self.list_safe()
    }

    /// The views of these flags that need no build.
    fn list_safe(&self) -> Affected {
        Affected {
            list: self.list,
            requests: self.requests,
            settings: self.settings,
            popup: self.popup,
            browse: self.browse.clone(),
            ..Affected::default()
        }
    }

    pub(crate) fn merge(&mut self, other: Affected) {
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
        self.display_item |= other.display_item;
        self.popup |= other.popup;
        self.notes |= other.notes;
        self.power |= other.power;
        self.gem_search |= other.gem_search;
        self.item_db |= other.item_db;
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
        if other.control_tooltip.is_some() {
            self.control_tooltip = other.control_tooltip;
        }
        self.item_text = other.item_text.or(self.item_text);
        self.link |= other.link;
        self.short_link |= other.short_link;
        self.requests |= other.requests;
        self.settings |= other.settings;
        self.share |= other.share;
        self.share_code |= other.share_code;
        self.account |= other.account;
        self.compare |= other.compare;
        if other.browse.is_some() {
            self.browse = other.browse;
        }
        if other.external_url.is_some() {
            self.external_url = other.external_url;
        }
    }
}

/// What `Backend::item_db_view` answers (R21).
// the view is sent at once, wrapped in an `Arc`; boxing it here would only add a copy
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum DbSnapshot {
    View(ItemDbView),
    /// The database closed or the build changed: `ItemDb(None)` is sent, once
    Ended,
    /// No database is open: nothing is sent
    Quiet,
}

pub trait Backend {
    fn apply(&mut self, command: Command, send: &dyn Fn(Event)) -> anyhow::Result<Affected>;
    fn recalculate(&mut self) -> anyhow::Result<()>;
    fn build_view(&mut self) -> anyhow::Result<BuildView>;
    fn build_list_view(&mut self) -> anyhow::Result<BuildListView>;
    /// Whether a build is open. PoB can start on its build list with none initialised, and
    /// every view but the list reads the build.
    fn build_loaded(&mut self) -> anyhow::Result<bool>;
    fn stats_view(&mut self) -> anyhow::Result<StatsView>;
    fn config_view(&mut self) -> anyhow::Result<ConfigView>;
    fn tree_layout_view(&mut self) -> anyhow::Result<TreeLayout>;
    fn tree_state_view(&mut self) -> anyhow::Result<TreeState>;
    fn calcs_view(&mut self) -> anyhow::Result<CalcsView>;
    fn skills_view(&mut self) -> anyhow::Result<SkillsView>;
    fn items_view(&mut self) -> anyhow::Result<ItemsView>;
    /// The item on the display panel, or none
    fn display_item_view(&mut self) -> anyhow::Result<Option<DisplayItemView>>;
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
    /// The open item database as it stands (R21)
    fn item_db_view(&mut self) -> anyhow::Result<DbSnapshot>;
    /// A gem tooltip. A failure is answered with empty lines (R15).
    fn gem_tooltip_view(&mut self, query: &GemTooltipQuery) -> anyhow::Result<GemTooltipView>;
    /// `None`: the item or slot is gone
    fn item_tooltip_view(
        &mut self,
        query: &ItemTooltipQuery,
    ) -> anyhow::Result<Option<ItemTooltipView>>;
    fn item_text(&mut self, id: u32) -> anyhow::Result<ItemText>;
    /// The popup session's window; `None` when no session is open
    fn popup_view(&mut self) -> anyhow::Result<Option<PopupView>>;
    /// `None`: the control or entry is gone
    fn control_tooltip_view(
        &mut self,
        query: &ControlTooltipQuery,
    ) -> anyhow::Result<Option<ControlTooltipView>>;
    fn node_power_view(&mut self) -> anyhow::Result<NodePower>;
    fn power_report_view(&mut self) -> anyhow::Result<PowerReport>;
    /// `Ok(None)` when the node no longer exists: nothing is sent.
    fn node_tooltip_view(&mut self, id: u32) -> anyhow::Result<Option<NodeTooltip>>;
    /// Empty sections when the cell is gone or has no breakdown.
    fn breakdown_view(&mut self, cell: &CellRef) -> anyhow::Result<BreakdownView>;
    fn tree_link_view(&mut self) -> anyhow::Result<String>;
    /// The short link a finished PoEURL shrink produced, or `None` once taken (R20).
    fn short_tree_link_view(&mut self) -> anyhow::Result<Option<String>>;
    /// The Import/Export tab's sharing state and import box (R16, R17)
    fn share_view(&mut self) -> anyhow::Result<ShareView>;
    /// The build's share code: PoB's Generate button, no network (R17)
    fn share_code(&mut self) -> anyhow::Result<String>;
    /// The Import/Export tab's account import (R18, R19)
    fn account_view(&mut self) -> anyhow::Result<AccountView>;
    /// The Compare tab; `None` with no comparison loaded (R4)
    fn compare_view(&mut self) -> anyhow::Result<Option<CompareView>>;
    /// The builds in `folder` below the Builds folder; reads the disk only, changes nothing
    fn browse_view(&mut self, folder: &str) -> anyhow::Result<BuildBrowseView>;
    /// Hands back the memory the last busy stretch left behind. The loop calls it once it has
    /// been idle for a while after work, never between a background job's steps.
    fn release_memory(&mut self);
    /// The pending network requests and the ones finished since the last call (R10)
    fn requests_view(&mut self) -> anyhow::Result<RequestsView>;
    /// PoB's settings and the overrides over them; needs no build (R12)
    fn settings_view(&mut self) -> anyhow::Result<SettingsView>;
    /// Gives the backend a way to wake the loop from other threads (R9). Called once, after
    /// `start`.
    fn set_waker(&mut self, _wake: Waker) {}
}

/// Sends the snapshots `flags` asks for, as the loop does after a command or a background
/// step (the gate also reads them, to check what reaches the UI).
pub fn send_snapshots(backend: &mut dyn Backend, seq: u64, flags: Affected, send: &dyn Fn(Event)) {
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
    // With no build open only the list, the requests, the settings and the popup can be
    // answered; the rest would raise
    // on a missing build, so they are dropped here, for every caller at once.
    let flags = if flags.list_safe_only() {
        flags
    } else {
        match backend.build_loaded() {
            Ok(true) => flags,
            Ok(false) => flags.list_safe(),
            Err(e) => {
                send(Event::Error {
                    seq,
                    message: format!("{e:#}"),
                });
                flags.list_safe()
            }
        }
    };
    // The requests go first: one that finished with a new build tells the UI to start over
    // (`new_build`), and the new build's views must come after that, not before.
    if flags.requests {
        emit(seq, send, backend.requests_view(), Snapshot::Requests);
    }
    if flags.build {
        emit(seq, send, backend.build_view(), Snapshot::Build);
    }
    if flags.list {
        emit(seq, send, backend.build_list_view(), Snapshot::BuildList);
    }
    if flags.settings {
        emit(seq, send, backend.settings_view(), Snapshot::Settings);
    }
    if let Some(folder) = &flags.browse {
        emit(
            seq,
            send,
            backend.browse_view(folder),
            Snapshot::BuildBrowse,
        );
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
    // unlike the gem search, "nothing displayed" is an answer too
    if flags.display_item {
        match backend.display_item_view() {
            Ok(view) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::DisplayItem(view.map(Arc::new)),
            }),
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
    }
    // "no session" is an answer too: it closes the window
    if flags.popup {
        match backend.popup_view() {
            Ok(view) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::Popup(view.map(Arc::new)),
            }),
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
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
    if flags.item_db {
        match backend.item_db_view() {
            Ok(DbSnapshot::View(view)) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::ItemDb(Some(Arc::new(view))),
            }),
            Ok(DbSnapshot::Ended) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::ItemDb(None),
            }),
            Ok(DbSnapshot::Quiet) => {}
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
    if let Some(query) = &flags.control_tooltip {
        match backend.control_tooltip_view(query) {
            Ok(Some(view)) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::ControlTooltip(Arc::new(view)),
            }),
            // a vanished control sends nothing
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
    if flags.short_link {
        match backend.short_tree_link_view() {
            Ok(Some(link)) => emit(seq, send, Ok(link), Snapshot::TreeLink),
            // already taken: nothing to send
            Ok(None) => {}
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
    }
    if flags.share {
        emit(seq, send, backend.share_view(), Snapshot::Share);
    }
    if flags.share_code {
        emit(seq, send, backend.share_code(), Snapshot::ShareCode);
    }
    if flags.account {
        emit(seq, send, backend.account_view(), Snapshot::Account);
    }
    // the loop sends a command's link itself; a caller that hands one over gets it sent here
    if let Some(url) = &flags.external_url {
        send(Event::Snapshot {
            seq,
            snapshot: Snapshot::ExternalUrl(Arc::new(url.clone())),
        });
    }
    // "no comparison" is an answer too: a build change drops them
    if flags.compare {
        match backend.compare_view() {
            Ok(view) => send(Event::Snapshot {
                seq,
                snapshot: Snapshot::Compare(view.map(Arc::new)),
            }),
            Err(e) => send(Event::Error {
                seq,
                message: format!("{e:#}"),
            }),
        }
    }
}

/// Recalculates once when `affected` asks for it, and adds the views that depend on the result.
fn recalculate_if_asked(
    backend: &mut dyn Backend,
    affected: &mut Affected,
    seq: u64,
    send: &dyn Fn(Event),
) {
    if !affected.recalc {
        return;
    }
    if let Err(e) = backend.recalculate() {
        send(Event::Error {
            seq,
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
    affected.display_item = true;
    // Draw restarts an open list on a new output revision (R14)
    affected.item_db = true;
    // the Summary diffs the comparison against this build's stats (R4)
    affected.compare = true;
}

/// Processes commands until `Shutdown` or until every sender is gone.
///
/// Each wake-up drains the queue into a batch, applies it, recalculates at most once, and
/// sends one snapshot per affected view, tagged with the batch's last `seq`.
///
/// While the backend has background work, the loop runs one step whenever no command is
/// waiting, so commands always go first. Background snapshots carry the last command's `seq`
/// (0 before any), so the UI's idea of "behind" never moves backwards.
///
/// Once work is done and no command has come for [`RELEASE_DELAY`], the backend releases
/// memory, once; then the loop sleeps until the next command.
pub fn run_loop(
    backend: &mut dyn Backend,
    commands: &Receiver<(u64, Command)>,
    send: &dyn Fn(Event),
) {
    run_loop_with_delay(backend, commands, send, RELEASE_DELAY);
}

/// How long the loop waits, idle after work, before it releases memory. A release takes a
/// full Lua collection (up to about half a second after node power), so it waits for a pause
/// rather than landing on the command that follows a job or a burst of edits.
pub const RELEASE_DELAY: Duration = Duration::from_secs(2);

fn run_loop_with_delay(
    backend: &mut dyn Backend,
    commands: &Receiver<(u64, Command)>,
    send: &dyn Fn(Event),
    release_delay: Duration,
) {
    let mut last_seq = 0;
    // work has run since the last release; starting the backend counts
    let mut worked = true;
    loop {
        let first = if let Some(task) = backend.next_background() {
            match commands.try_recv() {
                Ok(first) => first,
                Err(TryRecvError::Empty) => {
                    worked = true;
                    match backend.background_step(task, send) {
                        Ok(mut affected) => {
                            // a network step that edited or replaced the build (R14)
                            recalculate_if_asked(backend, &mut affected, last_seq, send);
                            send_snapshots(backend, last_seq, affected, send);
                        }
                        Err(e) => send(Event::BackgroundFailed {
                            task,
                            message: format!("{e:#}"),
                        }),
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => return,
            }
        } else if worked {
            match commands.recv_timeout(release_delay) {
                Ok(first) => first,
                Err(RecvTimeoutError::Timeout) => {
                    worked = false;
                    backend.release_memory();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match commands.recv() {
                Ok(first) => first,
                Err(_) => return,
            }
        };
        worked = true;
        let mut batch = vec![first];
        while let Ok(next) = commands.try_recv() {
            batch.push(next);
        }
        // a wake only gets the loop round to `next_background`; it is never applied and has no
        // seq of its own (R9)
        batch.retain(|(_, command)| !matches!(command, Command::Wake));
        if batch.is_empty() {
            continue;
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
                Ok(mut a) => {
                    // the link answers this press, not the batch (R20)
                    if let Some(url) = a.external_url.take() {
                        send(Event::Snapshot {
                            seq,
                            snapshot: Snapshot::ExternalUrl(Arc::new(url)),
                        });
                    }
                    affected.merge(a)
                }
                Err(e) => {
                    send(Event::Error {
                        seq,
                        message: format!("{e:#}"),
                    });
                    affected.merge(Affected::ALL_VIEWS);
                }
            }
        }
        recalculate_if_asked(backend, &mut affected, last_seq, send);
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

/// Tells whether an engine thread still runs, also after its [`Engine`] is gone.
#[derive(Clone)]
pub struct Liveness(Arc<AtomicBool>);

impl Liveness {
    pub fn is_alive(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// UI-side handle to the engine thread. Use from the UI thread only. Dropping it shuts the
/// thread down.
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
        let (command_tx, command_rx) = mpsc::channel::<(u64, Command)>();
        let (event_tx, event_rx) = mpsc::channel();
        let wake_tx = command_tx.clone();
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
            // subscript threads wake the loop through the command channel (R9)
            backend.set_waker(Arc::new(move || {
                let _ = wake_tx.send((0, Command::Wake));
            }));
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

    /// A handle that answers `is_alive` after this `Engine` is dropped.
    pub fn liveness(&self) -> Liveness {
        Liveness(self.alive.clone())
    }
}

impl Drop for Engine {
    /// The waker keeps a sender of the command channel, so the loop would never see it
    /// disconnect: the thread is told to stop instead (R9).
    fn drop(&mut self) {
        self.send(Command::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};

    use super::*;
    use crate::engine::commands::{Command, ConfigValue};
    use crate::engine::fake::{Fake, Log};

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
    fn external_url_carries_the_press_seq() {
        // the link answers its own press, even with a later command in the same batch (R20)
        let (mut fake, _log) = Fake::new();
        let events = run_queued(
            &mut fake,
            vec![
                Command::PressPopupButton {
                    name: "search".into(),
                },
                set_config(),
            ],
        );
        let links: Vec<_> = snapshots(&events)
            .into_iter()
            .filter_map(|(seq, s)| match s {
                Snapshot::ExternalUrl(link) => Some((seq, link.url.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(links, [(1, crate::engine::fake::FAKE_TRADE_URL.to_owned())]);
        // the batch's views carry its last seq
        assert!(
            snapshots(&events)
                .iter()
                .any(|(seq, s)| *seq == 2 && matches!(s, Snapshot::Calcs(_)))
        );
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
    fn recalc_sends_display_item() {
        let (mut fake, _log) = Fake::new();
        let events = run_queued(&mut fake, vec![set_config()]);
        let shown = snapshots(&events)
            .into_iter()
            .filter(|(_, s)| matches!(s, Snapshot::DisplayItem(_)))
            .count();
        assert_eq!(shown, 1);
    }

    #[test]
    fn display_item_none_is_sent() {
        // unlike the gem search, "nothing is displayed" is an answer (R10)
        let (mut fake, _log) = Fake::new();
        let events = run_queued(&mut fake, vec![Command::CancelDisplayItem { if_id: None }]);
        let shown: Vec<_> = snapshots(&events)
            .into_iter()
            .filter_map(|(_, s)| match s {
                Snapshot::DisplayItem(view) => Some(view.is_none()),
                _ => None,
            })
            .collect();
        assert_eq!(shown, [true]);
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
                    Snapshot::Settings(_) => "settings",
                    Snapshot::Stats(_) => "stats",
                    Snapshot::Config(_) => "config",
                    Snapshot::TreeLayout(_) => "tree_layout",
                    Snapshot::Tree(_) => "tree",
                    Snapshot::Calcs(_) => "calcs",
                    Snapshot::Skills(_) => "skills",
                    Snapshot::Items(_) => "items",
                    Snapshot::DisplayItem(_) => "display_item",
                    Snapshot::Popup(_) => "popup",
                    Snapshot::Notes(_) => "notes",
                    Snapshot::Requests(_) => "requests",
                    Snapshot::Share(_) => "share",
                    Snapshot::Account(_) => "account",
                    Snapshot::Compare(_) => "compare",
                    other => panic!("unexpected {other:?}"),
                }
            })
            .collect();
        // a failed command resends the views but not the layout, which cannot have changed
        assert_eq!(
            kinds,
            [
                "requests",
                "build",
                "list",
                "settings",
                "stats",
                "config",
                "tree",
                "calcs",
                "skills",
                "items",
                "display_item",
                "popup",
                "notes",
                "share",
                "account",
                "compare"
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
        assert_eq!(snaps.len(), 8);
        assert!(matches!(snaps[0], (2, Snapshot::Stats(_))));
        assert!(matches!(snaps[1], (2, Snapshot::Config(_))));
        assert!(matches!(snaps[2], (2, Snapshot::Tree(_))));
        assert!(matches!(snaps[3], (2, Snapshot::Calcs(_))));
        assert!(matches!(snaps[4], (2, Snapshot::Skills(_))));
        assert!(matches!(snaps[5], (2, Snapshot::Items(_))));
        assert!(matches!(snaps[6], (2, Snapshot::DisplayItem(_))));
        assert!(matches!(snaps[7], (2, Snapshot::Compare(None))));
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
        // Ready + 17 initial snapshots + Stats + Config + Tree + Calcs + Skills + Items +
        // DisplayItem + Compare; the channel disconnect means the thread (and so every wake) is
        // finished.
        assert_eq!(wakes.load(Ordering::SeqCst), 26);
    }

    #[test]
    fn startup_without_a_build_sends_only_the_list() {
        let (mut fake, _log) = Fake::new();
        fake.build_loaded = false;
        let engine = Engine::spawn_with(move |_send| Ok(Box::new(fake) as Box<dyn Backend>), || {});
        engine.send(Command::Shutdown);
        let mut kinds = Vec::new();
        while let Ok(event) = engine.events.recv() {
            match event {
                Event::Snapshot { snapshot, .. } => kinds.push(match snapshot {
                    Snapshot::BuildList(_) => "list",
                    Snapshot::Requests(_) => "requests",
                    Snapshot::Settings(_) => "settings",
                    Snapshot::Popup(_) => "popup",
                    _ => "other",
                }),
                Event::Error { message, .. } => panic!("unexpected error: {message}"),
                _ => {}
            }
        }
        // the requests need no build either (R10), nor do the settings (R12) and the popup
        // (a main-owned session, R19)
        assert_eq!(
            kinds,
            ["requests", "list", "settings", "popup"],
            "only the list-safe views go out"
        );
    }

    #[test]
    fn item_queries_without_a_build_send_nothing() {
        let queries = || {
            vec![
                Command::ItemTooltip(ItemTooltipQuery {
                    id: 1,
                    slot: None,
                    theirs: false,
                }),
                Command::CopyItem { id: 1 },
            ]
        };
        let kinds = |events: &[Event]| {
            snapshots(events)
                .into_iter()
                .filter(|(_, s)| matches!(s, Snapshot::ItemTooltip(_) | Snapshot::ItemText(_)))
                .count()
        };
        // with a build both are answered
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, queries());
        assert_eq!(kinds(&events), 2);
        assert_eq!(log.lock().unwrap().item_tooltips.len(), 1);
        // with none, neither reaches the backend
        let (mut fake, log) = Fake::new();
        fake.build_loaded = false;
        let events = run_queued(&mut fake, queries());
        assert_eq!(kinds(&events), 0);
        assert!(log.lock().unwrap().item_tooltips.is_empty());
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
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
            theirs: false,
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

    fn popup_query(control: &str) -> crate::engine::ControlTooltipQuery {
        crate::engine::ControlTooltipQuery::Popup {
            control: control.into(),
            index: Some(0),
        }
    }

    fn popups(events: &[Event]) -> Vec<bool> {
        snapshots(events)
            .into_iter()
            .filter_map(|(_, s)| match s {
                Snapshot::Popup(view) => Some(view.is_some()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn popup_none_is_sent_when_a_session_ends() {
        let (mut fake, _log) = Fake::new();
        let open = Command::OpenPopup(crate::engine::PopupKind::Craft);
        let events = run_queued(&mut fake, vec![open.clone()]);
        assert_eq!(popups(&events), [true]);
        // a command that isn't the session's ends it, and the window is told
        let (mut fake, _log) = Fake::new();
        let events = run_queued(&mut fake, vec![open, Command::SortItems]);
        assert_eq!(popups(&events), [false], "{events:?}");
    }

    #[test]
    fn a_failing_command_leaves_the_popup_session_open() {
        let (mut fake, _log) = Fake::new();
        fake.fail_set_config = Some("boom".into());
        let events = run_queued(
            &mut fake,
            vec![
                Command::OpenPopup(crate::engine::PopupKind::Craft),
                set_config(),
            ],
        );
        assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
        assert_eq!(popups(&events).last(), Some(&true), "{events:?}");
    }

    #[test]
    fn only_latest_control_tooltip_runs() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            vec![
                Command::ControlTooltip(popup_query("one")),
                Command::ControlTooltip(popup_query("two")),
            ],
        );
        let tooltips: Vec<_> = snapshots(&events)
            .into_iter()
            .filter_map(|(_, s)| match s {
                Snapshot::ControlTooltip(t) => Some(t.query.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(tooltips, [popup_query("two")]);
        assert_eq!(log.lock().unwrap().control_tooltips, [popup_query("two")]);
    }

    #[test]
    fn vanished_control_tooltip_sends_nothing() {
        let (mut fake, log) = Fake::new();
        fake.control_vanished = true;
        let events = run_queued(&mut fake, vec![Command::ControlTooltip(popup_query("one"))]);
        assert_eq!(
            log.lock().unwrap().control_tooltips.len(),
            1,
            "it was asked"
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::Snapshot {
                    snapshot: Snapshot::ControlTooltip(_),
                    ..
                } | Event::Error { .. }
            )),
            "{events:?}"
        );
    }

    #[test]
    fn crafting_views_masked_without_a_build() {
        let queries = || {
            vec![
                Command::OpenPopup(crate::engine::PopupKind::Craft),
                Command::ControlTooltip(popup_query("one")),
            ]
        };
        let kinds = |events: &[Event]| {
            snapshots(events)
                .into_iter()
                .filter(|(_, s)| {
                    matches!(s, Snapshot::Popup(Some(_)) | Snapshot::ControlTooltip(_))
                })
                .count()
        };
        // with a build both are answered
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, queries());
        assert_eq!(kinds(&events), 2);
        assert_eq!(log.lock().unwrap().control_tooltips.len(), 1);
        // with none, neither reaches the backend
        let (mut fake, log) = Fake::new();
        fake.build_loaded = false;
        let events = run_queued(&mut fake, queries());
        assert_eq!(kinds(&events), 0);
        assert!(log.lock().unwrap().control_tooltips.is_empty());
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
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

    fn db_query() -> crate::engine::ItemDbQuery {
        crate::engine::ItemDbQuery {
            db: crate::engine::ItemDb::Uniques,
            slot: 0,
            kind: 0,
            league: 0,
            requirement: 0,
            obtainable: 0,
            sort: 0,
            search: String::new(),
            search_mode: 0,
        }
    }

    fn step_affecting_item_db() -> Result<Affected, String> {
        Ok(Affected {
            item_db: true,
            ..Affected::default()
        })
    }

    #[test]
    fn jobs_run_in_order_gem_anoint_item_db_power() {
        // R20, R32: the gem picker's sort, the Anoint sort, the timeless search, the item
        // database, then node power
        let (mut fake, log) = Fake::new();
        fake.timeless_background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_gems()));
        fake.background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_power()));
        fake.item_db_background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_item_db()));
        fake.anoint_background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_gems()));
        fake.gem_background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_gems()));
        run_until_idle(
            &mut fake,
            vec![
                Command::SearchGems(gem_query("")),
                Command::SetItemDb(db_query()),
            ],
            10,
        );
        use BackgroundTask::{AnointSort, GemDps, ItemDb, NodePower, Timeless};
        assert_eq!(
            log.lock().unwrap().step_tasks,
            [
                GemDps, GemDps, AnointSort, AnointSort, Timeless, Timeless, ItemDb, ItemDb,
                NodePower, NodePower
            ]
        );
    }

    fn item_db_snapshots(events: &[Event]) -> Vec<Option<Arc<crate::engine::ItemDbView>>> {
        snapshots(events)
            .into_iter()
            .filter_map(|(_, s)| match s {
                Snapshot::ItemDb(view) => Some(view.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn item_db_closed_sends_none_once() {
        let (mut fake, _log) = Fake::new();
        let opened =
            item_db_snapshots(&run_queued(&mut fake, vec![Command::SetItemDb(db_query())]));
        assert!(matches!(opened.as_slice(), [Some(_)]));
        let closed = item_db_snapshots(&run_queued(&mut fake, vec![Command::CloseItemDb]));
        assert!(matches!(closed.as_slice(), [None]), "{closed:?}");
        // later batches (a recalculation asks for the view too) say nothing more
        let later = item_db_snapshots(&run_queued(&mut fake, vec![set_config()]));
        assert!(later.is_empty(), "{later:?}");
    }

    #[test]
    fn leaving_the_build_closes_the_item_db() {
        let (mut fake, _log) = Fake::new();
        run_queued(&mut fake, vec![Command::SetItemDb(db_query())]);
        let events = run_queued(&mut fake, vec![Command::NewBuild]);
        assert!(matches!(item_db_snapshots(&events).as_slice(), [None]));
    }

    #[test]
    fn recalc_resends_open_item_db() {
        let (mut fake, log) = Fake::new();
        run_queued(&mut fake, vec![Command::SetItemDb(db_query())]);
        assert_eq!(log.lock().unwrap().item_dbs.len(), 1, "the open alone");
        let events = run_queued(&mut fake, vec![set_config()]);
        assert!(matches!(item_db_snapshots(&events).as_slice(), [Some(_)]));
        assert_eq!(log.lock().unwrap().item_dbs.len(), 2);
    }

    #[test]
    fn item_db_masked_without_a_build() {
        // with a build the open is answered; with none, nothing reaches the backend
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, vec![Command::SetItemDb(db_query())]);
        assert_eq!(item_db_snapshots(&events).len(), 1);
        let (mut fake, log2) = Fake::new();
        fake.build_loaded = false;
        let events = run_queued(&mut fake, vec![Command::SetItemDb(db_query())]);
        assert!(item_db_snapshots(&events).is_empty());
        assert!(log2.lock().unwrap().item_dbs.is_empty());
        assert_eq!(log.lock().unwrap().item_dbs.len(), 1);
    }

    #[test]
    fn item_db_rows_only_when_finished() {
        let (mut fake, _log) = Fake::new();
        fake.item_db_background.lock().unwrap().extend([
            Ok(Affected::default()),
            Ok(Affected::default()),
            step_affecting_item_db(),
        ]);
        let events = run_until_idle(&mut fake, vec![Command::SetItemDb(db_query())], 3);
        let progress = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Progress {
                        task: BackgroundTask::ItemDb,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(progress, 3);
        // the open, and the step that finished; the two before it sent none
        assert_eq!(item_db_snapshots(&events).len(), 2);
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
    fn wake_is_not_applied_and_keeps_seq() {
        let (mut fake, log) = Fake::new();
        let (tx, rx) = mpsc::channel();
        tx.send((5, set_config())).unwrap();
        tx.send((0, Command::Wake)).unwrap();
        drop(tx);
        let events = RefCell::new(Vec::new());
        run_loop(&mut fake, &rx, &|e| events.borrow_mut().push(e));
        let events = events.into_inner();
        assert_eq!(log.lock().unwrap().applies, 1, "Wake was applied");
        let seqs: Vec<u64> = snapshots(&events).iter().map(|(seq, _)| *seq).collect();
        assert!(!seqs.is_empty());
        assert!(seqs.iter().all(|&seq| seq == 5), "{seqs:?}");
    }

    #[test]
    fn pending_request_does_not_spin_the_loop() {
        // Review Focus 5: a request waits (pending, nothing ready), so the loop sleeps
        let (mut fake, log) = Fake::new();
        fake.pending = vec![(1, "build")];
        let ready = fake.net_ready.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            run_loop(&mut fake, &rx, &|_| {});
        });
        std::thread::sleep(Duration::from_millis(500));
        {
            let log = log.lock().unwrap();
            assert_eq!(log.steps, 0, "the loop stepped while nothing was ready");
            assert_eq!(log.applies, 0);
        }
        // a subscript finished: its thread wakes the loop
        ready.store(true, Ordering::SeqCst);
        tx.send((0, Command::Wake)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while log.lock().unwrap().steps == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(200));
        {
            let log = log.lock().unwrap();
            assert_eq!(log.step_tasks, [BackgroundTask::Network]);
            assert_eq!(log.applies, 0, "Wake was applied");
        }
        tx.send((1, Command::Shutdown)).unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn network_runs_before_gem_dps() {
        let (mut fake, log) = Fake::new();
        fake.gem_background
            .lock()
            .unwrap()
            .extend((0..2).map(|_| step_affecting_gems()));
        fake.net_ready.store(true, Ordering::SeqCst);
        let events = run_until_idle(&mut fake, vec![Command::SearchGems(gem_query(""))], 3);
        use BackgroundTask::{GemDps, Network};
        assert_eq!(log.lock().unwrap().step_tasks, [Network, GemDps, GemDps]);
        let requests = snapshots(&events)
            .iter()
            .filter(|(_, s)| matches!(s, Snapshot::Requests(_)))
            .count();
        assert_eq!(requests, 1, "the Network step sends the requests");
    }

    #[test]
    fn network_step_that_edits_the_build_recalculates() {
        // a request's callback replaced the build (R14); the loop calculates it, as it does
        // for a command that asks, and the views that depend on the result follow
        let (mut fake, log) = Fake::new();
        fake.net_ready.store(true, Ordering::SeqCst);
        fake.network_recalc = true;
        let events = run_until_idle(&mut fake, vec![], 1);
        assert_eq!(log.lock().unwrap().recalculations, 1);
        assert_eq!(count_of(&events, |s| matches!(s, Snapshot::Stats(_))), 1);
        // a step that asks for nothing recalculates nothing
        let (mut fake, log) = Fake::new();
        fake.net_ready.store(true, Ordering::SeqCst);
        let events = run_until_idle(&mut fake, vec![], 1);
        assert_eq!(log.lock().unwrap().recalculations, 0);
        assert_eq!(count_of(&events, |s| matches!(s, Snapshot::Stats(_))), 0);
    }

    #[test]
    fn requests_view_is_sent_without_a_build() {
        let flags = Affected {
            requests: true,
            stats: true,
            ..Affected::default()
        };
        let kinds = |build_loaded: bool| {
            let (mut fake, _log) = Fake::new();
            fake.build_loaded = build_loaded;
            let events = RefCell::new(Vec::new());
            send_snapshots(&mut fake, 1, flags.clone(), &|e| {
                events.borrow_mut().push(e)
            });
            snapshots(&events.into_inner())
                .iter()
                .map(|(_, s)| match s {
                    Snapshot::Stats(_) => "stats",
                    Snapshot::Requests(_) => "requests",
                    _ => "other",
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(true), ["requests", "stats"]);
        assert_eq!(kinds(false), ["requests"]);
    }

    #[test]
    fn settings_snapshot_is_list_safe() {
        let flags = Affected {
            settings: true,
            stats: true,
            ..Affected::default()
        };
        let kinds = |build_loaded: bool| {
            let (mut fake, _log) = Fake::new();
            fake.build_loaded = build_loaded;
            let events = RefCell::new(Vec::new());
            send_snapshots(&mut fake, 1, flags.clone(), &|e| {
                events.borrow_mut().push(e)
            });
            let events = events.into_inner();
            assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
            snapshots(&events)
                .iter()
                .map(|(_, s)| match s {
                    Snapshot::Stats(_) => "stats",
                    Snapshot::Settings(_) => "settings",
                    _ => "other",
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(true), ["settings", "stats"]);
        // the settings are answered with no build open; the stats are not
        assert_eq!(kinds(false), ["settings"]);
        // a settings-only batch is list-safe by itself, so no build check is made
        assert!(
            Affected {
                settings: true,
                ..Affected::default()
            }
            .list_safe_only()
        );
    }

    #[test]
    fn options_popup_is_answered_without_a_build() {
        // a main-owned session outlives the build (R19); an item session does not
        let kinds = |popup: crate::engine::PopupKind| {
            let (mut fake, _log) = Fake::new();
            fake.build_loaded = false;
            fake.popup = Some(popup);
            let events = RefCell::new(Vec::new());
            send_snapshots(
                &mut fake,
                1,
                Affected {
                    popup: true,
                    ..Affected::default()
                },
                &|e| events.borrow_mut().push(e),
            );
            snapshots(&events.into_inner())
                .iter()
                .map(|(_, s)| matches!(s, Snapshot::Popup(Some(_))))
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(crate::engine::PopupKind::Options), [true]);
        assert_eq!(kinds(crate::engine::PopupKind::Corrupt), [false]);
    }

    #[test]
    fn requests_come_before_the_views_of_a_new_build() {
        // a request that finished with a new build tells the UI to start over, so the build's
        // views follow it
        let (mut fake, _log) = Fake::new();
        let events = RefCell::new(Vec::new());
        send_snapshots(&mut fake, 1, Affected::ALL_VIEWS, &|e| {
            events.borrow_mut().push(e)
        });
        let kinds: Vec<_> = snapshots(&events.into_inner())
            .iter()
            .map(|(_, s)| matches!(s, Snapshot::Requests(_)))
            .collect();
        assert!(kinds[0], "Requests is the step's first snapshot");
        assert_eq!(kinds.iter().filter(|k| **k).count(), 1);
    }

    fn count_of(events: &[Event], wanted: fn(&Snapshot) -> bool) -> usize {
        snapshots(events).iter().filter(|(_, s)| wanted(s)).count()
    }

    #[test]
    fn share_code_query_latest_wins() {
        let (mut fake, log) = Fake::new();
        let events = run_queued(
            &mut fake,
            vec![
                Command::ExportShareCode,
                Command::ExportShareCode,
                Command::ExportShareCode,
            ],
        );
        assert_eq!(
            count_of(&events, |s| matches!(s, Snapshot::ShareCode(_))),
            1
        );
        // the box is emptied, so the link showing is gone: the share view follows
        assert_eq!(count_of(&events, |s| matches!(s, Snapshot::Share(_))), 1);
        assert_eq!(log.lock().unwrap().share_codes, 1);
        // a later batch asks again
        let events = run_queued(&mut fake, vec![Command::ExportShareCode]);
        assert_eq!(
            count_of(&events, |s| matches!(s, Snapshot::ShareCode(_))),
            1
        );
        assert_eq!(log.lock().unwrap().share_codes, 2);
    }

    #[test]
    fn share_views_masked_without_a_build() {
        let queries = || {
            vec![
                Command::SetImportCode { text: "x".into() },
                Command::ExportShareCode,
            ]
        };
        let shares = |events: &[Event]| {
            count_of(events, |s| {
                matches!(s, Snapshot::Share(_) | Snapshot::ShareCode(_))
            })
        };
        // with a build both are answered
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, queries());
        assert_eq!(shares(&events), 2);
        assert_eq!(log.lock().unwrap().share_codes, 1);
        // with none, neither reaches the backend; the requests view still does
        let (mut fake, log) = Fake::new();
        fake.build_loaded = false;
        let events = run_queued(&mut fake, queries());
        assert_eq!(shares(&events), 0);
        assert_eq!(log.lock().unwrap().share_codes, 0);
        assert_eq!(log.lock().unwrap().share_views, 0);
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
        assert_eq!(count_of(&events, |s| matches!(s, Snapshot::Requests(_))), 1);
    }

    #[test]
    fn compare_is_build_only() {
        use crate::engine::CompareSource;
        let queries = || {
            vec![Command::LoadCompare {
                source: CompareSource::Code("x".into()),
            }]
        };
        let compares = |events: &[Event]| count_of(events, |s| matches!(s, Snapshot::Compare(_)));
        // with a build the view is answered, "no comparison" included
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, queries());
        assert_eq!(compares(&events), 1);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Snapshot {
                    snapshot: Snapshot::Compare(None),
                    ..
                }
            )),
            "{events:?}"
        );
        assert_eq!(log.lock().unwrap().compare_views, 1);
        // a recalculation of this build refreshes the diff
        let (mut fake, log) = Fake::new();
        let events = run_queued(&mut fake, vec![set_config()]);
        assert_eq!(compares(&events), 1);
        assert_eq!(log.lock().unwrap().compare_views, 1);
        // with none it never reaches the backend, nor does a whole-build refresh
        for commands in [queries(), vec![Command::NewBuild]] {
            let (mut fake, log) = Fake::new();
            fake.build_loaded = false;
            let events = run_queued(&mut fake, commands);
            assert_eq!(compares(&events), 0);
            assert_eq!(log.lock().unwrap().compare_views, 0);
            assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
        }
        const { assert!(Affected::ALL_VIEWS.compare) };
    }

    #[test]
    fn browsing_builds_answers_only_its_view_with_or_without_a_build() {
        for loaded in [true, false] {
            let (mut fake, log) = Fake::new();
            fake.build_loaded = loaded;
            let events = run_queued(
                &mut fake,
                vec![Command::BrowseBuilds {
                    folder: "sub/".into(),
                }],
            );
            let snapshots: Vec<&Snapshot> = events
                .iter()
                .filter_map(|e| match e {
                    Event::Snapshot { snapshot, .. } => Some(snapshot),
                    _ => None,
                })
                .collect();
            assert!(
                matches!(snapshots.as_slice(), [Snapshot::BuildBrowse(view)] if view.folder == "sub/"),
                "{events:?}"
            );
            assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
            // no request cancelled, nothing else applied
            assert_eq!(
                log.lock().unwrap().requests,
                ["apply BrowseBuilds { folder: \"sub/\" }"]
            );
        }
        let browse = Command::BrowseBuilds { folder: "".into() };
        assert_eq!(crate::engine::pob::cancels_requests(&browse), None);
        assert!(!crate::engine::pob::ends_popup_session(&browse));
        assert!(!crate::engine::pob::ends_item_db(&browse));
        // unlike `ListBuilds`, which leaves the build
        assert!(!crate::engine::pob::leaves_build(&browse));
    }

    #[test]
    fn short_link_is_sent_once_as_a_tree_link() {
        let (mut fake, _log) = Fake::new();
        fake.short_link = Some("http://poeurl.com/gAtE".into());
        let flags = Affected {
            short_link: true,
            ..Affected::default()
        };
        let mut sent = Vec::new();
        for _ in 0..2 {
            let events = RefCell::new(Vec::new());
            send_snapshots(&mut fake, 1, flags.clone(), &|e| {
                events.borrow_mut().push(e)
            });
            sent.push(
                snapshots(&events.into_inner())
                    .iter()
                    .filter_map(|(_, s)| match s {
                        Snapshot::TreeLink(link) => Some(link.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(sent, [vec!["http://poeurl.com/gAtE".to_string()], vec![]]);
    }

    #[test]
    fn dropping_the_engine_ends_its_thread() {
        // the fake keeps the waker, and with it a sender of the command channel (R9)
        let (fake, _log) = Fake::new();
        let engine = Engine::spawn_with(move |_send| Ok(Box::new(fake) as Box<dyn Backend>), || {});
        // Ready comes after the waker is set
        loop {
            match engine.events.recv_timeout(Duration::from_secs(5)) {
                Ok(Event::Ready) => break,
                Ok(_) => {}
                Err(e) => panic!("no Ready: {e}"),
            }
        }
        let alive = engine.liveness();
        assert!(alive.is_alive());
        drop(engine);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while alive.is_alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!alive.is_alive(), "the engine thread outlived its Engine");
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

    /// Runs `steps` power steps, then sends `after_last` (if any) from the last step, and
    /// ends the loop at its first memory release. Returns the releases seen while stepping.
    fn run_to_release(steps: usize, after_last: Option<Command>) -> (Arc<Mutex<Log>>, usize) {
        let (mut fake, log) = Fake::new();
        fake.background
            .lock()
            .unwrap()
            .extend((0..steps).map(|_| step_affecting_power()));
        let (tx, rx) = mpsc::channel();
        let mut command = Some(tx.clone());
        let step_log = log.clone();
        let during = Arc::new(AtomicUsize::new(0));
        let seen = during.clone();
        let expected_previews = usize::from(after_last.is_some());
        let mut after_last = after_last;
        fake.on_step = Some(Box::new(move |n| {
            seen.fetch_add(step_log.lock().unwrap().releases, Ordering::SeqCst);
            if n == steps
                && let (Some(tx), Some(c)) = (command.take(), after_last.take())
            {
                tx.send((2, c)).unwrap();
            }
        }));
        let mut stop = Some(tx.clone());
        let release_log = log.clone();
        fake.on_release = Some(Box::new(move |_| {
            let previews = release_log.lock().unwrap().previews.len();
            assert_eq!(
                previews, expected_previews,
                "released before the command ran"
            );
            stop.take();
        }));
        tx.send((1, set_config())).unwrap();
        drop(tx);
        run_loop_with_delay(&mut fake, &rx, &|_| {}, Duration::from_millis(20));
        let during = during.load(Ordering::SeqCst);
        (log, during)
    }

    #[test]
    fn memory_is_released_once_after_background_work() {
        let (log, during) = run_to_release(5, None);
        let log = log.lock().unwrap();
        assert_eq!(log.steps, 5);
        assert_eq!(during, 0, "released between a job's steps");
        assert_eq!(log.releases, 1);
    }

    #[test]
    fn command_after_the_job_delays_the_release() {
        let (log, during) = run_to_release(3, Some(Command::PreviewNode { id: 7 }));
        let log = log.lock().unwrap();
        assert_eq!(during, 0, "released between a job's steps");
        assert_eq!(log.previews, vec![7]);
        assert_eq!(log.releases, 1);
    }
}
