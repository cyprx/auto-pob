//! Gate tests: our engine against reference data from unmodified upstream PoB.
//!
//! Run with
//! `AUTO_POB_POB_DIR=<pinned PoB> cargo test -p auto-pob --test gate -- --ignored --test-threads=1`.

use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::{Golden, golden_of, load_xml, run_recipe, upstream_lua};
use auto_pob::engine::{
    Backend, Command, ConfigOption, ConfigValue, ConfigView, Engine, Event, Snapshot, StatsView,
    Tab,
};
use auto_pob::game::Game;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// PoB loads files relative to the process working directory, so only one test may run it.
static CWD: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    CWD.lock().unwrap_or_else(|e| e.into_inner())
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
        &[
            "Boneshatter",
            "Melee Physical Damage Support",
            "Brutality Support",
        ],
    ),
    (
        "dot",
        "Witch",
        90,
        20,
        &["Essence Drain", "Contagion", "Void Manipulation Support"],
    ),
    (
        "minion",
        "Witch",
        90,
        20,
        &[
            "Raise Zombie",
            "Minion Damage Support",
            "Melee Splash Support",
        ],
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
        let user =
            std::env::temp_dir().join(format!("auto-pob-gate-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&user);
        let backend = PobBackend::start(Game::Poe1, &pob_dir(), &user).expect("starting PoB");
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
    assert!(format!("{err:#}").contains("network features"), "{err:#}");
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
    let groups = vec![
        CustomModGroup {
            title: "Default".into(),
            enabled: false,
            text: "+100 to maximum Life".into(),
        },
        CustomModGroup {
            title: "Speed".into(),
            enabled: true,
            text: "+10% increased Attack Speed".into(),
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

#[test]
#[ignore = "needs the pinned PoB install"]
fn lab_draws_the_tree_and_ranks_nodes() {
    use auto_pob::engine::{LabCommand, LabEvent};
    use std::cell::RefCell;

    let _guard = lock();
    let mut s = Session::start("lab");
    let (name, xml) = fixtures().into_iter().next().expect("a fixture");
    s.open_fixture(&name, &xml);

    let events = RefCell::new(Vec::new());
    let mut lab = |command| {
        s.backend
            .apply(Command::Lab(command), &|e| events.borrow_mut().push(e))
            .expect("lab command");
    };
    lab(LabCommand::Tree);
    lab(LabCommand::NodePower {
        metric: "Life",
        max_distance: 4,
    });
    let events = events.into_inner();
    let tree = events
        .iter()
        .find_map(|e| match e {
            Event::Lab(LabEvent::Tree(t)) => Some(t),
            _ => None,
        })
        .expect("tree");
    assert!(tree.nodes.len() > 2000, "{}", tree.nodes.len());
    assert!(tree.edges.len() > 2000);
    assert!(!tree.allocated.is_empty(), "the build allocates nodes");
    assert!(tree.sheets.iter().all(Option::is_some));
    let scores = events
        .iter()
        .find_map(|e| match e {
            Event::Lab(LabEvent::NodePower(s)) => Some(s),
            _ => None,
        })
        .expect("node power");
    assert!(!scores.is_empty());
    assert!(scores.iter().all(|s| (1..=4).contains(&s.distance)));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Lab(LabEvent::Progress { .. }))),
        "progress is streamed"
    );
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
