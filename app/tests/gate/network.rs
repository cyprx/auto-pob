//! SP6 network gate tests: our engine's requests over the canned web.

use super::net_harness::{
    EXPIRED_TOKEN_SETTINGS, UpstreamNet, begin_fetch, canned, canned_web, net_session, net_user,
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
    // taken before the engine starts, so a launch made during startup is counted too
    let asked = web.requests().len();
    let mut s = net_session("direct", net, Some(EXPIRED_TOKEN_SETTINGS));
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

    // ---- A callback that makes a new build: Build:Init launches nothing, even in a request
    // (R4), and the step answers for the new build (R14) ----
    let id: u64 = s
        .backend
        .lua
        .load(
            r#"local url = ...
            return glue.net_begin("Test fetch", {}, function()
                launch:DownloadPage(url, function()
                    pob.main.main:SetMode("BUILD", false, "Net build")
                    glue.frame()
                end)
            end)"#,
        )
        .call(PAGE)
        .expect("net_begin");
    let steps = s.settle_net(Duration::from_secs(10));
    let rebuilt = steps
        .iter()
        .map(|step| step.as_ref().expect("a step failed"))
        .find(|a| a.requests && a.build && a.stats && a.list && a.tree && a.items && a.popup)
        .is_some();
    assert!(rebuilt, "the step didn't answer for the new build");
    let view = s.requests();
    let entry = finished(&view, id).expect("finished");
    assert!(entry.new_build, "{entry:?}");
    assert_eq!(entry.error, None);
    assert_eq!(s.eval::<String>("return pob.build.buildName"), "Net build");
    assert!(
        !web.requests().iter().any(|r| r.url.contains("pathofexile")),
        "the new build's ImportTab refreshed the token"
    );
    assert!(s.settings_xml().contains("lastToken=\"FAKE-ACCESS-0\""));
    s.assert_clean("new build");

    // ---- A callback that edits the build asks for a recalculation (R14) ----
    let id: u64 = s
        .backend
        .lua
        .load(
            r#"local url = ...
            return glue.net_begin("Test fetch", {}, function()
                launch:DownloadPage(url, function() pob.build.buildFlag = true end)
            end)"#,
        )
        .call(PAGE)
        .expect("net_begin");
    let steps = s.settle_net(Duration::from_secs(5));
    assert!(
        steps.iter().any(|step| step
            .as_ref()
            .is_ok_and(|a| a.recalc && a.build && a.requests)),
        "the edit asked for no recalculation"
    );
    let view = s.requests();
    assert!(!finished(&view, id).expect("finished").new_build);
    s.backend.recalculate().unwrap();

    // ---- Leaving for the list cancels the build's requests only; opening a build cancels
    // every one (R13) ----
    let in_flight = |s: &super::net_harness::NetSession, url: &str, count: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while web.requests().iter().filter(|r| r.url == url).count() < count {
            assert!(Instant::now() < deadline, "{url} never went out");
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = s;
    };
    let late_sent = web.requests().iter().filter(|r| r.url == LATE).count();
    let list_id: u64 = s
        .backend
        .lua
        .load(
            r#"local url = ...
            return glue.net_begin("Test fetch", { scope = "list" }, function()
                launch:DownloadPage(url, function() end)
            end)"#,
        )
        .call(LATE)
        .expect("net_begin");
    let build_id = begin_fetch(&s.backend.lua, LATE).expect("launched");
    in_flight(&s, LATE, late_sent + 2);
    let affected = s
        .backend
        .apply(
            Command::ListBuilds {
                folder: String::new(),
            },
            &|_| {},
        )
        .expect("list");
    assert!(affected.requests, "the cancel isn't sent");
    let view = s.requests();
    assert_eq!(
        finished(&view, build_id).map(|f| (f.cancelled, f.error.clone())),
        Some((true, Some("Cancelled".to_string())))
    );
    assert_eq!(
        view.pending.iter().map(|p| p.id).collect::<Vec<_>>(),
        [list_id]
    );
    // the list's own request is still out
    assert_eq!(s.eval::<usize>("return #pob.main.main.popups"), 0);
    let late_id = begin_fetch(&s.backend.lua, LATE).expect("launched");
    in_flight(&s, LATE, late_sent + 3);
    // ---- Nothing above left a scope open: still no unasked request ----
    let dest = s.user.join("Path of Building/Builds/dot.xml");
    std::fs::copy(super::fixture_xml("dot"), &dest).unwrap();
    let affected = s
        .backend
        .apply(Command::OpenBuild { path: dest }, &|_| {})
        .expect("open");
    assert!(affected.requests);
    let view = s.requests();
    for id in [list_id, late_id] {
        assert_eq!(
            finished(&view, id).map(|f| f.cancelled),
            Some(true),
            "{id}: {view:?}"
        );
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(s.settle_net(Duration::from_secs(1)).is_empty());
    assert_eq!(s.eval::<String>("return pob.build.buildName"), "dot");
    s.assert_clean("open");
    s.eval::<()>("pob.main.lastUpdateCheck = -1e9 glue.frame()");
    let urls: Vec<String> = web.requests()[asked..]
        .iter()
        .map(|r| r.url.clone())
        .filter(|url| !url.starts_with("https://example.test/"))
        .collect();
    assert!(urls.is_empty(), "unasked requests: {urls:?}");
    assert!(s.settings_xml().contains("lastToken=\"FAKE-ACCESS-0\""));
    assert!(s.settings_xml().contains("tokenExpiry=\"1\""));

    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    eprintln!(
        "requests_cancel_fail_and_stay_responsive took {:?}",
        started.elapsed()
    );
}

/// Log lines captured while a test runs, to see what reaches `log` (R21).
static LOGGED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        LOGGED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(format!("{}", record.args()));
    }

    fn flush(&self) {}
}

