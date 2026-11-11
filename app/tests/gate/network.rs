//! SP6 network gate tests: our engine's requests over the canned web.

use super::net_harness::{
    EXPIRED_TOKEN_SETTINGS, begin_fetch, canned, canned_web, net_session, net_user,
};
use super::{lock, pob_dir, set};
use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::{
    Backend, Command, ConfigValue, Engine, Event, FinishedRequest, RequestsView, Snapshot,
};
use auto_pob::game::Game;
use pob_runtime::net::Canned;
use std::time::{Duration, Instant};

const SLOW: &str = "https://example.test/slow";
const SLOW_DELAY: Duration = Duration::from_secs(2);
const HOLD: &str = "https://example.test/hold";
const LATE: &str = "https://example.test/late";
const SLOW_SERVER: &str = "https://example.test/slow-server";
const MISSING: &str = "https://example.test/missing";
const OFFLINE: &str = "https://example.test/offline";
const PAGE: &str = "https://example.test/page";

/// Waits for an event that `pick` takes, failing on any `Error`.
fn wait_for<T>(engine: &Engine, what: &str, mut pick: impl FnMut(Event) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("timed out waiting for {what}"));
        match engine.events.recv_timeout(left) {
            Ok(Event::Error { message, .. }) => panic!("error while waiting for {what}: {message}"),
            Ok(Event::StartupFailed { message }) => panic!("startup failed: {message}"),
            Ok(event) => {
                if let Some(found) = pick(event) {
                    return found;
                }
            }
            Err(e) => panic!("no event while waiting for {what}: {e}"),
        }
    }
}

fn requests_of(event: Event) -> Option<RequestsView> {
    match event {
        Event::Snapshot {
            snapshot: Snapshot::Requests(view),
            ..
        } => Some((*view).clone()),
        _ => None,
    }
}

