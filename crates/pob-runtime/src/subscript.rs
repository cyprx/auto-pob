use crate::{
    modules,
    net::{CancelFlag, NetConfig, Waker},
    util::{append_lua_package_dir, fix_script_header},
};
use anyhow::{Result, anyhow};
use mlua::{Integer, IntoLuaMulti, Lua, MultiValue, Number, Value};
use std::{
    ops::DerefMut,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// How long to wait for a subscript's next call after replying to one of its blocking calls.
const FOLLOW_UP_WAIT: Duration = Duration::from_millis(1);

/// A unit of work produced by advancing the running subscripts.
pub enum SubscriptEvent {
    /// A subscript requested that a function in the main Lua instance be called on its behalf.
    Call {
        /// Id of the subscript that made the call.
        id: u64,
        function_name: String,
        arguments: NativeMultiValue,
        /// Present for blocking calls. The return values of the callback must be sent back over it.
        reply_tx: Option<Sender<Result<NativeMultiValue>>>,
    },
    /// A subscript finished successfully.
    Finished {
        id: u64,
        return_values: NativeMultiValue,
    },
    /// A subscript errored or panicked.
    Error { id: u64, error: String },
}

#[derive(Default)]
pub struct SubscriptManager {
    next_id: u64,
    scripts: Vec<Subscript>,
    net: NetConfig,
    waker: Option<Waker>,
}

impl SubscriptManager {
    /// A manager whose subscripts use `net` for timeouts and transport.
    pub fn with_config(net: NetConfig) -> SubscriptManager {
        SubscriptManager {
            net,
            ..SubscriptManager::default()
        }
    }

    /// Sets a function called from subscript threads on each call sent and when a thread ends
    /// (also when it panics). Applies to subscripts started afterwards.
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Stops the subscript with the given id: curl and the socket shim notice within a moment
    /// and fail, and the manager forgets the subscript, so its result and calls are discarded.
    /// Returns `false` if it is not running.
    pub fn abort(&mut self, id: u64) -> bool {
        let Some(index) = self.scripts.iter().position(|s| s.id == id) else {
            return false;
        };
        let subscript = self.scripts.remove(index);
        subscript.cancel.store(true, Ordering::Relaxed);
        true
    }

    /// Returns `true` if [`Self::process`] would produce an event: a subscript has finished or
    /// has a call waiting.
    pub fn has_ready(&self) -> bool {
        self.scripts.iter().any(Subscript::is_ready)
    }

    /// Ids of the running subscripts.
    pub fn running_ids(&self) -> Vec<u64> {
        self.scripts.iter().map(|s| s.id).collect()
    }

    /// Starts a new subscript running `script_text` on its own thread and returns the id it was
    /// assigned.
    pub fn push(
        &mut self,
        script_dir: PathBuf,
        script_text: String,
        blocking_calls: Vec<String>,
        nonblocking_calls: Vec<String>,
        arguments: NativeMultiValue,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let subscript = Subscript::new(
            id,
            script_text,
            blocking_calls,
            nonblocking_calls,
            arguments,
            script_dir,
            self.net.clone(),
            self.waker.clone(),
        );
        self.scripts.push(subscript);
        id
    }

    /// Advances all running subscripts by one step.
    pub fn process(&mut self) -> Vec<SubscriptEvent> {
        let mut events = vec![];

        self.scripts.retain_mut(|subscript| {
            // checked before taking calls: a finished subscript can't send more, so draining its
            // queue afterwards delivers every call it made before its result
            let finished = subscript.is_finished();
            subscript.take_call(&mut events);
            if !finished {
                return true;
            }

            while subscript.take_call(&mut events) {}
            events.push(subscript.join());
            // subscript has finished or errored, remove it
            false
        });

        events
    }

    /// Waits up to `timeout` for the subscript with the given id to issue its next call.
    ///
    /// A received call is held back and returned by the next [`Self::process`]. Returns early
    /// if the subscript finishes in the meantime.
    pub fn wait_for_call(&mut self, id: u64, timeout: Duration) {
        if let Some(subscript) = self.scripts.iter_mut().find(|s| s.id == id)
            && subscript.pending_call.is_none()
        {
            subscript.pending_call = subscript.call_rx.recv_timeout(timeout).ok();
            if subscript.pending_call.is_some() {
                subscript.queued.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    /// Returns `true` if at least one subscript is running.
    pub fn has_running_subscripts(&self) -> bool {
        !self.scripts.is_empty()
    }

    /// Returns `true` if the subscript with the given id is running.
    pub fn is_running(&self, subscript_id: u64) -> bool {
        self.scripts
            .iter()
            .any(|subscript| subscript.id == subscript_id)
    }
}

/// A request sent from a subscript's thread to the main thread, asking it to invoke a function
/// defined in the main Lua instance on the subscript's behalf.
enum SubscriptCall {
    Blocking {
        function_name: String,
        arguments: NativeMultiValue,
        // used to send return values of function back to thread
        reply_tx: Sender<Result<NativeMultiValue>>,
    },
    NonBlocking {
        function_name: String,
        arguments: NativeMultiValue,
    },
}

/// Queues `call` for the main thread and wakes it.
fn send_call(
    call_tx: &Sender<SubscriptCall>,
    queued: &AtomicUsize,
    waker: &Option<Waker>,
    call: SubscriptCall,
) -> Result<()> {
    queued.fetch_add(1, Ordering::AcqRel);
    if let Err(e) = call_tx.send(call) {
        queued.fetch_sub(1, Ordering::AcqRel);
        return Err(anyhow!("{e}"));
    }
    if let Some(waker) = waker {
        waker();
    }
    Ok(())
}

/// A Lua script running on its own thread, in a separate `Lua` instance.
pub struct Subscript {
    id: u64,
    handle: Option<JoinHandle<anyhow::Result<NativeMultiValue>>>,
    /// Receives `SubscriptCall`s sent by the proxy functions running in the subscript's s thread.
    call_rx: Receiver<SubscriptCall>,
    /// A call received by [`SubscriptManager::wait_for_call`] that hasn't been processed yet.
    pending_call: Option<SubscriptCall>,
    /// Used for stats logging
    started: Instant,
    calls_serviced: u32,
    /// Set by [`SubscriptManager::abort`]; read by curl's progress callback and the socket shim.
    cancel: Arc<AtomicBool>,
    /// Set by the thread's last act, just before it wakes the main loop.
    done: Arc<AtomicBool>,
    /// Calls sent by the thread and not yet received.
    queued: Arc<AtomicUsize>,
}

/// Marks a subscript thread as done and wakes the main loop, even when the thread panics.
struct ThreadEnd {
    done: Arc<AtomicBool>,
    waker: Option<Waker>,
}

impl Drop for ThreadEnd {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(waker) = &self.waker {
            waker();
        }
    }
}

// Subscripts are lua scripts that are executed in their own instance on a separate thread.
//
// When a subscript needs to call a function defined in the main instance, a `SubscriptCall`
// message is send over a channel. At the beginning of each frame, the main thread checks for
// messages and executes the requested function with the provided arguments on behalf of the
// subscript.  For `BlockingCall`, the subscript waits for the main thread to send the return
// values of the function back over another channel.  For `NonBlockingCall`, the subscript doesn't
// wait on any return values and keeps executing the script after sending the message.  Subscripts
// are required to explicitly specify the names of all (non)-blocking function calls that appear in
// the script.
impl Subscript {
    /// Spawns a new thread, creates a fresh `Lua` instance on it, registers proxy functions for
    /// `blocking_calls` and `nonblocking_calls`, and starts running `script_text` with
    /// `arguments`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u64,
        script_text: String,
        blocking_calls: Vec<String>,
        nonblocking_calls: Vec<String>,
        arguments: NativeMultiValue,
        script_dir: PathBuf,
        net: NetConfig,
        waker: Option<Waker>,
    ) -> Self {
        let (call_tx, call_rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));

        let thread_cancel = cancel.clone();
        let thread_queued = queued.clone();
        let thread_done = done.clone();
        let handle = std::thread::spawn(move || {
            // declared first so it drops last, after the Lua state
            let _end = ThreadEnd {
                done: thread_done,
                waker: waker.clone(),
            };
            profiling::register_thread!(format!("Subscript {id} Thread"));

            // unsafe required to load C modules (curl)
            let lua = unsafe { Lua::unsafe_new() };
            append_lua_package_dir(&lua, &script_dir.join("lua"))?;

            // register modules
            lua.set_app_data(net);
            lua.set_app_data(CancelFlag(thread_cancel));
            modules::register(&lua)?;

            // register a proxy for each allowed blocking call.
            // forwards the call to the main thread and blocks on `reply_rx` until the main thread
            // sends back the return values
            for function_name in blocking_calls {
                let call_tx = call_tx.clone();
                let queued = thread_queued.clone();
                let waker = waker.clone();
                lua.globals().set(
                    function_name.clone(),
                    lua.create_function(move |_, args: MultiValue| {
                        let (reply_tx, reply_rx) = channel();
                        send_call(
                            &call_tx,
                            &queued,
                            &waker,
                            SubscriptCall::Blocking {
                                function_name: function_name.clone(),
                                arguments: args.try_into()?,
                                reply_tx,
                            },
                        )?;
                        let return_values = reply_rx.recv().map_err(|e| anyhow!("{e}"))??;
                        Ok(return_values)
                    })?,
                )?;
            }

            // register a proxy for each allowed non-blocking call.
            // forwards the call to the main thread and returns immediately
            for function_name in nonblocking_calls {
                let call_tx = call_tx.clone();
                let queued = thread_queued.clone();
                let waker = waker.clone();
                lua.globals().set(
                    function_name.clone(),
                    lua.create_function(move |_, args: MultiValue| {
                        send_call(
                            &call_tx,
                            &queued,
                            &waker,
                            SubscriptCall::NonBlocking {
                                function_name: function_name.clone(),
                                arguments: args.try_into()?,
                            },
                        )?;
                        Ok(())
                    })?,
                )?;
            }

            let mut script_text = script_text.into_bytes();
            fix_script_header(&mut script_text);
            let result = lua.load(script_text).call::<MultiValue>(arguments)?;
            result.try_into()
        });

        Self {
            id,
            handle: Some(handle),
            call_rx,
            pending_call: None,
            started: Instant::now(),
            calls_serviced: 0,
            cancel,
            done,
            queued,
        }
    }

    /// Receives the next queued call, if any.
    fn try_recv_call(&self) -> Option<SubscriptCall> {
        let call = self.call_rx.try_recv().ok()?;
        self.queued.fetch_sub(1, Ordering::AcqRel);
        Some(call)
    }

    /// A finished thread, or a call waiting to be taken.
    fn is_ready(&self) -> bool {
        self.is_finished() || self.pending_call.is_some() || self.queued.load(Ordering::Acquire) > 0
    }

    /// Collects a pending `SubscriptCall` into `events`.
    ///
    /// Returns `false` if there was no pending call.
    fn take_call(&mut self, events: &mut Vec<SubscriptEvent>) -> bool {
        let Some(call) = self.pending_call.take().or_else(|| self.try_recv_call()) else {
            // no pending call, or thread disconnected which is handled on join
            return false;
        };
        self.calls_serviced += 1;

        events.push(match call {
            SubscriptCall::Blocking {
                function_name,
                arguments,
                reply_tx,
            } => SubscriptEvent::Call {
                id: self.id,
                function_name,
                arguments,
                reply_tx: Some(reply_tx),
            },
            SubscriptCall::NonBlocking {
                function_name,
                arguments,
            } => SubscriptEvent::Call {
                id: self.id,
                function_name,
                arguments,
                reply_tx: None,
            },
        });
        true
    }

    fn is_finished(&self) -> bool {
        self.done.load(Ordering::Acquire)
            || self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Joins the subscript's finished thread and returns the resulting `SubscriptEvent`.
    fn join(&mut self) -> SubscriptEvent {
        log::info!(
            target: crate::STATS_TARGET,
            "subscript {} finished in {:.1}ms after {} main-thread calls",
            self.id,
            self.started.elapsed().as_secs_f64() * 1000.0,
            self.calls_serviced,
        );

        match self.handle.take().unwrap().join() {
            Ok(Ok(return_values)) => SubscriptEvent::Finished {
                id: self.id,
                return_values,
            },
            Ok(Err(err)) => SubscriptEvent::Error {
                id: self.id,
                error: err.to_string(),
            },
            // the thread panicked
            Err(_) => SubscriptEvent::Error {
                id: self.id,
                error: String::from("Subscript thread panicked!"),
            },
        }
    }
}

/// Advances subscripts and passes their events to `handle` until no more work is immediately
/// available or `budget` is spent.
///
/// A subscript making a blocking call can't issue its next call until it receives the reply, so
/// polling once per frame would limit it to one blocking call per frame. Instead, after a
/// blocking call is handled, this briefly waits for the subscript's follow-up call and services
/// it in the same frame.
///
/// `manager` is called to borrow the manager for each step and the borrow is released before
/// `handle` runs, so `handle` may access the manager itself (e.g. to launch another subscript).
pub fn service_subscripts<D, E>(
    mut manager: impl FnMut() -> D,
    budget: Duration,
    mut handle: impl FnMut(SubscriptEvent) -> Result<(), E>,
) -> Result<(), E>
where
    D: DerefMut<Target = SubscriptManager>,
{
    let deadline = Instant::now() + budget;

    loop {
        let events = manager().process();
        if events.is_empty() {
            return Ok(());
        }

        let mut awaiting_follow_up = Vec::new();
        for event in events {
            if let SubscriptEvent::Call {
                id,
                reply_tx: Some(_),
                ..
            } = &event
            {
                awaiting_follow_up.push(*id);
            }
            handle(event)?;
        }

        for id in awaiting_follow_up {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            manager().wait_for_call(id, FOLLOW_UP_WAIT.min(remaining));
        }

        if Instant::now() >= deadline {
            return Ok(());
        }
    }
}

/// Instance-independent stand-in for `mlua::Value`.
///
/// Values produced by one Lua instance (in particular strings) cannot be used directly with
/// another Lua instance. `NativeValue` acts as a stand-in to freely move between the main instance
/// and a subscript's instance. It only covers the types that are actually exchanged, namely `nil`,
/// booleans, numbers, and string.
#[derive(Debug)]
pub enum NativeValue {
    Nil,
    Number(Number),
    Integer(Integer),
    Boolean(bool),
    // Lua strings may not be valid UTF-8, so store as raw bytes instead of String
    String(Vec<u8>),
}

/// Instance-independent stand-in for `mlua::MultiValue`.
///
/// Used to pass function arguments and return values between the main Lua instance and a
/// subscript's Lua instance.
#[derive(Debug)]
pub struct NativeMultiValue(Vec<NativeValue>);

impl NativeMultiValue {
    pub fn new(values: Vec<NativeValue>) -> Self {
        Self(values)
    }

    pub fn values(&self) -> &[NativeValue] {
        &self.0
    }
}

impl TryFrom<Value> for NativeValue {
    type Error = anyhow::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Nil => Ok(Self::Nil),
            Value::Boolean(b) => Ok(Self::Boolean(b)),
            Value::Integer(i) => Ok(Self::Integer(i)),
            Value::Number(n) => Ok(Self::Number(n)),
            Value::String(s) => Ok(Self::String(s.as_bytes().to_vec())),
            other => Err(anyhow!("Unsupported value type: {}", other.type_name())),
        }
    }
}