fn capture_logs() {
    // another logger may be set already; then the lines don't arrive and the test says so
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Trace);
}

fn logged() -> Vec<String> {
    LOGGED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn downloads_log_no_url_and_guards_restore_popups() {
    let _guard = lock();
    capture_logs();
    let web = canned_web();
    web.route("GET", PAGE, Canned::ok("Fetched page"));
    let mut s = net_session("log-guard", canned(&web), None);
    s.open_fixture("new");

    // ---- M3: PoB's DownloadPage logs "Downloading page at: <url>" from its subscript; the
    // URL (it can name an account) must not reach the log (R21) ----
    let id = begin_fetch(&s.backend.lua, PAGE).expect("launched");
    assert!(
        s.settle_net(Duration::from_secs(5))
            .iter()
            .all(Result::is_ok)
    );
    let view = s.requests();
    assert_eq!(finished(&view, id).and_then(|f| f.error.clone()), None);
    // the download ran, so DownloadPage's subscript did log
    assert_eq!(
        s.eval::<String>("return pob.build.buildName"),
        "Fetched page"
    );
    let lines = logged();
    assert!(
        !lines.is_empty(),
        "the test logger saw nothing; it is not installed"
    );
    let leaked: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("example.test"))
        .collect();
    assert!(leaked.is_empty(), "a URL reached the log: {leaked:?}");
    s.assert_clean("download");

    // ---- M4: a failure while taking stock of the build must leave neither the owned popup on
    // the stack nor the request half made ----
    let id: u64 = s
        .backend
        .lua
        .load(
            r#"local url = ...
            pob.build.treeTab:OpenImportPopup()
            local popup = pob.main.main.popups[1]
            local real = glue.build_loaded
            glue.build_loaded = function() error("forced", 0) end
            local ok, id = pcall(glue.net_begin, "Guard", { popup = popup }, function()
                launch:DownloadPage(url, function() end)
            end)
            glue.build_loaded = real
            if not ok then error(id, 0) end
            return id"#,
        )
        .call(PAGE)
        .expect("net_begin must survive a failing edit check");
    assert_eq!(
        s.eval::<usize>("return #pob.main.main.popups"),
        0,
        "the owned popup stayed on the stack"
    );
    assert!(
        s.settle_net(Duration::from_secs(5))
            .iter()
            .all(Result::is_ok)
    );
    let view = s.requests();
    assert_eq!(finished(&view, id).and_then(|f| f.error.clone()), None);
    s.assert_clean("guard");
    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
}

