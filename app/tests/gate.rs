//! Gate tests: our engine against reference data from unmodified upstream PoB.
//!
//! Run with
//! `AUTO_POB_POB_DIR=<pinned PoB> cargo test -p auto-pob --features gate-probes --test gate -- --ignored --test-threads=1`.

use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::{Golden, golden_of, load_xml, run_recipe, upstream_lua};
use auto_pob::engine::{
    Backend, BackgroundTask, Command, CompareColour, ConfigOption, ConfigValue, ConfigView, Engine,
    Event, Snapshot, StatsView, Tab,
};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[path = "gate/build_list.rs"]
mod build_list;
#[path = "gate/build_open.rs"]
mod build_open;
#[path = "gate/calcs.rs"]
mod calcs;
#[path = "gate/compare.rs"]
mod compare;
#[path = "gate/crafting.rs"]
mod crafting;
#[path = "gate/fresh.rs"]
mod fresh;
#[path = "gate/item_db.rs"]
mod item_db;
#[path = "gate/items.rs"]
mod items;
#[path = "gate/items_harness.rs"]
mod items_harness;
#[path = "gate/longtail_harness.rs"]
mod longtail_harness;
#[path = "gate/names.rs"]
mod names;
#[path = "gate/net_harness.rs"]
mod net_harness;
#[path = "gate/network.rs"]
mod network;
#[path = "gate/parity.rs"]
mod parity;
#[path = "gate/parity_gaps.rs"]
mod parity_gaps;
#[path = "gate/parity_gaps_items.rs"]
mod parity_gaps_items;
#[path = "gate/parity_gaps_tabs.rs"]
mod parity_gaps_tabs;
#[path = "gate/settings.rs"]
mod settings;
#[path = "gate/skills.rs"]
mod skills;
#[path = "gate/tattoos.rs"]
mod tattoos;
#[path = "gate/timeless.rs"]
mod timeless;
#[path = "gate/trade.rs"]
mod trade;
#[path = "gate/tree_draw.rs"]
mod tree_draw;

/// PoB loads files relative to the process working directory, so only one test may run it.
static CWD: Mutex<()> = Mutex::new(());

/// Holds the process-wide lock and prints `gate-time <test> <secs>` to stderr when dropped,
/// so `scripts/gate.sh` and a plain serial run both show which tests are slow.
struct GateGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    began: std::time::Instant,
}

impl Drop for GateGuard {
    fn drop(&mut self) {
        let thread = std::thread::current();
        eprintln!(
            "gate-time {} {:.1}",
            thread.name().unwrap_or("?"),
            self.began.elapsed().as_secs_f64()
        );
    }
}

fn lock() -> GateGuard {
    GateGuard {
        _lock: CWD.lock().unwrap_or_else(|e| e.into_inner()),
        began: std::time::Instant::now(),
    }
}

fn pob_dir() -> PathBuf {
    let dir = std::env::var_os("AUTO_POB_POB_DIR").unwrap_or_else(|| {
        panic!(
            "set AUTO_POB_POB_DIR to the pinned PoB folder, e.g. \
             $HOME/.local/share/auto-pob/poe1/pob-2.67.2"
        )
    });
    PathBuf::from(dir)
        .canonicalize()
        .expect("AUTO_POB_POB_DIR doesn't exist")
}

/// The pinned `Data/TimelessJewelData`, by file: size and modification time. PoB caches an
/// inflated LUT there as a `.bin` (DataJewelFileLoader.lua:84-90), and upstream's wrapper,
/// whose `NewFileSearch` finds nothing, rewrites it on every load: a gate that loads a timeless
/// jewel runs on a farm (`timeless::farm`) and checks this is the same after it.
fn pinned_luts() -> std::collections::BTreeMap<String, (u64, std::time::SystemTime)> {
    fs::read_dir(pob_dir().join("Data/TimelessJewelData"))
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let meta = fs::symlink_metadata(e.path()).unwrap();
            (
                e.file_name().to_string_lossy().to_string(),
                (meta.len(), meta.modified().unwrap()),
            )
        })
        .collect()
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The committed fixtures plus the builds kept in `local/` (name, XML path).
fn fixtures() -> Vec<(String, PathBuf)> {
    let dir = fixtures_dir();
    let mut found = Vec::new();
    for folder in [dir.clone(), dir.join("local")] {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            if path.extension().is_some_and(|e| e == "xml")
                && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            {
                found.push((name.to_string(), path.clone()));
            }
        }
    }
    found.sort();
    found
}

fn golden_path(xml: &Path) -> PathBuf {
    xml.with_extension("golden.json")
}

fn golden(name: &str) -> Golden {
    let (_, xml) = fixtures()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"));
    let path = golden_path(&xml);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}; run the golden example", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

/// The golden files hold PoB's numbers as serde_json wrote them, the shortest text that reads
/// back as the same double; reading must give that double back, or an exact comparison fails
/// on a number PoB and the engine agree on (10.296650000000001 read as 10.29665 without
/// serde_json's `float_roundtrip`)
#[test]
fn golden_numbers_read_back_exactly() {
    for value in [
        10.296650000000001_f64,
        0.1 + 0.2,
        1.0 / 3.0,
        673.412_836_640_095_4,
        1e-7,
    ] {
        let text = serde_json::to_string(&value).unwrap();
        let back: f64 = serde_json::from_str(&text).unwrap();
        assert_eq!(back.to_bits(), value.to_bits(), "{text}");
    }
}

/// Builds the fixture in upstream PoB: from its recipe, or from its XML when it has none.
fn upstream_golden(xml: &Path, name: &str) -> Golden {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    let recipe = fixtures_dir().join(format!("{name}.lua"));
    if recipe.exists() {
        let lib = fs::read_to_string(fixtures_dir().join("lib.lua")).unwrap();
        run_recipe(&lua, &lib, &fs::read_to_string(recipe).unwrap()).expect("recipe");
    } else {
        load_xml(&lua, &fs::read_to_string(xml).unwrap(), name).expect("loading XML");
    }
    golden_of(&lua, Game::Poe1.pob_version()).expect("reading stats")
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn golden_files_are_current() {
    let _guard = lock();
    let all = fixtures();
    assert!(!all.is_empty(), "no fixtures found");
    for (name, xml) in all {
        let committed = golden(&name);
        assert_eq!(
            committed.pob_version,
            Game::Poe1.pob_version(),
            "{name}: golden data is from another PoB version; run the golden example"
        );
        let fresh = upstream_golden(&xml, &name);
        assert_eq!(
            fresh, committed,
            "{name}: golden data is stale; run the golden example"
        );
    }
}

/// (fixture, class, level, allocated nodes besides the class start, gem names)
const RECIPE_SHAPES: [(&str, &str, u32, u32, &[&str]); 4] = [
    ("new", "Scion", 1, 0, &[]),
    (
        "attack",
        "Marauder",
        90,
        20,
        &["Boneshatter", "Melee Physical Damage", "Brutality"],
    ),
    (
        "dot",
        "Witch",
        90,
        20,
        &["Essence Drain", "Contagion", "Void Manipulation"],
    ),
    (
        "minion",
        "Witch",
        90,
        20,
        &["Raise Zombie", "Minion Damage", "Melee Splash"],
    ),
];

/// The saved XML must load back into the same build as the recipe made.
#[test]
#[ignore = "needs the pinned PoB install"]
fn committed_xml_loads_as_described() {
    let _guard = lock();
    for (name, class, level, nodes, gems) in RECIPE_SHAPES {
        let xml = fixtures_dir().join(format!("{name}.xml"));
        let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
        load_xml(&lua, &fs::read_to_string(&xml).unwrap(), name).expect("loading XML");

        let (got_class, got_level, got_nodes, got_gems): (String, u32, u32, String) = lua
            .load(
                r#"
                local n = 0
                for _, node in pairs(build.spec.allocNodes) do
                    if node.type ~= "ClassStart" then n = n + 1 end
                end
                local gems = {}
                for _, group in ipairs(build.skillsTab.socketGroupList) do
                    for _, gem in ipairs(group.gemList) do gems[#gems + 1] = gem.nameSpec end
                end
                return build.spec.curClassName, build.characterLevel, n, table.concat(gems, ",")
                "#,
            )
            .eval()
            .expect("querying the build");
        assert_eq!(got_class, class, "{name}: class");
        assert_eq!(got_level, level, "{name}: level");
        assert_eq!(got_nodes, nodes, "{name}: allocated nodes");
        assert_eq!(got_gems, gems.join(","), "{name}: gems");

        let reloaded = golden_of(&lua, Game::Poe1.pob_version()).expect("reading stats");
        assert_eq!(reloaded, golden(name), "{name}: XML and golden data differ");

        // offence fixtures must have real damage numbers, or parity checks pass on zeros
        let dps = |stats: Option<&std::collections::BTreeMap<String, f64>>| {
            stats
                .and_then(|s| s.get("TotalDPS"))
                .copied()
                .unwrap_or(0.0)
        };
        match name {
            "attack" => assert!(dps(Some(&reloaded.stats)) > 0.0, "attack has no TotalDPS"),
            "minion" => assert!(
                dps(reloaded.minion_stats.as_ref()) > 0.0,
                "minion has no TotalDPS"
            ),
            _ => {}
        }
    }
}

// ---- builds and stats through PobBackend ----

/// A backend over a fresh temp user folder, which is removed when this drops.
struct Session {
    backend: PobBackend,
    user: PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.user);
    }
}

impl Session {
    fn start(test: &str) -> Session {
        Session::start_on(test, &pob_dir())
    }

