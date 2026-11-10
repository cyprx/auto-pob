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
        // the dump runs PoB's full SetDisplayGroup, which may rewrite gems, so ours is read before it
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
    // a state PoB's SetDisplayGroup would rewrite: a gem level no gem has (ProcessSocketGroup
    // clamps it), another group on display and a slot choice that isn't the group's own
    s.backend
        .lua
        .load(
            r#"
            local t = pob.build.skillsTab
            t.displayGroup = t.socketGroupList[1]
            t.controls.groupSlot.selIndex = 3
            t.socketGroupList[1].gemList[1].level = 99
            "#,
        )
        .exec()
        .unwrap();
    let state = |s: &Session| -> serde_json::Value {
        lua_value(
            &s.backend.lua,
            r#"
            local t = pob.build.skillsTab
            local gems = {}
            for i, g in ipairs(t.socketGroupList) do
                for j, gem in ipairs(g.gemList) do
                    gems[#gems + 1] = table.concat({ i, j, gem.level, gem.quality,
                        tostring(gem.enabled), tostring(gem.count) }, "/")
                end
            end
            return {
                gems = table.concat(gems, ","),
                same_group = t.displayGroup == t.socketGroupList[1],
                slot_index = t.controls.groupSlot.selIndex,
                undo = #t.undo,
                redo = #t.redo,
            }
            "#,
        )
    };
    let unsaved = s.backend.build_view().unwrap().unsaved;
    let before = state(&s);
    assert!(
        before["gems"].as_str().unwrap().contains("/99/"),
        "{before}"
    );
    let first = s.backend.skills_view().unwrap();
    let second = s.backend.skills_view().unwrap();
    assert_eq!(first, second);
    assert_eq!(state(&s), before);
    assert_eq!(s.backend.build_view().unwrap().unsaved, unsaved);
    // the gem slots follow the restored group
    let colour_matches: bool = lua_value(
        &s.backend.lua,
        r#"local t = pob.build.skillsTab
        return t.gemSlots[1].nameSpec.inactiveCol == t.displayGroup.gemList[1].color"#,
    );
    assert!(colour_matches, "gem slots don't match the restored group");
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
