//! Recording fake backend for engine tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::commands::{Command, GemSearchQuery, GemTooltipQuery, ItemTooltipQuery};
use super::run::{Affected, Backend, BackgroundTask, Event};
use super::views::{
    BreakdownView, BuildListView, BuildView, CalcsView, CellRef, ConfigView, DisplayItemView,
    GemSearchView, GemTooltipView, GroupTooltipView, ItemText, ItemTooltip, ItemTooltipView,
    ItemsView, NodePower, NodeTooltip, NotesView, Points, PowerReport, SkillsView, SocketGroupText,
    StatsView, TooltipLine, TreeLayout, TreeState,
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
    /// Recalculations that found a search open, and so marked its DPS job pending
    pub gem_restarts: usize,
    pub previews: Vec<u32>,
    pub breakdowns: Vec<CellRef>,
    pub copies: Vec<usize>,
    pub tooltips: Vec<usize>,
    pub item_tooltips: Vec<ItemTooltipQuery>,
}

/// One scripted background step: what it affects, or the error it fails with.
pub type Step = Result<Affected, String>;

pub struct Fake {
    pub log: Arc<Mutex<Log>>,
    pub fail_set_config: Option<String>,
    /// Node power steps still to run; the loop works through them while no command is waiting.
    pub background: Arc<Mutex<VecDeque<Step>>>,
    /// Gem DPS steps still to run; they go before the power steps.
    pub gem_background: Arc<Mutex<VecDeque<Step>>>,
    /// The open gem search
    pub gem_search: Option<GemSearchQuery>,
    /// Replaces the queue when a command returns `build` or `recalc`.
    pub restart_on_build: Vec<Step>,
    /// Called after each step with the number of steps run so far.
    pub on_step: Option<Box<dyn FnMut(usize) + Send>>,
    /// `node_tooltip_view` answers `None`, as for a node that no longer exists.
    pub tooltip_vanished: bool,
    /// `item_tooltip_view` answers `None`, as for an item that no longer exists.
    pub item_vanished: bool,
    /// `breakdown_view` fails with this message.
    pub fail_breakdown: Option<String>,
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
                gem_search: None,
                restart_on_build: Vec::new(),
                on_step: None,
                tooltip_vanished: false,
                item_vanished: false,
                fail_breakdown: None,
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
            Command::GemTooltip(query) => Ok(Affected {
                gem_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::ItemTooltip(query) => Ok(Affected {
                item_tooltip: Some(query),
                ..Affected::default()
            }),
            Command::CopyItem { id } => Ok(Affected {
                item_text: Some(id),
                ..Affected::default()
            }),
            Command::CancelDisplayItem => Ok(Affected {
                display_item: true,
                ..Affected::default()
            }),
            Command::ExportTreeLink => Ok(Affected {
                link: true,
                ..Affected::default()
            }),
            _ => Ok(Affected::default()),
        }
    }
}

impl Backend for Fake {
    fn apply(&mut self, command: Command, _send: &dyn Fn(Event)) -> anyhow::Result<Affected> {
        let affected = self.apply_command(command)?;
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
        })
    }

    fn build_list_view(&mut self) -> anyhow::Result<BuildListView> {
        Ok(BuildListView {
            folder: String::new(),
            entries: vec![],
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
        if !self.gem_background.lock().unwrap().is_empty() {
            Some(BackgroundTask::GemDps)
        } else if !self.background.lock().unwrap().is_empty() {
            Some(BackgroundTask::NodePower)
        } else {
            None
        }
    }

    fn background_step(
        &mut self,
        task: BackgroundTask,
        _send: &dyn Fn(Event),
    ) -> anyhow::Result<Affected> {
        let queue = match task {
            BackgroundTask::GemDps => self.gem_background.clone(),
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

    fn item_text(&mut self, id: u32) -> anyhow::Result<ItemText> {
        Ok(ItemText {
            id,
            text: String::new(),
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
        })
    }

    fn node_tooltip_view(&mut self, id: u32) -> anyhow::Result<Option<NodeTooltip>> {
        self.log.lock().unwrap().previews.push(id);
        if self.tooltip_vanished {
            return Ok(None);
        }
        Ok(Some(NodeTooltip {
            id,
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
    }
}
