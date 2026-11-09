//! Recording fake backend for engine tests.

use std::sync::{Arc, Mutex};

use super::commands::Command;
use super::run::{Affected, Backend, Event};
use super::views::{BuildListView, BuildView, ConfigView, StatsView};

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
        Ok(StatsView { stats: vec![] })
    }

    fn config_view(&mut self) -> anyhow::Result<ConfigView> {
        Ok(ConfigView {
            sets: vec![],
            active_set: 1,
            custom_mods: String::new(),
            sections: vec![],
        })
    }
}
