//! PoB's subscript natives on the engine's main Lua state, gated so that nothing reaches the
//! network unless the user asked (R4), and the servicing of finished subscripts and their
//! calls (R9).
//!
//! `LaunchSubScript` launches only while a request scope is open (`__net_scope`, raised by
//! `glue/net.lua` around a press and around each callback it dispatches) and no `Build:Init`
//! runs (`__net_in_init`). Anything else is refused: it returns nil, as the old stub did, and
//! is logged.

use anyhow::anyhow;
use mlua::{Lua, MultiValue, Value};
use pob_runtime::net::{NetConfig, Waker};
use pob_runtime::subscript::{
    NativeMultiValue, SubscriptEvent, SubscriptManager, service_subscripts,
};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// Logged for every launch outside a request.
pub const REFUSED: &str = "subscript refused outside a request";

/// How long one service may keep the engine thread; blocking calls are answered within it.
const SERVICE_BUDGET: Duration = Duration::from_millis(50);

/// The launch gate (R4).
#[derive(Default)]
struct Gate {
    /// Open request scopes; launches need at least one
    scope: Cell<i32>,
    /// A `Build:Init` is running; its `ImportTab` would validate a stored token
    in_init: Cell<i32>,
    /// Launches refused so far
    refused: Cell<usize>,
}

/// The subscripts of one Lua state and its launch gate.
#[derive(Clone)]
pub struct NetHandle {
    manager: Rc<RefCell<SubscriptManager>>,
    gate: Rc<Gate>,
}

/// Where finished subscripts and their calls go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// Through `glue.net_dispatch`, which guards popups and `promptMsg` and keeps the requests
    Glue,
    /// Straight to PoB's `launch:OnSub*`, as PoB's own frame loop does (upstream)
    Launch,
}

