//! Gate tests for the Calcs tab: our sections against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`. It lives in `tests/gate/`
//! because cargo would build a file in `tests/` as a test binary of its own.

use super::{Session, fixtures, fixtures_dir, load_xml, lock, lua_value, pob_dir, upstream_lua};
use auto_pob::engine::{Backend, CalcsSection};
use std::fs;

const MODES: [&str; 4] = ["UNBUFFED", "BUFFED", "COMBAT", "EFFECTIVE"];

/// Upstream PoB with a fixture loaded and `calcs_lib.lua` defined.
fn upstream_with_calcs_lib(xml: &std::path::Path, name: &str) -> mlua::Lua {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &fs::read_to_string(xml).unwrap(), name).unwrap();
    lua.load(fs::read_to_string(fixtures_dir().join("calcs_lib.lua")).unwrap())
        .set_name("=calcs_lib")
        .exec()
        .expect("calcs_lib.lua");
    lua
}

/// Picks the calculation mode through the dropdown's own callback, as `SetCalcsSelect` will.
fn set_our_mode(s: &mut Session, buff_mode: &str) {
    s.backend
        .lua
        .load(format!(
            r#"
            local calcs = pob.build.calcsTab
            for _, section in ipairs(calcs.sectionList) do
                for _, subSec in ipairs(section.subSection) do
                    for _, rowData in ipairs(subSec.data) do
                        for _, colData in ipairs(rowData) do
                            if colData.controlName == "mode" then
                                colData.control:SelByValue(calcs.input.misc_buffMode, "buffMode")
                                for i, item in ipairs(colData.control.list) do
                                    if item.buffMode == "{buff_mode}" then colData.control:SetSel(i) end
                                end
                            end
                        end
                    end
                end
            end
            "#
        ))
        .exec()
        .unwrap();
    s.backend.recalculate().unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn format_calc_str_found() {
    let _guard = lock();
    let s = Session::start("calcs-format");
    let kind: String = s
        .backend
        .lua
        .load("return type(formatCalcStr)")
        .eval()
        .unwrap();
    assert_eq!(kind, "function");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_cells_match_pob() {
    let _guard = lock();
    for (name, xml) in fixtures() {
        let mut s = Session::start(&format!("calcs-{name}"));
        s.open_fixture(&name, &xml);
        let lua = upstream_with_calcs_lib(&xml, &name);
        let mut modes_seen = Vec::new();
        if name == "attack" {
            let sections = s.backend.calcs_view().unwrap().sections;
            let disabled: Vec<_> = sections.iter().filter(|s| !s.enabled).collect();
            assert!(!disabled.is_empty(), "no disabled section on attack");
            for section in disabled {
                assert_eq!(section.subs.len(), 1, "{}", section.id);
                assert!(section.subs[0].rows.is_empty(), "{}", section.id);
            }
        }
        for mode in MODES {
            set_our_mode(&mut s, mode);
            lua.load(format!("setMode('{mode}')")).exec().unwrap();
            let ours = s.backend.calcs_view().unwrap();
            let theirs: Vec<CalcsSection> = lua_value(&lua, "return calcsDump()");
            assert!(
                ours.sections.iter().any(|section| section.enabled),
                "{name} {mode}: nothing is enabled"
            );
            modes_seen.push(serde_json::to_value(&ours.sections).unwrap());
            assert_eq!(
                serde_json::to_value(&ours.sections).unwrap(),
                serde_json::to_value(&theirs).unwrap(),
                "{name} {mode}"
            );
        }
        // the mode really changed what the sections show
        if name == "attack" {
            assert_ne!(
                modes_seen[0], modes_seen[3],
                "UNBUFFED and EFFECTIVE look alike"
            );
        }
    }
}

/// Collapsing sub-sections and searching, set the way PoB's own controls do.
const COLLAPSE_AND_SEARCH: &str = r#"
    local calcs = %BUILD%.calcsTab
    calcs.controls.search.buf = "a"
    local collapsed_first, collapsed_later = false, false
    for _, section in ipairs(calcs.sectionList) do
        if not calcs:CheckFlag(section) then section.subSection[1].collapsed = true end
        if section.id ~= "SkillSelect" then
            if not collapsed_first and calcs:CheckFlag(section) then
                section.subSection[1].collapsed = true
                collapsed_first = true
            elseif not collapsed_later and section.subSection[2] then
                section.subSection[2].collapsed = true
                collapsed_later = true
            end
        end
    end
"#;

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_collapse_and_search_match_pob() {
    let _guard = lock();
    let (name, xml) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "attack")
        .unwrap();
    let mut s = Session::start("calcs-collapse");
    s.open_fixture(&name, &xml);
    let lua = upstream_with_calcs_lib(&xml, &name);
    s.backend
        .lua
        .load(COLLAPSE_AND_SEARCH.replace("%BUILD%", "pob.build"))
        .exec()
        .unwrap();
    lua.load(COLLAPSE_AND_SEARCH.replace("%BUILD%", "build"))
        .exec()
        .unwrap();
    set_our_mode(&mut s, "EFFECTIVE");
    lua.load("setMode('EFFECTIVE')").exec().unwrap();
    let ours = s.backend.calcs_view().unwrap();
    let theirs: Vec<CalcsSection> = lua_value(&lua, "return calcsDump()");
    assert_eq!(ours.search, "a");
    assert!(
        ours.sections
            .iter()
            .any(|s| !s.enabled && s.subs[0].collapsed)
    );
    assert!(
        ours.sections
            .iter()
            .flat_map(|s| &s.subs)
            .any(|s| s.collapsed)
    );
    assert!(
        ours.sections
            .iter()
            .flat_map(|s| &s.subs)
            .any(|s| s.search_match)
    );
    assert!(
        ours.sections
            .iter()
            .flat_map(|s| &s.subs)
            .flat_map(|s| &s.rows)
            .any(|r| r.search_match)
    );
    assert_eq!(
        serde_json::to_value(&ours.sections).unwrap(),
        serde_json::to_value(&theirs).unwrap()
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_view_cost() {
    let _guard = lock();
    let mut s = Session::start("calcs-cost");
    s.open_fixture("attack", &fixtures_dir().join("attack.xml"));
    let mut times = Vec::new();
    let mut size = 0;
    for _ in 0..20 {
        let start = std::time::Instant::now();
        let view = s.backend.calcs_view().unwrap();
        times.push(start.elapsed());
        size = serde_json::to_string(&view).unwrap().len();
    }
    times.sort();
    println!(
        "CALCS attack: median {:?} of 20 calcs_view calls, {size} bytes of JSON",
        times[10]
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn section_ids_are_unique() {
    let _guard = lock();
    for (name, xml) in fixtures() {
        let mut s = Session::start(&format!("calcs-ids-{name}"));
        s.open_fixture(&name, &xml);
        let view = s.backend.calcs_view().unwrap();
        let mut ids: Vec<&str> = view.sections.iter().map(|s| s.id.as_str()).collect();
        let total = ids.len();
        assert!(total > 10, "{name}: only {total} sections");
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "{name}: duplicate section ids");
    }
}