impl TryFrom<MultiValue> for NativeMultiValue {
    type Error = anyhow::Error;

    fn try_from(values: MultiValue) -> Result<Self, Self::Error> {
        values
            .into_iter()
            .map(NativeValue::try_from)
            .collect::<Result<_, _>>()
            .map(Self)
    }
}

impl IntoLuaMulti for NativeMultiValue {
    fn into_lua_multi(self, lua: &Lua) -> mlua::Result<MultiValue> {
        self.0
            .into_iter()
            .map(|v| match v {
                NativeValue::Nil => Ok(Value::Nil),
                NativeValue::Boolean(b) => Ok(Value::Boolean(b)),
                NativeValue::Integer(i) => Ok(Value::Integer(i)),
                NativeValue::Number(n) => Ok(Value::Number(n)),
                NativeValue::String(s) => lua.create_string(s).map(Value::String),
            })
            .collect::<mlua::Result<Vec<_>>>()
            .map(MultiValue::from_vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const CALLS: u32 = 200;
    const FRAME_TIME: Duration = Duration::from_millis(2);

    /// Runs a subscript making `CALLS` blocking calls and returns how many frames it took to
    /// finish when servicing it with the given per-frame budget.
    fn frames_to_finish(budget: Duration) -> u32 {
        let manager = RefCell::new(SubscriptManager::default());
        manager.borrow_mut().push(
            std::env::temp_dir(),
            format!("for i = 1, {CALLS} do assert(Echo(i) == i) end"),
            vec!["Echo".into()],
            vec![],
            NativeMultiValue(vec![]),
        );

        let mut frames = 0;
        let mut finished = false;
        while !finished {
            frames += 1;
            assert!(frames < 100_000, "subscript did not finish");
            service_subscripts(
                || manager.borrow_mut(),
                budget,
                |event| -> Result<()> {
                    match event {
                        SubscriptEvent::Call {
                            arguments,
                            reply_tx,
                            ..
                        } => {
                            reply_tx.unwrap().send(Ok(arguments)).unwrap();
                        }
                        SubscriptEvent::Finished { .. } => finished = true,
                        SubscriptEvent::Error { error, .. } => panic!("{error}"),
                    }
                    Ok(())
                },
            )
            .unwrap();
            // the rest of the frame
            std::thread::sleep(FRAME_TIME);
        }
        frames
    }

    #[test]
    fn blocking_calls_are_serviced_within_a_frame() {
        // a zero budget services one call per subscript per frame
        let unbudgeted = frames_to_finish(Duration::ZERO);
        let budgeted = frames_to_finish(Duration::from_millis(4));
        eprintln!(
            "{CALLS} blocking calls: {unbudgeted} frames (no budget) vs {budgeted} frames (4ms budget)"
        );

        assert!(unbudgeted >= CALLS);
        assert!(budgeted < CALLS / 4);
    }

    /// Readable form of values passed between instances, e.g. `nil, true, 42, "text"`.
    fn describe(values: &NativeMultiValue) -> String {
        let parts: Vec<String> = values
            .0
            .iter()
            .map(|v| match v {
                NativeValue::Nil => "nil".into(),
                NativeValue::Boolean(b) => b.to_string(),
                NativeValue::Integer(i) => i.to_string(),
                NativeValue::Number(n) => n.to_string(),
                NativeValue::String(s) => format!("{:?}", String::from_utf8_lossy(s)),
            })
            .collect();
        parts.join(", ")
    }

    fn launch(
        manager: &RefCell<SubscriptManager>,
        script: &str,
        blocking: &[&str],
        nonblocking: &[&str],
        arguments: Vec<NativeValue>,
    ) -> u64 {
        manager.borrow_mut().push(
            std::env::temp_dir(),
            script.into(),
            blocking.iter().map(|s| s.to_string()).collect(),
            nonblocking.iter().map(|s| s.to_string()).collect(),
            NativeMultiValue(arguments),
        )
    }

    /// Services subscripts like the main loop does until none are running. Blocking calls are
    /// answered by `reply`. Returns a log line per event, e.g. `call 0 Echo(1)`, `finished 0: 1`.
    fn run_to_completion(
        manager: &RefCell<SubscriptManager>,
        mut reply: impl FnMut(&str, NativeMultiValue) -> Result<NativeMultiValue>,
    ) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut log = Vec::new();
        while manager.borrow().has_running_subscripts() {
            assert!(
                Instant::now() < deadline,
                "subscripts did not finish: {log:?}"
            );
            service_subscripts(
                || manager.borrow_mut(),
                Duration::from_millis(4),
                |event| -> Result<()> {
                    match event {
                        SubscriptEvent::Call {
                            id,
                            function_name,
                            arguments,
                            reply_tx,
                        } => {
                            log.push(format!(
                                "call {id} {function_name}({})",
                                describe(&arguments)
                            ));
                            if let Some(reply_tx) = reply_tx {
                                reply_tx.send(reply(&function_name, arguments)).unwrap();
                            }
                        }
                        SubscriptEvent::Finished { id, return_values } => {
                            log.push(format!("finished {id}: {}", describe(&return_values)));
                        }
                        SubscriptEvent::Error { id, error } => {
                            log.push(format!("error {id}: {error}"));
                        }
                    }
                    Ok(())
                },
            )
            .unwrap();
            std::thread::sleep(FRAME_TIME);
        }
        log
    }

    fn echo(_: &str, arguments: NativeMultiValue) -> Result<NativeMultiValue> {
        Ok(arguments)
    }

    #[test]
    fn values_round_trip_between_instances() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(
            &manager,
            r#"
            local a, b, c, d, e = Echo(nil, true, 42, 1.5, "bin\0\255")
            return a == nil, b == true, c == 42, d == 1.5, e == "bin\0\255"
            "#,
            &["Echo"],
            &[],
            vec![],
        );
        let log = run_to_completion(&manager, echo);
        assert_eq!(
            log.last().unwrap(),
            "finished 0: true, true, true, true, true"
        );
    }

