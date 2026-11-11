//! The network harness: the canned web both sides talk to (R8), PoB sessions over it, and the
//! settling of requests. Included from `gate.rs`, shared by the network tests.

use super::{fixture_xml, pob_dir};
use auto_pob::engine::net::{Dispatch, NetHandle};
use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::{load_xml, upstream_lua_with_net};
use auto_pob::engine::{Affected, Backend, BackgroundTask, RequestsView};
use auto_pob::game::Game;
use pob_runtime::net::{CannedWeb, NetConfig, Transport};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The canned web every network test starts from. Nothing is routed yet: a test adds the
/// routes it needs, and anything else fails with `no canned response for <url>`.
pub(crate) fn canned_web() -> Arc<CannedWeb> {
    Arc::new(CannedWeb::default())
}

/// A `NetConfig` serving `web`, with no timeouts unless the test sets them.
pub(crate) fn canned(web: &Arc<CannedWeb>) -> NetConfig {
    NetConfig {
        transport: Transport::Canned(web.clone()),
        ..NetConfig::default()
    }
}

/// `Settings.xml` with a stored sign-in whose token has expired: PoB would refresh it at the
/// first chance it gets. The tokens are fake.
pub(crate) const EXPIRED_TOKEN_SETTINGS: &str = "<PathOfBuilding>\n\t<Accounts lastToken=\"FAKE-ACCESS-0\" lastRefreshToken=\"FAKE-REFRESH-0\" tokenExpiry=\"1\"/>\n</PathOfBuilding>\n";

/// A PoB backend over the canned web, in a temp user folder removed on drop.
pub(crate) struct NetSession {
    pub backend: PobBackend,
    pub user: PathBuf,
}

impl Drop for NetSession {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.user);
    }
}

/// A fresh temp user folder for `test`, with `settings` as its `Settings.xml` when given.
pub(crate) fn net_user(test: &str, settings: Option<&str>) -> PathBuf {
    let user = std::env::temp_dir().join(format!("auto-pob-net-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&user);
    let folder = user.join("Path of Building");
    fs::create_dir_all(folder.join("Builds")).unwrap();
    if let Some(settings) = settings {
        fs::write(folder.join("Settings.xml"), settings).unwrap();
    }
    user
}

pub(crate) fn net_session(test: &str, net: NetConfig, settings: Option<&str>) -> NetSession {
    let user = net_user(test, settings);
    let backend = PobBackend::start_with(Game::Poe1, &pob_dir(), &user, net).expect("starting PoB");
    NetSession { backend, user }
}

impl NetSession {
    pub(crate) fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.backend
            .lua
            .load(code)
            .set_name("=net test")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    pub(crate) fn settings_xml(&self) -> String {
        fs::read_to_string(self.user.join("Path of Building/Settings.xml")).unwrap_or_default()
    }

    /// Copies a committed fixture into the Builds folder and opens it.
    pub(crate) fn open_fixture(&mut self, name: &str) {
        let dest = self
            .user
            .join(format!("Path of Building/Builds/{name}.xml"));
        fs::copy(fixture_xml(name), &dest).unwrap();
        self.backend
            .apply(auto_pob::engine::Command::OpenBuild { path: dest }, &|_| {})
            .expect("open");
    }

    /// Runs Network steps as the loop would until no request is pending, and returns each
    /// step's outcome.
    pub(crate) fn settle_net(&mut self, timeout: Duration) -> Vec<anyhow::Result<Affected>> {
        settle_net(&mut self.backend, timeout)
    }

    pub(crate) fn requests(&mut self) -> RequestsView {
        self.backend.requests_view().expect("requests view")
    }

    /// `#main.popups`, `next(launch.subScripts) == nil` and `launch.promptMsg == nil`: what a
    /// request must leave behind.
    pub(crate) fn assert_clean(&self, what: &str) {
        let (popups, subs, prompt): (usize, bool, bool) = self.eval(
            "return #pob.main.main.popups, next(pob.main.subScripts) == nil, pob.main.promptMsg == nil",
        );
        assert_eq!(popups, 0, "{what}: popups left open");
        assert!(subs, "{what}: launch.subScripts leaked an entry");
        assert!(prompt, "{what}: promptMsg left set");
    }
}

/// Steps `Network` on `backend` whenever a subscript is ready, until no request is pending.
pub(crate) fn settle_net(
    backend: &mut PobBackend,
    timeout: Duration,
) -> Vec<anyhow::Result<Affected>> {
    let deadline = Instant::now() + timeout;
    let mut steps = Vec::new();
    loop {
        if backend.next_background() == Some(BackgroundTask::Network) {
            let step = backend.background_step(BackgroundTask::Network, &|_| {});
            // the loop recalculates for a step that asks (R14)
            if step.as_ref().is_ok_and(|affected| affected.recalc) {
                backend.recalculate().expect("recalculating");
            }
            steps.push(step);
            continue;
        }
        let pending: usize = backend
            .lua
            .load("return glue.net_pending()")
            .eval()
            .unwrap();
        if pending == 0 {
            return steps;
        }
        assert!(
            Instant::now() < deadline,
            "{pending} requests still pending"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Begins a request through `glue.net_begin` that downloads `url`; its callback sets the
/// build's name to the body (a change the test can see). Returns the request id.
pub(crate) fn begin_fetch(lua: &mlua::Lua, url: &str) -> Option<u64> {
    lua.load(
        r#"local url = ...
        return glue.net_begin("Test fetch", {}, function()
            launch:DownloadPage(url, function(response, errMsg)
                if not errMsg then pob.build.buildName = response.body end
            end)
        end)"#,
    )
    .set_name("=begin fetch")
    .call(url)
    .expect("net_begin")
}

/// Unmodified upstream PoB over the canned web: its subscripts run through our natives and are
/// delivered to PoB's own `launch:OnSub*` by [`pump`](UpstreamNet::pump).
pub(crate) struct UpstreamNet {
    pub lua: mlua::Lua,
    pub net: NetHandle,
}

impl UpstreamNet {
    /// Starts upstream with `xml` loaded when given, and `tree_lib.lua` plus `net_lib.lua`.
    pub(crate) fn new(web: &Arc<CannedWeb>, xml: Option<&str>) -> UpstreamNet {
        let (lua, net) =
            upstream_lua_with_net(&pob_dir(), canned(web)).expect("starting upstream PoB");
        if let Some(xml) = xml {
            load_xml(&lua, xml, "net").expect("loading the build");
        }
        super::load_tree_lib(&lua);
        let lib = fs::read_to_string(super::fixtures_dir().join("net_lib.lua")).unwrap();
        lua.load(lib)
            .set_name("=net_lib")
            .exec()
            .expect("net_lib.lua");
        UpstreamNet { lua, net }
    }

    pub(crate) fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.lua
            .load(code)
            .set_name("=upstream net")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    /// Delivers subscript results and calls to PoB, as its frame loop does, until no subscript
    /// runs any more (a callback may start the next one).
    pub(crate) fn pump(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if self.net.has_ready() {
                self.net
                    .service(&self.lua, Dispatch::Launch)
                    .expect("delivering to upstream");
            } else if self.net.running_ids().is_empty() {
                return;
            } else {
                assert!(Instant::now() < deadline, "upstream subscripts still run");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