fn split_names(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Installs `net` on `lua` (lcurl fails closed without it), and registers `LaunchSubScript`,
/// `IsSubScriptRunning`, `AbortSubScript`, `__net_scope(delta)` and `__net_in_init(on)`.
/// Subscripts run with `pob_dir` as their script folder.
pub fn register(lua: &Lua, pob_dir: &Path, net: NetConfig) -> mlua::Result<NetHandle> {
    lua.set_app_data(net.clone());
    let handle = NetHandle {
        manager: Rc::new(RefCell::new(SubscriptManager::with_config(net))),
        gate: Rc::default(),
    };
    let g = lua.globals();

    let manager = handle.manager.clone();
    let gate = handle.gate.clone();
    let dir = pob_dir.to_owned();
    g.set(
        "LaunchSubScript",
        lua.create_function(
            move |_, (script, blocking, nonblocking, args): (String, String, String, MultiValue)| {
                if gate.scope.get() <= 0 || gate.in_init.get() > 0 {
                    gate.refused.set(gate.refused.get() + 1);
                    log::warn!("{REFUSED}");
                    return Ok(None);
                }
                let arguments = NativeMultiValue::try_from(args).map_err(mlua::Error::external)?;
                let id = manager.borrow_mut().push(
                    dir.clone(),
                    script,
                    split_names(&blocking),
                    split_names(&nonblocking),
                    arguments,
                );
                Ok(Some(id))
            },
        )?,
    )?;

    let manager = handle.manager.clone();
    g.set(
        "IsSubScriptRunning",
        lua.create_function(move |_, id: u64| Ok(manager.borrow().is_running(id)))?,
    )?;

    let manager = handle.manager.clone();
    g.set(
        "AbortSubScript",
        lua.create_function(move |_, id: u64| {
            manager.borrow_mut().abort(id);
            Ok(())
        })?,
    )?;

    let gate = handle.gate.clone();
    g.set(
        "__net_scope",
        lua.create_function(move |_, delta: i32| {
            gate.scope.set(gate.scope.get() + delta);
            Ok(())
        })?,
    )?;

    g.set(
        "__net_redact",
        lua.create_function(|_, text: String| Ok(super::native::redact(&text)))?,
    )?;

    let gate = handle.gate.clone();
    g.set(
        "__net_in_init",
        lua.create_function(move |_, on: bool| {
            // a depth, not a flag: a nested pair must not reopen the gate for the outer one
            gate.in_init
                .set((gate.in_init.get() + if on { 1 } else { -1 }).max(0));
            Ok(())
        })?,
    )?;

    Ok(handle)
}

/// Calls `glue.<name>(args)`.
fn call_glue(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) -> mlua::Result<MultiValue> {
    let glue: mlua::Table = lua.globals().get("glue")?;
    let function: mlua::Function = glue.get(name)?;
    function.call(args)
}

/// Calls PoB's `runCallback(name, args)`.
fn run_callback(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) -> mlua::Result<MultiValue> {
    let run: mlua::Function = lua.globals().get("runCallback")?;
    let mut all = MultiValue::new();
    all.push_back(Value::String(lua.create_string(name)?));
    all.extend(args.into_lua_multi(lua)?);
    run.call(all)
}

/// The first line of an error, without the position prefix mlua adds for runtime errors.
fn message_of(error: &mlua::Error) -> String {
    let text = match error {
        mlua::Error::RuntimeError(message) => message.clone(),
        mlua::Error::CallbackError { cause, .. } => return message_of(cause),
        other => other.to_string(),
    };
    text.lines().next().unwrap_or_default().to_string()
}

impl NetHandle {
    /// Delivers what the subscripts have ready: finished ones and their calls, for up to
    /// [`SERVICE_BUDGET`]. Returns how many events were handled.
    ///
    /// With [`Dispatch::Glue`] a failure inside one event finishes that event's request with the
    /// error (`glue.net_fail`) and the rest are still delivered; it never returns `Err` for one.
    pub fn service(&self, lua: &Lua, how: Dispatch) -> anyhow::Result<usize> {
        let mut handled = 0;
        service_subscripts(
            || self.manager.borrow_mut(),
            SERVICE_BUDGET,
            |event| -> anyhow::Result<()> {
                handled += 1;
                match how {
                    Dispatch::Glue => {
                        dispatch_glue(lua, event);
                        Ok(())
                    }
                    Dispatch::Launch => {
                        dispatch_launch(lua, event).map_err(|e| anyhow!("{}", message_of(&e)))
                    }
                }
            },
        )?;
        Ok(handled)
    }

    /// Whether a subscript has finished or has a call waiting (R9).
    pub fn has_ready(&self) -> bool {
        self.manager.borrow().has_ready()
    }

    /// Aborts a subscript (R6); false when it isn't running.
    pub fn abort(&self, id: u64) -> bool {
        self.manager.borrow_mut().abort(id)
    }

    pub fn set_waker(&self, wake: Waker) {
        self.manager.borrow_mut().set_waker(wake);
    }

    /// Ids of the running subscripts.
    pub fn running_ids(&self) -> Vec<u64> {
        self.manager.borrow().running_ids()
    }

    /// How many launches the gate refused so far.
    pub fn refused(&self) -> usize {
        self.gate.refused.get()
    }
}

/// One event through `glue.net_dispatch`. An error is the request's failure, not the step's.
fn dispatch_glue(lua: &Lua, event: SubscriptEvent) {
    let id = event.id();
    let failure = match event {
        SubscriptEvent::Call {
            id,
            function_name,
            arguments,
            reply_tx,
        } => {
            let result = call_glue(lua, "net_dispatch", ("call", id, function_name, arguments));
            let failure = result.as_ref().err().map(message_of);
            // a non-blocking call has no reply channel; a subscript that went away can't take
            // its reply
            if let Some(reply_tx) = reply_tx {
                let _ = reply_tx.send(
                    result
                        .map_err(|e| anyhow!("{}", message_of(&e)))
                        .and_then(NativeMultiValue::try_from),
                );
            }
            failure
        }
        SubscriptEvent::Finished { id, return_values } => {
            call_glue(lua, "net_dispatch", ("finished", id, return_values))
                .err()
                .map(|e| message_of(&e))
        }
        SubscriptEvent::Error { id, error } => {
            // the thread's error comes as mlua shows it: a "runtime error: " prefix, then a
            // traceback
            let first = error.lines().next().unwrap_or_default();
            let error = first.strip_prefix("runtime error: ").unwrap_or(first);
            call_glue(lua, "net_dispatch", ("error", id, error))
                .err()
                .map(|e| message_of(&e))
        }
    };
    // a callback that made a new build cancels the other requests now, before the rest of
    // the batch is delivered: their events are discarded with their aborted subscripts (R13)
    if let Err(e) = call_glue(lua, "net_cancel_replaced", ()) {
        log::warn!(
            "cancelling for the new build failed: {}",
            super::native::redact(&message_of(&e))
        );
    }
    if let Some(message) = failure {
        // the message becomes the request's error, which the UI shows: no URL or name in it
        let message = super::native::redact(&message);
        log::warn!("network callback failed: {message}");
        if let Err(e) = call_glue(lua, "net_fail", (id, message)) {
            log::warn!(
                "ending the request failed too: {}",
                super::native::redact(&message_of(&e))
            );
        }
    }
}

/// One event straight to PoB's handlers, as its own frame loop delivers them.
fn dispatch_launch(lua: &Lua, event: SubscriptEvent) -> mlua::Result<()> {
    match event {
        SubscriptEvent::Call {
            function_name,
            arguments,
            reply_tx,
            ..
        } => {
            let result = run_callback(lua, "OnSubCall", (function_name, arguments));
            if let Some(reply_tx) = reply_tx {
                let reply = match &result {
                    Ok(values) => NativeMultiValue::try_from(values.clone()),
                    Err(e) => Err(anyhow!("{}", message_of(e))),
                };
                let _ = reply_tx.send(reply);
            }
            result.map(|_| ())
        }
        SubscriptEvent::Finished { id, return_values } => {
            run_callback(lua, "OnSubFinished", (id, return_values)).map(|_| ())
        }
        SubscriptEvent::Error { id, error } => {
            run_callback(lua, "OnSubError", (id, error)).map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    fn state() -> (Lua, NetHandle) {
        let lua = Lua::new();
        let net = register(&lua, &std::env::temp_dir(), NetConfig::default()).unwrap();
        (lua, net)
    }

    fn launch(lua: &Lua, script: &str) -> Option<u64> {
        let f: mlua::Function = lua.globals().get("LaunchSubScript").unwrap();
        f.call((script, "", "ConPrintf")).unwrap()
    }

    fn exec(lua: &Lua, code: &str) {
        lua.load(code).exec().unwrap();
    }

    /// Services until `n` events were handled.
    fn deliver(lua: &Lua, net: &NetHandle, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut handled = 0;
        while handled < n {
            assert!(
                Instant::now() < deadline,
                "only {handled} of {n} events came"
            );
            if net.has_ready() {
                handled += net.service(lua, Dispatch::Launch).unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    #[test]
    fn a_failed_callback_ends_its_request_with_a_redacted_message() {
        let (lua, _net) = state();
        exec(
            &lua,
            r#"glue = {
                net_dispatch = function()
                    error("bad reply from https://pobb.in/abc for /character/Bob", 0)
                end,
                net_fail = function(id, message) failed = { id = id, message = message } end,
            }"#,
        );
        dispatch_glue(
            &lua,
            SubscriptEvent::Error {
                id: 7,
                error: "boom".into(),
            },
        );
        let (id, message): (u64, String) =
            lua.load("return failed.id, failed.message").eval().unwrap();
        assert_eq!(id, 7);
        assert!(
            !message.contains("pobb.in") && !message.contains("Bob"),
            "{message}"
        );
        assert!(message.contains("<redacted>"), "{message}");
    }

    #[test]
    fn launches_only_inside_a_scope_and_outside_init() {
        let (lua, net) = state();
        assert_eq!(launch(&lua, "return 1"), None, "launched outside a scope");
        assert_eq!(net.refused(), 1);
        exec(&lua, "__net_scope(1)");
        assert!(launch(&lua, "return 1").is_some());
        exec(&lua, "__net_in_init(true)");
        assert_eq!(launch(&lua, "return 1"), None, "launched during Build:Init");
        exec(&lua, "__net_in_init(true)");
        exec(&lua, "__net_in_init(false)");
        assert_eq!(
            launch(&lua, "return 1"),
            None,
            "a nested pair reopened the gate"
        );
        exec(&lua, "__net_in_init(false)");
        assert!(launch(&lua, "return 1").is_some());
        exec(&lua, "__net_scope(-1)");
        assert_eq!(launch(&lua, "return 1"), None);
        assert_eq!(net.refused(), 4);
    }

    #[test]
    fn launch_dispatch_reaches_pob_handlers() {
        let (lua, net) = state();
        exec(
            &lua,
            r##"seen = {}
            function runCallback(name, ...)
                local parts = { name }
                for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
                seen[#seen + 1] = table.concat(parts, " ")
            end
            __net_scope(1)"##,
        );
        let ok = launch(&lua, "ConPrintf('hi %d', 7) return 'body', nil, 'head'").unwrap();
        deliver(&lua, &net, 2);
        let failed = launch(&lua, "error('boom', 0)").unwrap();
        deliver(&lua, &net, 1);
        let seen: Vec<String> = lua.load("return seen").eval().unwrap();
        assert_eq!(
            seen[..2],
            [
                "OnSubCall ConPrintf hi %d 7".to_string(),
                format!("OnSubFinished {ok} body nil head"),
            ]
        );
        // upstream gets the error as the classic runtime hands it over
        let error_prefix = format!("OnSubError {failed} runtime error: boom\n");
        assert!(seen[2].starts_with(&error_prefix), "{:?}", seen[2]);
        assert_eq!(seen.len(), 3);
        assert!(!net.has_ready());
    }

    #[test]
    fn abort_drops_the_result_and_the_waker_wakes() {
        let (lua, net) = state();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        net.set_waker(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        exec(
            &lua,
            "__net_scope(1) function runCallback() error('delivered') end",
        );
        let id = launch(
            &lua,
            "local t = os.clock() while os.clock() - t < 0.2 do end return 1",
        )
        .unwrap();
        let running: bool = lua
            .load(format!("return IsSubScriptRunning({id})"))
            .eval()
            .unwrap();
        assert!(running);
        exec(&lua, &format!("AbortSubScript({id})"));
        assert!(net.running_ids().is_empty());
        // the thread still ends, and wakes the loop, but nothing is delivered
        let deadline = Instant::now() + Duration::from_secs(5);
        while wakes.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "no wake");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!net.has_ready());
        assert_eq!(net.service(&lua, Dispatch::Launch).unwrap(), 0);
    }
}