    #[test]
    fn arguments_and_return_values_reach_the_main_instance() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(
            &manager,
            "local a, b = ... return a .. b, #b",
            &[],
            &[],
            vec![
                NativeValue::String(b"x".to_vec()),
                NativeValue::String(b"yz".to_vec()),
            ],
        );
        assert_eq!(
            run_to_completion(&manager, echo),
            ["finished 0: \"xyz\", 2"]
        );
    }

    #[test]
    fn unsupported_argument_types_are_lua_errors() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(
            &manager,
            "local ok, err = pcall(Echo, {}) return ok, tostring(err)",
            &["Echo"],
            &[],
            vec![],
        );
        let log = run_to_completion(&manager, echo);
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].starts_with("finished 0: false"), "{log:?}");
        assert!(log[0].contains("Unsupported value type: table"), "{log:?}");
    }

    #[test]
    fn main_thread_errors_are_raised_in_the_subscript() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(
            &manager,
            "local ok, err = pcall(Fail) return ok, tostring(err)",
            &["Fail"],
            &[],
            vec![],
        );
        let log = run_to_completion(&manager, |_, _| Err(anyhow!("main says no")));
        assert!(log[1].starts_with("finished 0: false"), "{log:?}");
        assert!(log[1].contains("main says no"), "{log:?}");
    }

    #[test]
    fn uncaught_errors_become_error_events() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(&manager, r#"error("kaboom")"#, &[], &[], vec![]);
        launch(&manager, "Undeclared()", &["Echo"], &[], vec![]);
        launch(&manager, "this is not lua", &[], &[], vec![]);
        let mut log = run_to_completion(&manager, echo);
        log.sort();
        assert_eq!(log.len(), 3, "{log:?}");
        assert!(
            log[0].starts_with("error 0:") && log[0].contains("kaboom"),
            "{log:?}"
        );
        assert!(
            log[1].starts_with("error 1:") && log[1].contains("Undeclared"),
            "{log:?}"
        );
        assert!(
            log[2].starts_with("error 2:") && log[2].contains("syntax error"),
            "{log:?}"
        );
    }

    #[test]
    fn nonblocking_calls_arrive_in_order() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(
            &manager,
            r#"for i = 1, 50 do Notify(i) end return "done""#,
            &[],
            &["Notify"],
            vec![],
        );
        let log = run_to_completion(&manager, |name, _| {
            panic!("unexpected blocking call {name}")
        });
        let mut expected: Vec<String> = (1..=50).map(|i| format!("call 0 Notify({i})")).collect();
        expected.push("finished 0: \"done\"".into());
        assert_eq!(log, expected);
    }

    #[test]
    fn concurrent_subscripts_finish_independently() {
        let manager = RefCell::new(SubscriptManager::default());
        let ids: Vec<u64> = (0..8)
            .map(|n| {
                launch(
                    &manager,
                    "local n = ... for i = 1, 20 do Echo(i) end return Echo(n) * 2",
                    &["Echo"],
                    &[],
                    vec![NativeValue::Integer(n)],
                )
            })
            .collect();
        assert_eq!(ids, (0..8).collect::<Vec<_>>());
        assert!(ids.iter().all(|&id| manager.borrow().is_running(id)));

        let log = run_to_completion(&manager, echo);
        let mut finished: Vec<_> = log.iter().filter(|l| l.starts_with("finished")).collect();
        finished.sort();
        let expected: Vec<String> = (0..8).map(|n| format!("finished {n}: {}", n * 2)).collect();
        assert_eq!(finished, expected.iter().collect::<Vec<_>>());
        assert!(!ids.iter().any(|&id| manager.borrow().is_running(id)));
    }

    #[test]
    fn script_header_is_accepted() {
        let manager = RefCell::new(SubscriptManager::default());
        launch(&manager, "#@ SimpleGraphic\nreturn 1", &[], &[], vec![]);
        assert_eq!(run_to_completion(&manager, echo), ["finished 0: 1"]);
    }

    /// Drops the manager while `script` is running and returns how its thread ended.
    fn result_after_dropping_manager(script: &str, wait_for_call: bool) -> String {
        let mut manager = SubscriptManager::default();
        let id = manager.push(
            std::env::temp_dir(),
            script.into(),
            vec!["Echo".into()],
            vec![],
            NativeMultiValue(vec![]),
        );
        if wait_for_call {
            manager.wait_for_call(id, Duration::from_secs(5));
            assert!(manager.scripts[0].pending_call.is_some());
        }
        let handle = manager.scripts[0].handle.take().unwrap();
        drop(manager);

        let deadline = Instant::now() + Duration::from_secs(5);
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "subscript thread hung after shutdown"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        match handle.join() {
            Ok(Ok(values)) => format!("finished: {}", describe(&values)),
            Ok(Err(err)) => format!("error: {err}"),
            Err(_) => "panicked".into(),
        }
    }

    #[test]
    fn shutdown_unblocks_a_subscript_waiting_for_a_reply() {
        let result = result_after_dropping_manager(r#"Echo(1) return "unreachable""#, true);
        assert!(result.starts_with("error:"), "{result}");
    }

    #[test]
    fn shutdown_before_a_blocking_call_ends_the_subscript_cleanly() {
        let script = r#"
            local start = os.clock()
            while os.clock() - start < 0.05 do end
            Echo(1)
            return "unreachable"
        "#;
        let result = result_after_dropping_manager(script, false);
        assert!(result.starts_with("error:"), "{result}");
    }
}