fn finished(view: &RequestsView, id: u64) -> Option<&FinishedRequest> {
    view.finished.iter().find(|f| f.id == id)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn requests_cancel_fail_and_stay_responsive() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    // long enough to outlast the initial snapshots of a debug build, and short of the loop's
    // 2 s idle timer, so only the waker can deliver it on time
    web.route("GET", SLOW, Canned::ok("Fetched").delayed(SLOW_DELAY));
    web.route(
        "GET",
        HOLD,
        Canned::ok("Held").delayed(Duration::from_secs(30)),
    );
    web.route(
        "GET",
        LATE,
        Canned::ok("Late").delayed(Duration::from_millis(300)),
    );
    web.route(
        "GET",
        SLOW_SERVER,
        Canned::ok("Slow").delayed(Duration::from_secs(1)),
    );
    web.route("GET", MISSING, Canned::status(404, "nope"));
    web.route("GET", OFFLINE, Canned::fail("Couldn't resolve host name"));
    web.route("GET", PAGE, Canned::ok("Fetched page"));

    // ---- Responsive: a command goes first while a request waits on the network ----
    let user = net_user("engine", None);
    let (net, pob, start_user) = (canned(&web), pob_dir(), user.clone());
    let begun = std::sync::Arc::new(std::sync::Mutex::new(None));
    let begun_at = begun.clone();
    let engine = Engine::spawn_with(
        move |_send| {
            let backend = PobBackend::start_with(Game::Poe1, &pob, &start_user, net)?;
            // begun on the engine thread, as a press would be
            *begun_at.lock().unwrap() = Some(Instant::now());
            assert!(begin_fetch(&backend.lua, SLOW).is_some());
            assert!(begin_fetch(&backend.lua, HOLD).is_some());
            Ok(Box::new(backend) as Box<dyn Backend>)
        },
        || {},
    );
    let initial = wait_for(&engine, "the first requests view", requests_of);
    let pending: Vec<&str> = initial.pending.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(pending, ["Test fetch", "Test fetch"]);
    let (slow, hold) = (initial.pending[0].id, initial.pending[1].id);
    let seq = engine.send(set("usePowerCharges", ConfigValue::Bool(true)));
    // the command's stats come before the request finishes
    wait_for(&engine, "the command's stats", |event| match event {
        Event::Snapshot {
            seq: s,
            snapshot: Snapshot::Stats(_),
        } if s == seq => Some(()),
        Event::Snapshot {
            snapshot: Snapshot::Requests(view),
            ..
        } => {
            assert!(
                finished(&view, slow).is_none(),
                "the request finished before the command's stats came"
            );
            None
        }
        _ => None,
    });
    let done = wait_for(&engine, "the slow request to finish", |event| {
        requests_of(event).filter(|view| finished(view, slow).is_some())
    });
    // the subscript woke the loop: it didn't wait for the idle timer or a command
    let took = begun.lock().unwrap().unwrap().elapsed();
    assert!(
        took < SLOW_DELAY + Duration::from_millis(500),
        "delivered {took:?} after it began"
    );
    assert_eq!(
        finished(&done, slow),
        Some(&FinishedRequest {
            id: slow,
            label: "Test fetch".into(),
            error: None,
            cancelled: false,
            new_build: false,
        })
    );
    assert_eq!(
        done.pending.iter().map(|p| p.id).collect::<Vec<_>>(),
        [hold]
    );

    // ---- Restart: dropping the engine with a request pending ends its thread ----
    let alive = engine.liveness();
    drop(engine);
    let deadline = Instant::now() + Duration::from_secs(1);
    while alive.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!alive.is_alive(), "the engine thread outlived its Engine");
    let (net, pob, start_user) = (canned(&web), pob_dir(), user.clone());
    let engine = Engine::spawn_with(
        move |_send| {
            Ok(
                Box::new(PobBackend::start_with(Game::Poe1, &pob, &start_user, net)?)
                    as Box<dyn Backend>,
            )
        },
        || {},
    );
    let fresh = wait_for(&engine, "the new engine's requests", requests_of);
    assert_eq!(
        fresh,
        RequestsView::default(),
        "a new engine starts with no requests"
    );
    let alive = engine.liveness();
    drop(engine);
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!alive.is_alive());
    let _ = std::fs::remove_dir_all(&user);

    // ---- Unasked requests (R2, R4): a stored, expired sign-in is never refreshed unasked ----
    let mut net = canned(&web);
    net.timeout = Some(Duration::from_millis(200));
    let mut s = net_session("direct", net, Some(EXPIRED_TOKEN_SETTINGS));
    let asked = web.requests().len();
    let unasked = |s: &super::net_harness::NetSession, when: &str| {
        let urls: Vec<String> = web.requests()[asked..]
            .iter()
            .map(|r| r.url.clone())
            .collect();
        assert!(urls.is_empty(), "{when}: unasked requests {urls:?}");
        assert!(
            s.settings_xml().contains("lastToken=\"FAKE-ACCESS-0\""),
            "{when}: the stored sign-in was touched"
        );
        assert!(s.backend.net_handle().running_ids().is_empty(), "{when}");
    };
    unasked(&s, "at start");
    assert!(
        s.backend.net_handle().refused() >= 1,
        "the first build's token check was not even tried"
    );
    s.open_fixture("attack");
    unasked(&s, "opening attack");
    s.eval::<()>("pob.main.lastUpdateCheck = -1e9 glue.frame()");
    unasked(&s, "after 12 hours");
    let asked = web.requests().len();

    // ---- A request that succeeds changes the build ----
    let name: String = s.eval("return pob.build.buildName");
    let id = begin_fetch(&s.backend.lua, PAGE).expect("launched");
    assert!(
        s.settle_net(Duration::from_secs(5))
            .iter()
            .all(Result::is_ok)
    );
    let view = s.requests();
    assert_eq!(finished(&view, id).and_then(|f| f.error.clone()), None);
    assert_eq!(
        s.eval::<String>("return pob.build.buildName"),
        "Fetched page"
    );
    s.eval::<()>(&format!("pob.build.buildName = {name:?}"));
    s.assert_clean("success");

    // ---- Cancel runs PoB's failure path, and the late result never lands ----
    let id = begin_fetch(&s.backend.lua, LATE).expect("launched");
    // cancelled while the server is answering
    let deadline = Instant::now() + Duration::from_secs(5);
    while !web.requests().iter().any(|r| r.url == LATE) {
        assert!(Instant::now() < deadline, "the request never went out");
        std::thread::sleep(Duration::from_millis(2));
    }
    let view = s.requests();
    assert_eq!(view.pending.len(), 1);
    assert_eq!(view.pending[0].id, id);
    let affected = s
        .backend
        .apply(Command::CancelRequest { id }, &|_| {})
        .expect("cancel");
    assert!(affected.requests);
    let view = s.requests();
    assert_eq!(
        view.finished,
        [FinishedRequest {
            id,
            label: "Test fetch".into(),
            error: Some("Cancelled".into()),
            cancelled: true,
            new_build: false,
        }]
    );
    assert!(view.pending.is_empty());
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        s.settle_net(Duration::from_secs(1)).is_empty(),
        "the late result was delivered"
    );
    assert_eq!(s.eval::<String>("return pob.build.buildName"), name);
    assert_eq!(s.eval::<usize>("return glue.net_pending()"), 0);
    s.assert_clean("cancel");
    assert_eq!(
        s.backend
            .apply(Command::CancelRequest { id }, &|_| {})
            .unwrap_err()
            .to_string(),
        "no such request"
    );

    // ---- Failures finish the request with the reason, never as a step error (R25) ----
    let failures = [
        (SLOW_SERVER, "Timeout was reached"),
        (MISSING, "Response code: 404"),
        (OFFLINE, "Couldn't resolve host name"),
    ];
    for (url, reason) in failures {
        let id = begin_fetch(&s.backend.lua, url).expect("launched");
        let steps = s.settle_net(Duration::from_secs(5));
        assert!(steps.iter().all(Result::is_ok), "{url}: a step failed");
        let view = s.requests();
        let error = finished(&view, id)
            .and_then(|f| f.error.clone())
            .unwrap_or_else(|| panic!("{url}: no error in {view:?}"));
        assert!(error.contains(reason), "{url}: {error}");
        assert_eq!(
            s.eval::<String>("return pob.build.buildName"),
            name,
            "{url}"
        );
        s.assert_clean(url);
    }
    // a callback that raises: PoB shows it as a prompt, which becomes the request's error
    let id: u64 = s
        .backend
        .lua
        .load(
            r#"local url = ...
            return glue.net_begin("Test fetch", {}, function()
                launch:DownloadPage(url, function() error("the body made no sense") end)
            end)"#,
        )
        .call(PAGE)
        .expect("net_begin");
    assert!(
        s.settle_net(Duration::from_secs(5))
            .iter()
            .all(Result::is_ok)
    );
    let view = s.requests();
    let error = finished(&view, id).and_then(|f| f.error.clone());
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("the body made no sense")),
        "{error:?}"
    );
    s.assert_clean("raising callback");

    // ---- Nothing above left a scope open: still no unasked request ----
    s.open_fixture("dot");
    s.eval::<()>("pob.main.lastUpdateCheck = -1e9 glue.frame()");
    let urls: Vec<String> = web.requests()[asked..]
        .iter()
        .map(|r| r.url.clone())
        .collect();
    assert_eq!(
        urls,
        [PAGE, LATE, SLOW_SERVER, MISSING, OFFLINE, PAGE],
        "only the test's own requests went out"
    );
    assert!(s.settings_xml().contains("lastToken=\"FAKE-ACCESS-0\""));
    assert!(s.settings_xml().contains("tokenExpiry=\"1\""));

    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    eprintln!(
        "requests_cancel_fail_and_stay_responsive took {:?}",
        started.elapsed()
    );
}
