//! SP6 network gate tests: our engine's requests over the canned web.

use super::net_harness::{
    EXPIRED_TOKEN_SETTINGS, NetSession, UpstreamNet, begin_fetch, canned, canned_web, net_session,
    net_user,
};
use super::{assert_sidebar, lock, pob_dir, set};
use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::golden_of;
use auto_pob::engine::{
    Affected, Backend, BackgroundTask, CharacterEntry, CharacterPart, Command, ConfigValue, Engine,
    Event, FinishedRequest, ImportMode, OAuthView, PublicStage, PublicView, Realm, RequestsView,
    Snapshot, send_snapshots,
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

// ---- Build sites (Task 5) ----

/// Settings that start PoB on its build list, with no build open.
const LIST_SETTINGS: &str = "<PathOfBuilding>\n\t<Mode mode=\"LIST\"/>\n</PathOfBuilding>\n";

/// A build site as PoB's `buildSites.websiteList` has it.
#[derive(Debug, Clone, serde::Deserialize)]
struct SiteInfo {
    id: String,
    label: String,
    download: String,
    #[serde(default)]
    post_url: Option<String>,
    #[serde(default)]
    post_fields: Option<String>,
    #[serde(default)]
    code_out: Option<String>,
}

impl SiteInfo {
    fn can_share(&self) -> bool {
        self.post_url.is_some()
    }

    /// A link of this site, with the build id `gate`
    fn link(&self) -> &'static str {
        match self.id.as_str() {
            "Maxroll" => "https://maxroll.gg/poe/pob/gate",
            "PoBCodes" => "https://pob.codes/b/gate",
            "POBBin" => "https://pobb.in/gate",
            "PoeNinja" => "https://poe.ninja/poe1/pob/gate",
            "pastebin" => "https://pastebin.com/gate",
            "pastebinProxy" => "https://pastebinp.com/gate",
            "rentry" => "https://rentry.co/gate",
            "PoEDB" => "https://poedb.tw/pob/gate",
            other => panic!("no test link for the site {other}"),
        }
    }

    /// Where the raw code of build `gate` is served (the `%1` of PoB's pattern is the id)
    fn download_url(&self) -> String {
        format!(
            "https://{}",
            self.download.replace("%1", "gate").replace("%.", ".")
        )
    }
}

/// What upstream's `importDump()` says.
#[derive(Debug, serde::Deserialize)]
struct UpImport {
    detail: String,
    valid: bool,
    #[serde(default)]
    site: Option<String>,
    text: String,
    link: String,
    sites: Vec<String>,
    fetching: bool,
    share_label: String,
    #[serde(default)]
    last_site: Option<String>,
    unsaved: bool,
    #[serde(default)]
    import_link: Option<String>,
}

fn up_dump(up: &UpstreamNet) -> UpImport {
    use mlua::LuaSerdeExt;
    let value: mlua::Value = up.eval("return importDump()");
    up.lua.from_value(value).expect("upstream's import tab")
}

fn up_sites(up: &UpstreamNet) -> Vec<SiteInfo> {
    use mlua::LuaSerdeExt;
    let value: mlua::Value = up.eval("return siteList()");
    let mut sites: Vec<SiteInfo> = up.lua.from_value(value).expect("PoB's site list");
    sites.sort_by(|a, b| a.id.cmp(&b.id));
    sites
}