/// The state of the tree tab in the shape of upstream's `treeDump()`, plus whether it can undo.
fn our_tree(s: &mut super::net_harness::NetSession) -> (super::Checkpoint, bool) {
    let state = s.backend.tree_state_view().expect("tree state");
    let mut allocated = state.allocated;
    allocated.sort_unstable();
    let checkpoint = super::Checkpoint {
        allocated,
        tree_version: state.version,
        spec_titles: state.specs.iter().map(|e| e.title.clone()).collect(),
        active_spec: state.active_spec,
        compare: state.compare,
        sidebar: None,
        tattoos: None,
    };
    (checkpoint, state.can_undo)
}

fn upstream_tree(up: &UpstreamNet) -> (super::Checkpoint, bool) {
    use mlua::LuaSerdeExt;
    let value: mlua::Value = up.eval("return treeDump()");
    let checkpoint = up.lua.from_value(value).expect("upstream's tree state");
    (checkpoint, up.eval("return treeCanUndo()"))
}

/// Runs the Network steps until the request ends, then the one request's entry.
fn finish_request(s: &mut super::net_harness::NetSession) -> FinishedRequest {
    let steps = s.settle_net(Duration::from_secs(15));
    assert!(steps.iter().all(Result::is_ok), "a step failed");
    let mut view = s.requests();
    assert_eq!(view.finished.len(), 1, "{view:?}");
    view.finished.remove(0)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_links_over_network_match_pob() {
    let _guard = lock();
    let started = Instant::now();
    let attack_url = super::upstream_link("attack");
    let web = canned_web();
    web.route(
        "GET",
        "http://poeurl.com/gAtE",
        Canned::status(302, "").header("Location", &attack_url),
    );
    web.route(
        "GET",
        "http://poeurl.com/dead",
        Canned::status(302, "").header("Location", "http://poeurl.com/"),
    );
    let new_xml = std::fs::read_to_string(super::fixture_xml("new")).unwrap();
    let mut s = net_session("tree-links", canned(&web), None);
    s.open_fixture("new");
    let up = UpstreamNet::new(&web, Some(&new_xml));
    let before = our_tree(&mut s);
    assert_eq!(before, upstream_tree(&up));
    assert_eq!(before.0.spec_titles.len(), 1);

    // ---- A short link resolves and adds the tree ----
    up.eval::<()>("act.tree_import('http://poeurl.com/gAtE', 'Short')");
    let affected = s
        .backend
        .apply(
            Command::ImportTreeLink {
                title: "Short".into(),
                url: "http://poeurl.com/gAtE".into(),
            },
            &|_| {},
        )
        .expect("import");
    // it returns at once: the request is out, the tree not yet there
    assert!(affected.requests && !affected.build, "{affected:?}");
    assert_eq!(our_tree(&mut s).0.spec_titles.len(), 1);
    assert_eq!(s.requests().pending.len(), 1);
    assert_eq!(s.requests().pending[0].id, 1);
    let steps = s.settle_net(Duration::from_secs(15));
    assert!(
        steps
            .iter()
            .any(|a| a.as_ref().is_ok_and(|a| a.build && a.tree && a.recalc)),
        "the step that added the tree didn't answer for it"
    );
    s.backend.recalculate().unwrap();
    up.pump(Duration::from_secs(15));
    let (ours, ours_undo) = our_tree(&mut s);
    let (theirs, theirs_undo) = upstream_tree(&up);
    assert_eq!(ours, theirs);
    assert_eq!(ours.spec_titles, ["Default", "Short"]);
    assert_eq!(ours.active_spec, 1);
    assert!(ours.allocated.len() > 10, "the tree is not the attack tree");
    // a new tree has one undo state, so neither side can undo yet (the brief expected true)
    assert_eq!(ours_undo, theirs_undo);
    assert_eq!(up.eval::<usize>("return #main.popups"), 0);
    let entry = s.requests().finished;
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].label, "Resolving PoEURL link");
    assert_eq!(entry[0].error, None);
    s.assert_clean("short import");

    // ---- Shrinking the active tree's link ----
    let long: String = up.eval("return exportLink()");
    web.route(
        "GET",
        &format!("http://poeurl.com/shrink.php?url={long}"),
        Canned::ok("gAtE"),
    );
    up.eval::<()>("act.tree_shrink()");
    up.pump(Duration::from_secs(15));
    let theirs: String = up.eval("return shrunkLink()");
    assert_eq!(theirs, "http://poeurl.com/gAtE");
    up.eval::<()>("closePopups()");
    let affected = s
        .backend
        .apply(Command::ShrinkTreeLink, &|_| {})
        .expect("shrink");
    assert!(affected.requests && !affected.short_link, "{affected:?}");
    let steps = s.settle_net(Duration::from_secs(15));
    assert!(
        steps.iter().any(|a| a.as_ref().is_ok_and(|a| a.short_link)),
        "no step announced the short link"
    );
    assert_eq!(
        s.backend.short_tree_link_view().unwrap().as_deref(),
        Some(theirs.as_str())
    );
    assert_eq!(s.backend.short_tree_link_view().unwrap(), None);
    let entry = s.requests().finished;
    assert_eq!(entry[0].label, "Shrinking tree link");
    assert_eq!(entry[0].error, None);
    s.assert_clean("shrink");

    // ---- A link PoEURL can't resolve: PoB's message, and no tree ----
    let count = our_tree(&mut s).0.spec_titles.len();
    up.eval::<()>("act.tree_import('http://poeurl.com/dead', 'Dead')");
    up.pump(Duration::from_secs(15));
    let theirs: String = up.eval("return popupMessage()");
    assert_eq!(theirs, "Failed to resolve PoEURL link");
    up.eval::<()>("closePopups()");
    s.backend
        .apply(
            Command::ImportTreeLink {
                title: "Dead".into(),
                url: "http://poeurl.com/dead".into(),
            },
            &|_| {},
        )
        .expect("import");
    let entry = finish_request(&mut s);
    assert_eq!(entry.error.as_deref(), Some(theirs.as_str()));
    assert!(!entry.cancelled && !entry.new_build);
    assert_eq!(our_tree(&mut s).0.spec_titles.len(), count);
    assert_eq!(upstream_tree(&up).0, our_tree(&mut s).0);
    s.assert_clean("dead link");

    // ---- A shrink PoEURL fails: PoB's message, and no short link ----
    web.route(
        "GET",
        &format!("http://poeurl.com/shrink.php?url={long}"),
        Canned::status(500, ""),
    );
    up.eval::<()>("act.tree_shrink()");
    up.pump(Duration::from_secs(15));
    let theirs: String = up.eval("return popupMessage()");
    assert_eq!(theirs, "Failed to get PoEURL link. Try again later.");
    up.eval::<()>("closePopups()");
    s.backend.apply(Command::ShrinkTreeLink, &|_| {}).unwrap();
    let entry = finish_request(&mut s);
    assert_eq!(entry.error.as_deref(), Some(theirs.as_str()));
    assert_eq!(s.backend.short_tree_link_view().unwrap(), None);
    s.assert_clean("shrink failure");

    // ---- PoB drops a CUSTOM subscript's error without calling back (Launch.lua:198-220), so
    // its flags stay as they were. The request ends anyway, with the subscript's own error
    // and not the popup's stale "Resolving..." label ----
    let id: u64 = s
        .backend
        .lua
        .load(
            r#"called = false
            local popup = { controls = { msg = { label = "Resolving PoEURL link..." } } }
            return glue.net_begin("Custom", { popup = popup }, function()
                local id = LaunchSubScript("error('boom', 0)", "", "")
                launch:RegisterSubScript(id, function() called = true end)
            end)"#,
        )
        .eval()
        .expect("net_begin");
    let entry = finish_request(&mut s);
    assert_eq!(entry.id, id);
    assert_eq!(entry.error.as_deref(), Some("boom"));
    assert!(!s.eval::<bool>("return called"), "PoB called back");
    s.assert_clean("custom error");

    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    eprintln!(
        "tree_links_over_network_match_pob took {:?}",
        started.elapsed()
    );
}
