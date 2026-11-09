//! Gate tests: our engine against reference data from unmodified upstream PoB.
//!
//! Run with
//! `AUTO_POB_POB_DIR=<pinned PoB> cargo test -p auto-pob --test gate -- --ignored --test-threads=1`.

use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::{Golden, golden_of, load_xml, run_recipe, upstream_lua};
use auto_pob::engine::{Backend, Command, Engine, Event, Snapshot, StatsView};
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

    fn run(&mut self, command: Command) -> anyhow::Result<auto_pob::engine::Affected> {
        self.backend.apply(command, &|_| {})
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
        // the sidebar hides zero and flag-gated stats, so it shows only part of the golden set
        let life = view.stats.iter().find(|r| r.key == "Life");
        if name != "new" {
            assert!(
                life.is_some_and(|r| !r.text.is_empty()),
                "{name}: no Life row"
            );
        }
        match want.minion_stats {
            Some(ref minion) => assert_view_matches(&name, "minion stat", &view.minion, minion),
            None => assert!(view.minion.is_empty(), "{name}: unexpected minion rows"),
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
    // saving an unsaved-name build with Save fails, but a saved one works
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
    let user = std::env::temp_dir().join(format!("auto-pob-gate-spawn-{}", std::process::id()));
    let _ = fs::remove_dir_all(&user);
    let engine = Engine::spawn(Game::Poe1, std::env::temp_dir(), user.clone(), || {});
    let (mut ready, mut build, mut list, mut stats) = (false, false, false, false);
    let mut config_error = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !(ready && build && list && stats && config_error) && std::time::Instant::now() < deadline
    {
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
            // Task 7 implements the config view, then this branch goes away
            Ok(Event::Error { seq: 0, .. }) => config_error = true,
            Ok(Event::StartupFailed { message }) => panic!("startup failed: {message}"),
            Ok(_) => {}
            Err(e) => panic!("no event: {e}"),
        }
    }
    assert!(
        ready && build && list && stats,
        "{ready} {build} {list} {stats}"
    );
    engine.send(Command::Shutdown);
    let _ = fs::remove_dir_all(&user);
}
