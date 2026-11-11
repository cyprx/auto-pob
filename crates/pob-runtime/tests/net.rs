//! The runtime's network layer: canned transport, timeouts, abort and the waker.
//!
//! Nothing here leaves the machine: the curl tests talk to a listener the test starts on
//! 127.0.0.1, everything else uses the canned transport.

use pob_runtime::{
    net::{Canned, CannedWeb, NetConfig, Recorded, Transport, Waker},
    subscript::{
        NativeMultiValue, NativeValue, SubscriptEvent, SubscriptManager, service_subscripts,
    },
};
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn canned(web: &Arc<CannedWeb>, timeout: Option<Duration>) -> SubscriptManager {
    SubscriptManager::with_config(NetConfig {
        connect_timeout: None,
        timeout,
        transport: Transport::Canned(web.clone()),
    })
}

fn counting_waker(manager: &mut SubscriptManager) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let waker: Waker = Arc::new(move || {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    manager.set_waker(waker);
    count
}

fn launch(manager: &mut SubscriptManager, script: &str, nonblocking: &[&str]) -> u64 {
    manager.push(
        std::env::temp_dir(),
        script.to_string(),
        vec![],
        nonblocking.iter().map(|s| s.to_string()).collect(),
        NativeMultiValue::new(vec![]),
    )
}

/// What a finished subscript produced, with the non-blocking calls it made on the way.
struct Outcome {
    calls: Vec<(String, Vec<NativeValue>)>,
    result: Result<Vec<NativeValue>, String>,
}

/// Services `manager` until a subscript finishes or `within` passes.
fn wait_outcome(manager: &mut SubscriptManager, within: Duration) -> Option<Outcome> {
    let deadline = Instant::now() + within;
    let mut calls = vec![];
    while Instant::now() < deadline {
        for event in manager.process() {
            match event {
                SubscriptEvent::Call {
                    function_name,
                    arguments,
                    ..
                } => calls.push((function_name, arguments.values().to_vec_owned())),
                SubscriptEvent::Finished { return_values, .. } => {
                    return Some(Outcome {
                        calls,
                        result: Ok(return_values.values().to_vec_owned()),
                    });
                }
                SubscriptEvent::Error { error, .. } => {
                    return Some(Outcome {
                        calls,
                        result: Err(error),
                    });
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

trait ToVecOwned {
    fn to_vec_owned(&self) -> Vec<NativeValue>;
}

impl ToVecOwned for [NativeValue] {
    fn to_vec_owned(&self) -> Vec<NativeValue> {
        self.iter()
            .map(|v| match v {
                NativeValue::Nil => NativeValue::Nil,
                NativeValue::Number(n) => NativeValue::Number(*n),
                NativeValue::Integer(i) => NativeValue::Integer(*i),
                NativeValue::Boolean(b) => NativeValue::Boolean(*b),
                NativeValue::String(s) => NativeValue::String(s.clone()),
            })
            .collect()
    }
}

fn text(value: &NativeValue) -> Option<String> {
    match value {
        NativeValue::String(s) => Some(String::from_utf8_lossy(s).into_owned()),
        _ => None,
    }
}

/// Waits until the waker count passes `above`, i.e. a thread ended or made a call.
fn wait_count(count: &AtomicUsize, above: usize, within: Duration) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < within {
        if count.load(Ordering::SeqCst) > above {
            return Some(start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

/// `launch:DownloadPage`'s script, reduced: returns body, error message, header, status code.
fn download(url: &str, options: &str) -> String {
    format!(
        r#"
        local curl = require("lcurl.safe")
        local easy = curl.easy()
        easy:setopt_url("{url}")
        {options}
        local header, body = "", ""
        easy:setopt_headerfunction(function(d) header = header .. d return true end)
        easy:setopt_writefunction(function(d) body = body .. d return true end)
        local _, err = easy:perform()
        local code = easy:getinfo(curl.INFO_RESPONSE_CODE)
        easy:close()
        return body, err and err:msg() or nil, header, code
        "#
    )
}

fn run_download(web: &Arc<CannedWeb>, timeout: Option<Duration>, script: &str) -> Outcome {
    let mut manager = canned(web, timeout);
    launch(&mut manager, script, &[]);
    wait_outcome(&mut manager, Duration::from_secs(5)).expect("download did not finish")
}

#[test]
fn canned_get_returns_body_status_and_headers() {
    let web = Arc::new(CannedWeb::default());
    web.route(
        "GET",
        "https://x.test/a",
        Canned::ok("hello").header("Location", "https://x.test/b"),
    );
    let out = run_download(&web, None, &download("https://x.test/a", ""));
    let values = out.result.unwrap();
    assert_eq!(text(&values[0]).unwrap(), "hello");
    assert!(matches!(values[1], NativeValue::Nil));
    let header = text(&values[2]).unwrap();
    assert!(header.contains("HTTP/1.1 200"), "{header}");
    assert!(header.contains("Location: https://x.test/b"), "{header}");
    assert!(matches!(values[3], NativeValue::Integer(200)));
    assert!(web.unrouted().is_empty());
}

#[test]
fn canned_status_and_failure_reach_the_script() {
    let web = Arc::new(CannedWeb::default());
    web.route("GET", "https://x.test/missing", Canned::status(404, "nope"));
    web.route(
        "GET",
        "https://x.test/offline",
        Canned::fail("Couldn't resolve host name"),
    );
    let values = run_download(&web, None, &download("https://x.test/missing", ""))
        .result
        .unwrap();
    assert!(matches!(values[3], NativeValue::Integer(404)));
    assert_eq!(text(&values[0]).unwrap(), "nope");

    let values = run_download(&web, None, &download("https://x.test/offline", ""))
        .result
        .unwrap();
    assert_eq!(text(&values[1]).unwrap(), "Couldn't resolve host name");
    assert!(matches!(values[3], NativeValue::Integer(0)));
}

#[test]
fn canned_redirect_url_comes_from_the_location_header() {
    let web = Arc::new(CannedWeb::default());
    web.route(
        "GET",
        "https://x.test/short",
        Canned::status(302, "").header("Location", "https://x.test/long"),
    );
    let script = r#"
        local curl = require("lcurl.safe")
        local easy = curl.easy()
        easy:setopt_url("https://x.test/short")
        easy:perform()
        return easy:getinfo(curl.INFO_REDIRECT_URL), easy:getinfo(curl.INFO_RESPONSE_CODE)
    "#;
    let values = run_download(&web, None, script).result.unwrap();
    assert_eq!(text(&values[0]).unwrap(), "https://x.test/long");
    assert!(matches!(values[1], NativeValue::Integer(302)));
}

#[test]
fn unrouted_url_fails_and_is_recorded() {
    let web = Arc::new(CannedWeb::default());
    let out = run_download(&web, None, &download("https://x.test/a", ""));
    let values = out.result.unwrap();
    assert_eq!(
        text(&values[1]).unwrap(),
        "no canned response for https://x.test/a"
    );
    assert_eq!(web.unrouted(), vec!["GET https://x.test/a".to_string()]);
}

#[test]
fn canned_records_post_body_proxy_and_ip_resolve() {
    let web = Arc::new(CannedWeb::default());
    web.route("POST", "https://x.test/p", Canned::ok("done"));
    let options = r#"
        easy:setopt(curl.OPT_HTTPHEADER, {"X-One: 1", "X-Two: 2"})
        easy:setopt(curl.OPT_POST, true)
        easy:setopt(curl.OPT_POSTFIELDS, "a=1")
        easy:setopt(curl.OPT_PROXY, "http://p:1")
        easy:setopt(curl.OPT_IPRESOLVE, 1)
    "#;
    let values = run_download(&web, None, &download("https://x.test/p", options))
        .result
        .unwrap();
    assert_eq!(text(&values[0]).unwrap(), "done");
    assert_eq!(
        web.requests(),
        vec![Recorded {
            method: "POST".into(),
            url: "https://x.test/p".into(),
            headers: vec!["X-One: 1".into(), "X-Two: 2".into()],
            body: b"a=1".to_vec(),
            proxy: Some("http://p:1".into()),
            ip_resolve: Some(1),
        }]
    );
}

#[test]
fn post_routes_match_on_body_prefix() {
    let web = Arc::new(CannedWeb::default());
    web.route_post("https://x.test/p", "kind=", Canned::ok("generic"));
    web.route_post("https://x.test/p", "kind=tree", Canned::ok("tree"));
    web.route_post("https://x.test/p", "kind=items", Canned::ok("items"));
    let reply = |body: &str| {
        let options = format!(
            r#"easy:setopt(curl.OPT_POST, true) easy:setopt(curl.OPT_POSTFIELDS, "{body}")"#
        );
        let out = run_download(&web, None, &download("https://x.test/p", &options));
        text(&out.result.unwrap()[0]).unwrap()
    };
    assert_eq!(reply("kind=tree&x=1"), "tree");
    assert_eq!(reply("kind=items"), "items");
    assert_eq!(reply("kind=other"), "generic");
    // a POST with no matching prefix is not served by the prefixed routes
    assert_eq!(reply("other=1"), "");
    assert_eq!(web.unrouted(), vec!["POST https://x.test/p".to_string()]);
}

#[test]
fn canned_delay_over_timeout_is_curl_timeout() {
    let web = Arc::new(CannedWeb::default());
    web.route(
        "GET",
        "https://x.test/slow",
        Canned::ok("late").delayed(Duration::from_millis(300)),
    );
    let started = Instant::now();
    let values = run_download(
        &web,
        Some(Duration::from_millis(100)),
        &download("https://x.test/slow", ""),
    )
    .result
    .unwrap();
    assert_eq!(text(&values[1]).unwrap(), "Timeout was reached");
    // served at the timeout, not after the whole delay (generous: thread start and Lua init)
    assert!(started.elapsed() < Duration::from_millis(1500));

    // under the timeout the same delay is served
    let values = run_download(
        &web,
        Some(Duration::from_secs(2)),
        &download("https://x.test/slow", ""),
    )
    .result
    .unwrap();
    assert_eq!(text(&values[0]).unwrap(), "late");
}

#[test]
fn abort_stops_a_delayed_canned_request() {
    let web = Arc::new(CannedWeb::default());
    web.route(
        "GET",
        "https://x.test/slow",
        Canned::ok("late").delayed(Duration::from_secs(10)),
    );
    let mut manager = canned(&web, None);
    let count = counting_waker(&mut manager);
    let id = launch(&mut manager, &download("https://x.test/slow", ""), &[]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(manager.running_ids(), vec![id]);

    assert!(manager.abort(id));
    assert!(!manager.abort(id), "a second abort finds nothing");
    assert!(manager.running_ids().is_empty());
    let ended = wait_count(&count, 0, Duration::from_millis(500));
    assert!(ended.is_some(), "the thread outlived the abort");
    assert!(manager.process().is_empty(), "an aborted result is dropped");
    assert!(!manager.has_ready());
}

#[test]
fn abort_stops_a_waiting_listener() {
    let mut manager = SubscriptManager::default();
    let count = counting_waker(&mut manager);
    let id = launch(
        &mut manager,
        r#"
        local socket = require("socket")
        local s = socket.tcp4()
        s:bind("127.0.0.1", 0)
        s:listen(1)
        s:settimeout(30)
        return s:accept()
        "#,
        &[],
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(manager.is_running(id));
    assert_eq!(count.load(Ordering::SeqCst), 0, "still waiting");
    assert!(manager.abort(id));
    let ended = wait_count(&count, 0, Duration::from_secs(1));
    assert!(ended.is_some(), "accept ignored the abort");
}

/// The port announced by a `Port(port)` call.
fn take_port(manager: &mut SubscriptManager) -> u16 {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        for event in manager.process() {
            if let SubscriptEvent::Call { arguments, .. } = event {
                return match arguments.values()[0] {
                    NativeValue::Integer(p) => p as u16,
                    NativeValue::Number(p) => p as u16,
                    _ => panic!("port"),
                };
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no Port call");
}

const LISTEN_AND_RECEIVE: &str = r#"
    local socket = require("socket")
    local s = socket.tcp4()
    s:bind("127.0.0.1", 0)
    s:listen(1)
    local _, port = s:getsockname()
    Port(port)
    s:settimeout(30)
    local c = s:accept()
    c:settimeout(30)
    return c:receive("*l")
"#;

#[test]
fn receive_returns_a_partial_line_at_eof() {
    let mut manager = SubscriptManager::default();
    launch(&mut manager, LISTEN_AND_RECEIVE, &["Port"]);
    let port = take_port(&mut manager);
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    std::io::Write::write_all(&mut client, b"abc").unwrap();
    // long enough for the shim's read slices to time out with "abc" already read
    std::thread::sleep(Duration::from_millis(350));
    drop(client);
    let out = wait_outcome(&mut manager, Duration::from_secs(3)).expect("receive did not end");
    assert_eq!(text(&out.result.unwrap()[0]).unwrap(), "abc");
}

#[test]
fn dropping_the_manager_ends_a_waiting_listener() {
    let mut manager = SubscriptManager::default();
    let count = counting_waker(&mut manager);
    launch(&mut manager, LISTEN_AND_RECEIVE, &["Port"]);
    let port = take_port(&mut manager);
    let before = count.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(count.load(Ordering::SeqCst), before, "still waiting");
    drop(manager);
    assert!(
        wait_count(&count, before, Duration::from_secs(1)).is_some(),
        "the thread outlived its manager"
    );
    // the listener's port is free again
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn set_waker_reaches_subscripts_already_running() {
    let web = Arc::new(CannedWeb::default());
    web.route(
        "GET",
        "https://x.test/slow",
        Canned::ok("late").delayed(Duration::from_millis(300)),
    );
    let mut manager = canned(&web, None);
    launch(&mut manager, &download("https://x.test/slow", ""), &[]);
    // the waker is set after the push, as the engine does after PoB's init
    let count = counting_waker(&mut manager);
    assert!(
        wait_count(&count, 0, Duration::from_secs(3)).is_some(),
        "an earlier subscript never woke the loop"
    );
}

#[test]
fn events_of_an_aborted_subscript_are_discarded_in_a_batch() {
    let web = Arc::new(CannedWeb::default());
    let manager = std::cell::RefCell::new(canned(&web, None));
    let first = launch(&mut manager.borrow_mut(), "return 1", &[]);
    let second = launch(&mut manager.borrow_mut(), "return 2", &[]);
    // both are finished before the batch is taken
    std::thread::sleep(Duration::from_millis(300));
    let mut seen = vec![];
    service_subscripts(
        || manager.borrow_mut(),
        Duration::from_millis(50),
        |event| -> Result<(), ()> {
            seen.push(event.id());
            // the first event aborts the other subscript, whose result is in this batch
            let other = if event.id() == first { second } else { first };
            manager.borrow_mut().abort(other);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(seen, vec![first]);
}

#[test]
fn an_unconfigured_lua_state_is_refused_not_sent_to_curl() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let lua = mlua::Lua::new();
    pob_runtime::modules::register(&lua).unwrap();
    let result: Result<(), mlua::Error> = lua
        .load(format!(
            r#"
            local curl = require("lcurl.safe")
            local easy = curl.easy()
            easy:setopt_url("http://127.0.0.1:{port}/")
            easy:perform()
            "#
        ))
        .exec();
    let error = result
        .expect_err("perform ran without a NetConfig")
        .to_string();
    assert!(error.contains("not configured"), "{error}");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        listener.accept().is_err(),
        "a connection reached the listener"
    );
}

#[test]
fn real_curl_connect_timeout_applies() {
    // fill the listener's backlog so that further connects hang instead of completing
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut parked = vec![];
    let mut full = false;
    for _ in 0..2000 {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(50)) {
            Ok(stream) => parked.push(stream),
            Err(_) => {
                full = true;
                break;
            }
        }
    }
    assert!(full, "could not fill the backlog");

    let mut manager = SubscriptManager::with_config(NetConfig {
        connect_timeout: Some(Duration::from_millis(300)),
        ..NetConfig::default()
    });
    launch(&mut manager, &download(&format!("http://{addr}/"), ""), &[]);
    let out = wait_outcome(&mut manager, Duration::from_secs(4)).expect("no connect timeout");
    let message = text(&out.result.unwrap()[1]).expect("an error message");
    assert!(message.contains("Timeout"), "{message}");
    drop(parked);
}

#[test]
fn canned_postfields_alone_make_a_post() {
    let web = Arc::new(CannedWeb::default());
    web.route("POST", "https://x.test/p", Canned::ok("done"));
    let options = r#"easy:setopt(curl.OPT_POSTFIELDS, "a=1")"#;
    let values = run_download(&web, None, &download("https://x.test/p", options))
        .result
        .unwrap();
    assert_eq!(text(&values[0]).unwrap(), "done");
    assert_eq!(web.requests()[0].body, b"a=1".to_vec());
}

#[test]
fn abort_stops_a_waiting_receive() {
    let mut manager = SubscriptManager::default();
    let count = counting_waker(&mut manager);
    let id = launch(
        &mut manager,
        r#"
        local socket = require("socket")
        local s = socket.tcp4()
        s:bind("127.0.0.1", 0)
        s:listen(1)
        local _, port = s:getsockname()
        Port(port)
        s:settimeout(30)
        local c = s:accept()
        c:settimeout(30)
        return c:receive("*l")
        "#,
        &["Port"],
    );
    // the call announcing the port
    assert!(wait_count(&count, 0, Duration::from_secs(2)).is_some());
    let port = take_port(&mut manager);
    let _client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(manager.is_running(id));
    let before = count.load(Ordering::SeqCst);
    assert!(manager.abort(id));
    let ended = wait_count(&count, before, Duration::from_millis(2500));
    assert!(ended.is_some(), "receive ignored the abort");
}

/// A listener that completes TCP handshakes in the kernel and never answers.
fn silent_listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    (listener, url)
}

#[test]
fn real_curl_times_out_against_a_silent_listener() {
    let (_listener, url) = silent_listener();
    let mut manager = SubscriptManager::with_config(NetConfig {
        timeout: Some(Duration::from_millis(300)),
        ..NetConfig::default()
    });
    launch(&mut manager, &download(&url, ""), &[]);
    let started = Instant::now();
    let out = wait_outcome(&mut manager, Duration::from_secs(5)).expect("no timeout");
    let message = text(&out.result.unwrap()[1]).expect("an error message");
    assert!(message.contains("Timeout"), "{message}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn real_curl_abort_ends_the_transfer() {
    let (_listener, url) = silent_listener();
    let mut manager = SubscriptManager::default();
    let count = counting_waker(&mut manager);
    let id = launch(&mut manager, &download(&url, ""), &[]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(count.load(Ordering::SeqCst), 0, "still transferring");
    assert!(manager.abort(id));
    // curl notices at its next progress tick, up to a second away; the listener never ends it
    let latency = wait_count(&count, 0, Duration::from_millis(2500));
    assert!(latency.is_some(), "curl kept going after the abort");
    eprintln!("curl abort latency: {:?}", latency.unwrap());
}

#[test]
fn waker_fires_on_call_and_on_finish() {
    let web = Arc::new(CannedWeb::default());
    let mut manager = canned(&web, None);
    let count = counting_waker(&mut manager);
    launch(&mut manager, "Ping(1) return 7", &["Ping"]);
    let out = wait_outcome(&mut manager, Duration::from_secs(2)).unwrap();
    assert_eq!(out.calls.len(), 1);
    assert!(out.result.is_ok());
    assert!(wait_count(&count, 1, Duration::from_secs(1)).is_some());
    assert!(count.load(Ordering::SeqCst) >= 2);

    // a thread that panics still wakes the loop
    let mut manager = canned(&web, None);
    let count = counting_waker(&mut manager);
    launch(
        &mut manager,
        r#"local curl = require("lcurl.safe") curl.easy():setopt(99999, 1)"#,
        &[],
    );
    let out = wait_outcome(&mut manager, Duration::from_secs(2)).unwrap();
    assert!(out.result.is_err());
    assert!(count.load(Ordering::SeqCst) >= 1);
}

#[test]
fn has_ready_only_after_an_event() {
    let web = Arc::new(CannedWeb::default());
    web.route(
        "GET",
        "https://x.test/slow",
        Canned::ok("late").delayed(Duration::from_millis(400)),
    );
    let mut manager = canned(&web, None);
    assert!(!manager.has_ready(), "nothing running");
    launch(&mut manager, &download("https://x.test/slow", ""), &[]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!manager.has_ready());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !manager.has_ready() {
        assert!(Instant::now() < deadline, "never became ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(wait_outcome(&mut manager, Duration::from_secs(1)).is_some());
    assert!(!manager.has_ready());

    // a queued call counts too
    launch(&mut manager, "Ping(1) return 1", &["Ping"]);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !manager.has_ready() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn default_config_keeps_the_classic_behaviour() {
    let config = NetConfig::default();
    assert_eq!(config.connect_timeout, None);
    assert_eq!(config.timeout, None);
    assert!(matches!(config.transport, Transport::Curl));

    let app = NetConfig::app();
    assert_eq!(app.connect_timeout, Some(Duration::from_secs(15)));
    assert_eq!(app.timeout, Some(Duration::from_secs(60)));
    assert!(matches!(app.transport, Transport::Curl));

    // the default manager puts no time limit on a transfer: a silent server is waited on
    let (_listener, url) = silent_listener();
    let mut manager = SubscriptManager::default();
    let count = counting_waker(&mut manager);
    let id = launch(&mut manager, &download(&url, ""), &[]);
    std::thread::sleep(Duration::from_millis(1300));
    assert!(manager.is_running(id));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    manager.abort(id);
}

/// PoB's `LaunchServer.lua` up to its listener (lines 1-21), returning the port it got, with
/// PoB's ports 49082-49084 swapped for `ports`: those are machine-wide, so the test takes free
/// ephemeral ones instead. Its `bind(a) or bind(b) or bind(c)` needs luasocket's `nil, err`
/// contract.
fn launch_server_bind(ports: [u16; 3]) -> String {
    let [a, b, c] = ports;
    format!(
        r#"
local luaSocket = require("socket")
local server = luaSocket.tcp4()
local function bindSocket()
	local res, err
	res, err = server:bind("localhost", {a}) or server:bind("localhost", {b}) or server:bind("localhost", {c})
	if not res then
		server:close()
	else
		res, err = server:listen(1)
		if not res then
			server:close()
		else
			return server
		end
	end
	return nil, err
end
assert(bindSocket())
local host, port = server:getsockname()
server:close()
return port
"#
    )
}

/// Three ports the system just had free.
fn free_ports() -> [u16; 3] {
    let listeners: Vec<TcpListener> = (0..3)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let port = |i: usize| listeners[i].local_addr().unwrap().port();
    [port(0), port(1), port(2)]
}

#[test]
fn launch_server_falls_back_to_the_next_port() {
    // the first port held: PoB takes the next free one
    let ports = free_ports();
    let first = TcpListener::bind(("127.0.0.1", ports[0])).unwrap();
    let mut manager = SubscriptManager::default();
    launch(&mut manager, &launch_server_bind(ports), &[]);
    let out = wait_outcome(&mut manager, Duration::from_secs(3)).expect("no outcome");
    drop(first);
    let values = out.result.expect("bind failed with the first port held");
    let port = match values[0] {
        NativeValue::Integer(p) => p,
        NativeValue::Number(p) => p as i64,
        ref other => panic!("port: {other:?}"),
    };
    assert_eq!(port, i64::from(ports[1]));

    // all three held: a clean script error, no panic and no hang. The `or` chain keeps one
    // value, so `err` is nil and LuaJIT's `assert(nil, nil)` raises nil itself
    let ports = free_ports();
    let held: Vec<TcpListener> = ports
        .iter()
        .map(|port| TcpListener::bind(("127.0.0.1", *port)).unwrap())
        .collect();
    let mut manager = SubscriptManager::default();
    launch(&mut manager, &launch_server_bind(ports), &[]);
    let out = wait_outcome(&mut manager, Duration::from_secs(3)).expect("no outcome");
    drop(held);
    let error = out.result.expect_err("bound with every port held");
    assert!(error.starts_with("runtime error: nil\n"), "{error}");
}

/// luasocket's failures are `nil, message`: a bind on a held port, a listen without a bind,
/// an accept and a receive that time out. (No host name: that would ask a resolver.)
#[test]
fn socket_failures_return_nil_and_a_message() {
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let mut manager = SubscriptManager::default();
    launch(
        &mut manager,
        &format!(
            r#"
            local socket = require("socket")
            local out = {{}}
            local s = socket.tcp4()
            local ok, err = s:bind("127.0.0.1", {port})
            out[#out + 1] = tostring(ok) .. " " .. tostring(err)
            ok, err = s:listen(1)
            out[#out + 1] = tostring(ok) .. " " .. tostring(err)
            assert(s:bind("127.0.0.1", 0))
            s:settimeout(0.05)
            local c
            c, err = s:accept()
            out[#out + 1] = tostring(c) .. " " .. tostring(err)
            local _, p = s:getsockname()
            Port(p)
            s:settimeout(5)
            c = assert(s:accept())
            c:settimeout(0.05)
            local line
            line, err = c:receive("*l")
            out[#out + 1] = tostring(line) .. " " .. tostring(err)
            c:close()
            return table.concat(out, "|")
            "#
        ),
        &["Port"],
    );
    let listening = take_port(&mut manager);
    let _client = TcpStream::connect(("127.0.0.1", listening)).unwrap();
    let out = wait_outcome(&mut manager, Duration::from_secs(5)).expect("no outcome");
    let values = out.result.expect("a socket call raised");
    assert_eq!(
        text(&values[0]).unwrap(),
        "nil address already in use|nil socket is not bound|nil timeout|nil timeout"
    );
}