    /// `start` on another PoB folder, such as a `timeless::farm`
    fn start_on(test: &str, pob: &Path) -> Session {
        let user =
            std::env::temp_dir().join(format!("auto-pob-gate-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&user);
        // an empty canned web: a request a test did not mean to make fails with
        // `no canned response` instead of reaching the internet
        let net = net_harness::canned(&net_harness::canned_web());
        let backend = PobBackend::start_with(Game::Poe1, pob, &user, net).expect("starting PoB");
        Session { backend, user }
    }

    /// Applies a command, then recalculates when it asks for that, as the engine loop does.
    fn run(&mut self, command: Command) -> anyhow::Result<auto_pob::engine::Affected> {
        let affected = self.backend.apply(command, &|_| {})?;
        if affected.recalc {
            self.backend.recalculate()?;
        }
        Ok(affected)
    }

    fn config(&mut self) -> ConfigView {
        self.backend.config_view().expect("config view")
    }

    fn builds_dir(&self) -> PathBuf {
        let path: String = self
            .backend
            .lua
            .load("return pob.main.main.buildPath")
            .eval()
            .unwrap();
        let dir = PathBuf::from(path);
        assert!(
            dir.starts_with(&self.user),
            "Builds folder {} is outside the user folder {}",
            dir.display(),
            self.user.display()
        );
        dir
    }

    fn stats(&mut self) -> StatsView {
        self.backend.stats_view().expect("stats view")
    }

    /// Copies the fixture into the temp Builds folder and opens it.
    fn open_fixture(&mut self, name: &str, xml: &Path) -> PathBuf {
        let dest = self.builds_dir().join(format!("{name}.xml"));
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(xml, &dest).unwrap();
        self.run(Command::OpenBuild { path: dest.clone() })
            .expect("open");
        dest
    }
}

fn assert_view_matches(
    name: &str,
    what: &str,
    rows: &[auto_pob::engine::StatRow],
    golden: &std::collections::BTreeMap<String, f64>,
) {
    assert!(!rows.is_empty(), "{name}: no {what} rows");
    for row in rows {
        let want = golden
            .get(&row.key)
            .unwrap_or_else(|| panic!("{name}: {what} {} is not in the golden data", row.key));
        assert_eq!(row.value, *want, "{name}: {what} {}", row.key);
    }
}

/// `StripEscapes`: removes `^x` + 6 hex digits and `^` + a digit.
fn strip_escapes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '^' {
            if chars.get(i + 1) == Some(&'x')
                && chars.len() >= i + 8
                && chars[i + 2..i + 8].iter().all(|c| c.is_ascii_hexdigit())
            {
                i += 8;
                continue;
            }
            if chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                i += 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A row's notes in the golden data's shape (no colour codes).
fn plain_notes(notes: &[String]) -> Vec<String> {
    notes.iter().map(|n| strip_escapes(n)).collect()
}

/// The rows must be exactly the lines PoB's own sidebar showed, in order.
fn assert_sidebar(
    name: &str,
    what: &str,
    rows: &[auto_pob::engine::StatRow],
    sidebar: &[auto_pob::engine::upstream::SidebarLine],
) {
    let got: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.label.as_str(), r.text.as_str()))
        .collect();
    let want: Vec<(&str, &str)> = sidebar
        .iter()
        .map(|l| (l.label.as_str(), l.text.as_str()))
        .collect();
    assert_eq!(got, want, "{name}: {what} sidebar lines");
    let got_notes: Vec<Vec<String>> = rows.iter().map(|r| plain_notes(&r.notes)).collect();
    let want_notes: Vec<&Vec<String>> = sidebar.iter().map(|l| &l.notes).collect();
    assert_eq!(
        got_notes.iter().collect::<Vec<_>>(),
        want_notes,
        "{name}: {what} sidebar notes"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn stats_match_pob() {
    let _guard = lock();
    for (name, xml) in fixtures() {
        let mut s = Session::start(&format!("stats-{name}"));
        s.open_fixture(&name, &xml);
        let view = s.stats();
        let want = golden(&name);
        assert_view_matches(&name, "stat", &view.stats, &want.stats);
        assert_sidebar(&name, "stat", &view.stats, &want.sidebar);
        match (&want.minion_stats, &want.minion_sidebar) {
            (Some(minion), Some(sidebar)) => {
                assert_view_matches(&name, "minion stat", &view.minion, minion);
                assert_sidebar(&name, "minion stat", &view.minion, sidebar);
            }
            _ => assert!(view.minion.is_empty(), "{name}: unexpected minion rows"),
        }
    }
}

fn attack_xml() -> String {
    fs::read_to_string(fixtures_dir().join("attack.xml")).unwrap()
}

fn share_code(s: &Session, xml: &str) -> String {
    let f: mlua::Function = s
        .backend
        .lua
        .load("return function(xml) return (common.base64.encode(Deflate(xml)):gsub('+','-'):gsub('/','_')) end")
        .eval()
        .unwrap();
    f.call(xml).unwrap()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn share_code_matches_xml() {
    let _guard = lock();
    let mut s = Session::start("share-code");
    let code = share_code(&s, &attack_xml());
    s.run(Command::ImportShareCode { code }).expect("import");
    let view = s.stats();
    assert_view_matches("attack", "stat", &view.stats, &golden("attack").stats);
    let build = s.backend.build_view().unwrap();
    assert_eq!(build.name, "Imported build");
    assert_eq!(build.path, None);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn import_share_code_trims_and_rejects_urls() {
    let _guard = lock();
    let mut s = Session::start("share-trim");
    let code = share_code(&s, &attack_xml());
    s.run(Command::ImportShareCode {
        code: format!("  \n{code}\n "),
    })
    .expect("trimmed code");
    assert_view_matches("attack", "stat", &s.stats().stats, &golden("attack").stats);
    let err = s
        .run(Command::ImportShareCode {
            code: "https://pobb.in/abc".into(),
        })
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("that's a link; use Import from URL"),
        "{err:#}"
    );
}

fn save_as(name: &str, overwrite: bool) -> Command {
    Command::SaveAs {
        folder: String::new(),
        name: name.into(),
        overwrite,
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn save_as_refuses_existing_without_overwrite() {
    let _guard = lock();
    let mut s = Session::start("save-as");
    s.run(Command::NewBuild).unwrap();
    s.run(save_as("x", false)).expect("first save");
    assert!(s.builds_dir().join("x.xml").exists());
    let err = s.run(save_as("x", false)).unwrap_err();
    assert!(format!("{err:#}").contains("exists"), "{err:#}");
    s.run(save_as("x", true)).expect("overwrite");
    // plain Save writes to the file the build now has
    s.run(Command::Save).expect("save");
    let build = s.backend.build_view().unwrap();
    assert_eq!(build.name, "x");
    assert!(!build.unsaved);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn save_needs_a_file() {
    let _guard = lock();
    let mut s = Session::start("save-nofile");
    s.run(Command::NewBuild).unwrap();
    let err = s.run(Command::Save).unwrap_err();
    assert!(
        format!("{err:#}").contains("build has no file yet"),
        "{err:#}"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn builds_with_unusual_names_round_trip() {
    let _guard = lock();
    let mut s = Session::start("unusual-names");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let before = s.stats();
    s.run(Command::SaveAs {
        folder: "Sub Folder/".into(),
        name: "Ætheric – Build".into(),
        overwrite: false,
    })
    .expect("save as");
    s.run(Command::NewBuild).unwrap();
    s.run(Command::ListBuilds {
        folder: "Sub Folder/".into(),
    })
    .unwrap();
    let list = s.backend.build_list_view().unwrap();
    assert_eq!(list.folder, "Sub Folder/");
    let entry = list
        .entries
        .iter()
        .find(|e| e.name == "Ætheric – Build")
        .unwrap_or_else(|| panic!("not listed: {:?}", list.entries));
    assert!(!entry.is_folder);
    assert_eq!(entry.class.as_deref(), Some("Marauder"));
    assert_eq!(entry.level, Some(90));
    s.run(Command::OpenBuild {
        path: entry.path.clone(),
    })
    .expect("reopen");
    assert_eq!(s.stats(), before);
    assert_eq!(s.backend.build_view().unwrap().name, "Ætheric – Build");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn list_shows_folders_first_with_headers() {
    let _guard = lock();
    let mut s = Session::start("list");
    let builds = s.builds_dir();
    fs::create_dir_all(builds.join("Zeta folder")).unwrap();
    fs::create_dir_all(builds.join("Alpha folder")).unwrap();
    for name in ["attack", "dot"] {
        fs::copy(
            fixtures_dir().join(format!("{name}.xml")),
            builds.join(format!("{name}.xml")),
        )
        .unwrap();
    }
    fs::copy(
        fixtures_dir().join("minion.xml"),
        builds.join("Alpha folder/minion.xml"),
    )
    .unwrap();
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    let list = s.backend.build_list_view().unwrap();
    let names: Vec<_> = list
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.is_folder))
        .collect();
    assert_eq!(
        names,
        [
            ("Alpha folder", true),
            ("Zeta folder", true),
            ("attack", false),
            ("dot", false)
        ]
    );
    let attack = &list.entries[2];
    assert_eq!(attack.class.as_deref(), Some("Marauder"));
    assert_eq!(attack.level, Some(90));
    assert_eq!(attack.path, builds.join("attack.xml"));
    s.run(Command::ListBuilds {
        folder: "Alpha folder/".into(),
    })
    .unwrap();
    let sub = s.backend.build_list_view().unwrap();
    assert_eq!(sub.entries.len(), 1);
    assert_eq!(sub.entries[0].name, "minion");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn engine_spawn_reaches_ready_with_snapshots() {
    let _guard = lock();
    // fail early without AUTO_POB_POB_DIR, rather than downloading PoB into the temp folder
    pob_dir();
    let user = std::env::temp_dir().join(format!("auto-pob-gate-spawn-{}", std::process::id()));
    let _ = fs::remove_dir_all(&user);
    let engine = Engine::spawn(Game::Poe1, std::env::temp_dir(), user.clone(), || {});
    let (mut ready, mut build, mut list, mut stats) = (false, false, false, false);
    let mut config = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !(ready && build && list && stats && config) && std::time::Instant::now() < deadline {
        match engine
            .events
            .recv_timeout(std::time::Duration::from_secs(30))
        {
            Ok(Event::Ready) => ready = true,
            Ok(Event::Snapshot {
                snapshot: Snapshot::Build(_),
                ..
            }) => build = true,
            Ok(Event::Snapshot {
                snapshot: Snapshot::BuildList(_),
                ..
            }) => list = true,
            Ok(Event::Snapshot {
                snapshot: Snapshot::Stats(_),
                ..
            }) => stats = true,
            Ok(Event::Snapshot {
                snapshot: Snapshot::Config(_),
                ..
            }) => config = true,
            Ok(Event::Error { message, .. }) => panic!("startup error: {message}"),
            Ok(Event::StartupFailed { message }) => panic!("startup failed: {message}"),
            Ok(_) => {}
            Err(e) => panic!("no event: {e}"),
        }
    }
    assert!(
        ready && build && list && stats && config,
        "{ready} {build} {list} {stats} {config}"
    );
    engine.send(Command::Shutdown);
    // the engine thread has stopped once the channel disconnects; only then is the working
    // directory and the user folder free to go
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while engine
        .events
        .recv_timeout(std::time::Duration::from_secs(30))
        .is_ok()
        && std::time::Instant::now() < deadline
    {}
    assert!(!engine.is_alive(), "the engine didn't stop after Shutdown");
    let _ = fs::remove_dir_all(&user);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn engine_starting_on_the_build_list_raises_no_errors() {
    let _guard = lock();
    pob_dir();
    let user = std::env::temp_dir().join(format!("auto-pob-gate-listmode-{}", std::process::id()));
    let _ = fs::remove_dir_all(&user);
    // PoB keeps its settings in "Path of Building" under the user folder; Mode LIST is where
    // it starts after the user last left it on the build list
    let settings = user.join("Path of Building");
    fs::create_dir_all(settings.join("Builds")).unwrap();
    fs::write(
        settings.join("Settings.xml"),
        "<PathOfBuilding>\n\t<Mode mode=\"LIST\"/>\n</PathOfBuilding>\n",
    )
    .unwrap();
    let build = settings.join("Builds/minion.xml");
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/minion.xml"),
        &build,
    )
    .unwrap();
    let engine = Engine::spawn(Game::Poe1, std::env::temp_dir(), user.clone(), || {});
    let (mut ready, mut list, mut settings) = (false, false, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !(ready && list) && std::time::Instant::now() < deadline {
        match engine
            .events
            .recv_timeout(std::time::Duration::from_secs(30))
        {
            Ok(Event::Ready) => ready = true,
            Ok(Event::Snapshot {
                snapshot: Snapshot::BuildList(_),
                ..
            }) => list = true,
            Ok(Event::Snapshot {
                snapshot: Snapshot::Settings(_),
                ..
            }) => settings = true,
            Ok(Event::Error { message, .. }) => panic!("startup error: {message}"),
            Ok(Event::StartupFailed { message }) => panic!("startup failed: {message}"),
            Ok(_) => {}
            Err(e) => panic!("no event: {e}"),
        }
    }
    assert!(ready && list, "ready {ready}, list {list}");
    // nothing else follows: no error and no build view
    let quiet = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while let Some(left) = quiet.checked_duration_since(std::time::Instant::now()) {
        match engine.events.recv_timeout(left) {
            Ok(Event::Error { message, .. }) => panic!("error on the list: {message}"),
            Ok(Event::Snapshot { snapshot, .. }) => {
                // the list and the requests need no build (R10); neither do the settings
                // and the popup (Task 3, R12, R19: a main-owned session)
                settings |= matches!(snapshot, Snapshot::Settings(_));
                assert!(
                    matches!(
                        snapshot,
                        Snapshot::BuildList(_)
                            | Snapshot::Requests(_)
                            | Snapshot::Settings(_)
                            | Snapshot::Popup(_)
                    ),
                    "a build view went out with no build open"
                )
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(settings, "the settings are sent at startup with no build");
    // opening a build brings the build views, as before
    engine.send(Command::OpenBuild { path: build });
    let (mut build_view, mut stats, mut tree) = (false, false, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !(build_view && stats && tree) && std::time::Instant::now() < deadline {
        match engine
            .events
            .recv_timeout(std::time::Duration::from_secs(30))
        {
            Ok(Event::Snapshot { snapshot, .. }) => match snapshot {
                Snapshot::Build(_) => build_view = true,
                Snapshot::Stats(_) => stats = true,
                Snapshot::Tree(_) => tree = true,
                _ => {}
            },
            Ok(Event::Error { message, .. }) => panic!("error opening: {message}"),
            Ok(_) => {}
            Err(e) => panic!("no event: {e}"),
        }
    }
    assert!(build_view && stats && tree, "{build_view} {stats} {tree}");
    engine.send(Command::Shutdown);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while engine
        .events
        .recv_timeout(std::time::Duration::from_secs(30))
        .is_ok()
        && std::time::Instant::now() < deadline
    {}
    assert!(!engine.is_alive(), "the engine didn't stop after Shutdown");
    let _ = fs::remove_dir_all(&user);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn folder_without_trailing_slash_stays_inside_the_folder() {
    let _guard = lock();
    let mut s = Session::start("folder-slash");
    s.run(Command::NewBuild).unwrap();
    s.run(Command::SaveAs {
        folder: "Sub Folder".into(),
        name: "y".into(),
        overwrite: false,
    })
    .expect("save as");
    assert!(s.builds_dir().join("Sub Folder/y.xml").exists());
    s.run(Command::ListBuilds {
        folder: "Sub Folder".into(),
    })
    .unwrap();
    let list = s.backend.build_list_view().unwrap();
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].name, "y");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn failed_save_as_leaves_the_build_alone() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock();
    let mut s = Session::start("save-fails");
    s.run(Command::NewBuild).unwrap();
    s.run(save_as("keep", false)).unwrap();
    let before = s.backend.build_view().unwrap();
    let locked = s.builds_dir().join("Locked");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let result = s.run(Command::SaveAs {
        folder: "Locked/".into(),
        name: "z".into(),
        overwrite: false,
    });
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if fs::write(locked.join("probe"), "").is_ok() && result.is_ok() {
        panic!("save into a read-only folder unexpectedly worked");
    }
    result.unwrap_err();
    assert_eq!(s.backend.build_view().unwrap().path, before.path);
    assert_eq!(s.backend.build_view().unwrap().name, "keep");
}

// ---- PoB must not write build files behind the user's back ----

/// Every file under `dir` with its bytes, for before/after comparisons.
fn snapshot_dir(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, &mut out);
    out
}

fn fixture_xml(name: &str) -> PathBuf {
    fixtures()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"))
        .1
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn switching_builds_never_rewrites_files() {
    let _guard = lock();
    let mut s = Session::start("no-autosave-open");
    let a = s.open_fixture("attack", &fixture_xml("attack"));
    s.run(Command::SetConfig {
        var: "usePowerCharges".into(),
        value: ConfigValue::Bool(true),
    })
    .unwrap();
    let before = snapshot_dir(&s.builds_dir());
    let b = s.builds_dir().join("dot.xml");
    fs::copy(fixture_xml("dot"), &b).unwrap();
    let before = {
        let mut m = before;
        m.insert(b.clone(), fs::read(&b).unwrap());
        m
    };
    s.run(Command::OpenBuild { path: b }).expect("open B");
    assert_eq!(snapshot_dir(&s.builds_dir()), before, "A was rewritten");
    assert_eq!(fs::read(&a).unwrap(), before[&a]);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn new_build_never_rewrites_files() {
    let _guard = lock();
    let mut s = Session::start("no-autosave-new");
    s.open_fixture("attack", &fixture_xml("attack"));
    s.run(Command::SetConfig {
        var: "usePowerCharges".into(),
        value: ConfigValue::Bool(true),
    })
    .unwrap();
    let before = snapshot_dir(&s.builds_dir());
    s.run(Command::NewBuild).unwrap();
    assert_eq!(snapshot_dir(&s.builds_dir()), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn leaving_the_unnamed_build_drops_no_temp_file() {
    let _guard = lock();
    let mut s = Session::start("no-autosave-temp");
    let a = s.builds_dir().join("attack.xml");
    fs::create_dir_all(a.parent().unwrap()).unwrap();
    fs::copy(fixture_xml("attack"), &a).unwrap();
    let before = snapshot_dir(&s.builds_dir());
    s.run(Command::SetConfig {
        var: "usePowerCharges".into(),
        value: ConfigValue::Bool(true),
    })
    .unwrap();
    s.run(Command::OpenBuild { path: a }).unwrap();
    assert_eq!(snapshot_dir(&s.builds_dir()), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn a_garbage_build_file_is_an_error_and_does_not_wedge_the_engine() {
    let _guard = lock();
    let mut s = Session::start("garbage-build");
    let bad = s.builds_dir().join("bad.xml");
    fs::create_dir_all(bad.parent().unwrap()).unwrap();
    fs::write(&bad, "this is not xml at all").unwrap();
    let err = s.run(Command::OpenBuild { path: bad }).unwrap_err();
    let message = format!("{err:#}");
    // the engine still works afterwards
    s.open_fixture("attack", &fixture_xml("attack"));
    assert_view_matches("attack", "stat", &s.stats().stats, &golden("attack").stats);
    // and the error says why, not just "Error:"
    assert!(
        message.len() > "Error:".len() + 5 && !message.starts_with("Error:"),
        "{message}"
    );
}

/// A folder named like a build is a build PoB can't open, as on Windows where `io.open`
/// refuses a folder: the plain message, not a Lua error from inside its XML reader
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_folder_named_like_a_build_is_a_clean_error() {
    let _guard = lock();
    let mut s = Session::start("folder-build");
    let folder = s.builds_dir().join("folder.xml");
    fs::create_dir_all(&folder).unwrap();
    let err = s.run(Command::OpenBuild { path: folder }).unwrap_err();
    assert_eq!(format!("{err:#}"), "couldn't open that build");
    s.open_fixture("attack", &fixture_xml("attack"));
    assert_view_matches("attack", "stat", &s.stats().stats, &golden("attack").stats);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn recalculate_reports_frame_errors_once() {
    let _guard = lock();
    let mut s = Session::start("recalc-error");
    s.run(Command::NewBuild).unwrap();
    s.backend
        .lua
        .load(
            "local main = pob.main.main; local real = main.OnFrame; \
             main.OnFrame = function(...) main.OnFrame = real; error('boom from the frame') end",
        )
        .exec()
        .unwrap();
    let err = s.backend.recalculate().unwrap_err();
    assert!(
        format!("{err:#}").contains("boom from the frame"),
        "{err:#}"
    );
    // the failure was consumed, so it doesn't surface in the next, unrelated frame
    s.run(save_as("after", false))
        .expect("save after a failed recalc");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn failed_save_as_removes_the_folder_it_created() {
    let _guard = lock();
    let mut s = Session::start("save-fails-folder");
    s.run(Command::NewBuild).unwrap();
    s.backend
        .lua
        .load("pob.build.SaveDBFile = function() return 'disk full' end")
        .exec()
        .unwrap();
    let existing = s.builds_dir().join("Existing");
    fs::create_dir_all(&existing).unwrap();
    for folder in ["Fresh/", "Existing/"] {
        s.run(Command::SaveAs {
            folder: folder.into(),
            name: "z".into(),
            overwrite: false,
        })
        .unwrap_err();
    }
    assert!(!s.builds_dir().join("Fresh").exists(), "empty folder left");
    assert!(
        existing.is_dir(),
        "a folder that already existed was removed"
    );
}

// ---- config through PobBackend ----

fn option<'a>(view: &'a ConfigView, var: &str) -> &'a ConfigOption {
    view.sections
        .iter()
        .flat_map(|s| &s.options)
        .find(|o| o.var == var)
        .unwrap_or_else(|| panic!("config view has no option {var}"))
}

fn set(var: &str, value: ConfigValue) -> Command {
    Command::SetConfig {
        var: var.into(),
        value,
    }
}

/// Upstream PoB after the same three changes, made through the controls' own callbacks.
const UPSTREAM_CHANGES: &str = r#"
local ct = build.configTab
local check = ct.varControls.usePowerCharges
check.state = true
check.changeFunc(true)
local boss = ct.varControls.enemyIsBoss
for index, item in ipairs(boss.list) do
	if item.val == "Boss" then boss:SetSel(index) end
end
ct.varControls.playerCursedWithDespair:SetText("5", true)
build.buildFlag = true
runCallback("OnFrame")
"#;

fn three_changes() -> [Command; 3] {
    [
        set("usePowerCharges", ConfigValue::Bool(true)),
        set("enemyIsBoss", ConfigValue::Choice("Boss".into())),
        set("playerCursedWithDespair", ConfigValue::Number(5.0)),
    ]
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn config_matches_pob() {
    let _guard = lock();
    for (name, xml) in fixtures() {
        let mut s = Session::start(&format!("config-{name}"));
        s.open_fixture(&name, &xml);
        let view = s.config();
        let want = golden(&name);

        let mut shown: Vec<_> = view
            .sections
            .iter()
            .flat_map(|s| &s.options)
            .filter(|o| o.shown)
            .map(|o| o.var.clone())
            .collect();
        shown.sort();
        assert_eq!(shown, want.visible_config, "{name}: shown options");
        for var in &shown {
            assert_eq!(
                option(&view, var).value.as_ref(),
                want.config.get(var),
                "{name}: value of {var}"
            );
        }

        // every option that takes input is listed, shown or not
        let inputs: Vec<String> = s
            .backend
            .lua
            .load(
                r#"
                local vars = {}
                local types = { check = 1, count = 1, countAllowZero = 1, integer = 1, float = 1, list = 1, text = 1 }
                for _, varData in ipairs(LoadModule("Modules/ConfigOptions")) do
                    if varData.var and types[varData.type] then vars[#vars + 1] = varData.var end
                end
                return vars
                "#,
            )
            .eval()
            .unwrap();
        assert!(
            inputs.len() > 100,
            "{name}: only {} input options",
            inputs.len()
        );
        for var in inputs {
            option(&view, &var);
        }
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn config_changes_recalculate() {
    let _guard = lock();
    let mut s = Session::start("config-changes");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let before = s.stats();
    for command in three_changes() {
        s.run(command).expect("set config");
    }
    let ours = s.stats();
    assert_ne!(ours, before, "the changes should move some stat");
    let view = s.config();
    assert_eq!(
        option(&view, "usePowerCharges").value,
        Some(serde_json::json!(true))
    );
    assert_eq!(
        option(&view, "enemyIsBoss").value,
        Some(serde_json::json!("Boss"))
    );
    assert_eq!(
        option(&view, "playerCursedWithDespair").value,
        Some(serde_json::json!(5))
    );

    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &fs::read_to_string(&xml).unwrap(), "attack").unwrap();
    lua.load(UPSTREAM_CHANGES).exec().expect("upstream changes");
    let theirs = golden_of(&lua, Game::Poe1.pob_version()).unwrap();
    assert_view_matches("attack", "stat", &ours.stats, &theirs.stats);
    assert_sidebar("attack", "stat", &ours.stats, &theirs.sidebar);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn dynamic_tooltips_are_recorded() {
    let _guard = lock();
    let mut s = Session::start("config-tooltips");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let none = option(&s.config(), "pantheonMajorGod").tooltip.clone();
    assert!(none.is_empty(), "{none:?}");
    s.run(set(
        "pantheonMajorGod",
        ConfigValue::Choice("Lunaris".into()),
    ))
    .unwrap();
    let view = s.config();
    let lines = &option(&view, "pantheonMajorGod").tooltip;
    assert!(lines.len() > 1, "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| matches!(l, auto_pob::engine::TooltipLine::Separator { .. })),
        "{lines:?}"
    );
    // a plain tooltip is one line of text
    let plain = option(&view, "buffAdrenaline");
    assert!(
        matches!(
            plain.tooltip.first(),
            Some(auto_pob::engine::TooltipLine::Line { size: 14, .. })
        ),
        "{:?}",
        plain.tooltip
    );
}

/// Every option's tooltip, recorded afresh as the view did before its lines were cached by
/// text: PoB's `tooltipFunc` or `tooltipText` into a recorder, each line stripped
const FRESH_CONFIG_TOOLTIPS: &str = r#"
    local tab = pob.build.configTab
    local out = {}
    for _, varData in ipairs(LoadModule("Modules/ConfigOptions")) do
        local control = varData.var and tab.varControls[varData.var]
        if control then
            local recorder = glue.tooltip()
            local ok
            if varData.tooltipFunc then
                local index = control.selIndex
                local value = control.list and index and control.list[index]
                ok = pcall(varData.tooltipFunc, recorder, "BODY", index, value)
            else
                local text
                ok, text = pcall(control.GetProperty, control, "tooltipText")
                if ok and text and text ~= "" then recorder:AddLine(14, text) end
            end
            local lines = {}
            if ok then
                for _, line in ipairs(recorder:lines()) do
                    if line.text then
                        lines[#lines + 1] = { size = line.size, text = StripEscapes(tostring(line.text)) }
                    else
                        lines[#lines + 1] = line
                    end
                end
            end
            out[#out + 1] = { var = varData.var, lines = lines }
        end
    end
    return out
"#;

/// The view's tooltips equal a fresh recording of each, plus the invalid line for an invalid
/// option; returns how many options have one
fn assert_config_tooltips_fresh(s: &mut Session, what: &str) -> usize {
    use auto_pob::engine::TooltipLine;
    #[derive(serde::Deserialize)]
    struct Fresh {
        var: String,
        #[serde(default)]
        lines: Vec<TooltipLine>,
    }
    let value: mlua::Value = s.backend.lua.load(FRESH_CONFIG_TOOLTIPS).eval().unwrap();
    let fresh: Vec<Fresh> = s.backend.lua.from_value(value).unwrap();
    let view = s.config();
    let mut with_lines = 0;
    for f in &fresh {
        let ours = option(&view, &f.var);
        let mut expected = f.lines.clone();
        if ours.invalid {
            expected.push(TooltipLine::Line {
                size: 14,
                text: "This config option is conditional with missing source and is invalid."
                    .into(),
            });
        }
        assert_eq!(ours.tooltip, expected, "{what}: {}", f.var);
        with_lines += usize::from(!expected.is_empty());
    }
    with_lines
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn config_tooltips_stay_fresh() {
    let _guard = lock();
    let mut s = Session::start("config-tooltips-fresh");
    let fixture = |name: &str| fixtures().into_iter().find(|(n, _)| n == name).unwrap().1;
    s.open_fixture("attack", &fixture("attack"));
    let first = assert_config_tooltips_fresh(&mut s, "open");
    assert!(first > 100, "only {first} options have a tooltip");
    // twice in a row: the second view takes the kept lines
    assert_config_tooltips_fresh(&mut s, "again");
    // a `tooltipFunc` tooltip that grows with the choice
    s.run(set(
        "pantheonMajorGod",
        ConfigValue::Choice("Lunaris".into()),
    ))
    .unwrap();
    assert_config_tooltips_fresh(&mut s, "pantheon");
    // an option set while its condition isn't met gets the invalid line on a kept tooltip
    s.run(set(
        "absolutionSkillDamageCountedOnce",
        ConfigValue::Bool(true),
    ))
    .unwrap();
    assert!(option(&s.config(), "absolutionSkillDamageCountedOnce").invalid);
    assert_config_tooltips_fresh(&mut s, "invalid");
    assert_config_tooltips_fresh(&mut s, "invalid again");
    s.run(Command::NewConfigSet { title: "B".into() }).unwrap();
    assert_config_tooltips_fresh(&mut s, "new set");
    s.run(Command::SelectConfigSet { id: 1 }).unwrap();
    assert_config_tooltips_fresh(&mut s, "set switch");
    s.run(Command::Undo(Tab::Config)).unwrap();
    assert_config_tooltips_fresh(&mut s, "undo");
    s.open_fixture("minion", &fixture("minion"));
    assert_config_tooltips_fresh(&mut s, "another build");
    s.open_fixture("attack", &fixture("attack"));
    assert_config_tooltips_fresh(&mut s, "back");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn config_sets_round_trip() {
    let _guard = lock();
    let mut s = Session::start("config-sets");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    let path = s.open_fixture("attack", &xml);
    let power = |s: &mut Session| option(&s.config(), "usePowerCharges").value.clone();

    let view = s.config();
    assert_eq!(view.active_set, 1);
    assert_eq!(view.sets.len(), 1);
    let first_power = power(&mut s);
    assert_ne!(first_power, Some(serde_json::json!(true)));

    s.run(Command::NewConfigSet {
        title: "Mapping".into(),
    })
    .unwrap();
    let view = s.config();
    assert_eq!(view.active_set, 2);
    assert_eq!(
        view.sets
            .iter()
            .map(|s| (s.id, s.title.as_str()))
            .collect::<Vec<_>>(),
        [(1, "Default"), (2, "Mapping")]
    );
    s.run(set("usePowerCharges", ConfigValue::Bool(true)))
        .unwrap();
    assert_eq!(power(&mut s), Some(serde_json::json!(true)));

    s.run(Command::SelectConfigSet { id: 1 }).unwrap();
    assert_eq!(s.config().active_set, 1);
    assert_eq!(power(&mut s), first_power);
    s.run(Command::SelectConfigSet { id: 2 }).unwrap();
    assert_eq!(power(&mut s), Some(serde_json::json!(true)));

    // save with set 2 active, then reopen
    s.run(Command::Save).unwrap();
    s.run(Command::NewBuild).unwrap();
    assert_eq!(s.config().sets.len(), 1);
    s.run(Command::OpenBuild { path }).unwrap();
    let view = s.config();
    assert_eq!(view.active_set, 2);
    assert_eq!(view.sets.len(), 2);
    assert_eq!(view.sets[1].title, "Mapping");
    assert_eq!(power(&mut s), Some(serde_json::json!(true)));
    s.run(Command::SelectConfigSet { id: 1 }).unwrap();
    assert_eq!(power(&mut s), first_power);

    // rename, delete and the numbering of new sets
    s.run(Command::RenameConfigSet {
        id: 2,
        title: "Maps".into(),
    })
    .unwrap();
    assert_eq!(s.config().sets[1].title, "Maps");
    s.run(Command::NewConfigSet { title: "B".into() }).unwrap();
    assert_eq!(s.config().active_set, 3);
    s.run(Command::DeleteConfigSet { id: 2 }).unwrap();
    let view = s.config();
    assert_eq!(view.sets.iter().map(|s| s.id).collect::<Vec<_>>(), [1, 3]);
    s.run(Command::NewConfigSet { title: "C".into() }).unwrap();
    assert_eq!(s.config().active_set, 2, "the lowest free id is reused");
    // deleting the active set moves to a neighbour; the last set can't go
    s.run(Command::DeleteConfigSet { id: 2 }).unwrap();
    assert_eq!(s.config().active_set, 3);
    s.run(Command::DeleteConfigSet { id: 3 }).unwrap();
    assert_eq!(s.config().active_set, 1);
    let err = s.run(Command::DeleteConfigSet { id: 1 }).unwrap_err();
    assert!(format!("{err:#}").contains("only config set"), "{err:#}");
    let err = s.run(Command::SelectConfigSet { id: 42 }).unwrap_err();
    assert!(format!("{err:#}").contains("no config set"), "{err:#}");
}

/// PoB's number boxes set their option to `tonumber(buf)` (ConfigTab.lua:273-280): a count of
/// "0" is stored, and "-" alone clears the option. What our Config tab sends for that text
/// leaves the build as upstream's own box does.
#[test]
#[ignore = "needs the pinned PoB install"]
fn config_number_text_matches_pob() {
    use auto_pob::engine::ConfigValue;
    use auto_pob::ui::config::{NumberInput, parse_number_input};
    let _guard = lock();
    let mut s = Session::start("config-number-text");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &attack_xml(), "attack").unwrap();
    let kind_of = |s: &mut Session, var: &str| {
        s.config()
            .sections
            .iter()
            .flat_map(|section| section.options.iter())
            .find(|o| o.var == var)
            .map(|o| (o.kind, o.allow_zero))
            .unwrap_or_else(|| panic!("no option {var}"))
    };
    let input = |code: &str| format!("local v = build.configTab.input.{code} return v");
    for (var, texts) in [
        ("overrideEmptyRedSockets", ["3", "0"]),
        ("enemyRadius", ["5", "-"]),
    ] {
        let (kind, allow_zero) = kind_of(&mut s, var);
        for text in texts {
            // what our box sends for the text
            let value = match parse_number_input(text, kind, allow_zero) {
                NumberInput::Empty => ConfigValue::Clear,
                NumberInput::Valid(n) => ConfigValue::Number(n),
                NumberInput::Invalid => panic!("{var} {text:?}: nothing sent"),
            };
            s.run(Command::SetConfig {
                var: var.into(),
                value,
            })
            .unwrap();
            lua.load(format!(
                r#"build.configTab.varControls.{var}:SetText({text:?}, true)
                runCallback("OnFrame")"#
            ))
            .exec()
            .unwrap();
            let ours: Option<f64> = s
                .backend
                .lua
                .load(format!(
                    "local v = pob.build.configTab.input.{var} return v"
                ))
                .eval()
                .unwrap();
            let theirs: Option<f64> = lua.load(input(var)).eval().unwrap();
            assert_eq!(ours, theirs, "{var} {text:?}");
        }
    }
    // the stored 0 shows as 0, and the cleared option shows nothing
    let shown = |s: &mut Session, var: &str| {
        s.config()
            .sections
            .iter()
            .flat_map(|section| section.options.iter())
            .find(|o| o.var == var)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(
        shown(&mut s, "overrideEmptyRedSockets"),
        Some(serde_json::json!(0))
    );
    assert_eq!(shown(&mut s, "enemyRadius"), None);
    let theirs = golden_of(&lua, Game::Poe1.pob_version()).unwrap();
    assert_view_matches("attack", "stat", &s.stats().stats, &theirs.stats);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn custom_mods_change_stats() {
    let _guard = lock();
    let mut s = Session::start("custom-mods");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let before = s.stats();
    let texts = |s: &mut Session| -> Vec<String> {
        s.config().custom_mods.into_iter().map(|g| g.text).collect()
    };
    assert_eq!(texts(&mut s), [""]);
    s.run(Command::SetCustomModText {
        group: 0,
        text: "+100 to maximum Life\n+10% increased Attack Speed".into(),
    })
    .unwrap();
    assert_eq!(
        texts(&mut s),
        ["+100 to maximum Life\n+10% increased Attack Speed"]
    );
    let after = s.stats();
    assert_ne!(after, before);

    // upstream, through the Custom Modifiers edit control
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &attack_xml(), "attack").unwrap();
    lua.load(
        r#"
        build.configTab.customModsBlockControls[1].controls.textEdit:SetText("+100 to maximum Life\n+10% increased Attack Speed", true)
        build.buildFlag = true
        runCallback("OnFrame")
        "#,
    )
    .exec()
    .unwrap();
    let theirs = golden_of(&lua, Game::Poe1.pob_version()).unwrap();
    assert_view_matches("attack", "stat", &after.stats, &theirs.stats);

    // custom mods are undoable like any other change
    s.run(Command::Undo(Tab::Config)).unwrap();
    assert_eq!(texts(&mut s), [""]);
    assert_eq!(s.stats(), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn custom_mod_groups_match_pob() {
    use auto_pob::engine::CustomModGroup;
    let _guard = lock();
    let mut s = Session::start("custom-groups");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    let path = s.open_fixture("attack", &xml);
    s.run(Command::SetCustomModText {
        group: 0,
        text: "+100 to maximum Life".into(),
    })
    .unwrap();
    let one = s.stats();
    s.run(Command::AddCustomModGroup).unwrap();
    s.run(Command::SetCustomModTitle {
        group: 1,
        title: "Speed".into(),
    })
    .unwrap();
    s.run(Command::SetCustomModText {
        group: 1,
        text: "+10% increased Attack Speed".into(),
    })
    .unwrap();
    s.run(Command::SetCustomModEnabled {
        group: 0,
        enabled: false,
    })
    .unwrap();
    let ours = s.stats();
    assert_ne!(ours, one);
    // PoB's box shows a line it reads whole as a modifier in MAGIC, else UNSUPPORTED
    // (ConfigTab.lua:78-86)
    let groups = vec![
        CustomModGroup {
            title: "Default".into(),
            enabled: false,
            text: "+100 to maximum Life".into(),
            line_colours: vec!["^x8888FF".into()],
        },
        CustomModGroup {
            title: "Speed".into(),
            enabled: true,
            text: "+10% increased Attack Speed".into(),
            // PoB's parseMod leaves the "+" over on this one: UNSUPPORTED
            line_colours: vec!["^xF05050".into()],
        },
    ];
    assert_eq!(s.config().custom_mods, groups);

    // upstream, through the same controls
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &attack_xml(), "attack").unwrap();
    lua.load(
        r#"
        local ct = build.configTab
        ct.customModsBlockControls[1].controls.textEdit:SetText("+100 to maximum Life", true)
        ct.controls.customModsAddBlock.onClick()
        ct.customModsBlockControls[2].controls.titleEdit:SetText("Speed", true)
        ct.customModsBlockControls[2].controls.textEdit:SetText("+10% increased Attack Speed", true)
        local check = ct.customModsBlockControls[1].controls.enableCheck
        check.state = false
        check.changeFunc(false)
        build.buildFlag = true
        runCallback("OnFrame")
        "#,
    )
    .exec()
    .unwrap();
    let theirs = golden_of(&lua, Game::Poe1.pob_version()).unwrap();
    assert_view_matches("attack", "stat", &ours.stats, &theirs.stats);

    // each line's colour is the one upstream's unselected box shows it in: its inactiveText,
    // without the empty line PoB adds after a last line with no line break
    let mixed = "+10 to Strength\nnot a modifier\n\n5% increased Attack Speed\n";
    let mut s2 = Session::start("custom-colours");
    s2.open_fixture("attack", &xml);
    s2.run(Command::SetCustomModText {
        group: 0,
        text: mixed.into(),
    })
    .unwrap();
    let ours_colours = s2.config().custom_mods[0].line_colours.clone();
    let mut theirs_colours: Vec<String> = lua
        .load(format!(
            r#"local edit = build.configTab.customModsBlockControls[1].controls.textEdit
            local out = {{}}
            for line in edit.inactiveText({mixed:?}):gmatch("([^\n]*)\n") do
                out[#out + 1] = line:match("^(%^x%x%x%x%x%x%x)")
            end
            return out"#
        ))
        .eval()
        .unwrap();
    theirs_colours.truncate(mixed.split('\n').count());
    assert_eq!(ours_colours, theirs_colours);
    assert_eq!(
        ours_colours,
        ["^x8888FF", "^xF05050", "^xF05050", "^x8888FF", "^xF05050"],
        "a line each of the text"
    );
    drop(s2);

    // they survive saving and reopening
    s.run(Command::Save).unwrap();
    s.run(Command::NewBuild).unwrap();
    s.run(Command::OpenBuild { path }).unwrap();
    assert_eq!(s.config().custom_mods, groups);
    assert_eq!(s.stats(), ours);

    // removing a group, and groups out of range
    s.run(Command::RemoveCustomModGroup { group: 0 }).unwrap();
    assert_eq!(s.config().custom_mods, groups[1..]);
    for command in [
        Command::SetCustomModText {
            group: 1,
            text: String::new(),
        },
        Command::RemoveCustomModGroup { group: 5 },
        Command::SetCustomModEnabled {
            group: 9,
            enabled: true,
        },
    ] {
        let err = s.run(command).unwrap_err();
        assert!(
            format!("{err:#}").contains("no custom modifier group"),
            "{err:#}"
        );
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn invalid_options_are_flagged() {
    use auto_pob::engine::TooltipLine;
    let _guard = lock();
    let mut s = Session::start("config-invalid");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let var = "absolutionSkillDamageCountedOnce";
    let view = s.config();
    assert!(!option(&view, var).shown);
    assert!(
        view.sections
            .iter()
            .flat_map(|s| &s.options)
            .all(|o| !o.invalid)
    );
    s.run(set(var, ConfigValue::Bool(true))).unwrap();
    let view = s.config();
    let ours = option(&view, var);
    assert!(ours.invalid);
    let Some(TooltipLine::Line { text, .. }) = ours.tooltip.last() else {
        panic!("{:?}", ours.tooltip)
    };

    // upstream's wrapper: border colour and the tooltip line it appends
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &attack_xml(), "attack").unwrap();
    let (red, line): (bool, String) = lua
        .load(format!(
            r#"
            local control = build.configTab.varControls.{var}
            control.state = true
            control.changeFunc(true)
            runCallback("OnFrame")
            local lines = {{}}
            local tooltip = {{ Clear = function() lines = {{}} end, lines = lines,
                AddLine = function(_, size, text) lines[#lines + 1] = text end,
                AddSeparator = function() end }}
            control.tooltipFunc(tooltip, "BODY")
            return control.borderFunc() == 0.753, StripEscapes(lines[#lines])
            "#
        ))
        .eval()
        .unwrap();
    assert!(red);
    assert_eq!(*text, line);
    // choices of a list option say which is selected
    let boss = option(&view, "enemyIsBoss");
    assert_eq!(boss.selected.as_deref(), Some("Pinnacle"));
    assert_eq!(
        option(&view, "resistancePenalty").selected.as_deref(),
        Some("-60")
    );
}

/// A Boss Skill Preset greys out "Enemy Damage Type" (ConfigOptions.lua:2286-2289), and
/// "None" gives it back (2319-2323). Ours shows it greyed out as PoB does and refuses a choice
/// there, which PoB's preset would put back anyway.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_greyed_out_option_is_greyed_out_as_in_pob() {
    let _guard = lock();
    let mut s = Session::start("config-greyed");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &attack_xml(), "attack").unwrap();
    let upstream_enabled = |preset: &str| -> bool {
        lua.load(format!(
            r#"
            local preset = build.configTab.varControls.presetBossSkills
            for index, item in ipairs(preset.list) do
                if item.val == "{preset}" then preset:SetSel(index) end
            end
            build.buildFlag = true
            runCallback("OnFrame")
            return build.configTab.varControls.enemyDamageType:IsEnabled() and true or false
            "#
        ))
        .eval()
        .unwrap()
    };
    let var = "enemyDamageType";
    assert!(option(&s.config(), var).enabled);
    let preset = option(&s.config(), "presetBossSkills")
        .choices
        .iter()
        .find(|c| c.value != "None")
        .expect("a boss skill")
        .value
        .clone();
    s.run(set("presetBossSkills", ConfigValue::Choice(preset.clone())))
        .unwrap();
    let view = s.config();
    assert!(!upstream_enabled(&preset), "upstream greys it out");
    assert!(!option(&view, var).enabled, "ours shows it greyed out");
    let before = option(&view, var).value.clone();
    let err = s
        .run(set(var, ConfigValue::Choice("Untyped".into())))
        .unwrap_err();
    assert!(err.to_string().contains("isn't available"), "{err}");
    assert_eq!(option(&s.config(), var).value, before);

    s.run(set("presetBossSkills", ConfigValue::Choice("None".into())))
        .unwrap();
    assert!(upstream_enabled("None"));
    assert!(option(&s.config(), var).enabled);
    s.run(set(var, ConfigValue::Choice("Untyped".into())))
        .unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn undo_redo() {
    let _guard = lock();
    let mut s = Session::start("config-undo");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let flags = |s: &mut Session| {
        let b = s.backend.build_view().unwrap();
        (b.can_undo, b.can_redo)
    };
    let boss = |s: &mut Session| option(&s.config(), "enemyIsBoss").value.clone();
    let before = s.stats();
    let original = boss(&mut s);
    assert_eq!(original, Some(serde_json::json!("Pinnacle")));
    assert_eq!(flags(&mut s), (false, false));

    s.run(set("enemyIsBoss", ConfigValue::Choice("Boss".into())))
        .unwrap();
    let changed = s.stats();
    assert_ne!(changed, before);
    assert_eq!(boss(&mut s), Some(serde_json::json!("Boss")));
    assert_eq!(flags(&mut s), (true, false));

    s.run(Command::Undo(Tab::Config)).unwrap();
    assert_eq!(boss(&mut s), original);
    assert_eq!(s.stats(), before);
    assert_eq!(flags(&mut s), (false, true));

    s.run(Command::Redo(Tab::Config)).unwrap();
    assert_eq!(boss(&mut s), Some(serde_json::json!("Boss")));
    assert_eq!(s.stats(), changed);
    assert_eq!(flags(&mut s), (true, false));

    // undo with nothing to undo is harmless
    s.run(Command::Undo(Tab::Config)).unwrap();
    s.run(Command::Undo(Tab::Config)).unwrap();
    assert_eq!(flags(&mut s), (false, true));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn clearing_an_option_goes_back_to_its_placeholder() {
    let _guard = lock();
    let mut s = Session::start("config-clear");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let before = s.stats();
    s.run(set("playerCursedWithDespair", ConfigValue::Number(5.0)))
        .unwrap();
    assert_eq!(
        option(&s.config(), "playerCursedWithDespair").value,
        Some(serde_json::json!(5))
    );
    s.run(set("playerCursedWithDespair", ConfigValue::Clear))
        .unwrap();
    let view = s.config();
    assert_eq!(option(&view, "playerCursedWithDespair").value, None);
    assert_eq!(s.stats(), before);
    // values of the wrong kind are refused
    let err = s
        .run(set("usePowerCharges", ConfigValue::Number(1.0)))
        .unwrap_err();
    assert!(format!("{err:#}").contains("usePowerCharges"), "{err:#}");
    let err = s
        .run(set("enemyIsBoss", ConfigValue::Choice("Nope".into())))
        .unwrap_err();
    assert!(format!("{err:#}").contains("enemyIsBoss"), "{err:#}");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn round_trip() {
    let _guard = lock();
    let mut s = Session::start("config-round-trip");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "dot").unwrap();
    s.open_fixture("dot", &xml);
    for command in three_changes() {
        s.run(command).expect("set config");
    }
    s.run(save_as("Round Trip", false)).expect("save as");
    let ours = s.stats();
    let view = s.config();

    let saved = fs::read_to_string(s.builds_dir().join("Round Trip.xml")).unwrap();
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &saved, "Round Trip").unwrap();
    let theirs = golden_of(&lua, Game::Poe1.pob_version()).unwrap();
    for var in ["usePowerCharges", "enemyIsBoss", "playerCursedWithDespair"] {
        assert_eq!(
            option(&view, var).value.as_ref(),
            theirs.config.get(var),
            "{var}"
        );
    }
    assert_eq!(theirs.config["enemyIsBoss"], "Boss");
    assert_view_matches("dot", "stat", &ours.stats, &theirs.stats);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn unknown_config_var_is_an_error() {
    let _guard = lock();
    let mut s = Session::start("config-unknown");
    s.run(Command::NewBuild).unwrap();
    let err = s.run(set("nope", ConfigValue::Bool(true))).unwrap_err();
    assert!(
        format!("{err:#}").contains("unknown config option nope"),
        "{err:#}"
    );
}

// ---- tree layout and state ----

/// What upstream PoB reports for the loaded build, as read from its own globals.
#[derive(Debug, PartialEq)]
struct UpstreamTree {
    alloc: Vec<u32>,
    class_id: u32,
    ascendancy_id: u32,
    spec_titles: Vec<String>,
    used: u32,
    available: u32,
    ascendancy_used: u32,
    ascendancy_available: u32,
}

fn upstream_tree(xml: &Path, name: &str) -> UpstreamTree {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &fs::read_to_string(xml).unwrap(), name).unwrap();
    // The point display is Build.lua's `pointDisplay` text: "used / max   ascendancy / max"
    let (alloc, class_id, ascendancy_id, titles, used, available, asc_used, asc_available): (
        Vec<u32>,
        u32,
        u32,
        Vec<String>,
        u32,
        u32,
        u32,
        u32,
    ) = lua
        .load(
            r#"
            local alloc = {}
            for id in pairs(build.spec.allocNodes) do alloc[#alloc + 1] = id end
            table.sort(alloc)
            local titles = {}
            for i, spec in ipairs(build.treeTab.specList) do titles[i] = spec.title or "Default" end
            local text = build.controls.pointDisplay.str:gsub("%^x%x%x%x%x%x%x", ""):gsub("%^%d", "")
            local used, available, ascUsed, ascAvailable =
                text:match("^%s*(%d+)%s*/%s*(%d+)%s+(%d+)%s*/%s*(%d+)")
            return alloc, build.spec.curClassId, build.spec.curAscendClassId, titles,
                tonumber(used), tonumber(available), tonumber(ascUsed), tonumber(ascAvailable)
            "#,
        )
        .eval()
        .expect("reading upstream tree");
    UpstreamTree {
        alloc,
        class_id,
        ascendancy_id,
        spec_titles: titles,
        used,
        available,
        ascendancy_used: asc_used,
        ascendancy_available: asc_available,
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_state_matches_pob() {
    let _guard = lock();
    let mut s = Session::start("tree-state");
    for (name, xml) in fixtures() {
        s.open_fixture(&name, &xml);
        let ours = s.backend.tree_state_view().expect("tree state");
        let theirs = upstream_tree(&xml, &name);
        let mut allocated = ours.allocated.clone();
        allocated.sort_unstable();
        assert_eq!(allocated, theirs.alloc, "{name}: allocated");
        assert_eq!(ours.class_id, theirs.class_id, "{name}: class");
        assert_eq!(
            ours.ascendancy_id, theirs.ascendancy_id,
            "{name}: ascendancy"
        );
        let titles: Vec<&str> = ours.specs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, theirs.spec_titles, "{name}: spec titles");
        let points = (
            ours.points.used,
            ours.points.available,
            ours.points.ascendancy_used,
            ours.points.ascendancy_available,
        );
        let want = (
            theirs.used,
            theirs.available,
            theirs.ascendancy_used,
            theirs.ascendancy_available,
        );
        assert_eq!(points, want, "{name}: points");
        assert_eq!(ours.specs[ours.active_spec].points, theirs.used, "{name}");
        assert!(ours.classes.windows(2).all(|w| w[0].name < w[1].name));
        assert!(
            ours.classes
                .iter()
                .any(|c| c.id == ours.class_id && !c.needs_confirm),
            "{name}: the current class never asks"
        );
        assert!(!ours.can_redo, "{name}: nothing to redo after loading");
        assert!(
            ours.classes.iter().all(|c| c
                .ascendancies
                .first()
                .is_some_and(|a| a.id == 0 && a.name == "None")),
            "{name}: ascendancy 0 is None"
        );
        // With points spent, a class the tree doesn't connect to asks before resetting it.
        assert_eq!(
            ours.classes.iter().any(|c| c.needs_confirm),
            ours.points.used > 0,
            "{name}: needs_confirm"
        );
        assert!(
            ours.mastery_options
                .iter()
                .any(|m| m.options.iter().any(|o| !o.text.is_empty())),
            "{name}: masteries offer effects"
        );
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn layout_includes_spec_only_nodes() {
    let _guard = lock();
    let mut s = Session::start("layout-nodes");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    // the fixture has no ascendancy yet; take the first one of its class
    s.run(Command::SelectAscendancy { id: 1 }).unwrap();
    let layout = s.backend.tree_layout_view().expect("layout");
    let state = s.backend.tree_state_view().expect("state");
    assert!(layout.nodes.len() > 2000, "{}", layout.nodes.len());
    assert!(layout.edges.len() > 2000);
    assert!(!layout.sheets.is_empty());
    assert!(
        layout.sheets.iter().all(Option::is_some),
        "every sprite sheet loads"
    );
    let ids: std::collections::HashSet<u32> = layout.nodes.iter().map(|n| n.id).collect();
    // the current ascendancy's id is what its nodes carry in `ascendancy`
    let current = state
        .classes
        .iter()
        .find(|c| c.id == state.class_id)
        .and_then(|c| c.ascendancies.iter().find(|a| a.id == state.ascendancy_id))
        .filter(|a| a.id != 0)
        .expect("the attack fixture has an ascendancy");
    assert!(
        layout
            .nodes
            .iter()
            .any(|n| n.ascendancy.as_deref() == Some(current.ascend_id.as_str())),
        "no node carries the ascendancy id {:?}",
        current.ascend_id
    );
    assert!(!state.allocated.is_empty());
    for id in &state.allocated {
        assert!(ids.contains(id), "allocated node {id} is not in the layout");
    }
    assert!(!state.paths.is_empty(), "the build can grow");
    for path in state.paths.iter() {
        assert!(ids.contains(&path.node), "reachable node {}", path.node);
        assert!(path.path.iter().all(|id| ids.contains(id)));
    }
    assert!(
        layout.nodes.iter().any(|n| n.ascendancy.is_some()),
        "ascendancy nodes are included"
    );
    assert!(layout.key.starts_with(&layout.version));
    for &(a, b) in &layout.edges {
        assert!(a < layout.nodes.len() && b < layout.nodes.len());
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn layout_resent_on_version_change() {
    let _guard = lock();
    let mut s = Session::start("layout-resent");
    let fixtures = fixtures();
    let (name, xml) = &fixtures[0];
    // nothing has been sent yet, so opening a build brings the layout
    let dest = s.builds_dir().join(format!("{name}.xml"));
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::copy(xml, &dest).unwrap();
    let affected = s.run(Command::OpenBuild { path: dest.clone() }).unwrap();
    assert!(affected.tree_layout && affected.tree);
    let layout = s.backend.tree_layout_view().unwrap();

    // the same tree again needs no new layout
    let affected = s.run(Command::OpenBuild { path: dest }).unwrap();
    assert!(!affected.tree_layout, "an unchanged layout isn't resent");
    assert!(affected.tree);

    // the same build on an older tree version has another key, so the layout is resent
    let older = s.builds_dir().join("older.xml");
    let text = fs::read_to_string(xml).unwrap();
    assert!(text.contains("treeVersion=\"3_29\""));
    fs::write(
        &older,
        text.replace("treeVersion=\"3_29\"", "treeVersion=\"3_27\""),
    )
    .unwrap();
    let affected = s
        .run(Command::OpenBuild {
            path: older.clone(),
        })
        .unwrap();
    assert!(affected.tree_layout, "a new tree version brings its layout");
    let older_layout = s.backend.tree_layout_view().unwrap();
    assert_eq!(older_layout.version, "3_27");
    assert_ne!(older_layout.key, layout.key);
    assert_eq!(s.backend.tree_state_view().unwrap().version, "3_27");

    // and going back resends again, while repeating it does not
    let dest = s.builds_dir().join(format!("{name}.xml"));
    let affected = s.run(Command::OpenBuild { path: dest.clone() }).unwrap();
    assert!(affected.tree_layout);
    assert_eq!(s.backend.tree_layout_view().unwrap().key, layout.key);
    let affected = s.run(Command::OpenBuild { path: dest }).unwrap();
    assert!(!affected.tree_layout);
}

/// The stat lines the tree gives a mastery effect.
fn effect_stats(s: &Session, effect: u32) -> Vec<String> {
    s.backend
        .lua
        .load(format!(
            "return pob.build.spec.tree.masteryEffects[{effect}].sd"
        ))
        .eval()
        .unwrap()
}

fn layout_stats(layout: &auto_pob::engine::TreeLayout, node: u32) -> Vec<String> {
    layout
        .nodes
        .iter()
        .find(|n| n.id == node)
        .unwrap_or_else(|| panic!("node {node} is not in the layout"))
        .stats
        .clone()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn choosing_a_mastery_effect_resends_the_layout() {
    let _guard = lock();
    let mut s = attack_session("layout-mastery");
    let before = s.backend.tree_layout_view().unwrap();
    let mastery = upstream_node("attack", "pick.mastery()");
    let effect: u32 = upstream_with_tree_lib("attack")
        .load("return pick.mastery_effects(pick.mastery())[2]")
        .eval()
        .unwrap();
    // nothing else has changed: another edit leaves the layout alone
    let affected = s
        .run(Command::AllocNode {
            id: upstream_node("attack", "pick.notable(2)"),
        })
        .unwrap();
    assert!(!affected.tree_layout, "an unrelated allocation");
    let affected = s
        .run(Command::SetMastery {
            node: mastery,
            effect,
        })
        .unwrap();
    assert!(affected.tree_layout, "the mastery's text changed");
    let after = s.backend.tree_layout_view().unwrap();
    assert_ne!(after.key, before.key);
    assert!(after.key.starts_with(&after.version));
    let wanted = effect_stats(&s, effect);
    assert!(!wanted.is_empty());
    assert_eq!(layout_stats(&after, mastery), wanted);
    // a repeat of the same choice has nothing new to send
    let affected = s
        .run(Command::SetMastery {
            node: mastery,
            effect,
        })
        .unwrap();
    assert!(!affected.tree_layout);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn opening_a_build_after_the_blank_start_up_build_brings_its_content() {
    let _guard = lock();
    // a build with a mastery effect chosen, saved in a throw-away folder
    let mut first = attack_session("layout-open-source");
    let mastery = upstream_node("attack", "pick.mastery()");
    let effect: u32 = upstream_with_tree_lib("attack")
        .load("return pick.mastery_effects(pick.mastery())[2]")
        .eval()
        .unwrap();
    first
        .run(Command::SetMastery {
            node: mastery,
            effect,
        })
        .unwrap();
    first.run(Command::Save).unwrap();
    let saved = fs::read(first.builds_dir().join("attack.xml")).unwrap();

    // the engine's first layout comes from the blank build it starts with
    let mut s = Session::start("layout-open");
    let blank = s.backend.tree_layout_view().unwrap();
    let path = s.builds_dir().join("chosen.xml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, saved).unwrap();
    let affected = s.run(Command::OpenBuild { path }).unwrap();
    assert!(affected.tree_layout, "the build's tree reads differently");
    let layout = s.backend.tree_layout_view().unwrap();
    assert_ne!(layout.key, blank.key);
    assert_eq!(layout_stats(&layout, mastery), effect_stats(&s, effect));
    // every node's name and stats are what the build's own tree holds
    let lua = &s.backend.lua;
    for node in &layout.nodes {
        let (name, stats): (Option<String>, Vec<String>) = lua
            .load(format!(
                "local n = pob.build.spec.nodes[{}]; return n.dn, n.sd or {{}}",
                node.id
            ))
            .eval()
            .unwrap();
        assert_eq!(node.name, name.unwrap_or_default(), "node {}", node.id);
        assert_eq!(node.stats, stats, "node {}", node.id);
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn layout_nodes_are_sorted_by_id_and_the_key_is_cheap() {
    let _guard = lock();
    let mut s = attack_session("layout-key-cost");
    let layout = s.backend.tree_layout_view().unwrap();
    assert!(layout.nodes.windows(2).all(|w| w[0].id < w[1].id));
    let digest: mlua::Function = s
        .backend
        .lua
        .load("return glue.tree_layout_digest")
        .eval()
        .unwrap();
    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _: mlua::LuaString = digest.call(()).unwrap();
    }
    let each = start.elapsed() / 20;
    eprintln!("layout key digest: {each:?} per command on attack");
    assert!(each < std::time::Duration::from_millis(50), "{each:?}");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_editing_never_writes_files() {
    let _guard = lock();
    let mut s = Session::start("tree-no-writes");
    let fixtures = fixture_xml("attack");
    let text = fs::read_to_string(&fixtures).unwrap();
    // an older tree for the conversion, on disk before the first snapshot
    let older = s.builds_dir().join("older.xml");
    fs::create_dir_all(older.parent().unwrap()).unwrap();
    fs::write(
        &older,
        text.replace("treeVersion=\"3_29\"", "treeVersion=\"3_27\""),
    )
    .unwrap();
    s.open_fixture("attack", &fixtures);
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    let notable = upstream_node("attack", "pick.notable(2)");
    let mastery = upstream_node("attack", "pick.mastery()");
    let effect: u32 = upstream_with_tree_lib("attack")
        .load("return pick.mastery_effects(pick.mastery())[2]")
        .eval()
        .unwrap();
    let witch = s
        .backend
        .tree_state_view()
        .unwrap()
        .classes
        .iter()
        .find(|c| c.name == "Witch")
        .unwrap()
        .id;
    let commands = vec![
        Command::AllocNode { id: notable },
        Command::SetMastery {
            node: mastery,
            effect,
        },
        Command::Undo(Tab::Tree),
        Command::Redo(Tab::Tree),
        Command::SelectClass {
            id: witch,
            connect: false,
        },
        Command::ResetTree,
        Command::NewSpec {
            title: "Extra".into(),
        },
        Command::RenameSpec {
            index: 1,
            title: "Renamed".into(),
        },
        Command::SelectSpec { index: 1 },
        Command::SetCompareSpec { index: Some(0) },
        Command::SelectSpec { index: 0 },
        Command::DeleteSpec { index: 1 },
        search("life"),
    ];
    for command in commands {
        let shown = format!("{command:?}");
        s.run(command).unwrap_or_else(|e| panic!("{shown}: {e}"));
    }
    // a heat map, calculated to the end
    run_power(&mut s, "Life", Some(1));
    // converting needs an older tree
    s.run(Command::OpenBuild { path: older }).unwrap();
    let latest = {
        let latest: String = s
            .backend
            .lua
            .load("return latestTreeVersion")
            .eval()
            .unwrap();
        latest
    };
    s.run(Command::ConvertSpec {
        version: latest,
        keep_copy: true,
    })
    .unwrap();
    assert_eq!(s.backend.tree_state_view().unwrap().specs.len(), 2);
    let after = snapshot_dir(&dir);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "files appeared or went"
    );
    assert!(
        snapshot_dir(&s.user)
            .keys()
            .all(|p| !p.to_string_lossy().contains("~~temp~~")),
        "a temporary file was left"
    );
    for (path, bytes) in &before {
        assert!(after[path] == *bytes, "{} changed", path.display());
    }
}

// ---- tree editing ----

/// One recorded step of a tree recipe, as upstream's `act.*` helpers log it in the global
/// `actions`; each maps onto one of our commands.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
enum TreeAction {
    Alloc {
        alloc: u32,
    },
    Dealloc {
        dealloc: u32,
    },
    Class {
        class: u32,
        connect: bool,
    },
    Ascend {
        ascend: u32,
    },
    Reset {
        reset: bool,
    },
    Mastery {
        mastery: u32,
        effect: u32,
    },
    Undo {
        undo: bool,
    },
    Redo {
        redo: bool,
    },
    NewSpec {
        new_spec: String,
    },
    SelectSpec {
        select_spec: usize,
    },
    RenameSpec {
        rename_spec: usize,
        title: String,
    },
    DeleteSpec {
        delete_spec: usize,
    },
    Compare {
        compare: CompareTarget,
    },
    Convert {
        convert: String,
        keep_copy: bool,
    },
    ConvertAll {
        convert_all: String,
    },
    Import {
        import: String,
        title: String,
    },
    /// "Remove All Tattoos" on the Reset window
    RemoveAllTattoos {
        remove_all_tattoos: bool,
    },
    /// Recorded by popup_lib's `act.tattoo_open`, `act.popup_set`, `act.popup_press` and
    /// `act.popup_close`
    Popup(PopupAction),
    /// Not a command: upstream's tree state at this point in the recipe.
    Check {
        check: Checkpoint,
    },
}

/// A popup step of a tree recipe, as popup_lib records it.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "cmd")]
enum PopupAction {
    OpenPopup {
        kind: auto_pob::engine::PopupKind,
    },
    SetPopupControl {
        name: String,
        value: auto_pob::engine::PopupInput,
    },
    PressPopupButton {
        name: String,
    },
    ClosePopup,
}

/// A spec index, or `false` for "stop comparing" (the recipe's `act.compare`).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
enum CompareTarget {
    Spec(usize),
    Off(bool),
}

/// The part of the tree state a recipe's `checkpoint()` records.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct Checkpoint {
    allocated: Vec<u32>,
    /// The active tree's version, so a conversion to the wrong one is noticed
    tree_version: String,
    spec_titles: Vec<String>,
    active_spec: usize,
    #[serde(deserialize_with = "colours")]
    compare: Vec<CompareColour>,
    /// The sidebar, when the recipe asked for it (`checkpoint{ sidebar = true }`)
    #[serde(default)]
    sidebar: Option<Vec<auto_pob::engine::upstream::SidebarLine>>,
    /// The tattoo state, when the recipe asked for it (`checkpoint{ tattoos = true }`)
    #[serde(default)]
    tattoos: Option<TattooState>,
}

/// What `checkpoint{ tattoos = true }` records.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct TattooState {
    /// (node, tattoo name), sorted by node
    #[serde(deserialize_with = "lua_list")]
    overrides: Vec<OverrideRecord>,
    /// The stat lines of the nodes the recipe opened
    #[serde(deserialize_with = "lua_list")]
    touched: Vec<TouchedNode>,
    /// The nodes PoB's right-click offers a replacement for, and the masteries with effects
    #[serde(deserialize_with = "lua_list")]
    replaceable: Vec<u32>,
    #[serde(deserialize_with = "lua_list")]
    mastery_effects: Vec<u32>,
    undo_depth: usize,
    can_undo: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct OverrideRecord {
    id: u32,
    dn: String,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct TouchedNode {
    id: u32,
    #[serde(deserialize_with = "lua_list")]
    sd: Vec<String>,
}

/// A list from Lua, where an empty one arrives as an empty map.
fn lua_list<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum List<T> {
        Items(Vec<T>),
        Empty(#[allow(dead_code)] std::collections::BTreeMap<String, ()>),
    }
    Ok(match serde::Deserialize::deserialize(d)? {
        List::Items(items) => items,
        List::Empty(_) => Vec::new(),
    })
}

/// A list from Lua, where an empty one arrives as an empty map.
fn colours<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<CompareColour>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum List {
        Items(Vec<CompareColour>),
        Empty(#[allow(dead_code)] std::collections::BTreeMap<String, ()>),
    }
    Ok(match serde::Deserialize::deserialize(d)? {
        List::Items(items) => items,
        List::Empty(_) => Vec::new(),
    })
}

impl TreeAction {
    /// The command that replays this action; none for a checkpoint.
    fn command(&self) -> Option<Command> {
        Some(match self.clone() {
            TreeAction::Alloc { alloc } => Command::AllocNode { id: alloc },
            TreeAction::Dealloc { dealloc } => Command::DeallocNode { id: dealloc },
            TreeAction::Class { class, connect } => Command::SelectClass { id: class, connect },
            TreeAction::Ascend { ascend } => Command::SelectAscendancy { id: ascend },
            TreeAction::Reset { reset } => {
                assert!(reset);
                Command::ResetTree
            }
            TreeAction::Mastery { mastery, effect } => Command::SetMastery {
                node: mastery,
                effect,
            },
            TreeAction::Undo { undo } => {
                assert!(undo);
                Command::Undo(Tab::Tree)
            }
            TreeAction::Redo { redo } => {
                assert!(redo);
                Command::Redo(Tab::Tree)
            }
            TreeAction::NewSpec { new_spec } => Command::NewSpec { title: new_spec },
            TreeAction::SelectSpec { select_spec } => Command::SelectSpec { index: select_spec },
            TreeAction::RenameSpec { rename_spec, title } => Command::RenameSpec {
                index: rename_spec,
                title,
            },
            TreeAction::DeleteSpec { delete_spec } => Command::DeleteSpec { index: delete_spec },
            TreeAction::Compare { compare } => Command::SetCompareSpec {
                index: match compare {
                    CompareTarget::Spec(index) => Some(index),
                    CompareTarget::Off(off) => {
                        assert!(!off);
                        None
                    }
                },
            },
            TreeAction::Convert { convert, keep_copy } => Command::ConvertSpec {
                version: convert,
                keep_copy,
            },
            TreeAction::ConvertAll { convert_all } => Command::ConvertAllSpecs {
                version: convert_all,
            },
            TreeAction::Import { import, title } => Command::ImportTreeLink { title, url: import },
            TreeAction::RemoveAllTattoos { remove_all_tattoos } => {
                assert!(remove_all_tattoos);
                Command::RemoveAllTattoos
            }
            TreeAction::Popup(PopupAction::OpenPopup { kind }) => Command::OpenPopup(kind),
            TreeAction::Popup(PopupAction::SetPopupControl { name, value }) => {
                Command::SetPopupControl { name, value }
            }
            TreeAction::Popup(PopupAction::PressPopupButton { name }) => {
                Command::PressPopupButton { name }
            }
            TreeAction::Popup(PopupAction::ClosePopup) => Command::ClosePopup,
            TreeAction::Check { .. } => return None,
        })
    }
}

/// What upstream PoB shows for the tree after a recipe.
#[derive(Debug, PartialEq)]
struct TreeResult {
    /// Sorted
    allocated: Vec<u32>,
    /// The active tree's version
    tree_version: String,
    class_id: u32,
    ascendancy_id: u32,
    /// (node, effect), sorted
    masteries: Vec<(u32, u32)>,
    spec_titles: Vec<String>,
    /// 0-based
    active_spec: usize,
    /// Sorted by node
    compare: Vec<CompareColour>,
    sidebar: Vec<auto_pob::engine::upstream::SidebarLine>,
    /// Node to the name of the tattoo on it
    overrides: std::collections::BTreeMap<u32, String>,
    can_undo: bool,
}

/// A recipe run in upstream PoB: the steps it took and where they ended.
struct TreeRecipe {
    base: String,
    /// The build to start from when the recipe prepared one (saved after its setup)
    setup_xml: Option<String>,
    actions: Vec<TreeAction>,
    result: TreeResult,
    /// The saved build at the end, for a recipe that sets `save_xml`
    xml: Option<String>,
}

/// Loads a fixture into fresh upstream PoB and defines the recipe helpers from `tree_lib.lua`.
fn upstream_with_tree_lib(base: &str) -> mlua::Lua {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    let xml = fixtures_dir().join(format!("{base}.xml"));
    load_xml(&lua, &fs::read_to_string(xml).unwrap(), base).unwrap();
    load_tree_lib(&lua);
    // the popup steps of the tattoo recipe, and the sidebar a checkpoint can record
    lua.load(fs::read_to_string(fixtures_dir().join("popup_lib.lua")).unwrap())
        .set_name("=popup_lib")
        .exec()
        .expect("popup_lib.lua");
    lua.globals()
        .set(
            "goldenSidebar",
            lua.create_function(|lua, ()| {
                let golden = golden_of(lua, Game::Poe1.pob_version())
                    .map_err(|e| mlua::Error::external(e.to_string()))?;
                lua.to_value(&golden.sidebar)
            })
            .unwrap(),
        )
        .unwrap();
    lua
}

fn load_tree_lib(lua: &mlua::Lua) {
    lua.load(fs::read_to_string(fixtures_dir().join("tree_lib.lua")).unwrap())
        .set_name("=tree_lib")
        .exec()
        .expect("tree_lib.lua");
}

/// The tree link upstream exports for a fixture (`OpenExportPopup`).
fn upstream_link(base: &str) -> String {
    upstream_with_tree_lib(base)
        .load("return exportLink()")
        .eval()
        .unwrap()
}

/// What upstream shows for the tree of the build it has loaded (with `tree_lib.lua`).
fn upstream_result(lua: &mlua::Lua) -> TreeResult {
    #[derive(serde::Deserialize)]
    struct Mastery {
        node: u32,
        effect: u32,
    }
    #[derive(serde::Deserialize)]
    struct Raw {
        allocated: Vec<u32>,
        tree_version: String,
        class_id: u32,
        ascendancy_id: u32,
        #[serde(default)]
        masteries: Vec<Mastery>,
        #[serde(deserialize_with = "colours")]
        compare: Vec<CompareColour>,
        spec_titles: Vec<String>,
        active_spec: usize,
        #[serde(deserialize_with = "lua_list")]
        overrides: Vec<OverrideRecord>,
        can_undo: bool,
    }
    let raw: Raw = lua
        .from_value(
            lua.load(
                r#"
                local allocated, masteries, titles, overrides = {}, {}, {}, {}
                for id, node in pairs(build.spec.hashOverrides) do
                    overrides[#overrides + 1] = { id = id, dn = node.dn }
                end
                for id in pairs(build.spec.allocNodes) do allocated[#allocated + 1] = id end
                table.sort(allocated)
                for node, effect in pairs(build.spec.masterySelections) do
                    if build.spec.allocNodes[node] then masteries[#masteries + 1] = { node = node, effect = effect } end
                end
                table.sort(masteries, function(a, b) return a.node < b.node end)
                for i, spec in ipairs(build.treeTab.specList) do titles[i] = spec.title or "Default" end
                return {
                    allocated = allocated, tree_version = build.spec.treeVersion,
                    class_id = build.spec.curClassId,
                    ascendancy_id = build.spec.curAscendClassId, masteries = masteries,
                    compare = compareColours(),
                    spec_titles = titles, active_spec = build.treeTab.activeSpec - 1,
                    overrides = overrides, can_undo = build.spec.undo[2] ~= nil,
                }
                "#,
            )
            .eval::<mlua::Value>()
            .unwrap(),
        )
        .unwrap();
    let sidebar = golden_of(lua, Game::Poe1.pob_version()).unwrap().sidebar;
    TreeResult {
        allocated: raw.allocated,
        tree_version: raw.tree_version,
        class_id: raw.class_id,
        ascendancy_id: raw.ascendancy_id,
        masteries: raw.masteries.iter().map(|m| (m.node, m.effect)).collect(),
        compare: raw.compare,
        spec_titles: raw.spec_titles,
        active_spec: raw.active_spec,
        sidebar,
        overrides: raw.overrides.into_iter().map(|o| (o.id, o.dn)).collect(),
        can_undo: raw.can_undo,
    }
}

/// The id of the node a `tree_lib.lua` expression picks in the base fixture,
/// e.g. `pick.notable(2)`.
fn upstream_node(base: &str, expr: &str) -> u32 {
    let lua = upstream_with_tree_lib(base);
    lua.load(format!("return ({expr}).id"))
        .eval()
        .unwrap_or_else(|e| panic!("{expr}: {e}"))
}

/// The quoted value of a `key = "value"` line in a recipe file.
fn recipe_setting(source: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} = \"");
    source
        .lines()
        .find_map(|l| l.strip_prefix(&prefix)?.strip_suffix('"'))
        .map(str::to_string)
}

/// Runs `tree/<name>.lua` in upstream PoB on its base fixture.
fn upstream_tree_recipe(name: &str) -> TreeRecipe {
    let source = fs::read_to_string(fixtures_dir().join(format!("tree/{name}.lua"))).unwrap();
    // the recipe names its base fixture, which has to be loaded before its helpers run
    let base = recipe_setting(&source, "base").unwrap_or_else(|| panic!("{name}: no base"));
    // a recipe that imports a link names the fixture to export it from (in another upstream)
    let link = recipe_setting(&source, "link_from").map(|from| upstream_link(&from));
    let lua = upstream_with_tree_lib(&base);
    if let Some(link) = link {
        lua.globals().set("link_url", link).unwrap();
    }
    lua.load(source)
        .set_name(format!("={name}"))
        .exec()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    lua.load("recipe()")
        .exec()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let raw_actions: Vec<serde_json::Value> = lua
        .from_value(lua.load("return actions").eval().unwrap())
        .unwrap();
    let actions: Vec<TreeAction> = raw_actions
        .into_iter()
        .map(|a| serde_json::from_value(a.clone()).unwrap_or_else(|e| panic!("{name}: {a}: {e}")))
        .collect();
    let setup_xml: Option<String> = lua.globals().get("setup_xml").unwrap();
    TreeRecipe {
        base,
        setup_xml,
        actions,
        result: upstream_result(&lua),
        xml: lua
            .globals()
            .get::<bool>("save_xml")
            .unwrap_or(false)
            .then(|| auto_pob::engine::upstream::save_xml(&lua).expect("saving the build")),
    }
}

/// Our tree state and sidebar in the shape of `TreeResult`.
fn our_tree_result(s: &mut Session) -> TreeResult {
    let state = s.backend.tree_state_view().expect("tree state");
    let mut allocated = state.allocated.clone();
    allocated.sort_unstable();
    let mut masteries: Vec<(u32, u32)> =
        state.masteries.iter().map(|m| (m.node, m.effect)).collect();
    masteries.sort_unstable();
    let sidebar = sidebar_of(s);
    TreeResult {
        allocated,
        tree_version: state.version.clone(),
        class_id: state.class_id,
        ascendancy_id: state.ascendancy_id,
        masteries,
        spec_titles: state.specs.iter().map(|e| e.title.clone()).collect(),
        active_spec: state.active_spec,
        compare: state.compare,
        sidebar,
        overrides: our_overrides(s),
        can_undo: state.can_undo,
    }
}

/// The tattoos our PoB has on the active tree, node to name.
fn our_overrides(s: &Session) -> std::collections::BTreeMap<u32, String> {
    let list: Vec<OverrideRecord> = lua_value(
        &s.backend.lua,
        "local list = {} for id, node in pairs(pob.build.spec.hashOverrides) do \
         list[#list + 1] = { id = id, dn = node.dn } end return list",
    );
    list.into_iter().map(|o| (o.id, o.dn)).collect()
}

/// Our sidebar lines in the shape upstream's golden data has them.
fn sidebar_of(s: &mut Session) -> Vec<auto_pob::engine::upstream::SidebarLine> {
    s.stats()
        .stats
        .iter()
        .map(|r| auto_pob::engine::upstream::SidebarLine {
            label: r.label.clone(),
            text: r.text.clone(),
            notes: plain_notes(&r.notes),
        })
        .collect()
}

/// Our tree state in the shape of a recipe's `checkpoint()`.
fn our_checkpoint(s: &mut Session) -> Checkpoint {
    let state = s.backend.tree_state_view().expect("tree state");
    let mut allocated = state.allocated;
    allocated.sort_unstable();
    Checkpoint {
        allocated,
        tree_version: state.version,
        spec_titles: state.specs.iter().map(|e| e.title.clone()).collect(),
        active_spec: state.active_spec,
        compare: state.compare,
        sidebar: None,
        tattoos: None,
    }
}

fn attack_session(test: &str) -> Session {
    let mut s = Session::start(test);
    s.open_fixture("attack", &fixtures_dir().join("attack.xml"));
    s
}

/// Replays a recipe as our commands and checks every checkpoint and the end state against
/// upstream.
fn check_recipe(name: &str) -> TreeRecipe {
    check_recipe_with(name, &mut |_, _| {})
}

/// `check_recipe`, calling `hook` once before the first step (no checkpoint) and after each
/// checkpoint matched, with upstream's record of it.
fn check_recipe_with(
    name: &str,
    hook: &mut dyn FnMut(&mut Session, Option<&Checkpoint>),
) -> TreeRecipe {
    let recipe = upstream_tree_recipe(name);
    assert!(!recipe.actions.is_empty(), "{name}: no actions recorded");
    let mut s = Session::start(&format!("tree-recipe-{name}"));
    match &recipe.setup_xml {
        Some(xml) => {
            let path = s.user.join("setup.xml");
            fs::create_dir_all(&s.user).unwrap();
            fs::write(&path, xml).unwrap();
            s.open_fixture(&recipe.base, &path);
        }
        None => {
            s.open_fixture(
                &recipe.base,
                &fixtures_dir().join(format!("{}.xml", recipe.base)),
            );
        }
    }
    let before = our_tree_result(&mut s);
    hook(&mut s, None);
    for action in &recipe.actions {
        match action.command() {
            Some(command) => {
                s.run(command)
                    .unwrap_or_else(|e| panic!("{name}: {action:?}: {e}"));
            }
            None => {
                let TreeAction::Check { check } = action else {
                    unreachable!()
                };
                let mut ours = our_checkpoint(&mut s);
                // the sidebar and the tattoo state are compared where they are recorded
                ours.sidebar = check.sidebar.as_ref().map(|_| sidebar_of(&mut s));
                ours.tattoos = check.tattoos.clone();
                assert_eq!(&ours, check, "{name}: checkpoint");
                hook(&mut s, Some(check));
            }
        }
    }
    let ours = our_tree_result(&mut s);
    let checked = recipe
        .actions
        .iter()
        .any(|a| matches!(a, TreeAction::Check { .. }));
    assert!(
        checked || ours != before,
        "{name}: the recipe changes the tree"
    );
    assert_eq!(ours, recipe.result, "{name}");
    recipe
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_recipes_match_pob() {
    let _guard = lock();
    for name in ["alloc_path", "class_change", "mastery", "class_paths"] {
        check_recipe(name);
    }
}

/// A click on a bloodline node chooses that bloodline, from none or from another one, and
/// allocates the node (PassiveTreeView.lua:398-411, 499-505)
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_bloodline_node_click_chooses_the_bloodline() {
    let _guard = lock();
    // the top bar's bloodline at each checkpoint, by the id its nodes carry
    let mut chosen = Vec::new();
    let recipe = check_recipe_with("bloodline", &mut |s, check| {
        if check.is_some() {
            let id = s.backend.build_view().unwrap().secondary_selected;
            let name: Option<String> = s
                .backend
                .lua
                .load(format!(
                    "local a = pob.build.spec.tree.alternate_ascendancies[{id}] return a and a.id"
                ))
                .eval()
                .unwrap();
            chosen.push(name);
        }
    });
    // PoB's undo state reads `secondaryAscendClassId`, which no spec has, so an undo drops
    // the bloodline (PassiveSpec.lua:2289); the checkpoint's nodes show upstream did too
    let at = |name: &str| Some(name.to_owned());
    assert_eq!(chosen, [at("Trialmaster"), at("Farrul"), None]);
    let checks: Vec<&Checkpoint> = recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            TreeAction::Check { check } => Some(check),
            _ => None,
        })
        .collect();
    // the clicks changed the tree, and the undo took the last one back
    assert_eq!(checks.len(), 3);
    assert_ne!(checks[0].allocated, checks[1].allocated);
    assert_ne!(checks[1].allocated, checks[2].allocated);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn spec_recipes_match_pob() {
    let _guard = lock();
    for name in ["specs", "convert", "link_round_trip"] {
        check_recipe(name);
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn compare_colours_match_pob() {
    let _guard = lock();
    // the `specs` recipe checks the colours while comparing with the second tree
    let recipe = check_recipe("specs");
    let compared = recipe
        .actions
        .iter()
        .find_map(|a| match a {
            TreeAction::Check { check } if !check.compare.is_empty() => Some(check),
            _ => None,
        })
        .expect("a checkpoint while comparing");
    assert!(compared.compare.iter().any(|c| c.rgb == [1.0, 0.0, 0.0]));
    assert!(compared.compare.iter().any(|c| c.rgb == [0.0, 1.0, 0.0]));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_link_matches_pob() {
    let _guard = lock();
    let mut s = attack_session("tree-link");
    let affected = s.run(Command::ExportTreeLink).unwrap();
    assert!(affected.link);
    let link = s.backend.tree_link_view().unwrap();
    assert!(link.starts_with("https://"), "{link}");
    assert_eq!(link, upstream_link("attack"));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_round_trip() {
    let _guard = lock();
    let mut s = attack_session("tree-round-trip");
    let notable = upstream_node("attack", "pick.notable(2)");
    let mastery = upstream_node("attack", "pick.mastery()");
    let effect: u32 = upstream_with_tree_lib("attack")
        .load("return pick.mastery_effects(pick.mastery())[2]")
        .eval()
        .unwrap();
    s.run(Command::AllocNode { id: notable }).unwrap();
    s.run(Command::NewSpec {
        title: "Extra".into(),
    })
    .unwrap();
    s.run(Command::SetMastery {
        node: mastery,
        effect,
    })
    .unwrap();
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "round-trip".into(),
        overwrite: false,
    })
    .unwrap();
    let saved = s.builds_dir().join("round-trip.xml");

    let ours = our_tree_result(&mut s);
    assert!(ours.allocated.contains(&notable));
    assert_eq!(ours.masteries.len(), 1, "{:?}", ours.masteries);
    assert_eq!(ours.spec_titles.len(), 2);
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &fs::read_to_string(saved).unwrap(), "round-trip").unwrap();
    load_tree_lib(&lua);
    // a loaded build starts with no undo history; the edits made here have some
    let mut reloaded = upstream_result(&lua);
    assert!(ours.can_undo && !reloaded.can_undo);
    reloaded.can_undo = ours.can_undo;
    assert_eq!(ours, reloaded);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn delete_last_spec_is_an_error() {
    let _guard = lock();
    let mut s = attack_session("tree-delete-last");
    let error = s.run(Command::DeleteSpec { index: 0 }).unwrap_err();
    assert_eq!(error.to_string(), "the last tree can't be deleted");
    assert_eq!(s.backend.tree_state_view().unwrap().specs.len(), 1);
}

/// No link makes the import click fail after it added the tree (decoding errors come first),
/// so the step that follows the append, the new tree's first undo state, is made to fail.
#[test]
#[ignore = "needs the pinned PoB install"]
fn failed_import_leaves_no_half_added_tree() {
    let _guard = lock();
    let mut s = attack_session("tree-half-import");
    let before = our_checkpoint(&mut s);
    let link = upstream_link("attack");
    s.backend
        .lua
        .load(
            "local class = getmetatable(pob.build.spec).__index
             real_add_undo_state = class.AddUndoState
             class.AddUndoState = function() error('boom') end",
        )
        .exec()
        .unwrap();
    let result = s.run(Command::ImportTreeLink {
        title: "Half".into(),
        url: link,
    });
    // put the real step back before asserting
    s.backend
        .lua
        .load("getmetatable(pob.build.spec).__index.AddUndoState = real_add_undo_state")
        .exec()
        .unwrap();
    let error = result.unwrap_err();
    assert!(error.to_string().ends_with("boom"), "{error}");
    assert_eq!(
        our_checkpoint(&mut s),
        before,
        "a failed import left a tree"
    );
    let restored: bool = s
        .backend
        .lua
        .load("return getmetatable(pob.build.spec).__index.AddUndoState == real_add_undo_state")
        .eval()
        .unwrap();
    assert!(restored);
    // the engine still works: a good import adds the tree
    s.run(Command::ImportTreeLink {
        title: "Whole".into(),
        url: upstream_link("attack"),
    })
    .unwrap();
    assert_eq!(our_checkpoint(&mut s).spec_titles.len(), 2);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn spec_command_errors() {
    let _guard = lock();
    let mut s = attack_session("tree-spec-errors");
    let before = our_checkpoint(&mut s);
    let cases = [
        (Command::SelectSpec { index: 5 }, "unknown tree 5"),
        (
            Command::RenameSpec {
                index: 0,
                title: "  ".into(),
            },
            "the tree needs a name",
        ),
        (Command::SetCompareSpec { index: Some(3) }, "unknown tree 3"),
        (
            Command::ConvertSpec {
                version: "9_99".into(),
                keep_copy: false,
            },
            "unknown tree version 9_99",
        ),
        (
            Command::ConvertSpec {
                version: "3_29".into(),
                keep_copy: false,
            },
            "the tree already uses that version",
        ),
        (
            Command::ImportTreeLink {
                title: "Bad".into(),
                url: "https://www.pathofexile.com/passive-skill-tree/3.29.0/abc".into(),
            },
            "Invalid tree link (unrecognised format)",
        ),
        (
            // PoB's popup raises an error for a made-up tree type
            Command::ImportTreeLink {
                title: "Odd".into(),
                url: "https://www.pathofexile.com/passive-skill-tree/abc".into(),
            },
            "Classes/PassiveSpec.lua:42: attempt to index field 'tree' (a nil value)",
        ),
        (
            // a valid header naming a class that doesn't exist
            Command::ImportTreeLink {
                title: "Corrupt".into(),
                url: "https://www.pathofexile.com/passive-skill-tree/3.29.0/AAAABsgAAAAA".into(),
            },
            "Invalid tree link (bad class ID '200')",
        ),
    ];
    for (command, message) in cases {
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert_eq!(error.to_string(), message, "{command:?}");
        assert_eq!(
            our_checkpoint(&mut s),
            before,
            "{command:?} changed the trees"
        );
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn edits_mark_unsaved_and_undo() {
    let _guard = lock();
    let mut s = attack_session("tree-unsaved");
    let original = s.backend.tree_state_view().unwrap().allocated;
    assert!(
        !s.backend.build_view().unwrap().unsaved,
        "a loaded build is saved"
    );
    assert!(!s.backend.tree_state_view().unwrap().can_undo);

    let id = upstream_node("attack", "pick.notable(2)");
    let affected = s.run(Command::AllocNode { id }).unwrap();
    assert!(affected.build && affected.tree && affected.recalc);
    let state = s.backend.tree_state_view().unwrap();
    assert!(state.allocated.contains(&id));
    assert!(state.can_undo && !state.can_redo);
    assert!(
        s.backend.build_view().unwrap().unsaved,
        "an edit leaves unsaved changes"
    );

    s.run(Command::Undo(Tab::Tree)).unwrap();
    let state = s.backend.tree_state_view().unwrap();
    assert_eq!(state.allocated, original);
    assert!(state.can_redo);

    s.run(Command::Redo(Tab::Tree)).unwrap();
    assert!(s.backend.tree_state_view().unwrap().allocated.contains(&id));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_command_errors() {
    let _guard = lock();
    let mut s = attack_session("tree-errors");
    let before = s.backend.tree_state_view().unwrap().allocated;
    let mastery = upstream_node("attack", "pick.mastery()");
    let unreachable = upstream_node("attack", "pick.unreachable()");
    let cases = [
        (
            Command::AllocNode { id: 987_654_321 },
            "unknown passive node 987654321",
        ),
        (
            Command::DeallocNode { id: 987_654_321 },
            "unknown passive node 987654321",
        ),
        (
            Command::AllocNode { id: unreachable },
            "that passive isn't connected to your tree",
        ),
        (
            Command::SetMastery {
                node: mastery,
                effect: 987_654_321,
            },
            "that mastery effect isn't available here",
        ),
        (Command::AllocNode { id: mastery }, "pick a mastery effect"),
    ];
    for (command, message) in cases {
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert_eq!(error.to_string(), message, "{command:?}");
        assert_eq!(
            s.backend.tree_state_view().unwrap().allocated,
            before,
            "{command:?} changed the tree"
        );
    }
}

// ---- search, jewels, tooltips and node power ----

/// Reads a value from Lua code run in `lua`.
fn lua_value<T: serde::de::DeserializeOwned>(lua: &mlua::Lua, code: &str) -> T {
    let value: mlua::Value = lua.load(code).eval().expect("upstream Lua");
    lua.from_value(value).expect("upstream value")
}

fn search(text: &str) -> Command {
    Command::SetTreeSearch { text: text.into() }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn search_matches_pob() {
    let _guard = lock();
    let mut s = attack_session("tree-search");
    let lua = upstream_with_tree_lib("attack");
    let unsaved = |s: &mut Session| s.backend.build_view().unwrap().unsaved;
    let before = unsaved(&mut s);
    for text in ["life", "increased.fire.damage|area", ""] {
        let affected = s.run(search(text)).unwrap();
        assert_eq!(
            affected,
            auto_pob::engine::Affected {
                build: true,
                tree: true,
                ..Default::default()
            }
        );
        let ours = s.backend.tree_state_view().unwrap();
        assert_eq!(ours.search, text);
        // upstream: the viewer's own Draw caches the match of every node
        lua.globals().set("query", text).unwrap();
        let theirs: Vec<u32> = lua_value(
            &lua,
            r#"
            local viewer = build.treeTab.viewer
            viewer.searchStr = query
            runCallback("OnFrame")
            assert(viewer.searchParams, "the tree view didn't prepare the search")
            local ids = {}
            -- Draw tests nodes only when there are words to look for (PassiveTreeView.lua:801)
            for id, node in pairs(build.spec.nodes) do
                if #viewer.searchParams > 0 and viewer:DoesNodeMatchSearchParams(node) then
                    ids[#ids + 1] = id
                end
            end
            table.sort(ids)
            return ids
            "#,
        );
        assert_eq!(ours.search_matches, theirs, "search {text:?}");
        if text.is_empty() {
            assert!(theirs.is_empty());
        } else {
            assert!(!theirs.is_empty(), "search {text:?} matches nothing");
        }
    }
    // PoB saves the search with the build: a changed search is an unsaved change, and the
    // saved text is not (Build.lua:1303, TreeTab.lua:180)
    assert!(!before);
    // the command alone does it: the loop runs no recalculation for a search
    s.run(search("life")).unwrap();
    assert!(unsaved(&mut s), "a changed search marks the build unsaved");
    s.run(search("")).unwrap();
    assert!(!unsaved(&mut s), "the saved search is not a change");
}

/// The tree state reads the tree view's own search results only while nothing but searches ran
/// since its frame matched them: after any other command the nodes are matched again.
#[test]
#[ignore = "needs the pinned PoB install"]
fn search_matches_again_after_other_commands() {
    let _guard = lock();
    let mut s = attack_session("tree-search-again");
    let matches = |s: &mut Session| s.backend.tree_state_view().unwrap().search_matches;
    s.run(search("qzxv")).unwrap();
    assert!(matches(&mut s).is_empty());
    // a node now reads as the search, behind the view's back
    s.backend
        .lua
        .load(
            "local spec = pob.build.spec
            for _, n in pairs(spec.nodes[spec.curClass.startNodeId].linked) do
                if n.type == 'Normal' then n.dn = 'Qzxv' return end
            end
            error('no neighbour')",
        )
        .exec()
        .unwrap();
    // the same text again: the view's results are the ones its frame took, as PoB shows them
    s.run(search("qzxv")).unwrap();
    assert!(matches(&mut s).is_empty());
    // any other command: matched again, so the renamed node is found
    s.run(set("conditionFullLife", ConfigValue::Bool(true)))
        .unwrap();
    assert_eq!(matches(&mut s).len(), 1);
    // and the same text after it doesn't go back to the view's old results
    s.run(search("qzxv")).unwrap();
    assert_eq!(matches(&mut s).len(), 1);
}

/// A session with a copy of the tree in which only one node reads as "qzxv", and a fresh
/// search for it on the first tree (no match)
fn search_on_two_trees(test: &str) -> Session {
    let mut s = attack_session(test);
    s.run(Command::CopySpec {
        index: 0,
        title: "Copy".into(),
    })
    .unwrap();
    s.backend
        .lua
        .load(
            "local spec = pob.build.treeTab.specList[2]
            assert(spec ~= pob.build.spec, 'the copy is active')
            for _, n in pairs(spec.nodes[spec.curClass.startNodeId].linked) do
                if n.type == 'Normal' then n.dn = 'Qzxv' return end
            end
            error('no neighbour')",
        )
        .exec()
        .unwrap();
    s.backend.apply(search("qzxv"), &|_| {}).unwrap();
    assert!(
        s.backend
            .tree_state_view()
            .unwrap()
            .search_matches
            .is_empty()
    );
    s
}

/// The same search again, as its only command, and its matches
fn search_again(s: &mut Session) -> usize {
    s.backend.apply(search("qzxv"), &|_| {}).unwrap();
    s.backend.tree_state_view().unwrap().search_matches.len()
}

/// Switches to the copy behind the engine's back, with no command
fn switch_behind(s: &mut Session) {
    s.backend
        .lua
        .load("pob.build.treeTab:SetActiveSpec(2)")
        .exec()
        .unwrap();
}

/// Each way the engine learns the nodes may have changed clears the fresh search on its own:
/// a command with no recalculation after it, a recalculation alone, and a background step
/// alone.
#[test]
#[ignore = "needs the pinned PoB install"]
fn search_matches_again_after_each_kind_of_change() {
    let _guard = lock();
    // a command alone: the tree switch, with no recalculation
    let mut s = search_on_two_trees("tree-search-apply");
    s.backend
        .apply(Command::SelectSpec { index: 1 }, &|_| {})
        .unwrap();
    assert_eq!(search_again(&mut s), 1, "after a command");
    drop(s);
    // a recalculation alone
    let mut s = search_on_two_trees("tree-search-recalc");
    switch_behind(&mut s);
    s.backend.recalculate().unwrap();
    assert_eq!(search_again(&mut s), 1, "after a recalculation");
    drop(s);
    // a background step alone
    let mut s = search_on_two_trees("tree-search-step");
    switch_behind(&mut s);
    s.backend
        .background_step(BackgroundTask::ItemDb, &|_| {})
        .unwrap();
    assert_eq!(search_again(&mut s), 1, "after a background step");
}

/// The spec's reachable paths and dependants, built from scratch as the tree state lists them
fn paths_from_scratch(
    s: &Session,
) -> (
    Vec<auto_pob::engine::views::NodePath>,
    Vec<auto_pob::engine::views::NodePath>,
) {
    let value: mlua::Value = s
        .backend
        .lua
        .load(
            "local function list(field, alloc)
                local out = {}
                for id, node in pairs(pob.build.spec.nodes) do
                    if node[field] and (not node.alloc) == (not alloc) then
                        local path = {}
                        for i, n in ipairs(node[field]) do path[i] = n.id end
                        out[#out + 1] = { node = id, path = path }
                    end
                end
                table.sort(out, function(a, b) return a.node < b.node end)
                return out
            end
            return { list('path', false), list('depends', true) }",
        )
        .eval()
        .unwrap();
    s.backend.lua.from_value(value).unwrap()
}

/// The tree state keeps its paths and dependants while they stay the same, and they are never
/// stale: after each change they equal a list built from scratch.
#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_paths_are_kept_but_never_stale() {
    let _guard = lock();
    let mut s = attack_session("tree-paths-cache");
    let check = |s: &mut Session, what: &str| {
        let state = s.backend.tree_state_view().unwrap();
        let (paths, depends) = paths_from_scratch(s);
        assert!(!paths.is_empty(), "{what}");
        assert!(*state.paths == paths[..], "{what}: stale paths");
        assert!(*state.depends == depends[..], "{what}: stale dependants");
        state
    };
    let state = check(&mut s, "open");
    // an edit that leaves the tree alone: the glue doesn't send them again, across its
    // recalculation
    let token: i64 = s
        .backend
        .lua
        .load("local _, token = glue.tree_state(false, nil) return token")
        .eval()
        .unwrap();
    s.run(set("conditionFullLife", ConfigValue::Bool(true)))
        .unwrap();
    let (again, sent): (i64, bool) = s
        .backend
        .lua
        .load(format!(
            "local _, again, sent = glue.tree_state(false, {token}) return again, sent"
        ))
        .eval()
        .unwrap();
    assert_eq!(again, token);
    assert!(!sent, "unchanged paths were sent again");
    let edited = check(&mut s, "config edit");
    // the engine shares the lists it holds rather than copying them
    let again = s.backend.tree_state_view().unwrap();
    assert!(std::sync::Arc::ptr_eq(&edited.paths, &again.paths));
    assert!(std::sync::Arc::ptr_eq(&edited.depends, &again.depends));
    // every kind of change
    let reachable = state.paths.iter().find(|p| p.path.len() == 1).unwrap().node;
    s.run(Command::AllocNode { id: reachable }).unwrap();
    check(&mut s, "alloc");
    s.run(Command::DeallocNode { id: reachable }).unwrap();
    check(&mut s, "dealloc");
    s.run(Command::Undo(Tab::Tree)).unwrap();
    check(&mut s, "undo");
    s.run(Command::Redo(Tab::Tree)).unwrap();
    check(&mut s, "redo");
    s.run(Command::CopySpec {
        index: 0,
        title: "Copy".into(),
    })
    .unwrap();
    s.run(Command::SelectSpec { index: 1 }).unwrap();
    s.run(Command::AllocNode { id: reachable }).unwrap();
    check(&mut s, "alloc on the copy");
    s.run(Command::SelectSpec { index: 0 }).unwrap();
    check(&mut s, "back to the first tree");
    let other = state
        .classes
        .iter()
        .find(|c| c.id != state.class_id)
        .unwrap()
        .id;
    s.run(Command::SelectClass {
        id: other,
        connect: false,
    })
    .unwrap();
    check(&mut s, "class change");
    // a path or an allocation changed behind the view's back, as a jewel's reach would
    s.backend
        .lua
        .load(
            "for _, node in pairs(pob.build.spec.nodes) do
                if node.path and #node.path > 2 and not node.alloc then
                    node.path[2] = node.path[1]
                    return
                end
            end
            error('no long path')",
        )
        .exec()
        .unwrap();
    check(&mut s, "a path edited");
    s.backend
        .lua
        .load(
            "for _, node in pairs(pob.build.spec.nodes) do
                if node.path and not node.alloc then node.path[#node.path + 1] = node return end
            end",
        )
        .exec()
        .unwrap();
    check(&mut s, "a path grown");
    // another build
    s.open_fixture("minion", &fixtures_dir().join("minion.xml"));
    check(&mut s, "another build");
    // leaving a build lets its lists go: the same build opened again gets them built afresh
    let token: i64 = s
        .backend
        .lua
        .load("local _, token = glue.tree_state(false, nil) return token")
        .eval()
        .unwrap();
    s.open_fixture("minion", &fixtures_dir().join("minion.xml"));
    let sent: bool = s
        .backend
        .lua
        .load(format!(
            "local _, _, sent = glue.tree_state(false, {token}) return sent"
        ))
        .eval()
        .unwrap();
    assert!(sent, "the left build's lists were kept");
    check(&mut s, "the same build again");
}

/// Pastes, saves and socket a jewel into the nearest socket `pick` chooses; returns the number
/// of nodes the spec has afterwards
fn socket_jewel(s: &mut Session, pick: &str, text: &str) -> usize {
    let socket: u32 = s.backend.lua.load(pick).eval().unwrap();
    s.run(Command::AllocNode { id: socket }).unwrap();
    // the view takes the tree with the socket allocated, so the jewel is the only change
    s.backend.tree_state_view().unwrap();
    s.run(Command::PasteItem { text: text.into() }).unwrap();
    s.run(Command::SaveDisplayItem).unwrap();
    let jewel: u32 = s
        .backend
        .lua
        .load(
            "local m = 0 for id in pairs(pob.build.itemsTab.items) do if id > m then m = id end end return m",
        )
        .eval()
        .unwrap();
    s.run(Command::EquipItem {
        slot: format!("Jewel {socket}"),
        id: Some(jewel),
    })
    .unwrap();
    let count: usize = s
        .backend
        .lua
        .load("local n = 0 for _ in pairs(pob.build.spec.nodes) do n = n + 1 end return n")
        .eval()
        .unwrap();
    count
}

/// Jewels that change the tree's reach or its nodes: the paths and dependants follow them.
/// The timeless jewel's LUT is cached next to PoB's data, so the test runs on a farm and
/// leaves the pinned folder's LUTs as they were.
#[test]
#[ignore = "needs the pinned PoB install"]
fn tree_paths_follow_jewels() {
    let _guard = lock();
    let pinned_before = pinned_luts();
    let pob = timeless::farm("paths");
    let mut s = Session::start_on("tree-paths-jewels", &pob);
    s.open_fixture("attack", &fixtures_dir().join("attack.xml"));
    let check = |s: &mut Session, what: &str| {
        let state = s.backend.tree_state_view().unwrap();
        let (paths, depends) = paths_from_scratch(s);
        assert!(*state.paths == paths[..], "{what}: stale paths");
        assert!(*state.depends == depends[..], "{what}: stale dependants");
        state
    };
    let before = check(&mut s, "open");
    let nodes_before: usize = s
        .backend
        .lua
        .load("local n = 0 for _ in pairs(pob.build.spec.nodes) do n = n + 1 end return n")
        .eval()
        .unwrap();
    // a radius jewel that lets the nodes around it be taken unconnected: reach changes with
    // no change of allocation
    let nearest = |filter: &str| {
        format!(
            "local best, d = 0, 1e9
            for id in pairs(pob.build.itemsTab.sockets) do
                local n = pob.build.spec.nodes[id]
                if n and n.name ~= 'Charm Socket' and not n.alloc and n.path and ({filter})
                    and n.pathDist < d then best, d = id, n.pathDist end
            end
            assert(best ~= 0, 'no socket')
            return best"
        )
    };
    socket_jewel(
        &mut s,
        &nearest("not n.expansionJewel"),
        "Rarity: UNIQUE\nIntuitive Leap\nViridian Jewel\nRadius: Small\nImplicits: 0\n\
         Passives in Radius can be Allocated without being connected to your tree\nCorrupted",
    );
    let leap = check(&mut s, "intuitive leap");
    assert!(leap.paths != before.paths, "the jewel changed no path");
    // a large cluster jewel in an outer socket adds nodes to the spec
    let nodes = socket_jewel(
        &mut s,
        &nearest("n.expansionJewel and n.expansionJewel.size == 2"),
        "Rarity: RARE\nTest Cluster\nLarge Cluster Jewel\nItem Level: 84\nImplicits: 3\n\
         {crafted}Adds 8 Passive Skills\n{crafted}2 Added Passive Skills are Jewel Sockets\n\
         {crafted}Added Small Passive Skills grant: 12% increased Fire Damage",
    );
    assert!(nodes > nodes_before, "the cluster added no nodes ({nodes})");
    check(&mut s, "cluster jewel");
    // a timeless jewel conquers the nodes around it
    socket_jewel(
        &mut s,
        &nearest("not n.expansionJewel"),
        "Rarity: UNIQUE\nLethal Pride\nTimeless Jewel\nRadius: Large\nImplicits: 0\n\
         Commanded leadership over 12000 warriors under Kaom\n\
         Passives in radius are Conquered by the Karui\nCorrupted",
    );
    check(&mut s, "timeless jewel");
    assert!(
        pob.join("Data/TimelessJewelData/LethalPride.bin").is_file(),
        "the jewel read no LUT in the farm"
    );
    assert_eq!(
        pinned_luts(),
        pinned_before,
        "the pinned TimelessJewelData changed"
    );
    drop(s);
    let _ = fs::remove_dir_all(&pob);
}

/// A tree switch between two identical searches matches the new tree's nodes, not the old
/// tree's results.
#[test]
#[ignore = "needs the pinned PoB install"]
fn search_matches_again_after_a_tree_switch() {
    let _guard = lock();
    let mut s = attack_session("tree-search-switch");
    let matches = |s: &mut Session| s.backend.tree_state_view().unwrap().search_matches;
    s.run(Command::CopySpec {
        index: 0,
        title: "Copy".into(),
    })
    .unwrap();
    // only the copy has a node that reads as the search
    s.backend
        .lua
        .load(
            "local spec = pob.build.treeTab.specList[2]
            assert(spec ~= pob.build.spec, 'the copy is active')
            for _, n in pairs(spec.nodes[spec.curClass.startNodeId].linked) do
                if n.type == 'Normal' then n.dn = 'Qzxv' return end
            end
            error('no neighbour')",
        )
        .exec()
        .unwrap();
    s.run(search("qzxv")).unwrap();
    assert!(matches(&mut s).is_empty());
    s.run(Command::SelectSpec { index: 1 }).unwrap();
    s.run(search("qzxv")).unwrap();
    assert_eq!(matches(&mut s).len(), 1, "the old tree's results");
    s.run(Command::SelectSpec { index: 0 }).unwrap();
    s.run(search("qzxv")).unwrap();
    assert!(matches(&mut s).is_empty(), "the copy's results");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn jewel_radius_matches_pob() {
    let _guard = lock();
    let recipe = upstream_tree_recipe("jewel");
    let xml = recipe.xml.expect("the jewel recipe saves its build");
    let mut s = Session::start("tree-jewel");
    let path = s.user.join("jewel.xml");
    fs::create_dir_all(&s.user).unwrap();
    fs::write(&path, &xml).unwrap();
    s.open_fixture("jewel", &path);
    let ours = s.backend.tree_state_view().unwrap();
    assert_eq!(ours.jewels.len(), 1, "{:?}", ours.jewels);
    let lua = upstream_lua(&pob_dir()).unwrap();
    load_xml(&lua, &xml, "jewel").unwrap();
    #[derive(serde::Deserialize)]
    struct Theirs {
        node: u32,
        name: String,
        radius_nodes: Vec<u32>,
        ring: [f32; 2],
    }
    let theirs: Vec<Theirs> = lua_value(
        &lua,
        r#"
        local list = {}
        for id, node in pairs(build.spec.nodes) do
            local jewel = node.type == "Socket" and build.spec:GetSocketedJewel(id)
            if jewel then
                local index = jewel.jewelRadiusIndex
                local radius = {}
                for nodeId in pairs(node.nodesInRadius[index]) do radius[#radius + 1] = nodeId end
                table.sort(radius)
                local ring = data.jewelRadius[index]
                list[#list + 1] = { node = id, name = jewel.title, radius_nodes = radius, ring = { ring.inner, ring.outer } }
            end
        end
        return list
        "#,
    );
    assert_eq!(theirs.len(), 1);
    let (ours, theirs) = (&ours.jewels[0], &theirs[0]);
    assert_eq!(ours.node, theirs.node);
    assert_eq!(ours.name, theirs.name);
    assert_eq!(ours.name, "Eldritch Knowledge");
    let mut radius = ours.radius_nodes.clone();
    radius.sort_unstable();
    assert_eq!(radius, theirs.radius_nodes);
    assert!(radius.len() > 10, "{}", radius.len());
    assert_eq!(ours.ring, Some(theirs.ring));
}

/// Nodes allocated only in the compared tree (a cluster jewel's subgraph): PoB draws them and
/// gives them their own tooltip (PassiveTreeView.lua:1102-1153, 1748-1804)
#[test]
#[ignore = "needs the pinned PoB install"]
fn compare_only_nodes_match_pob() {
    let _guard = lock();
    // upstream at the recipe's end, kept for its tooltips
    let source = fs::read_to_string(fixtures_dir().join("tree/compare_only.lua")).unwrap();
    let lua = upstream_with_tree_lib("attack");
    lua.load(include_str!("../src/engine/glue/tooltip.lua"))
        .set_name("=recorder")
        .exec()
        .unwrap();
    lua.load(source).exec().unwrap();
    lua.load("recipe()").exec().unwrap();
    let theirs: Vec<auto_pob::engine::NodeTooltip> = lua_value(
        &lua,
        r#"
        local viewer, compareSpec = build.treeTab.viewer, build.treeTab.specList[1]
        local ids = {}
        for id, node in pairs(compareSpec.nodes) do
            if not build.spec.nodes[id] and node.alloc and node.x and node.type ~= "ClassStart"
                and node.type ~= "AscendClassStart" then
                ids[#ids + 1] = id
            end
        end
        table.sort(ids)
        local list = {}
        viewer.compareSpec = compareSpec
        for _, id in ipairs(ids) do
            local tooltip = glue.tooltip()
            viewer:AddCompareNodeTooltip(tooltip, compareSpec.nodes[id], build)
            list[#list + 1] = { id = id, lines = tooltip:lines() }
        end
        return list
        "#,
    );
    assert!(theirs.len() > 3, "{} compare-only nodes", theirs.len());
    let mut ours = Vec::new();
    check_recipe_with("compare_only", &mut |s, check| {
        if check.is_none() {
            return;
        }
        // the layout first, as the engine sends it before the state
        let sheets = s.backend.tree_layout_view().unwrap().sheets;
        let state = s.backend.tree_state_view().unwrap();
        // PoB draws each with its allocated art (PassiveTreeView.lua:1124-1131): an icon on a
        // sheet the layout sent
        for node in state.compare_only.iter().filter(|n| n.kind != "Socket") {
            node.art
                .as_ref()
                .and_then(|a| a.base)
                .and_then(|i| i.resolve(&sheets))
                .unwrap_or_else(|| panic!("node {} has no art", node.id));
        }
        for node in &state.compare_only {
            let tip = s.backend.node_tooltip_view(node.id).unwrap();
            ours.push((node.id, tip));
        }
        // the Tree tab's compare off: none
        s.run(Command::SetCompareSpec { index: None }).unwrap();
        assert!(s.backend.tree_state_view().unwrap().compare_only.is_empty());
        s.run(Command::SetCompareSpec { index: Some(0) }).unwrap();
    });
    let ids: Vec<u32> = ours.iter().map(|(id, _)| *id).collect();
    let want: Vec<u32> = theirs.iter().map(|t| t.id).collect();
    assert_eq!(ids, want);
    for ((_, tip), want) in ours.iter().zip(&theirs) {
        assert_eq!(tip.as_ref(), Some(want), "node {}", want.id);
    }
    assert!(
        theirs[0].lines.iter().any(|l| matches!(l,
            auto_pob::engine::TooltipLine::Line { text, .. } if text.contains("only in the compared build"))),
        "{:?}",
        theirs[0].lines
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tooltip_matches_pob() {
    let _guard = lock();
    // the allocated socket that holds the jewel recipe's jewel
    let recipe = upstream_tree_recipe("jewel");
    let xml = recipe.xml.expect("the jewel recipe saves its build");
    let mut s = Session::start("tree-tooltip");
    let path = s.user.join("jewel.xml");
    fs::create_dir_all(&s.user).unwrap();
    fs::write(&path, &xml).unwrap();
    s.open_fixture("jewel", &path);
    let socket = s.backend.tree_state_view().unwrap().jewels[0].node;
    let lua = upstream_lua(&pob_dir()).unwrap();
    load_xml(&lua, &xml, "jewel").unwrap();
    load_tree_lib(&lua);
    // the recorder of glue/tooltip.lua, which draws nothing and keeps the lines
    lua.load(include_str!("../src/engine/glue/tooltip.lua"))
        .set_name("=recorder")
        .exec()
        .unwrap();
    lua.globals().set("socket_id", socket).unwrap();
    let theirs: Vec<auto_pob::engine::NodeTooltip> = lua_value(
        &lua,
        r#"
        -- the lowest id of each kind among the unallocated nodes
        local ids = {}
        for _, kind in ipairs({ "Normal", "Notable", "Keystone", "Mastery" }) do
            local best
            for id, node in pairs(build.spec.nodes) do
                if node.type == kind and not node.alloc and (kind ~= "Mastery" or node.masteryEffects)
                    and (not best or id < best) then
                    best = id
                end
            end
            ids[#ids + 1] = best
        end
        ids[#ids + 1] = socket_id
        local list = {}
        for _, id in ipairs(ids) do
            local node = build.spec.nodes[id]
            assert(id == socket_id or not node.alloc, "node " .. id .. " is allocated")
            local tooltip = glue.tooltip()
            build.treeTab.viewer:AddNodeTooltip(tooltip, node, build)
            list[#list + 1] = { id = id, lines = tooltip:lines() }
        end
        return list
        "#,
    );
    assert_eq!(theirs.len(), 5);
    // the jewel is part of the socket's tooltip
    let jewel_lines = &theirs[4].lines;
    assert!(
        jewel_lines.iter().any(|l| matches!(l,
            auto_pob::engine::TooltipLine::Line { text, .. } if text.contains("Eldritch Knowledge"))),
        "{jewel_lines:?}"
    );
    for expected in &theirs {
        assert!(expected.lines.len() > 3, "{expected:?}");
        let affected = s.run(Command::PreviewNode { id: expected.id }).unwrap();
        assert_eq!(affected.preview, Some(expected.id));
        let ours = s
            .backend
            .node_tooltip_view(expected.id)
            .unwrap()
            .expect("the node exists");
        assert_eq!(&ours, expected, "node {}", expected.id);
    }
    assert!(
        s.backend
            .node_tooltip_view(4_000_000_000)
            .unwrap()
            .is_none()
    );
}

/// Runs `SetHeatmap` and then steps until the backend has no work left.
fn run_power(s: &mut Session, stat: &str, max_depth: Option<u32>) -> Vec<u32> {
    s.run(Command::SetHeatmap {
        stat: Some(stat.into()),
        max_depth,
    })
    .unwrap();
    let progress = std::cell::RefCell::new(Vec::new());
    let mut steps = 0;
    while let Some(task) = s.backend.next_background() {
        assert_eq!(task, BackgroundTask::NodePower);
        let affected = s
            .backend
            .background_step(task, &|event| {
                if let Event::Progress { percent, .. } = event {
                    progress.borrow_mut().push(percent);
                }
            })
            .expect("power step");
        assert!(!affected.recalc);
        steps += 1;
        assert!(steps < 100_000, "the power calculation never ends");
    }
    progress.into_inner()
}

/// Upstream's node power for `stat`: `SetPowerCalc`, a frame, then `BuildPower` to the end.
fn upstream_power(stat: &str, max_depth: u32) -> mlua::Lua {
    let lua = upstream_with_tree_lib("attack");
    lua.globals().set("stat_label", stat).unwrap();
    lua.globals().set("max_depth", max_depth).unwrap();
    lua.load(
        r#"
        local tab = build.treeTab
        for _, entry in ipairs(tab.powerStatList) do
            if entry.label == stat_label then stat = entry end
        end
        tab:SetPowerCalc(stat)
        build.calcsTab.nodePowerMaxDepth = max_depth
        runCallback("OnFrame")
        repeat build.calcsTab:BuildPower() until not build.calcsTab.powerBuilder
        "#,
    )
    .exec()
    .unwrap();
    lua
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn node_power_matches_pob() {
    let _guard = lock();
    let mut s = attack_session("tree-node-power");
    let state = s.backend.tree_state_view().unwrap();
    let dps = state
        .power_stats
        .iter()
        .find(|label| label.contains("DPS"))
        .expect("a DPS stat")
        .clone();
    assert!(state.power_stats.iter().any(|label| label == "Life"));
    for stat in ["Life", dps.as_str()] {
        run_power(&mut s, stat, Some(5));
        let ours = s.backend.node_power_view().unwrap();
        assert!(ours.finished, "{stat}");
        assert_eq!(ours.stat.as_deref(), Some(stat));
        let state = s.backend.tree_state_view().unwrap();
        assert_eq!(state.heatmap.as_deref(), Some(stat));
        assert_eq!(state.max_depth, Some(5));
        let lua = upstream_power(stat, 5);
        let theirs: Vec<auto_pob::engine::NodePowerEntry> = lua_value(
            &lua,
            r#"
            local list = {}
            for id, node in pairs(build.spec.nodes) do
                local p = node.power
                if p and (p.singleStat or p.pathPower or p.offence or p.defence) then
                    list[#list + 1] = { id = id, single = p.singleStat or 0, path = p.pathPower,
                        offence = p.offence, defence = p.defence }
                end
            end
            table.sort(list, function(a, b) return a.id < b.id end)
            return list
            "#,
        );
        assert!(theirs.len() > 20, "{stat}: {} nodes", theirs.len());
        assert!(theirs.iter().any(|n| n.single != 0.0), "{stat}");
        assert_eq!(ours.nodes.len(), theirs.len(), "{stat}: node sets");
        for (a, b) in ours.nodes.iter().zip(&theirs) {
            assert_eq!(a, b, "{stat}: node {}", b.id);
        }
        let max_single: f64 = lua_value(&lua, "return build.calcsTab.powerMax.singleStat");
        assert_eq!(ours.max_single, max_single, "{stat}");
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn power_report_matches_pob() {
    let _guard = lock();
    let mut s = attack_session("tree-power-report");
    let progress = run_power(&mut s, "Life", Some(5));
    assert!(progress.iter().all(|p| *p <= 100), "{progress:?}");
    let mut ours = s.backend.power_report_view().unwrap();
    assert_eq!(ours.stat, "Life");
    let lua = upstream_power("Life", 5);
    let mut theirs: Vec<auto_pob::engine::PowerReportRow> = lua_value(
        &lua,
        r#"
        local calcs = build.calcsTab
        local rows = {}
        for _, row in ipairs(build.treeTab:BuildPowerReportList(calcs.powerStat)) do
            rows[#rows + 1] = {
                id = row.id, name = row.name, kind = row.type, power = row.power,
                power_text = row.powerStr, path_power = row.pathPower,
                path_power_text = row.pathPowerStr, allocated = row.allocated and true or false,
                path_dist = type(row.pathDist) == "number" and row.pathDist or 0,
            }
        end
        return rows
        "#,
    );
    assert!(theirs.len() > 100, "{}", theirs.len());
    // rows of equal power may sort either way
    let key = |r: &auto_pob::engine::PowerReportRow| (r.id, r.name.clone());
    for rows in [&mut ours.rows, &mut theirs] {
        rows.sort_by_key(key);
    }
    assert_eq!(ours.rows, theirs);
    // the text keeps PoB's colour codes
    assert!(ours.rows.iter().any(|r| r.power_text.starts_with('^')));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn no_power_work_without_heatmap() {
    let _guard = lock();
    let mut s = attack_session("tree-no-power");
    s.run(set("buffAdrenaline", ConfigValue::Bool(true)))
        .unwrap();
    assert!(s.backend.next_background().is_none());
    s.run(Command::SetHeatmap {
        stat: Some("Life".into()),
        max_depth: Some(5),
    })
    .unwrap();
    assert!(s.backend.next_background() == Some(BackgroundTask::NodePower));
    s.run(Command::SetHeatmap {
        stat: None,
        max_depth: None,
    })
    .unwrap();
    assert!(s.backend.next_background().is_none());
    s.run(set("buffAdrenaline", ConfigValue::Bool(false)))
        .unwrap();
    assert!(s.backend.next_background().is_none());
    let state = s.backend.tree_state_view().unwrap();
    assert_eq!(state.heatmap, None);
    let off = s.backend.node_power_view().unwrap();
    assert!(off.nodes.is_empty());
    assert_eq!(off.stat, None);
    assert!(
        off.finished,
        "nothing is calculated while the heat map is off"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn leaving_for_the_list_stops_power_work() {
    let _guard = lock();
    let mut s = attack_session("tree-list-stops-power");
    s.run(Command::SetHeatmap {
        stat: Some("Life".into()),
        max_depth: Some(5),
    })
    .unwrap();
    assert!(s.backend.next_background() == Some(BackgroundTask::NodePower));
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    assert!(s.backend.next_background().is_none());
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn unknown_heatmap_stat_changes_nothing() {
    let _guard = lock();
    let mut s = attack_session("tree-heatmap-unknown");
    let on = Command::SetHeatmap {
        stat: Some("Life".into()),
        max_depth: Some(5),
    };
    s.run(on).unwrap();
    let error = s
        .run(Command::SetHeatmap {
            stat: Some("No Such Stat".into()),
            max_depth: Some(10),
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("unknown heat map stat"),
        "{error}"
    );
    let state = s.backend.tree_state_view().unwrap();
    assert_eq!(state.heatmap.as_deref(), Some("Life"));
    assert_eq!(state.max_depth, Some(5));
    assert!(
        s.backend.next_background() == Some(BackgroundTask::NodePower),
        "the heat map is still on"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn failed_power_step_stops_background_work() {
    let _guard = lock();
    let mut s = attack_session("tree-power-fails");
    let heatmap = || Command::SetHeatmap {
        stat: Some("Life".into()),
        max_depth: Some(5),
    };
    s.run(heatmap()).unwrap();
    s.backend
        .lua
        .load("pob.build.calcsTab.PowerBuilder = function() error('boom', 0) end")
        .exec()
        .unwrap();
    assert!(s.backend.next_background() == Some(BackgroundTask::NodePower));
    let error = s
        .backend
        .background_step(BackgroundTask::NodePower, &|_| {})
        .unwrap_err();
    assert!(error.to_string().contains("boom"), "{error}");
    // a recalculation sets powerBuildFlag again (CalcsTab.lua:435)
    s.run(set("buffAdrenaline", ConfigValue::Bool(true)))
        .unwrap();
    let pending: bool = s
        .backend
        .lua
        .load("return glue.power_pending()")
        .eval()
        .unwrap();
    assert!(pending, "the recalculation queued the power calculation");
    assert!(
        s.backend.next_background().is_none(),
        "a failed step must end the background work until the next SetHeatmap"
    );
    // the next SetHeatmap tries again
    s.backend
        .lua
        .load("pob.build.calcsTab.PowerBuilder = nil")
        .exec()
        .unwrap();
    s.run(heatmap()).unwrap();
    assert!(s.backend.next_background() == Some(BackgroundTask::NodePower));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn commands_answered_during_power_report() {
    use std::time::{Duration, Instant};
    let _guard = lock();
    pob_dir();
    let user = std::env::temp_dir().join(format!("auto-pob-gate-power-{}", std::process::id()));
    let _ = fs::remove_dir_all(&user);
    fs::create_dir_all(&user).unwrap();
    let build = user.join("attack.xml");
    fs::copy(fixtures_dir().join("attack.xml"), &build).unwrap();
    let engine = Engine::spawn(Game::Poe1, std::env::temp_dir(), user.clone(), || {});
    engine.send(Command::OpenBuild { path: build });
    engine.send(Command::SetHeatmap {
        stat: Some("Life".into()),
        max_depth: None,
    });
    let deadline = Instant::now() + Duration::from_secs(300);
    let next = |deadline: Instant| {
        let wait = deadline.saturating_duration_since(Instant::now());
        engine.events.recv_timeout(wait).expect("an event")
    };
    let mut finished = false;
    // the calculation has started when its first progress arrives
    loop {
        match next(deadline) {
            Event::Progress { .. } => break,
            Event::Error { message, .. } | Event::StartupFailed { message } => {
                panic!("{message}")
            }
            Event::Snapshot {
                snapshot: Snapshot::NodePower(power),
                ..
            } => finished |= power.finished,
            _ => {}
        }
    }
    assert!(
        !finished,
        "the calculation ended before it reported progress"
    );
    for text in ["life", "fire", "area"] {
        engine.send(search(text));
        loop {
            match next(deadline) {
                Event::Snapshot {
                    snapshot: Snapshot::Tree(tree),
                    ..
                } if tree.search == text => break,
                Event::Snapshot {
                    snapshot: Snapshot::NodePower(power),
                    ..
                } => assert!(
                    !power.finished,
                    "the calculation ended before the search {text:?} was answered"
                ),
                Event::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        }
    }
    // the calculation still ends
    loop {
        if let Event::Snapshot {
            snapshot: Snapshot::NodePower(power),
            ..
        } = next(deadline)
            && power.finished
        {
            break;
        }
    }
    engine.send(Command::Shutdown);
    while engine.events.recv_timeout(Duration::from_secs(30)).is_ok() {}
    assert!(!engine.is_alive());
    let _ = fs::remove_dir_all(&user);
}

#[test]
#[ignore = "spike: prints numbers, needs the pinned PoB install"]
fn spike_tree_numbers() {
    let _guard = lock();
    let mut s = Session::start("spike-tree");
    let (_, xml) = fixtures().into_iter().find(|(n, _)| n == "attack").unwrap();
    s.open_fixture("attack", &xml);
    let lua = &s.backend.lua;

    let paths: String = lua
        .load(
            r#"
            local out = {}
            for id, node in pairs(pob.build.spec.nodes) do
                if node.path and not node.alloc then
                    local ids = {}
                    for _, p in ipairs(node.path) do ids[#ids + 1] = p.id end
                    out[#out + 1] = { node_id = id, path = ids }
                end
            end
            return require("dkjson").encode(out)
            "#,
        )
        .eval()
        .expect("collect paths");
    let parsed: serde_json::Value = serde_json::from_str(&paths).expect("paths json");
    let bytes = serde_json::to_vec(&parsed).unwrap().len();
    println!(
        "SPIKE path records: {}",
        parsed.as_array().map_or(0, |a| a.len())
    );
    println!("SPIKE path json bytes: {bytes}");
    assert!(bytes < 1_000_000, "path json is {bytes} bytes");

    // PoB's PowerBuilder yields once up front, whatever the clock says, so the
    // first BuildPower proves nothing. The second runs real work and only
    // yields mid-way when GetTime advances past its 100 ms slice.
    let (first_ms, second_ms, alive, steps): (i64, i64, bool, i64) = lua
        .load(
            r#"
            local calcs = pob.build.calcsTab
            for _, entry in ipairs(data.powerStatList) do
                if entry.stat == "Life" then calcs.powerStat = entry end
            end
            calcs.powerBuildFlag = true
            local t0 = GetTime()
            calcs:BuildPower()
            local t1 = GetTime()
            calcs:BuildPower()
            local t2 = GetTime()
            local alive = calcs.powerBuilder ~= nil
            local steps = 2
            while calcs.powerBuilder do
                calcs:BuildPower()
                steps = steps + 1
            end
            return t1 - t0, t2 - t1, alive, steps
            "#,
        )
        .eval()
        .expect("BuildPower");
    println!(
        "SPIKE BuildPower #1: {first_ms} ms, #2: {second_ms} ms, alive after #2={alive}, steps to finish={steps}"
    );
    assert!(
        alive,
        "the power coroutine should still be alive after its first real step"
    );
}

/// PoB's strings are bytes: a build saved by a Windows-1252 editor keeps bytes that aren't
/// UTF-8 in its notes, labels, items and custom modifiers. PoB shows such a build; every view
/// of ours shows it too, each such byte as a private-use escape (`bytes_text`), where serde
/// refused the whole view. An edit to such a field keeps the bytes on save, as PoB's does.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_build_with_bytes_that_are_not_utf8_shows_in_every_view() {
    let _guard = lock();
    let mut s = Session::start("non-utf8");
    let good = fs::read_to_string(fixture_xml("attack")).unwrap();
    // \u{1} marks where a lone Windows-1252 "é" byte goes; \u{2} and \u{3} are the bytes
    // 0x92 (a continuation byte) and 0xFC (never a lead byte)
    let marked = good
        .replace("label=\"\"", "label=\"Caf\u{1}\u{2}\u{3}\"")
        .replace(
            "Rarity: NORMAL\nDriftwood Maul",
            "Rarity: RARE\nCaf\u{1} Smasher\nDriftwood Maul",
        )
        .replace("<Notes>", "<Notes>caf\u{1} notes")
        // a loadout: the four sets share a title (Build.lua:301-415)
        .replace("<Spec ", "<Spec title=\"Tr\u{1}e\" ")
        .replace(
            "<SkillSet id=\"1\">",
            "<SkillSet id=\"1\" title=\"Tr\u{1}e\">",
        )
        .replace(
            "<ConfigSet id=\"1\">",
            "<ConfigSet id=\"1\" title=\"Tr\u{1}e\">\n<Input name=\"customMods\" \
             string=\"+5 to Strength&#10;Caf\u{1} line\"/>",
        )
        .replace(
            "<ItemSet useSecondWeaponSet=\"nil\" id=\"1\">",
            "<ItemSet useSecondWeaponSet=\"nil\" id=\"1\" title=\"Tr\u{1}e\">",
        );
    let bytes: Vec<u8> = marked
        .into_bytes()
        .into_iter()
        .map(|b| match b {
            1 => 0xE9,
            2 => 0x92,
            3 => 0xFC,
            b => b,
        })
        .collect();
    let path = s.builds_dir().join("latin.xml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    s.run(Command::OpenBuild { path: path.clone() })
        .expect("open");
    // the byte 0xE9 as the views show it
    const E: char = '\u{F7E9}';
    // and 0x92, 0xFC
    const Q: char = '\u{F792}';
    const U: char = '\u{F7FC}';
    let notes = s.backend.notes_view().expect("notes view");
    assert!(
        notes.text.contains(&format!("caf{E} notes")),
        "{:?}",
        notes.text
    );
    let skills = s.backend.skills_view().expect("skills view");
    assert_eq!(skills.groups[0].label, format!("Caf{E}{Q}{U}"));
    s.backend.items_view().expect("items view");
    let item = s.backend.item_text(1).expect("item text");
    assert!(
        item.text.contains(&format!("Caf{E} Smasher")),
        "{}",
        item.text
    );
    let group = s.backend.socket_group_text(0).expect("group text");
    assert!(group.text.contains(&format!("Caf{E}")), "{}", group.text);
    let mods = s.config().custom_mods[0].text.clone();
    assert!(mods.contains(&format!("Caf{E} line")), "{mods:?}");

    // edits to each field, as the UI sends them: the text it showed, changed
    s.run(Command::SetNotes {
        text: format!("{} more", notes.text),
    })
    .expect("notes");
    // each edit deletes the character just before a byte: the box's diff then holds the
    // bytes after it
    s.run(Command::SetGroupLabel {
        group: 0,
        text: format!("Ca{E}{Q}{U}"),
    })
    .expect("label");
    s.run(Command::SetGroupLabel {
        group: 0,
        text: format!("Ca{Q}{U}!"),
    })
    .expect("label");
    s.run(Command::SetCustomModText {
        group: 0,
        text: format!("{mods}\n+1 to Dexterity"),
    })
    .expect("custom mods");
    s.run(Command::EditItem { id: 1 }).expect("edit the item");
    let body = item
        .text
        .lines()
        .filter(|l| !l.starts_with("Rarity:"))
        .collect::<Vec<_>>()
        .join("\n");
    s.run(Command::SetDisplayItemText {
        rarity: 2,
        text: body.replacen("Smasher", "Smashers", 1),
    })
    .expect("item text");
    s.run(Command::SaveDisplayItem).expect("save the item");
    s.run(Command::Save).expect("save");
    let saved = fs::read(&path).unwrap();
    let has = |needle: &[u8]| saved.windows(needle.len()).any(|w| w == needle);
    assert!(
        !has("\u{fffd}".as_bytes()) && ![E, Q, U].iter().any(|c| has(c.to_string().as_bytes())),
        "an edit wrote the byte as text"
    );
    for needle in [
        &b"caf\xE9 notes"[..],
        b"label=\"Ca\x92\xFC!\"",
        b"Caf\xE9 line",
        b"+1 to Dexterity",
        // PoB's Save makes the item through sanitiseText (Item.lua:93): one byte, one "?"
        b"Caf? Smashers",
        b"Tr\xE9e",
    ] {
        assert!(
            has(needle),
            "{:?} isn't in the saved file",
            String::from_utf8_lossy(needle)
        );
    }
    let skills = s.backend.skills_view().expect("skills view");
    // what the UI shows is what it sends back: lookups by that text find the entry
    let build = s.backend.build_view().expect("build view");
    let loadout = build
        .loadouts
        .iter()
        .find(|l| l.contains(E))
        .unwrap_or_else(|| panic!("no loadout: {:?}", build.loadouts))
        .clone();
    s.run(Command::SelectLoadout { label: loadout })
        .expect("select the loadout");
    s.run(Command::DeleteSocketGroup {
        group: 0,
        name: Some(skills.groups[0].label.clone()),
    })
    .expect("delete the group");
    assert!(
        s.backend.skills_view().unwrap().groups.is_empty(),
        "the group is still there"
    );
}

/// A build's text can hold a real private-use character in the escapes' range (U+F7E9 here)
/// in a view serde takes, beside a byte that isn't UTF-8 elsewhere. Every view escapes it, in
/// whatever order the views come, so an edit saves it as it was, not as the byte 0xE9.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_private_use_character_round_trips_beside_bytes_that_are_not_utf8() {
    let _guard = lock();
    let mut s = Session::start("private-use");
    let good = fs::read_to_string(fixture_xml("attack")).unwrap();
    let mut bytes = good
        .replace("label=\"\"", "label=\"X\u{F7E9}Y\"")
        .replace("<Notes>", "<Notes>caf\u{1} notes")
        .into_bytes();
    for b in &mut bytes {
        if *b == 1 {
            *b = 0xE9;
        }
    }
    let path = s.builds_dir().join("pua.xml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    s.run(Command::OpenBuild { path: path.clone() })
        .expect("open");
    // the engine's order: Skills before Notes
    let skills = s.backend.skills_view().expect("skills view");
    let label = skills.groups[0].label.clone();
    assert_eq!(
        label, "X\u{F7EF}\u{F79F}\u{F7A9}Y",
        "the character isn't escaped"
    );
    s.backend.notes_view().expect("notes view");
    // an error that echoes text with such a byte shows it escaped too, not as U+FFFD
    let error = s
        .run(Command::SetGroupSlot {
            group: 0,
            slot: Some("Caf\u{F7E9}".into()),
        })
        .expect_err("no such slot")
        .to_string();
    assert_eq!(error, "unknown slot Caf\u{F7E9}");
    s.run(Command::SetGroupLabel {
        group: 0,
        text: format!("{label}!"),
    })
    .expect("label");
    s.run(Command::Save).expect("save");
    let saved = fs::read(&path).unwrap();
    let needle = "label=\"X\u{F7E9}Y!\"".as_bytes();
    assert!(
        saved.windows(needle.len()).any(|w| w == needle),
        "the label changed on save"
    );
    // file names are no build text: one in the escapes' range lists, opens and saves as
    // itself after the escapes are in use
    let named = s.builds_dir().join("x\u{F7E9}.xml");
    fs::copy(fixture_xml("attack"), &named).unwrap();
    let list = s.backend.build_list_view().expect("list");
    assert!(
        list.entries.iter().any(|e| e.name == "x\u{F7E9}"),
        "{:?}",
        list.entries.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    s.run(Command::OpenBuild {
        path: named.clone(),
    })
    .expect("open the build named in the range");
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "y\u{F7E9}".into(),
        overwrite: false,
    })
    .expect("save as a name in the range");
    assert!(s.builds_dir().join("y\u{F7E9}.xml").exists());
}

/// Once a build with bytes that aren't UTF-8 is left, the next build with none sends a
/// private-use character in the escapes' range as itself: the escapes went with the build
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_build_opened_after_one_with_escapes_saves_a_private_use_character_as_itself() {
    let _guard = lock();
    let mut s = Session::start("escapes-reset");
    let mut bytes = fs::read_to_string(fixture_xml("attack"))
        .unwrap()
        .replace("<Notes>", "<Notes>caf\u{1} notes")
        .into_bytes();
    for b in &mut bytes {
        if *b == 1 {
            *b = 0xE9;
        }
    }
    let latin = s.builds_dir().join("latin.xml");
    fs::create_dir_all(latin.parent().unwrap()).unwrap();
    fs::write(&latin, bytes).unwrap();
    s.run(Command::OpenBuild {
        path: latin.clone(),
    })
    .expect("open");
    let notes = s.backend.notes_view().expect("notes view");
    assert!(notes.text.contains('\u{F7E9}'), "{:?}", notes.text);
    // an import that fails keeps the build, and its escapes still go back as bytes
    s.run(Command::ImportShareCode {
        code: "not a code".into(),
    })
    .expect_err("not a code");
    s.run(Command::SetNotes {
        text: format!("{} more", notes.text),
    })
    .expect("notes");
    s.run(Command::Save).expect("save");
    let saved = fs::read(&latin).unwrap();
    let needle = b"caf\xE9 notes";
    assert!(
        saved.windows(needle.len()).any(|w| w == needle),
        "the byte didn't go back as itself"
    );
    let clean = s.builds_dir().join("clean.xml");
    fs::copy(fixture_xml("attack"), &clean).unwrap();
    s.run(Command::OpenBuild {
        path: clean.clone(),
    })
    .expect("open the clean build");
    s.backend.notes_view().expect("notes view");
    s.run(Command::SetNotes {
        text: "pua \u{F7E9} here".into(),
    })
    .expect("notes");
    s.run(Command::Save).expect("save");
    let saved = fs::read(&clean).unwrap();
    let needle = "pua \u{F7E9} here".as_bytes();
    assert!(
        saved.windows(needle.len()).any(|w| w == needle),
        "the character went out as a byte"
    );
}

/// A build file named with a real character in the escapes' range (U+F7E9), in a process
/// that never showed an escape: the build view gives its name and path as they are, and Save
/// As with the name its box opens on writes that same file, not a new one
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_build_named_in_the_escapes_range_saves_as_itself() {
    let _guard = lock();
    let mut s = Session::start("pua-name");
    let builds = s.builds_dir();
    fs::create_dir_all(&builds).unwrap();
    let named = builds.join("x\u{F7E9}.xml");
    fs::copy(fixture_xml("attack"), &named).unwrap();
    s.run(Command::OpenBuild {
        path: named.clone(),
    })
    .expect("open");
    let view = s.backend.build_view().unwrap();
    assert_eq!(view.name, "x\u{F7E9}");
    assert_eq!(view.save_as_name, "x\u{F7E9}");
    assert_eq!(view.path.as_deref(), Some(named.as_path()));
    s.run(Command::SaveAs {
        folder: String::new(),
        name: view.save_as_name.clone(),
        overwrite: true,
    })
    .expect("save as");
    let files: Vec<String> = fs::read_dir(&builds)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["x\u{F7E9}.xml"], "Save As wrote another file");
}

/// A build code PoB can't load raises in Build:Init (here a gem level that isn't a number:
/// CalcTools.lua:46). PoB reuses one build object, and its next Init ran SyncLoadouts against
/// the half-loaded tabs (Build.lua:601-604, 859), so every later open, import or new build
/// failed until a restart. Now the error is PoB's own reason and the next build loads.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_build_that_raises_while_loading_leaves_the_engine_working() {
    let _guard = lock();
    let mut s = Session::start("raises-loading");
    s.open_fixture("attack", &fixture_xml("attack"));
    let good = fs::read_to_string(fixture_xml("attack")).unwrap();
    let bad = good.replace("quality=\"0\" level=\"", "quality=\"0\" level=\"x");
    let code = |s: &Session, xml: &str| -> String {
        s.backend
            .lua
            .load(format!(
                "return (common.base64.encode(Deflate({xml:?})):gsub('+','-'):gsub('/','_'))"
            ))
            .eval::<String>()
            .unwrap()
    };
    let c = code(&s, &bad);
    let error = s
        .run(Command::ImportShareCode { code: c })
        .expect_err("PoB can't load that build")
        .to_string();
    assert!(error.contains("CalcTools.lua:46"), "{error}");
    s.backend.items_view().expect("items view");
    s.backend.skills_view().expect("skills view");
    let c = code(&s, &good);
    s.run(Command::ImportShareCode { code: c })
        .expect("a good code imports");
    assert!(!s.backend.skills_view().unwrap().groups.is_empty());
    // a file that raises the same way, then another build
    let path = s.builds_dir().join("raises.xml");
    fs::write(&path, &bad).unwrap();
    s.run(Command::OpenBuild { path })
        .expect_err("PoB can't load that build");
    let path = s.builds_dir().join("attack.xml");
    s.run(Command::OpenBuild { path }).expect("open");
    assert!(!s.backend.skills_view().unwrap().groups.is_empty());
    s.run(Command::NewBuild).expect("new");
    // a load by another way in than an open or a code (a link's answer lands through PoB's
    // own SetMode): the Init wrapper itself puts a new build in place, with its tabs
    s.backend
        .lua
        .globals()
        .set("RAISING_XML", bad.as_str())
        .unwrap();
    let state: String = s
        .backend
        .lua
        .load(
            r#"local ok = pcall(function()
                pob.main.main:SetMode("BUILD", false, "Landed", RAISING_XML)
                glue.frame()
            end)
            local b = pob.build
            return tostring(ok) .. " " .. tostring(b.skillsTab ~= nil and b.treeTab ~= nil
                and b.itemsTab ~= nil and b.configTab ~= nil) .. " " .. tostring(b.buildName)"#,
        )
        .eval()
        .unwrap();
    assert_eq!(
        state, "false true Unnamed build",
        "the raising load left no build"
    );
}

/// PoB pastes into its Import Comparison box at once (EditControl:Insert); ours typed the
/// text one character at a time, joining as it went, which took minutes for a long paste
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_long_comparison_paste_is_quick() {
    let _guard = lock();
    let mut s = Session::start("long-compare");
    s.open_fixture("attack", &fixture_xml("attack"));
    let start = std::time::Instant::now();
    let error = s
        .run(Command::LoadCompare {
            source: auto_pob::engine::CompareSource::Import {
                text: "A".repeat(400_000),
                name: None,
            },
        })
        .expect_err("not a build code")
        .to_string();
    assert!(error.contains("Invalid build code"), "{error}");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "took {:?}",
        start.elapsed()
    );
}

/// Undo and redo are each tab's own (PoB keeps an undo list per tab): an edit in one tab, then
/// edits in others, then that tab's undo takes back its own edit only; redo puts it back;
/// a save in between keeps the undo lists, and a reload shows what was saved
#[test]
#[ignore = "needs the pinned PoB install"]
fn undo_and_redo_stay_in_their_tab() {
    let _guard = lock();
    let mut s = Session::start("undo-tabs");
    let path = s.open_fixture("attack", &fixture_xml("attack"));
    let label = |s: &mut Session| s.backend.skills_view().unwrap().groups[0].label.clone();
    let allocated = |s: &mut Session| s.backend.tree_state_view().unwrap().allocated.len();
    let items = |s: &mut Session| s.backend.items_view().unwrap().items.len();
    let (label0, tree0, items0) = (label(&mut s), allocated(&mut s), items(&mut s));
    assert!(tree0 > 1 && items0 > 0);

    s.run(Command::SetGroupLabel {
        group: 0,
        text: "Edited".into(),
    })
    .unwrap();
    s.run(Command::ResetTree).unwrap();
    let id = s.backend.items_view().unwrap().items[0].id;
    s.run(Command::DeleteItem {
        id,
        confirmed: true,
    })
    .unwrap();
    let tree1 = allocated(&mut s);
    assert!(tree1 < tree0);
    assert_eq!(items(&mut s), items0 - 1);

    // the Skills undo takes back the label only
    s.run(Command::Undo(Tab::Skills)).unwrap();
    assert_eq!(label(&mut s), label0);
    assert_eq!(allocated(&mut s), tree1, "the Skills undo touched the tree");
    assert_eq!(
        items(&mut s),
        items0 - 1,
        "the Skills undo touched the items"
    );
    // the Items undo, then the Tree undo, each their own
    s.run(Command::Undo(Tab::Items)).unwrap();
    assert_eq!(items(&mut s), items0);
    assert_eq!(allocated(&mut s), tree1, "the Items undo touched the tree");
    s.run(Command::Undo(Tab::Tree)).unwrap();
    assert_eq!(allocated(&mut s), tree0);
    assert_eq!(label(&mut s), label0);
    // redo in another order
    s.run(Command::Redo(Tab::Tree)).unwrap();
    s.run(Command::Redo(Tab::Skills)).unwrap();
    assert_eq!(label(&mut s), "Edited");
    assert_eq!(allocated(&mut s), tree1);
    assert_eq!(
        items(&mut s),
        items0,
        "a redo of another tab redid the item delete"
    );
    assert!(s.backend.build_view().unwrap().unsaved);

    // a save keeps each tab's undo, and a reload shows what was saved
    s.run(Command::Save).unwrap();
    assert!(!s.backend.build_view().unwrap().unsaved);
    s.run(Command::Undo(Tab::Skills)).unwrap();
    assert_eq!(label(&mut s), label0, "the save dropped the Skills undo");
    s.run(Command::Redo(Tab::Skills)).unwrap();
    s.run(Command::Save).unwrap();
    s.run(Command::OpenBuild { path }).unwrap();
    assert_eq!(label(&mut s), "Edited");
    assert_eq!(allocated(&mut s), tree1);
    assert_eq!(items(&mut s), items0);
}

/// A quoted search word is put into a Lua pattern unescaped (PassiveTreeView.lua:789), so
/// `"[x"` raises in PoB's own Draw, and every frame after it while the text stays. The search
/// is refused instead, with PoB's reason; the last good search and the tree view stay. Other
/// pattern characters search as in PoB.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_search_pob_cant_read_is_refused_and_the_tree_keeps_working() {
    let _guard = lock();
    let mut s = attack_session("odd-search");
    s.backend.apply(search("life"), &|_| {}).expect("search");
    let before = s
        .backend
        .tree_state_view()
        .expect("tree view")
        .search_matches;
    assert!(!before.is_empty());
    let error = s
        .backend
        .apply(search("\"[x\""), &|_| {})
        .expect_err("a malformed pattern")
        .to_string();
    assert_eq!(error, "malformed pattern (missing ']')");
    let state = s
        .backend
        .tree_state_view()
        .expect("the tree view still works");
    assert_eq!(state.search, "life");
    assert_eq!(state.search_matches, before);
    for text in [
        "[x", "50%", "\"50%\"", "\"a(b\"", "%", "\"", "\"\"", "(", "oil:", "",
    ] {
        s.backend.apply(search(text), &|_| {}).expect(text);
        s.backend.tree_state_view().expect(text);
    }
}

/// A build file saved with a search PoB can't read (main could save one before the search
/// was refused): PoB's Draw raised at every frame, so the build didn't open. It opens with
/// the search blank.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_build_saved_with_a_search_pob_cant_read_opens() {
    let _guard = lock();
    let mut s = Session::start("saved-bad-search");
    let xml = fs::read_to_string(fixture_xml("attack"))
        .unwrap()
        .replace("searchStr=\"\"", "searchStr=\"&quot;[x&quot;\"");
    let path = s.builds_dir().join("bad.xml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, xml).unwrap();
    s.run(Command::OpenBuild { path }).expect("open");
    let state = s.backend.tree_state_view().expect("tree view");
    assert_eq!(state.search, "");
    assert!(!s.backend.build_view().unwrap().unsaved);
}

/// A build file whose name isn't UTF-8 (a Windows-1252 "é"): PoB on Linux lists it and opens
/// it. The list shows its name escaped, and its path opens and saves that same file.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_build_file_named_with_bytes_that_arent_utf8_opens() {
    use std::os::unix::ffi::OsStrExt;
    let _guard = lock();
    let mut s = Session::start("latin-name");
    let builds = s.builds_dir();
    fs::create_dir_all(&builds).unwrap();
    let named = builds.join(std::ffi::OsStr::from_bytes(b"caf\xE9.xml"));
    fs::copy(fixture_xml("attack"), &named).unwrap();
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .expect("list");
    let list = s.backend.build_list_view().expect("the list view");
    let entry = list
        .entries
        .iter()
        .find(|e| e.name == "caf\u{F7E9}")
        .unwrap_or_else(|| {
            panic!(
                "{:?}",
                list.entries.iter().map(|e| &e.name).collect::<Vec<_>>()
            )
        });
    assert_eq!(entry.path, named);
    s.run(Command::OpenBuild {
        path: entry.path.clone(),
    })
    .expect("open");
    assert_eq!(s.backend.build_view().unwrap().name, "caf\u{F7E9}");
    s.run(Command::SetNotes {
        text: "saved here".into(),
    })
    .expect("notes");
    s.run(Command::Save).expect("save");
    let saved = fs::read(&named).unwrap();
    assert!(
        saved.windows(10).any(|w| w == b"saved here"),
        "saved elsewhere"
    );
    assert_eq!(
        fs::read_dir(&builds).unwrap().count(),
        1,
        "another file was written"
    );
}

/// A failing `popup_live` doesn't stop the rest of leaving a build: the escapes flag is
/// kept (the safe side) and the open goes on (review M4)
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_failing_popup_check_leaves_the_build_change_working() {
    let _guard = lock();
    let mut s = attack_session("popup-live-fails");
    s.backend
        .lua
        .load("glue.popup_live = function() error('boom') end")
        .exec()
        .unwrap();
    let path = s.builds_dir().join("attack.xml");
    s.run(Command::OpenBuild { path })
        .expect("the open goes on");
    assert_eq!(s.backend.build_view().unwrap().name, "attack");
}

/// The bad-search guard loads PoB's viewer class only when PoB hasn't, and installs once:
/// the class and its Load stay the ones every viewer has (review M5)
#[test]
#[ignore = "needs the pinned PoB install"]
fn the_bad_search_guard_keeps_pobs_viewer_class() {
    let _guard = lock();
    let s = attack_session("guard-class");
    let same: bool = s
        .backend
        .lua
        .load(
            "local class = common.classes.PassiveTreeView
            local load = class.Load
            AutoPobGuardTreeSearch()
            return common.classes.PassiveTreeView == class
                and class.Load == load
                and getmetatable(pob.build.treeTab.viewer).__index == class",
        )
        .eval()
        .unwrap();
    assert!(same, "the class or its Load was replaced");
}

/// Save As of a build whose file name isn't UTF-8 keeps the bytes the view showed: its name
/// box opens with the name escaped, a name typed on to it is those bytes, and a folder the
/// list showed escaped is that folder (PoB's box holds the bytes, Build.lua:1399-1400)
#[test]
#[ignore = "needs the pinned PoB install"]
fn save_as_keeps_the_bytes_of_a_name_that_isnt_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let _guard = lock();
    let mut s = Session::start("latin-save-as");
    let builds = s.builds_dir();
    let file = |rel: &[u8]| builds.join(std::ffi::OsStr::from_bytes(rel));
    fs::create_dir_all(file(b"d\xE9p")).unwrap();
    fs::copy(fixture_xml("attack"), file(b"caf\xE9.xml")).unwrap();
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .expect("list");
    // the list the UI holds, whose spelling the folder below comes in
    let list = s.backend.build_list_view().expect("the list view");
    assert!(list.tree.iter().any(|f| f.path == "d\u{F7E9}p/"));
    s.run(Command::OpenBuild {
        path: file(b"caf\xE9.xml"),
    })
    .expect("open");
    let view = s.backend.build_view().unwrap();
    assert_eq!(view.save_as_name, "caf\u{F7E9}");
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "caf\u{F7E9} 2".into(),
        overwrite: false,
    })
    .expect("save as");
    assert!(file(b"caf\xE9 2.xml").is_file(), "not saved as its bytes");
    let view = s.backend.build_view().unwrap();
    s.run(Command::SaveAs {
        folder: "d\u{F7E9}p/".into(),
        name: format!("{} 3", view.save_as_name),
        overwrite: false,
    })
    .expect("save as into the folder");
    assert!(
        file(b"d\xE9p/caf\xE9 2 3.xml").is_file(),
        "not saved in its folder"
    );
    let names: Vec<_> = fs::read_dir(&builds)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names.len(),
        3,
        "another file or folder was written: {names:?}"
    );
}
