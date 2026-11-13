//! Recording fake backend for engine tests.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pob_runtime::net::Waker;

use super::commands::{
    Command, ControlTooltipQuery, GemSearchQuery, GemTooltipQuery, ItemTooltipQuery, PopupKind,
};
use super::pob::{CancelScope, cancels_requests, ends_item_db, ends_popup_session};
use super::run::{Affected, Backend, BackgroundTask, DbSnapshot, Event};
use super::views::{
    AccountView, BreakdownView, BuildBrowseView, BuildListView, BuildView, CalcsView, CellRef,
    CompareView, ConfigView, ControlTooltipView, DisplayItemView, DropList, GemSearchView,
    GemTooltipView, GroupTooltipView, ItemDbView, ItemText, ItemTooltip, ItemTooltipView,
    ItemsView, NodePower, NodeTooltip, NotesView, PendingRequest, Points, PopupView, PowerReport,
    RequestsView, SettingsView, ShareView, SkillsView, SocketGroupText, StatsView, TooltipLine,
    TreeLayout, TreeState,
};

#[derive(Default, Debug)]
pub struct Log {
    pub set_configs: usize,
    pub recalculations: usize,
    pub steps: usize,
    /// The task of each step, in order
    pub step_tasks: Vec<BackgroundTask>,
    /// The text of each gem search view sent
    pub gem_searches: Vec<String>,
    /// The query of each item database view sent
    pub item_dbs: Vec<crate::engine::ItemDbQuery>,
    /// Recalculations that found a search open, and so marked its DPS job pending
    pub gem_restarts: usize,
    pub previews: Vec<u32>,
    pub breakdowns: Vec<CellRef>,
    pub copies: Vec<usize>,
    pub tooltips: Vec<usize>,
    pub item_tooltips: Vec<ItemTooltipQuery>,
    pub control_tooltips: Vec<ControlTooltipQuery>,
    pub releases: usize,
    /// Cancels and applied commands, in order: `cancel <id>`, `apply <command>`
    pub requests: Vec<String>,
    /// Share codes asked for
    pub share_codes: usize,
    /// Share views asked for
    pub share_views: usize,
    /// Compare views asked for
    pub compare_views: usize,
    pub applies: usize,
    /// Hover tooltips answered (R22)
    pub hovers: Vec<crate::engine::HoverTarget>,
    /// `shutdown` calls that finished
    pub shutdowns: usize,
}

/// The link a fake trade window's `search` press answers
pub const FAKE_TRADE_URL: &str = "https://www.pathofexile.com/trade/search/Standard?q=fake";

/// One scripted background step: what it affects, or the error it fails with.
pub type Step = Result<Affected, String>;

