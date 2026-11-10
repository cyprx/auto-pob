//! Gate tests for the Skills tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{Session, fixtures, fixtures_dir, load_xml, lock, lua_value, pob_dir, upstream_lua};
use auto_pob::engine::{Backend, SkillsView};
use std::fs;

/// Upstream PoB with a fixture loaded and `skills_lib.lua` defined.
fn upstream_with_skills_lib(xml: &std::path::Path, name: &str) -> mlua::Lua {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &fs::read_to_string(xml).unwrap(), name).unwrap();
    lua.load(fs::read_to_string(fixtures_dir().join("skills_lib.lua")).unwrap())
        .set_name("=skills_lib")
        .exec()
        .expect("skills_lib.lua");
    lua
}

/// The view as JSON, without the undo flags upstream's dump leaves out.
fn comparable(view: &SkillsView) -> serde_json::Value {
    let mut value = serde_json::to_value(view).unwrap();
    let map = value.as_object_mut().unwrap();
    map.remove("can_undo");
    map.remove("can_redo");
    value
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn skills_view_matches_pob() {
    let _guard = lock();
    for (name, xml) in fixtures() {
        let mut s = Session::start(&format!("skills-{name}"));
        s.open_fixture(&name, &xml);
        let lua = upstream_with_skills_lib(&xml, &name);
        let ours = s.backend.skills_view().unwrap();
        // the dump runs PoB's full SetDisplayGroup, which may rewrite gems; read ours first
        let value: mlua::Value = lua.load("return skillsDump()").eval().unwrap();
        let theirs: serde_json::Value =
            serde_json::to_value(mlua::LuaSerdeExt::from_value::<SkillsView>(&lua, value).unwrap())
                .unwrap();
        let mut theirs = theirs;
        let map = theirs.as_object_mut().unwrap();
        map.remove("can_undo");
        map.remove("can_redo");
        assert_eq!(comparable(&ours), theirs, "{name}");
        if name != "new" {
            assert!(!ours.groups.is_empty(), "{name}: no socket groups");
            assert!(
                ours.groups[0].gems.iter().all(|g| g.error.is_none()),
                "{name}: a gem has an error"
            );
        }
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn skills_view_is_pure() {
    let _guard = lock();
    let (_, xml) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "attack")
        .expect("attack fixture");
    let mut s = Session::start("skills-pure");
    s.open_fixture("attack", &xml);
    let unsaved = s.backend.build_view().unwrap().unsaved;
    let depth =
        |s: &Session| -> usize { lua_value(&s.backend.lua, "return #pob.build.skillsTab.undo") };
    let before = depth(&s);
    let first = s.backend.skills_view().unwrap();
    let second = s.backend.skills_view().unwrap();
    assert_eq!(first, second);
    assert_eq!(s.backend.build_view().unwrap().unsaved, unsaved);
    assert_eq!(depth(&s), before);
}

/// Prints the median time of `skills_view()` and the size of its JSON, for the task report.
#[test]
#[ignore = "needs the pinned PoB install"]
fn skills_view_is_cheap() {
    let _guard = lock();
    let (_, xml) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "attack")
        .expect("attack fixture");
    let mut s = Session::start("skills-time");
    s.open_fixture("attack", &xml);
    let mut times: Vec<std::time::Duration> = (0..20)
        .map(|_| {
            let start = std::time::Instant::now();
            s.backend.skills_view().unwrap();
            start.elapsed()
        })
        .collect();
    times.sort();
    let json = serde_json::to_string(&s.backend.skills_view().unwrap()).unwrap();
    eprintln!("skills_view median {:?}, {} bytes", times[10], json.len());
    eprintln!("{json}");
    assert!(times[10] < std::time::Duration::from_millis(100));
}
