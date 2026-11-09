//! The engine thread: a command loop over a [`Backend`], and the [`Engine`] handle the UI holds.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::pob_files::DownloadProgress;

use super::commands::Command;
use super::lab::LabEvent;
use super::views::{BuildListView, BuildView, ConfigView, StatsView};

#[derive(Debug, Clone)]
pub enum Snapshot {
    Build(Arc<BuildView>),
    BuildList(Arc<BuildListView>),
    Stats(Arc<StatsView>),
    Config(Arc<ConfigView>),
}

#[derive(Debug, Clone)]
pub enum Event {
    Downloading(DownloadProgress),
    Ready,
    StartupFailed { message: String },
    Snapshot { seq: u64, snapshot: Snapshot },
    Error { seq: u64, message: String },
    Lab(LabEvent),
}

/// What a command changed, so the loop knows which snapshots to send.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Affected {
    pub build: bool,
    pub list: bool,
    pub stats: bool,
    pub config: bool,
    pub recalc: bool,
}

impl Affected {
    pub(crate) const ALL_VIEWS: Affected = Affected {
        build: true,
        list: true,
        stats: true,
        config: true,
        recalc: false,
    };

    fn merge(&mut self, other: Affected) {
        self.build |= other.build;
        self.list |= other.list;
        self.stats |= other.stats;
        self.config |= other.config;
        self.recalc |= other.recalc;
    }
}

pub trait Backend {
    fn apply(&mut self, command: Command, send: &dyn Fn(Event)) -> anyhow::Result<Affected>;
    fn recalculate(&mut self) -> anyhow::Result<()>;
    fn build_view(&mut self) -> anyhow::Result<BuildView>;
    fn build_list_view(&mut self) -> anyhow::Result<BuildListView>;
    fn stats_view(&mut self) -> anyhow::Result<StatsView>;
    fn config_view(&mut self) -> anyhow::Result<ConfigView>;
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
}

/// Processes commands until `Shutdown` or until every sender is gone.
///
/// Each wake-up drains the queue into a batch, applies it, recalculates at most once, and
/// sends one snapshot per affected view, tagged with the batch's last `seq`.
pub fn run_loop(
    backend: &mut dyn Backend,
    commands: &Receiver<(u64, Command)>,
    send: &dyn Fn(Event),
) {
    while let Ok(first) = commands.recv() {
        let mut batch = vec![first];
        while let Ok(next) = commands.try_recv() {
            batch.push(next);
        }
        let mut affected = Affected::default();
        let mut last_seq = 0;
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
            send_snapshots(backend.as_mut(), 0, Affected::ALL_VIEWS, &send);
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
                }
            })
            .collect();
        assert_eq!(kinds, ["build", "list", "stats", "config"]);
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
        assert_eq!(snaps.len(), 2);
        assert!(matches!(snaps[0], (2, Snapshot::Stats(_))));
        assert!(matches!(snaps[1], (2, Snapshot::Config(_))));
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

        // Ready, then four seq-0 snapshots, then the batch's snapshots tagged seq 3.
        assert!(matches!(engine.events.recv().unwrap(), Event::Ready));
        for _ in 0..4 {
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
        // Ready + 4 initial snapshots + Stats + Config; the channel disconnect means the
        // thread (and so every wake) is finished.
        assert_eq!(wakes.load(Ordering::SeqCst), 7);
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
}
