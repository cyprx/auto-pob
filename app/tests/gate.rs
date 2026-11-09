//! Gate tests: our engine against reference data from unmodified upstream PoB.
//!
//! Run with
//! `AUTO_POB_POB_DIR=<pinned PoB> cargo test -p auto-pob --test gate -- --ignored --test-threads=1`.

use auto_pob::engine::upstream::{Golden, golden_of, load_xml, run_recipe, upstream_lua};
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
