//! Recording fake backend for engine tests.

use std::sync::{Arc, Mutex};

use super::commands::Command;
use super::run::{Affected, Backend, Event};
use super::views::{
    BuildListView, BuildView, ConfigView, Points, StatsView, TreeLayout, TreeState,
};

#[derive(Default, Debug)]
pub struct Log {
    pub set_configs: usize,
    pub recalculations: usize,
}

pub struct Fake {
    pub log: Arc<Mutex<Log>>,
    pub fail_set_config: Option<String>,
}

impl Fake {
    pub fn new() -> (Fake, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        (
            Fake {
                log: log.clone(),
                fail_set_config: None,
            },
            log,
        )
    }
}

impl Backend for Fake {
    fn apply(&mut self, command: Command, _send: &dyn Fn(Event)) -> anyhow::Result<Affected> {
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
            _ => Ok(Affected::default()),
        }
    }

    fn recalculate(&mut self) -> anyhow::Result<()> {
        self.log.lock().unwrap().recalculations += 1;
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
