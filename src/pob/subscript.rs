use crate::{
    pob::modules,
    util::{append_lua_package_dir, fix_script_header},
};
use anyhow::{Result, anyhow};
use mlua::{Integer, IntoLuaMulti, Lua, MultiValue, Number, Value};
use std::{
    ops::DerefMut,
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, channel},
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
}

impl SubscriptManager {
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
        );
        self.scripts.push(subscript);
        id
    }

    /// Advances all running subscripts by one step.
    pub fn process(&mut self) -> Vec<SubscriptEvent> {
        let mut events = vec![];

        self.scripts.retain_mut(|subscript| {
            subscript.take_call(&mut events);

            subscript.try_join().is_none_or(|event| {
                events.push(event);
                // subscript has finished or errored, remove it
                false
            })
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
    pub fn new(
        id: u64,
        script_text: String,
        blocking_calls: Vec<String>,
        nonblocking_calls: Vec<String>,
        arguments: NativeMultiValue,
        script_dir: PathBuf,
    ) -> Self {
        let (call_tx, call_rx) = channel();

        let handle = std::thread::spawn(move || {
            profiling::register_thread!(format!("Subscript {id} Thread"));

            // unsafe required to load C modules (curl)
            let lua = unsafe { Lua::unsafe_new() };
            append_lua_package_dir(&lua, &script_dir.join("lua"))?;

            // register modules
            modules::register(&lua)?;

            // register a proxy for each allowed blocking call.
            // forwards the call to the main thread and blocks on `reply_rx` until the main thread
            // sends back the return values
            for function_name in blocking_calls {
                let call_tx = call_tx.clone();
                lua.globals().set(
                    function_name.clone(),
                    lua.create_function(move |_, args: MultiValue| {
                        let (reply_tx, reply_rx) = channel();
                        call_tx
                            .send(SubscriptCall::Blocking {
                                function_name: function_name.clone(),
                                arguments: args.try_into()?,
                                reply_tx,
                            })
                            .unwrap();
                        let return_values = reply_rx.recv().map_err(|e| anyhow!("{e}"))??;
                        Ok(return_values)
                    })?,
                )?;
            }

            // register a proxy for each allowed non-blocking call.
            // forwards the call to the main thread and returns immediately
            for function_name in nonblocking_calls {
                let call_tx = call_tx.clone();
                lua.globals().set(
                    function_name.clone(),
                    lua.create_function(move |_, args: MultiValue| {
                        call_tx
                            .send(SubscriptCall::NonBlocking {
                                function_name: function_name.clone(),
                                arguments: args.try_into()?,
                            })
                            .map_err(|e| anyhow!("{e}"))?;
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
        }
    }

    /// Collects a pending `SubscriptCall` into `events`.
    ///
    /// Does nothing if there is no pending call.
    fn take_call(&mut self, events: &mut Vec<SubscriptEvent>) {
        let Some(call) = self
            .pending_call
            .take()
            .or_else(|| self.call_rx.try_recv().ok())
        else {
            // no pending call, or thread disconnected which is handled on join
            return;
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
    }

    /// If the subscript's thread has finished, joins it and returns the resulting
    /// `SubscriptEvent`.
    fn try_join(&mut self) -> Option<SubscriptEvent> {
        if !self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            return None;
        }

        log::info!(
            target: crate::stats::TARGET,
            "subscript {} finished in {:.1}ms after {} main-thread calls",
            self.id,
            self.started.elapsed().as_secs_f64() * 1000.0,
            self.calls_serviced,
        );

        let event = match self.handle.take().unwrap().join() {
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
        };
        Some(event)
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
}