pub struct Fake {
    pub log: Arc<Mutex<Log>>,
    pub fail_set_config: Option<String>,
    /// Node power steps still to run; the loop works through them while no command is waiting.
    pub background: Arc<Mutex<VecDeque<Step>>>,
    /// Gem DPS steps still to run; they go before the power steps.
    pub gem_background: Arc<Mutex<VecDeque<Step>>>,
    /// Anoint sort steps still to run; they go after the gem steps and before the power steps.
    pub anoint_background: Arc<Mutex<VecDeque<Step>>>,
    /// Timeless search steps still to run; after the Anoint sort, before the item databases
    pub timeless_background: Arc<Mutex<VecDeque<Step>>>,
    /// Item database steps still to run; they go after the Anoint steps, before the power steps.
    pub item_db_background: Arc<Mutex<VecDeque<Step>>>,
    /// The open gem search
    pub gem_search: Option<GemSearchQuery>,
    /// The open item database
    pub item_db: Option<crate::engine::ItemDbQuery>,
    /// The database closed; `ItemDb(None)` is still to be sent
    item_db_ended: bool,
    /// Replaces the queue when a command returns `build` or `recalc`.
    pub restart_on_build: Vec<Step>,
    /// Called after each step with the number of steps run so far.
    pub on_step: Option<Box<dyn FnMut(usize) + Send>>,
    /// Called after each memory release with the number of releases so far.
    pub on_release: Option<Box<dyn FnMut(usize) + Send>>,
    /// `node_tooltip_view` answers `None`, as for a node that no longer exists.
    pub tooltip_vanished: bool,
    /// `item_tooltip_view` answers `None`, as for an item that no longer exists.
    pub item_vanished: bool,
    /// `control_tooltip_view` answers `None`, as for a control that no longer exists.
    pub control_vanished: bool,
    /// The open popup session
    pub popup: Option<PopupKind>,
    /// `breakdown_view` fails with this message.
    pub fail_breakdown: Option<String>,
    /// `build_loaded` answers this; PoB starts without a build when it was left on the list.
    pub build_loaded: bool,
    /// A subscript has something to deliver: `Network` is offered while this is set, and a
    /// Network step clears it.
    pub net_ready: Arc<AtomicBool>,
    /// Pending requests: id and scope ("build" or "list")
    pub pending: Vec<(u64, &'static str)>,
    /// The waker the engine gave
    pub waker: Option<Waker>,
    /// The short link `short_tree_link_view` answers once
    pub short_link: Option<String>,
    /// A Network step reports that it changed the build and asks for a recalculation
    pub network_recalc: bool,
    /// How long `shutdown` takes, as PoB's exit save of `Settings.xml` does
    pub shutdown_delay: std::time::Duration,
}

impl Fake {
    pub fn new() -> (Fake, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        (
            Fake {
                log: log.clone(),
                fail_set_config: None,
                background: Arc::default(),
                gem_background: Arc::default(),
                anoint_background: Arc::default(),
                timeless_background: Arc::default(),
                item_db_background: Arc::default(),
                item_db: None,
                item_db_ended: false,
                gem_search: None,
                restart_on_build: Vec::new(),
                on_step: None,
                on_release: None,
                tooltip_vanished: false,
                item_vanished: false,
                control_vanished: false,
                popup: None,
                fail_breakdown: None,
                build_loaded: true,
                net_ready: Arc::default(),
                pending: Vec::new(),
                waker: None,
                short_link: None,
                network_recalc: false,
                shutdown_delay: std::time::Duration::ZERO,
            },
            log,
        )
    }
}

impl Fake {
    fn apply_command(&mut self, command: Command) -> anyhow::Result<Affected> {
        match command {
            Command::SetConfig { .. } => {
                self.log.lock().unwrap().set_configs += 1;
                if let Some(message) = &self.fail_set_config {
                    anyhow::bail!("{message}");
                }
                Ok(Affected {
                    recalc: true,
                    ..Affected::default()
                })
            }
            Command::NewBuild => Ok(Affected {
                tree_layout: true,
                ..Affected::ALL_VIEWS
            }),
            Command::PreviewNode { id } => Ok(Affected {
                preview: Some(id),
                ..Affected::default()
            }),
            Command::Breakdown(cell) => Ok(Affected {
                breakdown: Some(cell),
                ..Affected::default()
            }),
            Command::GroupTooltip { group } => Ok(Affected {
                group_tooltip: Some(group),
                ..Affected::default()
            }),
            Command::HoverTooltip(target) => Ok(Affected {
                hover: Some(target),
                ..Affected::default()
            }),
            Command::CopySocketGroup { group } => Ok(Affected {
                group_text: Some(group),
                ..Affected::default()
            }),
            Command::SearchGems(query) => {
                self.gem_search = Some(query);
                Ok(Affected {
                    gem_search: true,
                    ..Affected::default()
                })
            }
            Command::CloseGemSearch => {
                self.gem_search = None;
                Ok(Affected::default())
            }
            Command::SetItemDb(query) => {
                self.item_db = Some(query);
                self.item_db_ended = false;
                Ok(Affected {
                    item_db: true,
                    ..Affected::default()
                })
            }
            Command::GemTooltip(query) => Ok(Affected {
                gem_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::ItemTooltip(query) => Ok(Affected {
                item_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::OpenPopup(kind) => {
                self.popup = Some(kind);
                Ok(Affected {
                    popup: true,
                    ..Affected::default()
                })
            }
            // a press answers with the popup, as the real engine's does; a trade window's
            // search also with its link (R20)
            Command::PressPopupButton { name } => Ok(Affected {
                popup: true,
                external_url: (name == "search").then(|| crate::engine::ExternalUrl {
                    url: FAKE_TRADE_URL.into(),
                }),
                ..Affected::default()
            }),
            // the real engine answers with the settings (R12)
            Command::SetOverrides { .. } => Ok(Affected {
                settings: true,
                ..Affected::default()
            }),
            Command::ControlTooltip(query) => Ok(Affected {
                control_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::CopyItem { id } => Ok(Affected {
                item_text: Some(id),
                ..Affected::default()
            }),
            Command::CancelDisplayItem { .. } => Ok(Affected {
                display_item: true,
                ..Affected::default()
            }),
            Command::ExportTreeLink => Ok(Affected {
                link: true,
                ..Affected::default()
            }),
            Command::ShrinkTreeLink => Ok(Affected {
                short_link: true,
                requests: true,
                ..Affected::default()
            }),
            Command::SetImportCode { .. } | Command::ShareBuild { .. } => Ok(Affected {
                share: true,
                requests: true,
                ..Affected::default()
            }),
            Command::ImportFromUrl { .. } => Ok(Affected {
                share: true,
                requests: true,
                ..Affected::default()
            }),
            Command::ListPublicCharacters { .. }
            | Command::ImportPublicCharacter { .. }
            | Command::SignIn
            | Command::ListCharacters { .. }
            | Command::ImportCharacter { .. } => Ok(Affected {
                account: true,
                requests: true,
                ..Affected::default()
            }),
            Command::ClosePublicCharacters | Command::SignOut => Ok(Affected {
                account: true,
                ..Affected::default()
            }),
            Command::ExportShareCode => Ok(Affected {
                share: true,
                share_code: true,
                ..Affected::default()
            }),
            Command::BrowseBuilds { folder } => Ok(Affected {
                browse: Some(folder),
                ..Affected::default()
            }),
            // PoB's text for a code it can't load (CompareTab.lua:1256-1260)
            Command::LoadCompare {
                source: crate::engine::CompareSource::Code(code),
            } if code == "bad" => anyhow::bail!("Failed to import build for comparison."),
            Command::LoadCompare { .. }
            | Command::SelectCompare { .. }
            | Command::RemoveCompare { .. }
            | Command::SetCompareControl { .. } => Ok(Affected {
                compare: true,
                tree: true,
                ..Affected::default()
            }),
            // the copies edit the build, as the real engine answers them (R8)
            Command::CopyCompareTree { .. } => Ok(Affected {
                build: true,
                tree: true,
                recalc: true,
                compare: true,
                ..Affected::default()
            }),
            Command::CopyCompareItem { .. } => Ok(Affected {
                build: true,
                items: true,
                recalc: true,
                compare: true,
                ..Affected::default()
            }),
            Command::CopyCompareConfig => Ok(Affected {
                build: true,
                config: true,
                recalc: true,
                compare: true,
                ..Affected::default()
            }),
            Command::SetCompareOverlay { .. } => Ok(Affected {
                tree: true,
                ..Affected::default()
            }),
            Command::Help => Ok(Affected {
                help: true,
                ..Affected::default()
            }),
            _ => Ok(Affected::default()),
        }
    }
}

impl Backend for Fake {
    fn shutdown(&mut self) {
        std::thread::sleep(self.shutdown_delay);
        self.log.lock().unwrap().shutdowns += 1;
    }

    fn apply(&mut self, command: Command, _send: &dyn Fn(Event)) -> anyhow::Result<Affected> {
        if let Some(scope) = cancels_requests(&command) {
            let mut log = self.log.lock().unwrap();
            self.pending.retain(|&(id, request_scope)| {
                let cancel = scope == CancelScope::All || request_scope == "build";
                if cancel {
                    log.requests.push(format!("cancel {id}"));
                }
                !cancel
            });
        }
        {
            let mut log = self.log.lock().unwrap();
            log.applies += 1;
            log.requests.push(format!("apply {command:?}"));
        }
        let ends = ends_popup_session(&command);
        let ends_db = ends_item_db(&command);
        let mut affected = self.apply_command(command)?;
        if ends_db && self.item_db.take().is_some() {
            self.item_db_ended = true;
            affected.item_db = true;
        }
        // a command that failed changed nothing, so it leaves the session open
        if ends && self.popup.take().is_some() {
            affected.popup = true;
        }
        if (affected.build || affected.recalc) && !self.restart_on_build.is_empty() {
            *self.background.lock().unwrap() = self.restart_on_build.iter().cloned().collect();
        }
        Ok(affected)
    }

    fn recalculate(&mut self) -> anyhow::Result<()> {
        let mut log = self.log.lock().unwrap();
        log.recalculations += 1;
        if self.gem_search.is_some() {
            log.gem_restarts += 1;
        }
        Ok(())
    }

    fn build_view(&mut self) -> anyhow::Result<BuildView> {
        Ok(BuildView {
            name: "fake".into(),
            path: None,
            class: "Witch".into(),
            ascendancy: None,
            level: 1,
            unsaved: false,
            can_undo: false,
            can_redo: false,
            ..BuildView::default()
        })
    }

    fn build_loaded(&mut self) -> anyhow::Result<bool> {
        Ok(self.build_loaded)
    }

    fn build_list_view(&mut self) -> anyhow::Result<BuildListView> {
        Ok(BuildListView {
            folder: String::new(),
            entries: vec![],
            sort: Default::default(),
            filter: String::new(),
            ..Default::default()
        })
    }

    fn stats_view(&mut self) -> anyhow::Result<StatsView> {
        Ok(StatsView {
            stats: vec![],
            minion: vec![],
            ..StatsView::default()
        })
    }

    fn config_view(&mut self) -> anyhow::Result<ConfigView> {
        Ok(ConfigView {
            sets: vec![],
            active_set: 1,
            custom_mods: Vec::new(),
            sections: vec![],
        })
    }

    fn tree_layout_view(&mut self) -> anyhow::Result<TreeLayout> {
        Ok(empty_layout())
    }

    fn tree_state_view(&mut self) -> anyhow::Result<TreeState> {
        Ok(empty_tree_state())
    }

    fn calcs_view(&mut self) -> anyhow::Result<CalcsView> {
        Ok(CalcsView::default())
    }

    fn skills_view(&mut self) -> anyhow::Result<SkillsView> {
        Ok(SkillsView::default())
    }

    fn items_view(&mut self) -> anyhow::Result<ItemsView> {
        Ok(ItemsView::default())
    }

    fn display_item_view(&mut self) -> anyhow::Result<Option<DisplayItemView>> {
        Ok(None)
    }

    fn socket_group_text(&mut self, group: usize) -> anyhow::Result<SocketGroupText> {
        self.log.lock().unwrap().copies.push(group);
        Ok(SocketGroupText {
            group,
            text: format!("group {group}"),
        })
    }

    fn group_tooltip_view(&mut self, group: usize) -> anyhow::Result<GroupTooltipView> {
        self.log.lock().unwrap().tooltips.push(group);
        Ok(GroupTooltipView {
            group,
            lines: vec![],
        })
    }

    fn notes_view(&mut self) -> anyhow::Result<NotesView> {
        Ok(NotesView::default())
    }

    fn next_background(&self) -> Option<BackgroundTask> {
        if self.net_ready.load(Ordering::SeqCst) {
            Some(BackgroundTask::Network)
        } else if !self.gem_background.lock().unwrap().is_empty() {
            Some(BackgroundTask::GemDps)
        } else if !self.anoint_background.lock().unwrap().is_empty() {
            Some(BackgroundTask::AnointSort)
        } else if !self.timeless_background.lock().unwrap().is_empty() {
            Some(BackgroundTask::Timeless)
        } else if self.item_db.is_some() && !self.item_db_background.lock().unwrap().is_empty() {
            Some(BackgroundTask::ItemDb)
        } else if !self.background.lock().unwrap().is_empty() {
            Some(BackgroundTask::NodePower)
        } else {
            None
        }
    }

    fn background_step(
        &mut self,
        task: BackgroundTask,
        send: &dyn Fn(Event),
    ) -> anyhow::Result<Affected> {
        if task == BackgroundTask::Network {
            self.net_ready.store(false, Ordering::SeqCst);
            let steps = {
                let mut log = self.log.lock().unwrap();
                log.steps += 1;
                log.step_tasks.push(task);
                log.steps
            };
            if let Some(on_step) = &mut self.on_step {
                on_step(steps);
            }
            return Ok(Affected {
                requests: true,
                recalc: self.network_recalc,
                ..Affected::default()
            });
        }
        if task == BackgroundTask::ItemDb {
            send(Event::Progress { task, percent: 50 });
        }
        let queue = match task {
            BackgroundTask::Network => unreachable!("handled above"),
            BackgroundTask::GemDps => self.gem_background.clone(),
            BackgroundTask::AnointSort => self.anoint_background.clone(),
            BackgroundTask::Timeless => self.timeless_background.clone(),
            BackgroundTask::ItemDb => self.item_db_background.clone(),
            BackgroundTask::NodePower => self.background.clone(),
        };
        let step = queue.lock().unwrap().pop_front();
        let steps = {
            let mut log = self.log.lock().unwrap();
            log.steps += 1;
            log.step_tasks.push(task);
            log.steps
        };
        if let Some(on_step) = &mut self.on_step {
            on_step(steps);
        }
        match step {
            Some(Ok(affected)) => Ok(affected),
            Some(Err(message)) => {
                // a failed calculation has no more work
                queue.lock().unwrap().clear();
                anyhow::bail!("{message}")
            }
            None => Ok(Affected::default()),
        }
    }

    fn gem_search_view(&mut self) -> anyhow::Result<Option<GemSearchView>> {
        let Some(query) = self.gem_search.clone() else {
            return Ok(None);
        };
        self.log
            .lock()
            .unwrap()
            .gem_searches
            .push(query.text.clone());
        let sorting = !self.gem_background.lock().unwrap().is_empty();
        Ok(Some(GemSearchView {
            query,
            rows: vec![],
            base_dps: 0.0,
            sorting,
            percent: None,
            ended: false,
        }))
    }

    fn item_db_view(&mut self) -> anyhow::Result<DbSnapshot> {
        if std::mem::take(&mut self.item_db_ended) {
            return Ok(DbSnapshot::Ended);
        }
        let Some(query) = self.item_db.clone() else {
            return Ok(DbSnapshot::Quiet);
        };
        self.log.lock().unwrap().item_dbs.push(query.clone());
        Ok(DbSnapshot::View(ItemDbView {
            db: query.db,
            loading: false,
            sorting: !self.item_db_background.lock().unwrap().is_empty(),
            percent: None,
            slot: DropList::default(),
            kind: DropList::default(),
            league: None,
            requirement: None,
            obtainable: None,
            sort: None,
            search: query.search,
            search_mode: DropList::default(),
            rows: vec![],
            note: String::new(),
        }))
    }

    fn gem_tooltip_view(&mut self, query: &GemTooltipQuery) -> anyhow::Result<GemTooltipView> {
        Ok(GemTooltipView {
            query: query.clone(),
            lines: vec![],
        })
    }

    fn item_tooltip_view(
        &mut self,
        query: &ItemTooltipQuery,
    ) -> anyhow::Result<Option<ItemTooltipView>> {
        self.log.lock().unwrap().item_tooltips.push(query.clone());
        if self.item_vanished {
            return Ok(None);
        }
        Ok(Some(ItemTooltipView {
            query: query.clone(),
            tooltip: ItemTooltip::default(),
        }))
    }

    fn popup_view(&mut self) -> anyhow::Result<Option<PopupView>> {
        // a session of the item or the build ends with the build; only Options outlives it (R19)
        let build_loaded = self.build_loaded;
        Ok(self
            .popup
            .clone()
            .filter(|kind| build_loaded || *kind == PopupKind::Options)
            .map(|kind| PopupView {
                kind,
                title: "popup".into(),
                enter: None,
                controls: vec![],
            }))
    }

    fn control_tooltip_view(
        &mut self,
        query: &ControlTooltipQuery,
    ) -> anyhow::Result<Option<ControlTooltipView>> {
        self.log
            .lock()
            .unwrap()
            .control_tooltips
            .push(query.clone());
        if self.control_vanished {
            return Ok(None);
        }
        Ok(Some(ControlTooltipView {
            query: query.clone(),
            tooltip: ItemTooltip::default(),
        }))
    }

    fn item_text(&mut self, id: u32) -> anyhow::Result<ItemText> {
        Ok(ItemText {
            id,
            text: String::new(),
            shared: false,
        })
    }

    fn node_power_view(&mut self) -> anyhow::Result<NodePower> {
        Ok(NodePower {
            stat: None,
            nodes: vec![],
            max_single: 0.0,
            max_offence: 0.0,
            max_defence: 0.0,
            finished: false,
        })
    }

    fn power_report_view(&mut self) -> anyhow::Result<PowerReport> {
        Ok(PowerReport {
            stat: String::new(),
            rows: vec![],
            listed: vec![],
            filter: 0,
            masteries: true,
        })
    }

    fn node_tooltip_view(&mut self, id: u32) -> anyhow::Result<Option<NodeTooltip>> {
        self.log.lock().unwrap().previews.push(id);
        if self.tooltip_vanished {
            return Ok(None);
        }
        Ok(Some(NodeTooltip {
            id,
            trace: vec![],
            lines: vec![TooltipLine::Line {
                size: 14,
                text: format!("node {id}"),
            }],
        }))
    }

    fn breakdown_view(&mut self, cell: &CellRef) -> anyhow::Result<BreakdownView> {
        self.log.lock().unwrap().breakdowns.push(cell.clone());
        if let Some(message) = &self.fail_breakdown {
            anyhow::bail!("{message}");
        }
        Ok(BreakdownView {
            cell: cell.clone(),
            sections: vec![],
            error: None,
        })
    }

    fn tree_link_view(&mut self) -> anyhow::Result<String> {
        Ok("https://example.test/tree".into())
    }

    fn short_tree_link_view(&mut self) -> anyhow::Result<Option<String>> {
        Ok(self.short_link.take())
    }

    fn share_view(&mut self) -> anyhow::Result<ShareView> {
        self.log.lock().unwrap().share_views += 1;
        Ok(ShareView::default())
    }

    fn account_view(&mut self) -> anyhow::Result<AccountView> {
        Ok(AccountView::default())
    }

    fn compare_view(&mut self) -> anyhow::Result<Option<CompareView>> {
        self.log.lock().unwrap().compare_views += 1;
        Ok(None)
    }

    fn browse_view(&mut self, folder: &str) -> anyhow::Result<BuildBrowseView> {
        // a folder that went since the build list was read
        if folder == "gone/" {
            anyhow::bail!("no such folder");
        }
        Ok(BuildBrowseView {
            folder: folder.to_owned(),
            entries: vec![crate::engine::views::BuildBrowseEntry {
                name: format!("{folder}fake"),
                path: format!("/b/{folder}fake.xml").into(),
            }],
            error: None,
        })
    }

    fn share_code(&mut self) -> anyhow::Result<String> {
        self.log.lock().unwrap().share_codes += 1;
        Ok("fake-code".into())
    }

    fn requests_view(&mut self) -> anyhow::Result<RequestsView> {
        Ok(RequestsView {
            pending: self
                .pending
                .iter()
                .map(|&(id, _)| PendingRequest {
                    id,
                    label: format!("request {id}"),
                })
                .collect(),
            finished: vec![],
        })
    }

    fn settings_view(&mut self) -> anyhow::Result<SettingsView> {
        Ok(SettingsView::default())
    }

    fn set_waker(&mut self, wake: Waker) {
        self.waker = Some(wake);
    }

    fn hover_tooltip_view(
        &mut self,
        target: &crate::engine::HoverTarget,
    ) -> anyhow::Result<Option<crate::engine::HoverTooltipView>> {
        self.log.lock().unwrap().hovers.push(target.clone());
        Ok(Some(crate::engine::HoverTooltipView {
            target: target.clone(),
            lines: vec![],
        }))
    }

    fn release_memory(&mut self) {
        let releases = {
            let mut log = self.log.lock().unwrap();
            log.releases += 1;
            log.releases
        };
        if let Some(on_release) = &mut self.on_release {
            on_release(releases);
        }
    }

    fn help_view(&mut self) -> anyhow::Result<crate::engine::HelpView> {
        Ok(crate::engine::HelpView {
            sections: vec![crate::engine::HelpSection {
                title: "Help File Introduction".into(),
                lines: vec!["^7This file contains a list of shortcuts".into()],
            }],
            header: vec![],
            changelog: vec![],
            missing: vec![],
        })
    }
}

/// A tree with no nodes, for backends that don't care about the tree.
pub fn empty_layout() -> TreeLayout {
    TreeLayout {
        version: "3_28".into(),
        nodes: vec![],
        edges: vec![],
        sheets: vec![],
        key: "3_28:0:0".into(),
    }
}

/// A build with nothing allocated.
pub fn empty_tree_state() -> TreeState {
    TreeState {
        version: "3_28".into(),
        version_label: "3.28".into(),
        convertible_to: None,
        class_id: 0,
        ascendancy_id: 0,
        classes: vec![],
        points: Points {
            used: 0,
            available: 122,
            ascendancy_used: 0,
            ascendancy_available: 8,
        },
        allocated: vec![],
        masteries: vec![],
        mastery_options: vec![],
        paths: vec![],
        depends: vec![],
        search: String::new(),
        search_matches: vec![],
        compare: vec![],
        jewels: vec![],
        specs: vec![],
        active_spec: 0,
        compare_spec: None,
        power_stats: vec![],
        heatmap: None,
        max_depth: None,
        can_undo: false,
        can_redo: false,
        heatmap_shown: false,
        show_stat_differences: true,
        versions: vec![],
    }
}