/// The `importLink` attribute of the XML PoB would save for the build
fn saved_import_link(xml: &str) -> Option<String> {
    xml.split("importLink=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .map(str::to_owned)
}

fn import_command(url: &str, mode: ImportMode) -> Command {
    Command::ImportFromUrl {
        url: url.into(),
        mode,
    }
}

/// Imports and runs the request to its end, returning its entry.
fn import_link(s: &mut NetSession, url: &str, mode: ImportMode) -> FinishedRequest {
    s.backend
        .apply(import_command(url, mode), &|_| {})
        .expect("import");
    finish_request(s)
}

fn assert_golden(s: &mut NetSession, name: &str, what: &str) {
    let rows = s.backend.stats_view().expect("stats").stats;
    assert_sidebar(name, what, &rows, &super::golden(name).sidebar);
}

fn our_view(s: &mut NetSession) -> auto_pob::engine::ShareView {
    s.backend.share_view().expect("share view")
}

/// Build XML with each tag's attributes in alphabetical order: PoB writes them in `pairs`
/// order, which differs from one Lua state (and one call) to the next.
fn canonical_xml(xml: &str) -> String {
    xml.lines()
        .map(|line| {
            let line = line.trim_end();
            let Some((head, rest)) = line.split_once(' ') else {
                return line.to_owned();
            };
            if !head.trim_start().starts_with('<') || rest.is_empty() {
                return line.to_owned();
            }
            let mut pairs: Vec<&str> = Vec::new();
            let mut left = rest;
            while let Some(eq) = left.find("=\"") {
                let end = left[eq + 2..]
                    .find('"')
                    .map_or(left.len(), |i| eq + 2 + i + 1);
                pairs.push(left[..end].trim());
                left = &left[end..];
            }
            pairs.sort_unstable();
            format!("{head} {}{}", pairs.join(" "), left.trim())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn our_saved_xml(s: &NetSession) -> String {
    s.eval("return pob.build:SaveDB('code')")
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn build_sites_match_pob() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    let mut s = net_session("build-sites", canned(&web), None);
    s.open_fixture("attack");
    let attack_code = s.backend.share_code().expect("share code");
    let attack_xml = std::fs::read_to_string(super::fixture_xml("attack")).unwrap();
    let up = UpstreamNet::new(&web, Some(&attack_xml));
    // the headless wrapper runs PoB in dev mode, which saves a build to its file whenever it
    // is shut down; the app runs with it off
    up.eval::<()>("launch.devMode = false");
    let sites = up_sites(&up);
    assert_eq!(sites.len(), 8);
    let sharing: Vec<&SiteInfo> = sites.iter().filter(|site| site.can_share()).collect();
    assert_eq!(sharing.len(), 5);

    // ---- Routes: every site serves the attack code; the five that take uploads answer them ----
    for site in &sites {
        web.route("GET", &site.download_url(), Canned::ok(attack_code.clone()));
        if let Some(post_url) = &site.post_url {
            web.route("POST", post_url, Canned::ok(format!("gate-{}", site.id)));
        }
    }

    // ---- Live validation: PoB's verdict, as the box shows it ----
    for site in &sites {
        up.eval::<()>(&format!("act.code_set('{}')", site.link()));
        let theirs = up_dump(&up);
        s.backend
            .apply(
                Command::SetImportCode {
                    text: site.link().into(),
                },
                &|_| {},
            )
            .expect("set import code");
        let ours = our_view(&mut s).import;
        assert_eq!(ours.detail, format!("URL is valid ({})", site.label));
        assert_eq!(ours.detail, theirs.detail, "{}", site.id);
        assert!(ours.valid && theirs.valid);
        assert_eq!(ours.site.as_deref(), Some(site.id.as_str()));
        assert_eq!(ours.site, theirs.site);
        assert_eq!(ours.text, site.link());
        assert_eq!(ours.text, theirs.text);
        assert!(!ours.fetching && !theirs.fetching);
        assert!(ours.can_import_to_this_build, "attack has a file");
    }
    // surrounding whitespace is trimmed, as PoB's box does
    s.backend
        .apply(
            Command::SetImportCode {
                text: "  https://pobb.in/gate \n".into(),
            },
            &|_| {},
        )
        .unwrap();
    assert_eq!(our_view(&mut s).import.text, "https://pobb.in/gate");
    for (text, detail) in [
        ("not a build at all", "Invalid input"),
        (attack_code.as_str(), "Code is valid"),
    ] {
        up.eval::<()>(&format!("act.code_set('{text}')"));
        s.backend
            .apply(Command::SetImportCode { text: text.into() }, &|_| {})
            .unwrap();
        let ours = our_view(&mut s).import;
        assert_eq!(ours.detail, detail);
        assert_eq!(ours.detail, up_dump(&up).detail);
        assert_eq!(ours.valid, detail == "Code is valid");
        assert_eq!(ours.site, None);
    }

    // a redirect link is unwrapped by the box (ImportTab.lua:590-594), on either route
    for wrapped in [
        "https://www.google.com/url?sa=t&q=https%3A%2F%2Fpobb.in%2Fgate&usg=x",
        "https://www.youtube.com/redirect?event=c&q=https%3A%2F%2Fpobb.in%2Fgate&v=x",
    ] {
        up.eval::<()>(&format!("act.code_set('{wrapped}')"));
        s.backend
            .apply(
                Command::SetImportCode {
                    text: wrapped.into(),
                },
                &|_| {},
            )
            .unwrap();
        let (ours, theirs) = (our_view(&mut s).import, up_dump(&up));
        assert_eq!(ours.detail, "URL is valid (pobb.in)");
        assert_eq!(ours.detail, theirs.detail);
        assert_eq!(ours.text, "https://pobb.in/gate");
        assert_eq!(ours.text, theirs.text);
    }

    // ---- Every site imports into a new build, equal to the fixture ----
    for site in &sites {
        up.eval::<()>(&format!("act.code_import('{}', 'new')", site.link()));
        up.pump(Duration::from_secs(15));
        let entry = import_link(&mut s, site.link(), ImportMode::NewBuild);
        assert_eq!(entry.label, format!("Fetching build from {}", site.label));
        assert_eq!(entry.error, None, "{}", site.id);
        assert!(entry.new_build && !entry.cancelled);
        let build = s.backend.build_view().unwrap();
        assert_eq!(build.name, "Imported build");
        assert_eq!(build.path, None);
        assert_golden(&mut s, "attack", &site.id);
        s.assert_clean(&site.id);
        assert_eq!(
            up.eval::<usize>("return #main.popups"),
            0,
            "{:?}",
            up.eval::<Option<String>>("return popupMessage()")
        );
        // PoB stores no link for a fetch through the import box (Step 0)
        assert_eq!(
            saved_import_link(&our_saved_xml(&s)),
            saved_import_link(&up.eval::<String>("return build:SaveDB('code')")),
            "{}",
            site.id
        );
    }
    let wrapped = "https://www.google.com/url?q=https%3A%2F%2Fpobb.in%2Fgate&sa=D";
    up.eval::<()>(&format!("act.code_import('{wrapped}', 'new')"));
    up.pump(Duration::from_secs(15));
    let entry = import_link(&mut s, wrapped, ImportMode::NewBuild);
    assert_eq!(entry.error, None);
    assert_eq!(entry.label, "Fetching build from pobb.in");
    assert_golden(&mut s, "attack", "redirect link");
    // the upstream build is the same one
    let theirs = golden_of(&up.lua, Game::Poe1.pob_version()).expect("upstream stats");
    assert_sidebar(
        "attack",
        "upstream import",
        &s.backend.stats_view().unwrap().stats,
        &theirs.sidebar,
    );

    // ---- A new build has no file: ThisBuild is refused ----
    let err = s
        .backend
        .apply(
            import_command(sites[0].link(), ImportMode::ThisBuild),
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(
        format!("{err:#}"),
        "this build has no file yet; import into a new build"
    );
    assert!(!our_view(&mut s).import.can_import_to_this_build);
    assert!(s.requests().pending.is_empty());

    // ---- Share to each site that takes uploads ----
    let our_code = s.backend.share_code().unwrap();
    for site in &sharing {
        let post_url = site.post_url.clone().unwrap();
        let fields = site.post_fields.clone().unwrap();
        let before = web.requests().len();
        up.eval::<()>(&format!("act.share('{}')", site.id));
        up.pump(Duration::from_secs(15));
        let theirs = up_dump(&up);
        let their_post = web.requests()[before..]
            .iter()
            .find(|r| r.method == "POST" && r.url == post_url)
            .expect("upstream's upload")
            .clone();

        let before = web.requests().len();
        let affected = s
            .backend
            .apply(
                Command::ShareBuild {
                    site: site.id.clone(),
                },
                &|_| {},
            )
            .expect("share");
        assert!(affected.requests && affected.share);
        // while it uploads
        let mid = our_view(&mut s);
        assert!(mid.sharing, "{}", site.id);
        assert_eq!(mid.link, None);
        let pending = s.requests().pending;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].label, format!("Sharing on {}", site.label));
        // PoB disables the button while an upload runs: a second press starts nothing
        if site.id == "Maxroll" {
            let posts = |web: &pob_runtime::net::CannedWeb| {
                web.requests().iter().filter(|r| r.method == "POST").count()
            };
            let asked = posts(&web);
            let err = s
                .backend
                .apply(
                    Command::ShareBuild {
                        site: site.id.clone(),
                    },
                    &|_| {},
                )
                .unwrap_err();
            assert_eq!(format!("{err:#}"), "already sharing");
            assert_eq!(s.requests().pending.len(), 1);
            assert!(our_view(&mut s).sharing);
            let entry = finish_request(&mut s);
            assert_eq!(entry.error, None);
            assert_eq!(posts(&web), asked + 1, "a second upload started");
        } else {
            let entry = finish_request(&mut s);
            assert_eq!(entry.error, None, "{}", site.id);
            assert!(!entry.new_build);
        }
        let want = format!("{}gate-{}", site.code_out.clone().unwrap(), site.id);
        let ours = our_view(&mut s);
        assert_eq!(ours.link.as_deref(), Some(want.as_str()), "{}", site.id);
        assert_eq!(theirs.link, want);
        assert_eq!(theirs.share_label, "Share");
        // PoB remembers a choice made in its drop-down; picking the entry it already showed
        // changes nothing
        assert!(
            theirs.last_site.is_none() || theirs.last_site.as_deref() == Some(site.id.as_str())
        );
        assert!(!ours.sharing);
        assert_eq!(ours.last_site.as_deref(), Some(site.id.as_str()));
        let ids: Vec<&str> = ours.sites.iter().map(|r| r.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            theirs.sites.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(ours.sites.len(), 5);
        // the upload carries PoB's field prefix and the build's code
        let our_post = web.requests()[before..]
            .iter()
            .find(|r| r.method == "POST" && r.url == post_url)
            .expect("our upload")
            .clone();
        assert_eq!(
            String::from_utf8_lossy(&our_post.body),
            format!("{fields}{our_code}"),
            "{}",
            site.id
        );
        // upstream's code is a different serialisation of the same build (PoB writes its
        // stats in `pairs` order), but the field prefix is PoB's own
        assert!(
            their_post.body.starts_with(fields.as_bytes()),
            "{}",
            site.id
        );
        let decode_body = |body: &[u8]| -> Vec<String> {
            let code = String::from_utf8(body[fields.len()..].to_vec()).unwrap();
            let xml: String = s
                .backend
                .lua
                .load("local c = ... return Inflate(common.base64.decode((c:gsub('-','+'):gsub('_','/'))))")
                .call(code)
                .expect("an upload that is a build code");
            let mut lines: Vec<String> = canonical_xml(&xml).lines().map(str::to_owned).collect();
            lines.sort();
            lines
        };
        assert_eq!(
            decode_body(&our_post.body),
            decode_body(&their_post.body),
            "{}: the uploaded builds differ",
            site.id
        );
        s.assert_clean(&site.id);
    }
    assert!(our_view(&mut s).sites.iter().any(|r| r.label == "pobb.in"));

    // ---- The share code: PoB's Generate button ----
    let code = s.backend.share_code().unwrap();
    let inflated: String = s
        .backend
        .lua
        .load("local c = ... return Inflate(common.base64.decode((c:gsub('-','+'):gsub('_','/'))))")
        .call(code.clone())
        .unwrap();
    assert_eq!(canonical_xml(&inflated), canonical_xml(&our_saved_xml(&s)));
    assert!(inflated.contains("<PathOfBuilding>"));

    // ---- ThisBuild replaces the open build once PoB's confirm is pressed ----
    let dot_xml = std::fs::read_to_string(super::fixture_xml("dot")).unwrap();
    s.open_fixture("dot");
    let dot_path = s.user.join("Path of Building/Builds/dot.xml");
    let on_disk = std::fs::read(&dot_path).unwrap();
    assert!(our_view(&mut s).import.can_import_to_this_build);
    let before_rows = s.backend.stats_view().unwrap().stats;
    auto_pob::engine::upstream::load_xml(&up.lua, &dot_xml, "dot").expect("loading dot");
    up.eval::<()>("build.dbFileName = '/nonexistent/dot.xml'");
    assert_sidebar("dot", "before", &before_rows, &super::golden("dot").sidebar);
    up.eval::<()>("act.code_import('https://pobb.in/gate', 'this')");
    up.pump(Duration::from_secs(15));
    // PoB asks first; the build is still dot
    assert_eq!(up.eval::<usize>("return #main.popups"), 1);
    up.eval::<()>("act.confirm()");
    let affected = s
        .backend
        .apply(
            import_command("https://pobb.in/gate", ImportMode::ThisBuild),
            &|_| {},
        )
        .expect("import");
    assert!(affected.requests && !affected.build, "{affected:?}");
    let steps = s.settle_net(Duration::from_secs(15));
    assert!(
        steps
            .iter()
            .any(|a| a.as_ref().is_ok_and(|a| a.build && a.stats && a.recalc)),
        "no step answered for the replaced build"
    );
    let entry = s.requests().finished.remove(0);
    assert_eq!((entry.error.clone(), entry.new_build), (None, true));
    let ours = s.backend.stats_view().unwrap().stats;
    let theirs = golden_of(&up.lua, Game::Poe1.pob_version()).expect("upstream stats");
    assert_sidebar("attack", "this build", &ours, &theirs.sidebar);
    assert_sidebar(
        "attack",
        "this build",
        &ours,
        &super::golden("attack").sidebar,
    );
    let build = s.backend.build_view().unwrap();
    // it is still the file's build, now with changes (Step 0)
    assert_eq!((build.name.as_str(), build.unsaved), ("dot", true));
    assert_eq!(build.path.as_deref(), Some(dot_path.as_path()));
    assert_eq!(up_dump(&up).unsaved, build.unsaved);
    assert_eq!(
        std::fs::read(&dot_path).unwrap(),
        on_disk,
        "the file changed"
    );
    s.assert_clean("this build");
    assert_eq!(up.eval::<usize>("return #main.popups"), 0);
    // PoB stores no link here either (Step 0)
    assert_eq!(saved_import_link(&our_saved_xml(&s)), None);
    assert_eq!(up_dump(&up).import_link, None);

    // ---- A failure leaves nothing stuck, and the same request then works ----
    let pobbin = sites.iter().find(|site| site.id == "POBBin").unwrap();
    let (get, post) = (pobbin.download_url(), pobbin.post_url.clone().unwrap());
    let flags = |s: &NetSession| -> (bool, String) {
        s.backend
            .lua
            .load("local t = pob.build.importTab return t.importCodeFetching and true or false, t.controls.generateCodeByLink.label")
            .eval()
            .unwrap()
    };
    let cases: [(&str, Canned, &str); 3] = [
        ("a 500", Canned::status(500, "oops"), "Response code: 500"),
        (
            "an HTML page",
            Canned::ok("<html><body>Not found</body></html>"),
            "that isn't a link PoB can import",
        ),
        (
            "offline",
            Canned::fail("Couldn't resolve host name"),
            "Couldn't resolve host name",
        ),
    ];
    for (what, reply, want) in cases {
        web.route("GET", &get, reply);
        up.eval::<()>("act.code_import('https://pobb.in/gate', 'this')");
        up.pump(Duration::from_secs(15));
        let theirs = up_dump(&up);
        let entry = import_link(&mut s, "https://pobb.in/gate", ImportMode::ThisBuild);
        assert_eq!(entry.error.as_deref(), Some(want), "{what}");
        assert!(!entry.new_build && !entry.cancelled, "{what}");
        // PoB's own text: its box says the same (an unreadable page is "Invalid input")
        let ours = our_view(&mut s).import;
        assert_eq!(ours.detail, theirs.detail, "{what}");
        assert!(!ours.fetching && !theirs.fetching, "{what}");
        assert!(!flags(&s).0);
        assert_eq!(flags(&s).1, "Share");
        s.assert_clean(what);
        assert_eq!(up.eval::<usize>("return #main.popups"), 0, "{what}");
        // the build is still the imported one
        assert_golden(&mut s, "attack", what);
    }
    web.route("GET", &get, Canned::ok(attack_code.clone()));
    let entry = import_link(&mut s, "https://pobb.in/gate", ImportMode::NewBuild);
    assert_eq!(entry.error, None, "the good route works again");
    assert!(entry.new_build);

    // Share failing: PoB's message popup, captured as the error
    web.route(
        "POST",
        &post,
        Canned::status(500, format!("boom {}", "é".repeat(400))),
    );
    up.eval::<()>("act.share('POBBin')");
    up.pump(Duration::from_secs(15));
    let message: String = up.eval("return popupMessage()");
    assert_eq!(message, "Error creating link:");
    up.eval::<()>("closePopups()");
    assert_eq!(up_dump(&up).share_label, "Share");
    s.backend
        .apply(
            Command::ShareBuild {
                site: "POBBin".into(),
            },
            &|_| {},
        )
        .unwrap();
    let entry = finish_request(&mut s);
    let error = entry.error.expect("the share fails");
    assert!(error.starts_with(&message), "{error}");
    assert!(error.contains("boom"), "{error}");
    // a site's error page is cut to its start, on a character boundary
    assert!(
        error.chars().count() <= 204 && error.ends_with("..."),
        "{error}"
    );
    assert_eq!(our_view(&mut s).link, None);
    assert_eq!(flags(&s).1, "Share");
    s.assert_clean("share 500");
    web.route("POST", &post, Canned::ok("gate-POBBin"));
    s.backend
        .apply(
            Command::ShareBuild {
                site: "POBBin".into(),
            },
            &|_| {},
        )
        .unwrap();
    assert_eq!(finish_request(&mut s).error, None);
    assert_eq!(
        our_view(&mut s).link.as_deref(),
        Some("https://pobb.in/gate-POBBin")
    );
    // a site PoB doesn't offer
    let err = s
        .backend
        .apply(
            Command::ShareBuild {
                site: "pastebin".into(),
            },
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(format!("{err:#}"), "no such site");

    // ---- A late result never lands on another build (Review Focus 1) ----
    s.open_fixture("dot");
    web.route(
        "GET",
        &get,
        Canned::ok(attack_code.clone()).delayed(Duration::from_millis(300)),
    );
    s.backend
        .apply(
            import_command("https://pobb.in/gate", ImportMode::ThisBuild),
            &|_| {},
        )
        .unwrap();
    let pending = s.requests().pending;
    assert_eq!(pending.len(), 1);
    assert!(our_view(&mut s).import.fetching);
    s.open_fixture("new");
    let view = s.requests();
    assert!(view.pending.is_empty());
    assert_eq!(view.finished.len(), 1);
    assert_eq!(view.finished[0].id, pending[0].id);
    assert!(view.finished[0].cancelled);
    assert_eq!(view.finished[0].error.as_deref(), Some("Cancelled"));
    assert!(!view.finished[0].new_build);
    std::thread::sleep(Duration::from_millis(500));
    assert!(s.settle_net(Duration::from_secs(1)).is_empty());
    assert_eq!(s.backend.build_view().unwrap().name, "new");
    assert_golden(&mut s, "new", "after the late result");
    assert!(!our_view(&mut s).import.fetching);
    s.assert_clean("late result");

    // ---- The proxy is PoB's own (R26) ----
    let asked = web.requests().len();
    assert!(web.requests().iter().all(|r| r.proxy.is_none()));
    s.eval::<()>("pob.main.proxyURL = 'http://127.0.0.1:9'");
    web.route("GET", &get, Canned::ok(attack_code.clone()));
    let entry = import_link(&mut s, "https://pobb.in/gate", ImportMode::NewBuild);
    assert_eq!(entry.error, None);
    let proxied: Vec<_> = web.requests()[asked..]
        .iter()
        .map(|r| r.proxy.clone())
        .collect();
    assert_eq!(proxied, [Some("http://127.0.0.1:9".to_string())]);
    s.eval::<()>("pob.main.proxyURL = nil");

    // ---- A code imports at once, with no request ----
    for (mode, name, unsaved) in [
        (ImportMode::NewBuild, "Imported build", true),
        (ImportMode::ThisBuild, "dot", true),
    ] {
        if mode == ImportMode::ThisBuild {
            s.open_fixture("dot");
        }
        let affected = s
            .backend
            .apply(import_command(&format!(" {attack_code}\n"), mode), &|_| {})
            .expect("import a code");
        assert!(
            affected.build && affected.stats && affected.share,
            "{affected:?}"
        );
        assert!(s.requests().pending.is_empty());
        let build = s.backend.build_view().unwrap();
        assert_eq!((build.name.as_str(), build.unsaved), (name, unsaved));
        assert_golden(&mut s, "attack", "a code");
        s.assert_clean("a code");
    }
    let err = s
        .backend
        .apply(import_command("not a build", ImportMode::NewBuild), &|_| {})
        .unwrap_err();
    assert_eq!(format!("{err:#}"), "that isn't a link PoB can import");

    // ---- From the build list: no build, so no ImportTab; the link is fetched and opened as
    // Main:Init does for a protocol link (R15) ----
    let mut list = net_session("sites-list", canned(&web), Some(LIST_SETTINGS));
    assert!(!list.backend.build_loaded().unwrap());
    let pob_link = "pob://pobbin/gate";
    let want_link: Option<String> =
        up.eval("return buildSites.ParseImportLinkFromURI('pob://pobbin/gate')");
    assert_eq!(want_link.as_deref(), Some("https://pobb.in/gate"));
    for (url, mode, want) in [
        (
            "pob://nosuch/gate",
            ImportMode::NewBuild,
            "that pob:// link names no site PoB knows",
        ),
        (
            "nonsense",
            ImportMode::NewBuild,
            "that isn't a link PoB can import",
        ),
        (
            pob_link,
            ImportMode::ThisBuild,
            "this build has no file yet; import into a new build",
        ),
    ] {
        let err = list
            .backend
            .apply(import_command(url, mode), &|_| {})
            .unwrap_err();
        assert_eq!(format!("{err:#}"), want, "{url}");
    }
    assert!(
        up.eval::<Option<String>>("return buildSites.ParseImportLinkFromURI('pob://nosuch/gate')")
            .is_none()
    );
    for (what, reply, want) in [
        ("a 500", Canned::status(500, "oops"), "Response code: 500"),
        (
            "an HTML page",
            Canned::ok("<html></html>"),
            "that isn't a link PoB can import",
        ),
    ] {
        web.route("GET", &get, reply);
        let entry = import_link(&mut list, pob_link, ImportMode::NewBuild);
        // a redirect link is unwrapped as the box does: the unwrapped site is fetched
        let wrapped = "https://www.google.com/url?q=https%3A%2F%2Fpobb.in%2Fgate&sa=D";
        let before = web.requests().len();
        let again = import_link(&mut list, wrapped, ImportMode::NewBuild);
        assert_eq!(again.error, entry.error, "{what}: redirect link");
        assert_eq!(web.requests()[before..].len(), 1);
        assert_eq!(web.requests()[before].url, get);
        assert_eq!(entry.error.as_deref(), Some(want), "{what}");
        assert_eq!(entry.label, "Fetching build from pobb.in");
        assert!(!entry.new_build);
        assert!(!list.backend.build_loaded().unwrap(), "{what}");
        list.assert_clean(what);
    }
    // a slow good one, with the user browsing the list meanwhile (R13)
    web.route(
        "GET",
        &get,
        Canned::ok(attack_code.clone()).delayed(Duration::from_millis(300)),
    );
    let affected = list
        .backend
        .apply(import_command(pob_link, ImportMode::NewBuild), &|_| {})
        .expect("import from the list");
    assert!(affected.requests && !affected.build, "{affected:?}");
    let pending = list.requests().pending;
    assert_eq!(pending.len(), 1);
    list.backend
        .apply(
            Command::ListBuilds {
                folder: String::new(),
            },
            &|_| {},
        )
        .unwrap();
    // text PoB's box rejects changes nothing, so it cancels nothing
    let err = list
        .backend
        .apply(import_command("nonsense", ImportMode::NewBuild), &|_| {})
        .unwrap_err();
    assert_eq!(format!("{err:#}"), "that isn't a link PoB can import");
    assert_eq!(
        list.requests().pending,
        pending,
        "a rejected import cancelled it"
    );
    assert_eq!(
        list.requests().pending,
        pending,
        "browsing cancelled the import"
    );
    assert!(!list.backend.build_loaded().unwrap());
    let steps = list.settle_net(Duration::from_secs(15));
    assert!(
        steps
            .iter()
            .any(|a| a.as_ref().is_ok_and(|a| a.build && a.stats && a.tree)),
        "no step answered for the new build"
    );
    let entry = list.requests().finished.remove(0);
    assert_eq!(entry.id, pending[0].id);
    assert_eq!(
        (entry.error.clone(), entry.new_build, entry.cancelled),
        (None, true, false)
    );
    assert!(list.backend.build_loaded().unwrap());
    let build = list.backend.build_view().unwrap();
    assert_eq!(
        (build.name.as_str(), build.path.as_deref()),
        ("Imported build", None)
    );
    assert_golden(&mut list, "attack", "from the list");
    // the link is stored as Main.lua does: what the download fetched (Main.lua:70-77)
    up.eval::<()>("act.arg_import('pob://pobbin/gate')");
    up.pump(Duration::from_secs(15));
    up.eval::<()>("runCallback('OnFrame')");
    let theirs = saved_import_link(&up.eval::<String>("return build:SaveDB('code')"));
    assert_eq!(theirs.as_deref(), Some(get.as_str()));
    assert_eq!(saved_import_link(&our_saved_xml(&list)), theirs);
    list.assert_clean("from the list");

    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    eprintln!("build_sites_match_pob took {:?}", started.elapsed());
}

// ---- Import by account name (Task 6) ----

const ACCOUNT: &str = "GateTester#1234";
const CHARACTERS_URL: &str = "https://www.pathofexile.com/character-window/get-characters?accountName=GateTester%231234&realm=pc";
const PROFILE_URL: &str = "https://www.pathofexile.com/account/view-profile/GateTester%231234";

fn character_url(part: &str, name: &str) -> String {
    format!(
        "https://www.pathofexile.com/character-window/get-{part}?accountName=GateTester%231234&character={name}&realm=pc"
    )
}

fn net_fixture(name: &str) -> String {
    std::fs::read_to_string(super::fixtures_dir().join("net").join(name)).unwrap()
}

/// What upstream's `accountDump()` says.
#[derive(Debug, serde::Deserialize)]
struct UpAccount {
    account: String,
    realm: String,
    stage: String,
    status: String,
    history: Vec<String>,
    leagues: Vec<String>,
    characters: Vec<CharacterEntry>,
    prompt: bool,
    popups: usize,
}

fn up_account(up: &UpstreamNet) -> UpAccount {
    use mlua::LuaSerdeExt;
    let value: mlua::Value = up.eval("return accountDump()");
    up.lua.from_value(value).expect("upstream's account state")
}

fn stage_of(pob: &str) -> PublicStage {
    match pob {
        "GETACCOUNTNAME" => PublicStage::EnterAccount,
        "DOWNLOADCHARLIST" => PublicStage::Fetching,
        "SELECTCHAR" => PublicStage::SelectCharacter,
        "IMPORTING" => PublicStage::Importing,
        other => panic!("no stage {other}"),
    }
}

fn our_public(s: &mut NetSession) -> PublicView {
    s.backend.account_view().expect("account view").public
}

/// Our public view against upstream's dump. The leagues and characters are compared while a
/// list is showing.
fn assert_public_equal(ours: &PublicView, theirs: &UpAccount, what: &str) {
    assert_eq!(theirs.popups, 0, "{what}: popups left open upstream");
    assert_eq!(ours.account, theirs.account, "{what}: account");
    assert_eq!(ours.realm.pob_id(), theirs.realm, "{what}: realm");
    assert_eq!(ours.stage, stage_of(&theirs.stage), "{what}: stage");
    assert_eq!(ours.status, theirs.status, "{what}: status");
    assert_eq!(ours.history, theirs.history, "{what}: history");
    if matches!(
        ours.stage,
        PublicStage::SelectCharacter | PublicStage::Importing
    ) {
        assert_eq!(ours.leagues, theirs.leagues, "{what}: leagues");
        assert_eq!(ours.characters, theirs.characters, "{what}: characters");
    }
}

fn list_command(account: &str) -> Command {
    Command::ListPublicCharacters {
        account: account.into(),
        realm: Realm::Pc,
    }
}

fn import_character(league: Option<&str>, name: &str, part: CharacterPart) -> Command {
    Command::ImportPublicCharacter {
        league: league.map(str::to_owned),
        name: name.into(),
        part,
    }
}

/// Upstream's Items and Skills tabs, with `items_lib.lua` and `skills_lib.lua` loaded.
fn load_dump_libs(up: &UpstreamNet) {
    // those libraries start `act` afresh and redefine `checkpoint`, so the tree and network
    // helpers come back after them
    let load = |lib: &str| {
        let text = std::fs::read_to_string(super::fixtures_dir().join(lib)).unwrap();
        up.lua
            .load(text)
            .set_name(format!("={lib}"))
            .exec()
            .unwrap();
    };
    load("items_lib.lua");
    load("skills_lib.lua");
    super::load_tree_lib(&up.lua);
    load("net_lib.lua");
}

/// Runs an upstream request to its end and lets PoB's frame handle what it changed.
fn up_settle(up: &UpstreamNet) {
    up.pump(Duration::from_secs(15));
    up.eval::<()>("act.frame()");
}

/// Compares everything an import changes, against upstream.
fn assert_imported_equal(s: &mut NetSession, up: &UpstreamNet, what: &str) {
    use super::items_harness::{first_difference, json, upstream_json};
    use mlua::LuaSerdeExt;
    // ours is read first: the dumps run PoB's full display, which may rewrite gems
    let our_items = json(&s.backend.items_view().expect("items view"));
    let our_skills = s.backend.skills_view().expect("skills view");
    let sidebar = s.backend.stats_view().expect("stats").stats;
    let (ours, ours_undo) = our_tree(s);
    let theirs_items = upstream_json(&up.lua);
    assert_eq!(
        first_difference("", &our_items, &theirs_items),
        None,
        "{what}: items"
    );
    let value: mlua::Value = up.eval("return skillsDump()");
    let their_skills: auto_pob::engine::SkillsView = up.lua.from_value(value).unwrap();
    assert_eq!(our_skills, their_skills, "{what}: skills");
    let (theirs, theirs_undo) = upstream_tree(up);
    assert_eq!(ours, theirs, "{what}: tree");
    assert_eq!(ours_undo, theirs_undo, "{what}: tree undo");
    let golden = golden_of(&up.lua, Game::Poe1.pob_version()).expect("upstream stats");
    assert_sidebar("public", what, &sidebar, &golden.sidebar);
}

#[derive(Debug, serde::Deserialize)]
struct Facts {
    groups: usize,
    nodes: usize,
    equipped: usize,
    ascend: u32,
    level: u32,
    build_flag: bool,
    mod_flag: bool,
    spec_mod: bool,
    items_mod: bool,
    config_mod: bool,
}

fn up_facts(up: &UpstreamNet) -> Facts {
    use mlua::LuaSerdeExt;
    let value: mlua::Value = up.eval("return importedFacts()");
    up.lua.from_value(value).unwrap()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn public_import_matches_pob() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    web.route(
        "GET",
        CHARACTERS_URL,
        Canned::ok(net_fixture("public_characters.json")),
    );
    web.route(
        "GET",
        PROFILE_URL,
        Canned::ok(net_fixture("public_profile.html")),
    );
    for name in ["GateOne", "GateTwo"] {
        web.route(
            "GET",
            &character_url("passive-skills", name),
            Canned::ok(net_fixture("public_passives.json")),
        );
        web.route(
            "GET",
            &character_url("items", name),
            Canned::ok(net_fixture("public_items.json")),
        );
    }
    let new_xml = std::fs::read_to_string(super::fixture_xml("new")).unwrap();
    let mut s = net_session("public-import", canned(&web), None);
    s.open_fixture("new");
    let up = UpstreamNet::new(&web, Some(&new_xml));
    load_dump_libs(&up);
    up.eval::<()>("launch.devMode = false");
    // before any list the view is PoB's idle box
    let idle = our_public(&mut s);
    assert_eq!(idle.stage, PublicStage::EnterAccount);
    assert_public_equal(&idle, &up_account(&up), "idle");

    // ---- The list ----
    up.eval::<()>(&format!("act.public_list('{ACCOUNT}', 'PC')"));
    up.pump(Duration::from_secs(15));
    s.backend
        .apply(list_command(ACCOUNT), &|_| {})
        .expect("list");
    assert_eq!(our_public(&mut s).stage, PublicStage::Fetching);
    assert_eq!(s.requests().pending[0].label, "Fetching characters");
    let steps = s.settle_net(Duration::from_secs(15));
    assert!(
        steps.iter().any(|a| a.as_ref().is_ok_and(|a| a.account)),
        "no step announced the account"
    );
    let listed = our_public(&mut s);
    let theirs = up_account(&up);
    assert_public_equal(&listed, &theirs, "list");
    assert_eq!(listed.stage, PublicStage::SelectCharacter);
    assert_eq!(listed.status, "Character list successfully retrieved.");
    assert_eq!(listed.leagues, ["Mirage Gate", "Standard", "All"]);
    assert_eq!(listed.history, [ACCOUNT]);
    assert_eq!(listed.account, ACCOUNT);
    let names: Vec<&str> = listed.characters.iter().map(|c| c.name.as_str()).collect();
    // by lowercase name, not by byte; the Xbox character is another realm's
    assert_eq!(names, ["alphaGate", "GateOne", "gateThree", "GateTwo"]);
    assert_eq!(listed.characters[1].league, "Mirage Gate");
    assert_eq!(listed.characters[1].level, 90);
    let entry = s.requests().finished.remove(0);
    assert_eq!((entry.error, entry.cancelled), (None, false));
    s.assert_clean("list");

    // ---- Both parts of GateOne, league "All" ----
    up.eval::<()>("act.public_import(nil, 'GateOne', 'tree', true)");
    if up.eval::<usize>("return #main.popups") > 0 {
        up.eval::<()>("act.confirm()");
    }
    // the flags as the callback leaves them, before PoB's frame consumes `buildFlag` (Step 0)
    up.pump(Duration::from_secs(15));
    let tree_only = up_facts(&up);
    up.eval::<()>("act.frame()");
    s.backend
        .apply(
            import_character(
                None,
                "GateOne",
                CharacterPart::Tree {
                    delete_jewels: true,
                },
            ),
            &|_| {},
        )
        .expect("tree import");
    assert_eq!(s.requests().pending[0].label, "Importing passive tree");
    assert_eq!(our_public(&mut s).stage, PublicStage::Importing);
    let steps = s.settle_net(Duration::from_secs(15));
    assert!(
        steps
            .iter()
            .any(|a| a.as_ref().is_ok_and(|a| a.build && a.tree && a.recalc)),
        "the step that imported the tree didn't answer for it"
    );
    let entry = s.requests().finished.remove(0);
    assert_eq!((entry.error, entry.cancelled), (None, false));
    eprintln!("tree-only import flags: {tree_only:?}");
    assert!(
        tree_only.build_flag,
        "a tree import sets buildFlag: {tree_only:?}"
    );
    assert!(
        tree_only.mod_flag && tree_only.spec_mod && tree_only.items_mod && tree_only.config_mod,
        "{tree_only:?}"
    );
    assert_imported_equal(&mut s, &up, "tree");
    assert_public_equal(&our_public(&mut s), &up_account(&up), "after the tree");

    up.eval::<()>("act.public_import(nil, 'GateOne', 'items', true, true, true)");
    up_settle(&up);
    s.backend
        .apply(
            import_character(
                None,
                "GateOne",
                CharacterPart::ItemsAndSkills {
                    clear_items: true,
                    clear_skills: true,
                    ignore_weapon_swap: true,
                },
            ),
            &|_| {},
        )
        .expect("items import");
    assert_eq!(s.requests().pending[0].label, "Importing items and skills");
    s.settle_net(Duration::from_secs(15));
    let entry = s.requests().finished.remove(0);
    assert_eq!((entry.error, entry.cancelled), (None, false));
    assert_imported_equal(&mut s, &up, "items");
    let facts = up_facts(&up);
    assert!(facts.groups >= 1, "{facts:?}");
    assert!(facts.nodes > 10, "{facts:?}");
    assert!(facts.equipped >= 4, "{facts:?}");
    assert_eq!(facts.level, 90);
    assert_ne!(facts.ascend, 0, "no ascendancy: {facts:?}");
    assert!(s.backend.build_view().unwrap().unsaved);
    assert_public_equal(&our_public(&mut s), &up_account(&up), "after the items");
    s.assert_clean("imports");

    // ---- A second tree import, of another league's character, asks first ----
    up.eval::<()>("act.public_import('Standard', 'GateTwo', 'tree', false)");
    assert_eq!(
        up.eval::<usize>("return #main.popups"),
        1,
        "no confirm upstream"
    );
    up.eval::<()>("act.confirm()");
    up_settle(&up);
    s.backend
        .apply(
            import_character(
                Some("Standard"),
                "GateTwo",
                CharacterPart::Tree {
                    delete_jewels: false,
                },
            ),
            &|_| {},
        )
        .expect("second tree import");
    s.settle_net(Duration::from_secs(15));
    let entry = s.requests().finished.remove(0);
    assert_eq!((entry.error, entry.cancelled), (None, false));
    assert_eq!(up.eval::<usize>("return #main.popups"), 0);
    assert_imported_equal(&mut s, &up, "second tree");
    assert_ne!(up_facts(&up).ascend, facts.ascend, "same ascendancy");
    s.assert_clean("second tree");

    // ---- Options that are not PoB's defaults: each box does its own job (the gems differ so
    // that a kept skill group shows) ----
    let alt = net_fixture("public_items.json")
        .replace("Brutality Support", "Melee Physical Damage Support");
    web.route("GET", &character_url("items", "GateTwo"), Canned::ok(alt));
    for (clear_items, clear_skills, wanted_groups) in [(true, false, 2), (false, true, 1)] {
        up.eval::<()>(&format!(
            "act.public_import('Standard', 'GateTwo', 'items', {clear_items}, {clear_skills}, false)"
        ));
        up_settle(&up);
        s.backend
            .apply(
                import_character(
                    Some("Standard"),
                    "GateTwo",
                    CharacterPart::ItemsAndSkills {
                        clear_items,
                        clear_skills,
                        ignore_weapon_swap: false,
                    },
                ),
                &|_| {},
            )
            .expect("items import");
        s.settle_net(Duration::from_secs(15));
        let entry = s.requests().finished.remove(0);
        assert_eq!((entry.error, entry.cancelled), (None, false));
        let what = format!("items, clear_items {clear_items}, clear_skills {clear_skills}");
        assert_imported_equal(&mut s, &up, &what);
        assert_eq!(up_facts(&up).groups, wanted_groups, "{what}");
    }

    // ---- Our refusals ----
    for command in [
        import_character(
            Some("Standard"),
            "GateOne",
            CharacterPart::Tree {
                delete_jewels: true,
            },
        ),
        import_character(
            Some("Nowhere"),
            "GateOne",
            CharacterPart::Tree {
                delete_jewels: true,
            },
        ),
        import_character(
            None,
            "Nobody",
            CharacterPart::Tree {
                delete_jewels: true,
            },
        ),
    ] {
        let message = s.backend.apply(command, &|_| {}).unwrap_err().to_string();
        assert_eq!(message, "no such character");
    }
    assert_eq!(s.requests().pending.len(), 0);

    // ---- Errors: each one's PoB text, flags back to idle, and a good retry works ----
    s.backend
        .apply(Command::ClosePublicCharacters, &|_| {})
        .unwrap();
    up.eval::<()>("act.public_close()");
    assert_eq!(our_public(&mut s).stage, PublicStage::EnterAccount);
    assert_public_equal(&our_public(&mut s), &up_account(&up), "closed");
    // nothing to import from before a list
    let message = s
        .backend
        .apply(
            import_character(
                None,
                "GateOne",
                CharacterPart::Tree {
                    delete_jewels: true,
                },
            ),
            &|_| {},
        )
        .unwrap_err()
        .to_string();
    assert_eq!(message, "PoB doesn't offer that here");
    // no discriminator
    let theirs: String = up.eval("return act.public_list('GateTester', 'PC')");
    let ours = s
        .backend
        .apply(list_command("GateTester"), &|_| {})
        .unwrap_err()
        .to_string();
    assert_eq!(ours, "Missing discriminator e.g. GateTester#1234");
    assert_eq!(ours, theirs);
    assert_eq!(s.requests().pending.len(), 0);
    for (status, message) in [
        (404, "Account name is incorrect."),
        (403, "Account profile is private or does not exist."),
    ] {
        web.route("GET", CHARACTERS_URL, Canned::status(status, ""));
        up.eval::<()>(&format!("act.public_list('{ACCOUNT}', 'PC')"));
        up.pump(Duration::from_secs(15));
        let theirs = up_account(&up);
        assert_eq!(theirs.status, message);
        s.backend.apply(list_command(ACCOUNT), &|_| {}).unwrap();
        let entry = finish_request(&mut s);
        assert_eq!(entry.error.as_deref(), Some(message), "{status}");
        assert_public_equal(&our_public(&mut s), &theirs, &status.to_string());
        assert_eq!(our_public(&mut s).stage, PublicStage::EnterAccount);
        s.assert_clean(&status.to_string());
    }
    // a profile page without the link: the callback raises
    web.route(
        "GET",
        CHARACTERS_URL,
        Canned::ok(net_fixture("public_characters.json")),
    );
    web.route("GET", PROFILE_URL, Canned::ok("<html>no link here</html>"));
    up.eval::<()>(&format!("act.public_list('{ACCOUNT}', 'PC')"));
    up.pump(Duration::from_secs(15));
    assert!(up_account(&up).prompt, "upstream didn't raise");
    up.eval::<()>("launch.promptMsg = nil");
    s.backend.apply(list_command(ACCOUNT), &|_| {}).unwrap();
    let entry = finish_request(&mut s);
    assert!(entry.error.is_some() && !entry.cancelled, "{entry:?}");
    let broken = our_public(&mut s);
    assert_eq!(broken.stage, PublicStage::EnterAccount);
    assert_eq!(broken.status, "Failed to retrieve character list.");
    assert_eq!(entry.error.as_deref(), Some(broken.status.as_str()));
    s.assert_clean("profile page");
    // a good retry
    web.route(
        "GET",
        PROFILE_URL,
        Canned::ok(net_fixture("public_profile.html")),
    );
    s.backend.apply(list_command(ACCOUNT), &|_| {}).unwrap();
    assert_eq!(finish_request(&mut s).error, None);
    up.eval::<()>(&format!("act.public_list('{ACCOUNT}', 'PC')"));
    up.pump(Duration::from_secs(15));
    assert_public_equal(&our_public(&mut s), &up_account(&up), "retry");
    assert_eq!(our_public(&mut s).stage, PublicStage::SelectCharacter);
    // a body that isn't JSON
    let good_items = net_fixture("public_items.json");
    web.route("GET", &character_url("items", "GateOne"), Canned::ok("{"));
    up.eval::<()>("act.public_import(nil, 'GateOne', 'items', true, true, true)");
    up.pump(Duration::from_secs(15));
    assert!(up_account(&up).prompt, "upstream didn't raise");
    up.eval::<()>("launch.promptMsg = nil");
    s.backend
        .apply(
            import_character(
                None,
                "GateOne",
                CharacterPart::ItemsAndSkills {
                    clear_items: true,
                    clear_skills: true,
                    ignore_weapon_swap: true,
                },
            ),
            &|_| {},
        )
        .unwrap();
    let entry = finish_request(&mut s);
    assert!(entry.error.is_some() && !entry.cancelled, "{entry:?}");
    // PoB's callback raised on the missing table: its failure text, and a status to recover from
    let recoverable = "Failed to retrieve character data, try again.";
    assert_eq!(entry.error.as_deref(), Some(recoverable));
    let after = our_public(&mut s);
    assert_eq!(after.stage, PublicStage::SelectCharacter);
    assert_eq!(after.status, recoverable);
    s.assert_clean("bad items");
    // the same for a passive tree body that isn't JSON
    web.route(
        "GET",
        &character_url("passive-skills", "GateOne"),
        Canned::ok("{"),
    );
    s.backend
        .apply(
            import_character(
                None,
                "GateOne",
                CharacterPart::Tree {
                    delete_jewels: true,
                },
            ),
            &|_| {},
        )
        .unwrap();
    let entry = finish_request(&mut s);
    assert_eq!(entry.error.as_deref(), Some(recoverable));
    let after = our_public(&mut s);
    assert_eq!(
        (after.stage, after.status.as_str()),
        (PublicStage::SelectCharacter, recoverable)
    );
    s.assert_clean("bad passives");
    web.route(
        "GET",
        &character_url("passive-skills", "GateOne"),
        Canned::ok(net_fixture("public_passives.json")),
    );
    // a good retry
    web.route(
        "GET",
        &character_url("items", "GateOne"),
        Canned::ok(good_items),
    );
    s.backend
        .apply(
            import_character(
                None,
                "GateOne",
                CharacterPart::ItemsAndSkills {
                    clear_items: true,
                    clear_skills: true,
                    ignore_weapon_swap: true,
                },
            ),
            &|_| {},
        )
        .unwrap();
    assert_eq!(finish_request(&mut s).error, None);
    s.assert_clean("items retry");

    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    let took = started.elapsed();
    eprintln!("public_import_matches_pob took {took:?}");
    assert!(took < Duration::from_secs(20), "over its budget: {took:?}");
}

// ---- Sign in with Path of Exile (Task 7) ----

const TOKEN_URL: &str = "https://www.pathofexile.com/oauth/token";
const API: &str = "https://api.pathofexile.com/character";
const SIGN_IN_GRANT: &str = "client_id=pob&grant_type=authorization_code";
const REFRESH_GRANT: &str = "client_id=pob&grant_type=refresh_token";
const AUTH_PREFIX: &str = "https://www.pathofexile.com/oauth/authorize?client_id=pob";
const MALFORMED: &str = "Malformed response";

/// Everything the session sent towards a UI, as the loop would send it: each event's `{:?}`
/// (R21's check reads them) and the finished requests, which a `Requests` snapshot takes.
#[derive(Default)]
struct Recorder {
    shown: std::cell::RefCell<Vec<String>>,
    finished: std::cell::RefCell<Vec<FinishedRequest>>,
}

impl Recorder {
    fn send(&self, event: Event) {
        if let Event::Snapshot {
            snapshot: Snapshot::Requests(view),
            ..
        } = &event
        {
            self.finished
                .borrow_mut()
                .extend(view.finished.iter().cloned());
        }
        self.shown.borrow_mut().push(format!("{event:?}"));
    }

    fn take_finished(&self) -> Vec<FinishedRequest> {
        std::mem::take(&mut *self.finished.borrow_mut())
    }
}

/// Applies `command` as the loop does: the command, then the snapshots it asks for (an error
/// is an `Error` event and every view).
fn run(s: &mut NetSession, rec: &Recorder, command: Command) -> Result<(), String> {
    let send = |event: Event| rec.send(event);
    let (flags, result) = match s.backend.apply(command, &send) {
        Ok(mut flags) => {
            if flags.recalc {
                s.backend.recalculate().expect("recalculating");
                flags.merge_views();
            }
            (flags, Ok(()))
        }
        Err(e) => {
            let message = format!("{e:#}");
            send(Event::Error {
                seq: 0,
                message: message.clone(),
            });
            (Affected::ALL_VIEWS, Err(message))
        }
    };
    send_snapshots(&mut s.backend, 0, flags, &send);
    result
}

trait MergeViews {
    fn merge_views(&mut self);
}

impl MergeViews for Affected {
    /// What the loop refreshes after a recalculation
    fn merge_views(&mut self) {
        self.stats = true;
        self.config = true;
        self.tree = true;
        self.calcs = true;
        self.skills = true;
        self.items = true;
        self.display_item = true;
    }
}

/// One Network step, as the loop runs it, when a subscript is ready.
fn net_step(s: &mut NetSession, rec: &Recorder) -> bool {
    if s.backend.next_background() != Some(BackgroundTask::Network) {
        return false;
    }
    let send = |event: Event| rec.send(event);
    let mut flags = s
        .backend
        .background_step(BackgroundTask::Network, &send)
        .expect("a network step");
    if flags.recalc {
        s.backend.recalculate().expect("recalculating");
        flags.merge_views();
    }
    send_snapshots(&mut s.backend, 0, flags, &send);
    true
}

/// Network steps until no request is pending.
fn settle(s: &mut NetSession, rec: &Recorder) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if net_step(s, rec) {
            continue;
        }
        if s.eval::<usize>("return glue.net_pending()") == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "a request still runs");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The one request that ended since the last look.
fn ended(s: &mut NetSession, rec: &Recorder) -> FinishedRequest {
    settle(s, rec);
    let mut finished = rec.take_finished();
    assert_eq!(finished.len(), 1, "{finished:?}");
    finished.remove(0)
}

fn oauth(s: &mut NetSession, rec: &Recorder) -> OAuthView {
    let view = s.backend.account_view().expect("account view");
    rec.shown.borrow_mut().push(format!("{view:?}"));
    view.oauth
}

/// Steps until PoB's listener asked to open the sign-in link, and returns it.
fn wait_for_link(s: &mut NetSession, rec: &Recorder) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        net_step(s, rec);
        if let Some(url) = oauth(s, rec).auth_url {
            return url;
        }
        assert!(Instant::now() < deadline, "no sign-in link");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The link's `state` and the listener's port, from its `redirect_uri`.
fn link_parts(url: &str) -> (String, u16) {
    let state = url
        .split(['?', '&'])
        .find_map(|part| part.strip_prefix("state="))
        .expect("a state")
        .to_string();
    let port = url
        .rsplit_once("&redirect_uri=http://localhost:")
        .expect("a redirect_uri")
        .1
        .parse()
        .expect("a port");
    (state, port)
}

/// The browser's redirect into PoB's listener; returns the page it answers.
fn redirect(port: u16, query: &str) -> String {
    use std::io::{Read, Write};
    let mut stream =
        std::net::TcpStream::connect(("127.0.0.1", port)).expect("PoB's sign-in listener");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "GET /?{query} HTTP/1.1\r\n").unwrap();
    let mut page = String::new();
    let _ = stream.read_to_string(&mut page);
    page
}

/// Sign in, the redirect with `state` (the link's own when `None`), and the request's end.
fn sign_in(s: &mut NetSession, rec: &Recorder, state: Option<&str>) -> (FinishedRequest, String) {
    run(s, rec, Command::SignIn).expect("sign in");
    let link = wait_for_link(s, rec);
    let (own, port) = link_parts(&link);
    let page = redirect(
        port,
        &format!("code=gatecode&state={}", state.unwrap_or(&own)),
    );
    assert!(page.contains("Authentication Successful"), "{page}");
    (ended(s, rec), link)
}

/// PoB's three OAuth ports are machine-wide: when another process holds one, wait a moment
/// and look once more before the steps that need them free.
fn wait_for_oauth_ports() {
    let free =
        || (49082..=49084).all(|port| std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    if !free() {
        std::thread::sleep(Duration::from_secs(1));
        assert!(free(), "another process holds PoB's ports 49082-49084");
    }
}

/// Settings.xml's `<Accounts>` element.
fn accounts(s: &NetSession) -> String {
    let xml = s.settings_xml();
    let start = xml.find("<Accounts").expect("an Accounts element");
    let end = start + xml[start..].find('>').unwrap() + 1;
    xml[start..end].to_string()
}

#[derive(Debug, serde::Deserialize)]
struct UpOAuth {
    leagues: Vec<String>,
    characters: Vec<CharacterEntry>,
}

fn import_oauth(realm: Realm, league: Option<&str>, name: &str, part: CharacterPart) -> Command {
    Command::ImportCharacter {
        realm,
        league: league.map(str::to_owned),
        name: name.into(),
        part,
    }
}

fn tree_part() -> CharacterPart {
    CharacterPart::Tree {
        delete_jewels: true,
    }
}

fn items_part() -> CharacterPart {
    CharacterPart::ItemsAndSkills {
        clear_items: true,
        clear_skills: true,
        ignore_weapon_swap: false,
    }
}

/// Settles the canned web's character routes for the sign-in flow.
fn route_oauth(web: &pob_runtime::net::CannedWeb) {
    web.route_post(
        TOKEN_URL,
        SIGN_IN_GRANT,
        Canned::ok(net_fixture("token.json")),
    );
    web.route_post(
        TOKEN_URL,
        REFRESH_GRANT,
        Canned::ok(net_fixture("token_refreshed.json")),
    );
    web.route("GET", API, Canned::ok(net_fixture("oauth_characters.json")));
    web.route(
        "GET",
        &format!("{API}/GateOne"),
        Canned::ok(net_fixture("oauth_character.json")),
    );
}

/// The headers of a 429 that make PoB's limiter hold the next fetch (Step 0, check 7).
fn rate_limited() -> Canned {
    let mut reply = Canned::status(429, "");
    for line in net_fixture("rate_limited_headers.txt").lines() {
        let (name, value) = line.split_once(": ").expect("a header line");
        reply = reply.header(name, value);
    }
    reply
}

/// The facts a character import sets beyond the tabs: level and the quest choices.
fn quest_facts(lua: &mlua::Lua, build: &str) -> (u32, String, String, String) {
    lua.load(format!(
        "local b = {build} return b.characterLevel, b.configTab.input.bandit, \
         b.configTab.input.pantheonMajorGod, b.configTab.input.pantheonMinorGod"
    ))
    .eval()
    .unwrap()
}

fn requests_to(web: &pob_runtime::net::CannedWeb, method: &str, url: &str) -> usize {
    web.requests()
        .iter()
        .filter(|r| r.method == method && r.url == url)
        .count()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn oauth_flow_matches_pob() {
    let _guard = lock();
    capture_logs();
    LOGGED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let started = Instant::now();
    let web = canned_web();
    route_oauth(&web);
    // the sign-in's own list is slow, so a fetch can be sent while it is out
    web.route(
        "GET",
        API,
        Canned::ok(net_fixture("oauth_characters.json")).delayed(Duration::from_millis(300)),
    );
    let new_xml = std::fs::read_to_string(super::fixture_xml("new")).unwrap();
    let mut s = net_session("oauth", canned(&web), None);
    s.open_fixture("new");
    let up = UpstreamNet::new(&web, Some(&new_xml));
    load_dump_libs(&up);
    up.eval::<()>("launch.devMode = false");
    let rec = Recorder::default();

    // ---- Signed out: PoB's status, and our refusals ----
    let out = oauth(&mut s, &rec);
    assert!(!out.signed_in && !out.signing_in && !out.busy, "{out:?}");
    assert_eq!(out.status, "Not authenticated");
    assert_eq!(
        (out.realm, out.auth_url, out.error),
        (Realm::Pc, None, None)
    );
    for command in [
        Command::SignOut,
        Command::ListCharacters { realm: Realm::Pc },
        import_oauth(Realm::Pc, None, "GateOne", tree_part()),
    ] {
        assert_eq!(run(&mut s, &rec, command), Err("sign in first".into()));
    }

    // ---- 1. Sign in: the link, with 49082 taken so PoB falls back to the next port ----
    wait_for_oauth_ports();
    let held = std::net::TcpListener::bind(("127.0.0.1", 49082)).ok();
    run(&mut s, &rec, Command::SignIn).expect("sign in");
    let pending = s.requests().pending;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].label, "Signing in");
    let link = wait_for_link(&mut s, &rec);
    drop(held);
    assert!(link.starts_with(AUTH_PREFIX), "{link}");
    let (state, port) = link_parts(&link);
    assert!(
        link.ends_with(&format!("&redirect_uri=http://localhost:{port}")),
        "{link}"
    );
    assert!(port == 49083 || port == 49084, "port {port}");
    let waiting = oauth(&mut s, &rec);
    assert!(waiting.signing_in && !waiting.signed_in, "{waiting:?}");
    let now = s.eval::<i64>("return os.time()");
    let deadline = waiting.login_deadline.expect("a deadline");
    assert!(
        (now + 58..=now + 60).contains(&deadline),
        "{deadline} vs {now}"
    );
    assert!(
        waiting.status.starts_with("Logging in..."),
        "{}",
        waiting.status
    );
    assert_eq!(
        run(&mut s, &rec, Command::SignIn),
        Err("already signing in".into())
    );
    // R1: any other URL the listener asks to open is dropped
    s.eval::<()>(
        r#"for id in pairs(pob.main.subScripts) do
            glue.net_dispatch("call", id, "OpenURL", "https://evil.test/")
            glue.net_dispatch("call", id, "Copy", "https://evil.test/")
        end"#,
    );
    assert_eq!(oauth(&mut s, &rec).auth_url.as_deref(), Some(link.as_str()));

    // ---- 2-3. The redirect; PoB fetches the token, then the characters ----
    let page = redirect(port, &format!("code=gatecode&state={state}"));
    assert!(page.contains("Authentication Successful"), "{page}");
    // a fetch while the sign-in's own list is out is refused: PoB's limiter, knowing no
    // limits yet, would hold it until 2032 and keep that time for the tab's life
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests_to(&web, "GET", API) == 0 {
        net_step(&mut s, &rec);
        assert!(
            Instant::now() < deadline,
            "the sign-in's list never went out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    for realm in [Realm::Pc, Realm::Xbox] {
        assert_eq!(
            run(&mut s, &rec, Command::ListCharacters { realm }),
            Err("PoB is still loading your characters".into())
        );
    }
    let entry = ended(&mut s, &rec);
    assert_eq!(
        (
            entry.label.as_str(),
            entry.error.as_deref(),
            entry.cancelled
        ),
        ("Signing in", None, false)
    );
    let signed = oauth(&mut s, &rec);
    assert!(
        signed.signed_in && !signed.signing_in && !signed.busy,
        "{signed:?}"
    );
    assert_eq!(signed.auth_url, None);
    assert!(signed.fetched);
    assert_eq!(signed.status, "Authenticated");
    let theirs: UpOAuth = {
        use mlua::LuaSerdeExt;
        up.lua
            .globals()
            .set("characters", net_fixture("oauth_characters.json"))
            .unwrap();
        let value: mlua::Value = up.eval("return oauthDump(characters, 'pc')");
        up.lua.from_value(value).unwrap()
    };
    assert_eq!(signed.characters, theirs.characters);
    assert_eq!(signed.leagues, theirs.leagues);
    let names: Vec<&str> = signed.characters.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["alphaGate", "GateOne", "gateThree", "GateTwo"]);
    assert_eq!(
        signed.leagues,
        ["Hardcore Mirage Gate", "Mirage Gate", "Standard", "Any"]
    );
    // the token went to PoB's Settings.xml, and the exchange carried the code and the port
    let saved = accounts(&s);
    assert!(
        saved.contains(r#"lastToken="FAKE-ACCESS-1""#),
        "no token saved"
    );
    assert!(saved.contains(r#"lastRefreshToken="FAKE-REFRESH-1""#));
    let exchange = web
        .requests()
        .into_iter()
        .find(|r| r.url == TOKEN_URL)
        .expect("the token exchange");
    let body = String::from_utf8(exchange.body).unwrap();
    assert!(
        body.starts_with(&format!(
            "{SIGN_IN_GRANT}&code=gatecode&redirect_uri=http://localhost:{port}&"
        )),
        "the exchange's body"
    );
    let list = web
        .requests()
        .into_iter()
        .find(|r| r.url == API)
        .expect("the list");
    assert!(
        list.headers
            .iter()
            .any(|h| h == "Authorization: Bearer FAKE-ACCESS-1"),
        "the list went without the token"
    );
    s.assert_clean("sign in");

    // ---- 4. Both parts of GateOne. Step 0 check 7: a second press before the first answer
    // meets PoB's limiter (no limits known yet); after it, with no rate headers, it goes ----
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, Some("Mirage Gate"), "GateOne", tree_part()),
    )
    .expect("tree import");
    assert_eq!(s.requests().pending[0].label, "Importing passive tree");
    assert!(oauth(&mut s, &rec).busy);
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, Some("Mirage Gate"), "GateOne", items_part()),
    )
    .expect("an items import the limiter holds");
    let held_back = oauth(&mut s, &rec);
    assert_eq!(
        held_back.error.as_deref(),
        Some("Could not import: Response code: 429")
    );
    assert_eq!(s.requests().pending.len(), 1, "the limiter let it go");
    let entry = ended(&mut s, &rec);
    assert_eq!((entry.error, entry.cancelled), (None, false));
    up.lua
        .globals()
        .set("character", net_fixture("oauth_character.json"))
        .unwrap();
    up.eval::<()>("oauthImport(character, 'tree')");
    assert_imported_equal(&mut s, &up, "oauth tree");
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, Some("Mirage Gate"), "GateOne", items_part()),
    )
    .expect("items import");
    let entry = ended(&mut s, &rec);
    assert_eq!((entry.error, entry.cancelled), (None, false));
    up.eval::<()>("oauthImport(character, 'items')");
    assert_imported_equal(&mut s, &up, "oauth items");
    let ours = quest_facts(&s.backend.lua, "pob.build");
    assert_eq!(ours, quest_facts(&up.lua, "build"));
    assert_eq!(
        ours,
        (90, "Alira".into(), "Lunaris".into(), "Shakari".into())
    );
    assert_eq!(requests_to(&web, "GET", &format!("{API}/GateOne")), 2);
    let imported = oauth(&mut s, &rec);
    assert_eq!((imported.error, imported.busy), (None, false));
    assert!(s.backend.build_view().unwrap().unsaved);
    s.assert_clean("imports");

    // ---- 5. An expired token is refreshed inside the user's import ----
    s.eval::<()>("pob.main.main.api.tokenExpiry = 1");
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, None, "GateOne", items_part()),
    )
    .expect("import with a refresh");
    let entry = ended(&mut s, &rec);
    assert_eq!(entry.error, None);
    let refresh = web
        .requests()
        .into_iter()
        .filter(|r| r.url == TOKEN_URL)
        .nth(1)
        .expect("a refresh");
    assert!(
        refresh
            .body
            .starts_with(format!("{REFRESH_GRANT}&refresh_token=FAKE-REFRESH-1").as_bytes()),
        "the refresh's body"
    );
    assert!(accounts(&s).contains(r#"lastToken="FAKE-ACCESS-2""#));
    // every view while signed in, for the check at the end (R21)
    send_snapshots(&mut s.backend, 0, Affected::ALL_VIEWS, &|e| rec.send(e));
    let last = web.requests().pop().unwrap();
    assert!(
        last.headers
            .iter()
            .any(|h| h == "Authorization: Bearer FAKE-ACCESS-2")
    );
    s.assert_clean("refresh");

    // ---- A 401 that every good refresh meets again: PoB would refresh forever; the request
    // ends after 2 refreshes, the tokens left as PoB has them (deviation, to spare GGG) ----
    let gate_one = format!("{API}/GateOne");
    web.route("GET", &gate_one, Canned::status(401, ""));
    let refreshes = || {
        web.requests()
            .iter()
            .filter(|r| r.url == TOKEN_URL && r.body.starts_with(REFRESH_GRANT.as_bytes()))
            .count()
    };
    let before = refreshes();
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, None, "GateOne", items_part()),
    )
    .unwrap();
    let entry = ended(&mut s, &rec);
    let expired = "Your Path of Exile session expired. Please sign in again.";
    assert_eq!(entry.error.as_deref(), Some(expired));
    assert_eq!(refreshes() - before, 2, "refreshes in the request");
    let view = oauth(&mut s, &rec);
    assert_eq!(view.error.as_deref(), Some(expired));
    assert!(view.signed_in && !view.busy, "{view:?}");
    assert!(accounts(&s).contains(r#"lastToken="FAKE-ACCESS-2""#));
    s.assert_clean("refresh limit");
    web.route(
        "GET",
        &gate_one,
        Canned::ok(net_fixture("oauth_character.json")),
    );

    // ---- Review Focus 2: the list and the character failing, then a good retry ----
    let sony = format!("{API}/sony");
    for (reply, wanted) in [
        (Canned::status(500, "oops"), "Response code: 500"),
        (Canned::ok("{"), MALFORMED),
        (
            Canned::fail("Couldn't resolve host name"),
            "Couldn't resolve host name",
        ),
    ] {
        web.route("GET", &sony, reply);
        run(&mut s, &rec, Command::ListCharacters { realm: Realm::Sony }).unwrap();
        assert!(oauth(&mut s, &rec).busy);
        let entry = ended(&mut s, &rec);
        assert_eq!(entry.error.as_deref(), Some(wanted), "list: {wanted}");
        let view = oauth(&mut s, &rec);
        assert_eq!(view.error.as_deref(), Some(wanted));
        assert!(!view.busy && !view.fetched && view.signed_in, "{view:?}");
        assert_eq!(view.realm, Realm::Sony);
        s.assert_clean(wanted);
    }
    web.route(
        "GET",
        &sony,
        Canned::ok(net_fixture("oauth_characters.json")),
    );
    run(&mut s, &rec, Command::ListCharacters { realm: Realm::Sony }).unwrap();
    assert_eq!(ended(&mut s, &rec).error, None);
    let listed = oauth(&mut s, &rec);
    assert!(listed.fetched && listed.error.is_none(), "{listed:?}");
    let names: Vec<&str> = listed.characters.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["aSonyOne"]);
    // a fetched realm isn't fetched again (PoB's button says "Fetched")
    let before = web.requests().len();
    run(&mut s, &rec, Command::ListCharacters { realm: Realm::Pc }).unwrap();
    assert_eq!(s.requests().pending.len(), 0);
    assert_eq!(web.requests().len(), before);
    assert_eq!(oauth(&mut s, &rec).realm, Realm::Pc);
    for (reply, wanted) in [
        (
            Canned::status(500, "oops"),
            "Could not import: Response code: 500",
        ),
        (Canned::ok("{"), "Could not import character"),
        (
            Canned::fail("Couldn't resolve host name"),
            "Could not import: Couldn't resolve host name",
        ),
    ] {
        web.route("GET", &gate_one, reply);
        run(
            &mut s,
            &rec,
            import_oauth(Realm::Pc, None, "GateOne", items_part()),
        )
        .unwrap();
        let entry = ended(&mut s, &rec);
        assert_eq!(entry.error.as_deref(), Some(wanted));
        let view = oauth(&mut s, &rec);
        assert_eq!(view.error.as_deref(), Some(wanted));
        assert!(!view.busy && view.signed_in, "{view:?}");
        s.assert_clean(wanted);
    }
    web.route(
        "GET",
        &gate_one,
        Canned::ok(net_fixture("oauth_character.json")),
    );
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, None, "GateOne", items_part()),
    )
    .unwrap();
    assert_eq!(ended(&mut s, &rec).error, None);
    assert_eq!(oauth(&mut s, &rec).error, None);
    // our refusals
    for command in [
        import_oauth(Realm::Pc, Some("Nowhere"), "GateOne", tree_part()),
        import_oauth(Realm::Pc, None, "Nobody", tree_part()),
        import_oauth(Realm::Xbox, None, "XboxOne", tree_part()),
    ] {
        assert_eq!(run(&mut s, &rec, command), Err("no such character".into()));
    }

    // ---- 7. A 429 with GGG's rate headers: the next fetch is held, PoB counts down ----
    let xbox = format!("{API}/xbox");
    web.route("GET", &xbox, rate_limited());
    run(&mut s, &rec, Command::ListCharacters { realm: Realm::Xbox }).unwrap();
    let entry = ended(&mut s, &rec);
    assert_eq!(entry.error.as_deref(), Some("Response code: 429"));
    run(&mut s, &rec, Command::ListCharacters { realm: Realm::Xbox }).unwrap();
    assert_eq!(s.requests().pending.len(), 0, "the limiter let it go");
    assert_eq!(requests_to(&web, "GET", &xbox), 1);
    let limited = oauth(&mut s, &rec);
    let now = s.eval::<i64>("return os.time()");
    let retry_at = limited.retry_at.expect("a retry time");
    assert!(
        retry_at > now && retry_at <= now + 62,
        "{retry_at} vs {now}"
    );
    assert!(limited.status.contains("too fast"), "{}", limited.status);
    assert!(!limited.busy);
    s.assert_clean("429");

    // ---- 8. Sign out: PoB forgets the tokens ----
    run(&mut s, &rec, Command::SignOut).unwrap();
    let out = oauth(&mut s, &rec);
    assert!(!out.signed_in, "{out:?}");
    let saved = accounts(&s);
    eprintln!("after sign-out: {}", saved.replace("FAKE-", "<token>"));
    assert!(!saved.contains("lastToken="), "the token stayed");
    assert!(!saved.contains("lastRefreshToken="));
    // PoB writes `tostring(nil)` (Main.lua:725)
    assert!(saved.contains(r#"tokenExpiry="nil""#), "{saved}");
    assert_eq!(
        run(&mut s, &rec, Command::SignOut),
        Err("sign in first".into())
    );

    // ---- Review Focus 2: the token exchange failing, then a good sign-in ----
    for (reply, wanted) in [
        (Canned::status(500, "oops"), "Response code: 500"),
        (Canned::ok("{"), MALFORMED),
        (
            Canned::fail("Couldn't resolve host name"),
            "Couldn't resolve host name",
        ),
    ] {
        web.route_post(TOKEN_URL, SIGN_IN_GRANT, reply);
        let (entry, _) = sign_in(&mut s, &rec, None);
        assert_eq!(entry.error.as_deref(), Some(wanted), "token: {wanted}");
        let view = oauth(&mut s, &rec);
        assert_eq!(view.error.as_deref(), Some(wanted));
        assert!(
            !view.signed_in && !view.signing_in && !view.busy,
            "{view:?}"
        );
        assert!(!accounts(&s).contains("lastToken="));
        s.assert_clean(wanted);
    }
    web.route_post(
        TOKEN_URL,
        SIGN_IN_GRANT,
        Canned::ok(net_fixture("token.json")),
    );

    // ---- 9. Sign in, cancel, sign in again at once: the listener's port is free again ----
    wait_for_oauth_ports();
    run(&mut s, &rec, Command::SignIn).unwrap();
    let first = link_parts(&wait_for_link(&mut s, &rec)).1;
    let id = s.requests().pending[0].id;
    run(&mut s, &rec, Command::CancelRequest { id }).unwrap();
    let cancelled = rec.take_finished();
    assert_eq!(cancelled.len(), 1, "{cancelled:?}");
    assert!(cancelled[0].cancelled);
    assert_eq!(cancelled[0].error.as_deref(), Some("Cancelled"));
    let view = oauth(&mut s, &rec);
    assert!(!view.signing_in && !view.signed_in, "{view:?}");
    let again = Instant::now();
    let (entry, link) = sign_in(&mut s, &rec, None);
    assert!(
        again.elapsed() < Duration::from_secs(2),
        "{:?}",
        again.elapsed()
    );
    eprintln!(
        "sign-in after a cancel: port {first}, then {}",
        link_parts(&link).1
    );
    assert_eq!(entry.error, None);
    assert!(oauth(&mut s, &rec).signed_in);
    assert!(accounts(&s).contains(r#"lastToken="FAKE-ACCESS-1""#));
    // the cancelled listener let its port go, as the finished one did
    // (one more second if another process took it meanwhile)
    for port in [first, link_parts(&link).1] {
        let freed = Instant::now() + Duration::from_secs(2);
        while std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < freed, "port {port} is still held");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    // ---- 6. A 401 the refresh can't cure signs PoB out ----
    web.route("GET", &gate_one, Canned::status(401, ""));
    web.route_post(TOKEN_URL, REFRESH_GRANT, Canned::status(401, ""));
    run(
        &mut s,
        &rec,
        import_oauth(Realm::Pc, None, "GateOne", tree_part()),
    )
    .unwrap();
    let entry = ended(&mut s, &rec);
    let wanted = "Could not import: Response code: 401";
    assert_eq!(entry.error.as_deref(), Some(wanted));
    let view = oauth(&mut s, &rec);
    assert_eq!(view.error.as_deref(), Some(wanted));
    assert!(!view.signed_in && !view.busy, "{view:?}");
    assert!(!accounts(&s).contains("lastToken="));
    s.assert_clean("401");

    // ---- 10. A redirect with another state is refused ----
    let (entry, _) = sign_in(&mut s, &rec, Some("0123456789abcdef"));
    assert_eq!(entry.error.as_deref(), Some("OAuth state mismatch"));
    let view = oauth(&mut s, &rec);
    assert_eq!(view.error.as_deref(), Some("OAuth state mismatch"));
    assert!(!view.signed_in && !view.signing_in, "{view:?}");

    // ---- PoB's three ports all taken: the sign-in ends at once, with our text for PoB's nil ----
    let held: Vec<_> = (49082..=49084)
        .map(|port| std::net::TcpListener::bind(("127.0.0.1", port)).ok())
        .collect();
    run(&mut s, &rec, Command::SignIn).unwrap();
    let entry = ended(&mut s, &rec);
    drop(held);
    let no_port = "couldn't listen on localhost ports 49082-49084 for the sign-in";
    assert_eq!(entry.error.as_deref(), Some(no_port));
    let view = oauth(&mut s, &rec);
    assert_eq!(view.error.as_deref(), Some(no_port));
    assert!(!view.signing_in && !view.signed_in, "{view:?}");
    s.assert_clean("ports taken");

    // ---- 11. No token left Settings.xml (R21), with positive controls ----
    send_snapshots(&mut s.backend, 0, Affected::ALL_VIEWS, &|e| rec.send(e));
    let lines = logged();
    assert!(!lines.is_empty(), "the test logger saw nothing");
    let leaked: Vec<&String> = lines.iter().filter(|l| l.contains("FAKE-")).collect();
    assert!(leaked.is_empty(), "a token reached the log: {leaked:?}");
    // nor a character's or an account's name: PoB logs the endpoints it fails on
    let names = [
        "GateOne",
        "GateTwo",
        "gateThree",
        "alphaGate",
        "XboxOne",
        "aSonyOne",
        "GateTester",
    ];
    let named: Vec<&String> = lines
        .iter()
        .filter(|l| names.iter().any(|name| l.contains(name)))
        .collect();
    assert!(named.is_empty(), "a name reached the log: {named:?}");
    assert!(
        lines.iter().any(|l| l.contains("/character/<redacted>")),
        "PoB's failed downloads weren't logged"
    );
    let shown = rec.shown.borrow();
    assert!(shown.len() > 50, "only {} events", shown.len());
    assert!(
        shown.iter().any(|e| e.contains("aSonyOne")),
        "the recorder missed the account views"
    );
    let leaked: Vec<&String> = shown.iter().filter(|e| e.contains("FAKE-")).collect();
    assert!(
        leaked.is_empty(),
        "a token reached a view or event: {} of them",
        leaked.len()
    );
    assert!(
        !pob_dir().join("poe_api_response.json").exists(),
        "PoB's dev dump was written"
    );

    // ---- 12. Clean ----
    s.assert_clean("end");
    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    let took = started.elapsed();
    eprintln!("oauth_flow_matches_pob took {took:?}");
    assert!(took < Duration::from_secs(30), "over its budget: {took:?}");
}

/// `xml` without its `<elem>` element.
fn without_element(xml: &str, elem: &str) -> String {
    let Some(start) = xml.find(&format!("<{elem}")) else {
        return xml.to_string();
    };
    let open_end = start + xml[start..].find('>').unwrap();
    let end = if xml.as_bytes()[open_end - 1] == b'/' {
        open_end + 1
    } else {
        let close = format!("</{elem}>");
        start + xml[start..].find(&close).unwrap() + close.len()
    };
    format!("{}{}", &xml[..start], &xml[end..])
}

/// Every SP6 command once, with the canned web: only Save writes the build, and PoB's settings
/// change only in the sign-in's `<Accounts>` and the `<Mode>` of the open build.
#[test]
#[ignore = "needs the pinned PoB install"]
fn no_silent_writes_network() {
    let _guard = lock();
    let started = Instant::now();
    let web = canned_web();
    route_oauth(&web);
    let mut s = net_session("no-writes-net", canned(&web), None);
    s.open_fixture("attack");
    // the baseline is PoB's own file: every save rewrites all of it
    s.eval::<()>("pob.main.main:SaveSettings()");
    let settings_before = s.settings_xml();
    assert!(settings_before.contains("<Misc"), "{settings_before}");
    let dir = s.user.join("Path of Building/Builds");
    let before = super::snapshot_dir(&dir);
    assert!(!before.is_empty());
    let rec = Recorder::default();

    // routes: the share site, PoEURL both ways, the public account, the signed-in account
    let (download, post): (String, String) = s.eval(
        r#"for _, site in ipairs(buildSites.websiteList) do
            if site.id == "POBBin" then return site.downloadURL, site.postUrl end
        end"#,
    );
    let code = s.backend.share_code().expect("share code");
    web.route(
        "GET",
        &format!(
            "https://{}",
            download.replace("%1", "gate").replace("%.", ".")
        ),
        Canned::ok(code.clone()),
    );
    web.route("POST", &post, Canned::ok("gate-POBBin"));
    let long: String =
        s.eval("return pob.build.spec:EncodeURL(treeVersions[pob.build.spec.treeVersion].url)");
    web.route(
        "GET",
        &format!("http://poeurl.com/shrink.php?url={long}"),
        Canned::ok("gAtE"),
    );
    web.route(
        "GET",
        "http://poeurl.com/gAtE",
        Canned::status(302, "").header("Location", &long),
    );
    web.route(
        "GET",
        CHARACTERS_URL,
        Canned::ok(net_fixture("public_characters.json")),
    );
    web.route(
        "GET",
        PROFILE_URL,
        Canned::ok(net_fixture("public_profile.html")),
    );
    for part in ["passive-skills", "items"] {
        let body = if part == "items" {
            "public_items.json"
        } else {
            "public_passives.json"
        };
        web.route(
            "GET",
            &character_url(part, "GateOne"),
            Canned::ok(net_fixture(body)),
        );
    }
    web.route(
        "GET",
        &format!("{API}/sony"),
        Canned::ok(net_fixture("oauth_characters.json")),
    );

    let mut each = |command: Command| {
        let shown = format!("{command:?}");
        run(&mut s, &rec, command).unwrap_or_else(|e| panic!("{shown}: {e}"));
        settle(&mut s, &rec);
        for entry in rec.take_finished() {
            assert_eq!(entry.error, None, "{shown}: {entry:?}");
        }
    };
    each(Command::ExportShareCode);
    each(Command::SetImportCode { text: code });
    each(Command::ShrinkTreeLink);
    each(Command::ShareBuild {
        site: "POBBin".into(),
    });
    each(Command::ImportTreeLink {
        title: "Short".into(),
        url: "http://poeurl.com/gAtE".into(),
    });
    each(list_command(ACCOUNT));
    each(import_character(None, "GateOne", tree_part()));
    each(import_character(None, "GateOne", items_part()));
    each(Command::ClosePublicCharacters);
    drop(each);
    let (entry, _) = sign_in(&mut s, &rec, None);
    assert_eq!(entry.error, None);
    let mut each = |command: Command| {
        let shown = format!("{command:?}");
        run(&mut s, &rec, command).unwrap_or_else(|e| panic!("{shown}: {e}"));
        settle(&mut s, &rec);
        for entry in rec.take_finished() {
            assert_eq!(entry.error, None, "{shown}: {entry:?}");
        }
    };
    each(Command::ListCharacters { realm: Realm::Sony });
    each(import_oauth(Realm::Pc, None, "GateOne", tree_part()));
    each(import_oauth(Realm::Pc, None, "GateOne", items_part()));
    each(Command::SignOut);
    drop(each);
    run(&mut s, &rec, Command::SignIn).unwrap();
    wait_for_link(&mut s, &rec);
    let id = s.requests().pending[0].id;
    run(&mut s, &rec, Command::CancelRequest { id }).unwrap();
    assert!(rec.take_finished()[0].cancelled);
    run(
        &mut s,
        &rec,
        Command::ImportFromUrl {
            url: "https://pobb.in/gate".into(),
            mode: ImportMode::ThisBuild,
        },
    )
    .unwrap();
    settle(&mut s, &rec);
    let entry = rec.take_finished().remove(0);
    assert_eq!((entry.error, entry.new_build), (None, true));
    assert!(web.unrouted().is_empty(), "unrouted: {:?}", web.unrouted());
    s.assert_clean("commands");

    // nothing written but PoB's settings, and those only where the user asked
    assert!(
        super::snapshot_dir(&dir) == before,
        "a build file was written before Save"
    );
    let settings_after = s.settings_xml();
    assert_ne!(settings_after, settings_before, "the sign-in saved nothing");
    let rest = |xml: &str| without_element(&without_element(xml, "Accounts"), "Mode");
    assert_eq!(
        rest(&settings_after),
        rest(&settings_before),
        "Settings.xml changed outside <Accounts> and <Mode>"
    );
    // only an explicit Save writes
    run(&mut s, &rec, Command::Save).unwrap();
    assert!(
        super::snapshot_dir(&dir) != before,
        "Save didn't write the build"
    );
    let took = started.elapsed();
    eprintln!("no_silent_writes_network took {took:?}");
    assert!(took < Duration::from_secs(10), "over its budget: {took:?}");
}
