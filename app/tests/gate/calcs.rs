//! Gate tests for the Calcs tab: our sections against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`. It lives in `tests/gate/`
//! because cargo would build a file in `tests/` as a test binary of its own.

use super::{
    Session, assert_sidebar, fixtures, fixtures_dir, load_xml, lock, lua_value, pob_dir,
    snapshot_dir, upstream_lua,
};
use auto_pob::engine::upstream::golden_of;
use auto_pob::engine::{
    Backend, BreakdownSection, BreakdownView, CalcsControl, CalcsCount, CalcsSection,
    CalcsSelector, CellRef, Command, Tab,
};
use auto_pob::game::Game;
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

/// Picks the calculation mode through the selector's dropdown, as the UI does.
fn set_our_mode(s: &mut Session, buff_mode: &str) {
    let index = MODES.iter().position(|m| *m == buff_mode).unwrap();
    s.run(Command::SetCalcsSelect {
        control: CalcsControl::Mode,
        index,
    })
    .unwrap();
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
    for fixture in ["attack", "minion"] {
        let mut s = Session::start("calcs-cost");
        s.open_fixture(fixture, &fixtures_dir().join(format!("{fixture}.xml")));
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
            "CALCS {fixture}: median {:?} of 20 calcs_view calls, {size} bytes of JSON",
            times[10]
        );
    }
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

fn fixture_xml(name: &str) -> std::path::PathBuf {
    fixtures()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"))
        .1
}

/// A session and an upstream state on the same fixture.
fn both(name: &str) -> (Session, mlua::Lua) {
    let xml = fixture_xml(name);
    let mut s = Session::start(&format!("calcs-sel-{name}"));
    s.open_fixture(name, &xml);
    (s, upstream_with_calcs_lib(&xml, name))
}

/// Our sections, selector and sidebar against upstream's, after `step`.
fn assert_matches_pob(s: &mut Session, lua: &mlua::Lua, step: &str) -> CalcsSelector {
    let ours = s.backend.calcs_view().unwrap();
    let sections: Vec<CalcsSection> = lua_value(lua, "return calcsDump()");
    assert_eq!(
        serde_json::to_value(&ours.sections).unwrap(),
        serde_json::to_value(&sections).unwrap(),
        "{step}: sections"
    );
    let selector: CalcsSelector = lua_value(lua, "return selectorDump()");
    assert_eq!(
        serde_json::to_value(&ours.selector).unwrap(),
        serde_json::to_value(&selector).unwrap(),
        "{step}: selector"
    );
    let want = golden_of(lua, Game::Poe1.pob_version()).unwrap();
    let stats = s.stats();
    assert_sidebar(step, "calcs", &stats.stats, &want.sidebar);
    if let Some(minion) = &want.minion_sidebar {
        assert_sidebar(step, "calcs minion", &stats.minion, minion);
    }
    ours.selector
}

fn select(s: &mut Session, lua: &mlua::Lua, control: CalcsControl, pob_name: &str, index: usize) {
    s.run(Command::SetCalcsSelect { control, index }).unwrap();
    lua.load(format!("calcsSelect('{pob_name}', {})", index + 1))
        .exec()
        .unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn skill_selector_matches_pob() {
    let _guard = lock();

    // attack: a multi-part skill, then every mode
    let (mut s, lua) = both("attack");
    let first = assert_matches_pob(&mut s, &lua, "attack start");
    let part = first.part.expect("Boneshatter has parts");
    assert!(part.labels.len() > 1);
    assert_eq!(first.mode.labels.len(), 4);
    assert_eq!(first.mode.selected, 3);
    assert!(first.mode_tooltip.contains("Unbuffed"));
    let before = s.backend.calcs_view().unwrap().sections;
    select(&mut s, &lua, CalcsControl::Part, "mainSkillPart", 1);
    let after = assert_matches_pob(&mut s, &lua, "attack part 1");
    assert_ne!(before, s.backend.calcs_view().unwrap().sections, "part 1");
    assert_eq!(after.part.unwrap().selected, 1);
    for (i, mode) in MODES.iter().enumerate() {
        select(&mut s, &lua, CalcsControl::Mode, "mode", i);
        let sel = assert_matches_pob(&mut s, &lua, &format!("attack {mode}"));
        assert_eq!(sel.mode.selected, i);
    }

    // dot: the second skill of the group
    let (mut s, lua) = both("dot");
    let first = assert_matches_pob(&mut s, &lua, "dot start");
    let skill = first.skill.expect("a skill list");
    assert!(skill.labels.len() > 1, "{:?}", skill.labels);
    let before = s.backend.calcs_view().unwrap().sections;
    select(&mut s, &lua, CalcsControl::Skill, "mainSkill", 1);
    let after = assert_matches_pob(&mut s, &lua, "dot skill 1");
    assert_ne!(before, s.backend.calcs_view().unwrap().sections, "skill 1");
    assert_eq!(after.skill.unwrap().selected, 1);

    // minion: show the minion, then pick its other skill
    let (mut s, lua) = both("minion");
    let first = assert_matches_pob(&mut s, &lua, "minion start");
    assert_eq!(first.show_minion, Some(false));
    let before = s.backend.calcs_view().unwrap().sections;
    s.run(Command::SetShowMinion { show: true }).unwrap();
    lua.load("calcsShowMinion(true)").exec().unwrap();
    let shown = assert_matches_pob(&mut s, &lua, "minion shown");
    assert_eq!(shown.show_minion, Some(true));
    let minion_cells = s.backend.calcs_view().unwrap().sections;
    assert_ne!(before, minion_cells, "showing the minion changed nothing");
    let skills = shown.minion_skill.expect("a minion skill list");
    assert!(skills.labels.len() > 1, "{:?}", skills.labels);
    select(
        &mut s,
        &lua,
        CalcsControl::MinionSkill,
        "mainSkillMinionSkill",
        1,
    );
    let after = assert_matches_pob(&mut s, &lua, "minion skill 1");
    assert_eq!(after.minion_skill.unwrap().selected, 1);
    assert_ne!(
        minion_cells,
        s.backend.calcs_view().unwrap().sections,
        "the minion skill changed nothing"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn hidden_calcs_control_is_an_error() {
    let _guard = lock();
    let (mut s, _lua) = both("dot");
    let before = s.backend.calcs_view().unwrap();
    assert!(before.selector.part.is_none());
    let err = s
        .run(Command::SetCalcsSelect {
            control: CalcsControl::Part,
            index: 0,
        })
        .unwrap_err();
    assert!(err.to_string().contains("isn't available"), "{err}");
    assert_eq!(s.backend.calcs_view().unwrap(), before);
    assert!(!s.backend.build_view().unwrap().unsaved);

    let mut s = Session::start("calcs-hidden-new");
    s.open_fixture("new", &fixture_xml("new"));
    for control in [CalcsControl::Skill, CalcsControl::Group] {
        let err = s
            .run(Command::SetCalcsSelect { control, index: 0 })
            .unwrap_err();
        assert!(err.to_string().contains("isn't available"), "{err}");
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn show_minion_without_minion_falls_back_to_player() {
    let _guard = lock();
    let (mut s, lua) = both("dot");
    let before = s.backend.calcs_view().unwrap();
    s.run(Command::SetShowMinion { show: true }).unwrap();
    let after = s.backend.calcs_view().unwrap();
    assert_eq!(
        serde_json::to_value(&before.sections).unwrap(),
        serde_json::to_value(&after.sections).unwrap()
    );
    lua.load("calcsShowMinion(true)").exec().unwrap();
    assert_matches_pob(&mut s, &lua, "dot minion shown");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn stage_count_text_is_filtered_like_pob() {
    let _guard = lock();
    let (mut s, lua) = both("attack");
    // the box is hidden for Boneshatter, but its callback works all the same
    for (text, want) in [("abc", ""), ("12x", "12")] {
        s.run(Command::SetCalcsCount {
            control: CalcsCount::Stages,
            text: text.into(),
        })
        .unwrap();
        lua.load(format!("calcsSetCount('mainSkillStageCount', '{text}')"))
            .exec()
            .unwrap();
        let ours: String = s
            .backend
            .lua
            .load(
                r#"for _, section in ipairs(pob.build.calcsTab.sectionList) do
                    if section.id == "SkillSelect" then return section.controls.mainSkillStageCount.buf end
                end"#,
            )
            .eval()
            .unwrap();
        let theirs: String = lua_value(&lua, "return calcsCountBuf('mainSkillStageCount')");
        assert_eq!(ours, want, "{text}");
        assert_eq!(theirs, want, "{text}");
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_collapse_is_saved_and_marks_unsaved() {
    let _guard = lock();
    let (mut s, _lua) = both("attack");
    assert!(!s.backend.build_view().unwrap().unsaved);
    s.run(Command::SetCalcsCollapsed {
        section: "HitDamage".into(),
        sub: 0,
        collapsed: true,
    })
    .unwrap();
    let collapsed = |s: &mut Session| {
        let view = s.backend.calcs_view().unwrap();
        view.sections
            .iter()
            .find(|x| x.id == "HitDamage")
            .unwrap()
            .subs[0]
            .collapsed
    };
    assert!(collapsed(&mut s));
    assert!(s.backend.build_view().unwrap().unsaved);
    s.run(Command::Save).unwrap();
    assert!(!s.backend.build_view().unwrap().unsaved);
    let path = s.backend.build_view().unwrap().path.unwrap();
    s.run(Command::NewBuild).unwrap();
    s.run(Command::OpenBuild { path }).unwrap();
    assert!(collapsed(&mut s));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_undo_restores_mode() {
    let _guard = lock();
    let (mut s, _lua) = both("attack");
    let original = s.backend.calcs_view().unwrap();
    assert!(!original.can_undo);
    set_our_mode(&mut s, "UNBUFFED");
    let changed = s.backend.calcs_view().unwrap();
    assert_eq!(changed.selector.mode.selected, 0);
    assert!(changed.can_undo);
    assert_ne!(changed.sections, original.sections);
    s.run(Command::Undo(Tab::Calcs)).unwrap();
    let undone = s.backend.calcs_view().unwrap();
    assert_eq!(undone.selector.mode.selected, 3);
    assert_eq!(undone.sections, original.sections);
    assert!(undone.can_redo);
    s.run(Command::Redo(Tab::Calcs)).unwrap();
    assert_eq!(s.backend.calcs_view().unwrap().selector.mode.selected, 0);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_search_flags_labels() {
    let _guard = lock();
    let (mut s, _lua) = both("attack");
    let unsaved = s.backend.build_view().unwrap().unsaved;
    let before = s.backend.calcs_view().unwrap();
    assert!(
        before
            .sections
            .iter()
            .flat_map(|x| &x.subs)
            .all(|x| !x.search_match)
    );
    s.run(Command::SetCalcsSearch {
        text: "damage".into(),
    })
    .unwrap();
    let view = s.backend.calcs_view().unwrap();
    assert_eq!(view.search, "damage");
    assert!(
        view.sections
            .iter()
            .flat_map(|x| &x.subs)
            .any(|x| x.search_match)
    );
    assert!(
        view.sections
            .iter()
            .flat_map(|x| &x.subs)
            .flat_map(|x| &x.rows)
            .any(|r| r.search_match)
    );
    // the cells (values) are the same
    let cells = |v: &auto_pob::engine::CalcsView| -> Vec<_> {
        v.sections
            .iter()
            .flat_map(|x| &x.subs)
            .flat_map(|x| &x.rows)
            .map(|r| r.cells.clone())
            .collect()
    };
    assert_eq!(cells(&before), cells(&view));
    assert_eq!(s.backend.build_view().unwrap().unsaved, unsaved);
}

/// `findCell` for the stat: it has to find one.
fn find_cell(lua: &mlua::Lua, stat: &str) -> CellRef {
    let cell: Option<CellRef> = lua_value(lua, &format!("return findCell({stat:?})"));
    cell.unwrap_or_else(|| panic!("upstream shows no breakdown cell for {stat}"))
}

/// Our breakdown of the cell against upstream's; the sections found.
fn assert_breakdown_matches(s: &mut Session, lua: &mlua::Lua, cell: &CellRef, step: &str) -> usize {
    let ours = s
        .backend
        .breakdown_view(cell)
        .unwrap_or_else(|e| panic!("{step}: {e:#}"));
    let theirs: BreakdownView = lua_value(
        lua,
        &format!(
            "return breakdownDump({{ section = {:?}, sub = {}, row = {}, cell = {} }})",
            cell.section, cell.sub, cell.row, cell.cell
        ),
    );
    assert_eq!(
        serde_json::to_value(&ours).unwrap(),
        serde_json::to_value(&theirs).unwrap(),
        "{step}: breakdown"
    );
    eprintln!("BREAKDOWN {step}: {} sections", theirs.sections.len());
    theirs.sections.len()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn breakdowns_match_pob() {
    let _guard = lock();
    let (mut s, lua) = both("attack");
    // Life has no breakdown without increases or more/less, and Armour none without armour
    // slots (this build has neither); both sides must agree on the empty answer.
    for (stat, empty) in [
        ("Life", true),
        ("Armour", true),
        ("FireResist", false),
        ("TotalDPS", false),
    ] {
        let cell = find_cell(&lua, stat);
        let n = assert_breakdown_matches(&mut s, &lua, &cell, stat);
        // asking again must not depend on the control's earlier data
        assert_eq!(assert_breakdown_matches(&mut s, &lua, &cell, stat), n);
        assert_eq!(n == 0, empty, "{stat}: {n} sections");
    }
    // Skill Average Hit shows for a skill that isn't an attack (`notFlag = "attack"`)
    let (mut s, lua) = both("dot");
    let cell = find_cell(&lua, "AverageHit");
    let n = assert_breakdown_matches(&mut s, &lua, &cell, "dot AverageHit");
    assert!(n > 0, "dot AverageHit has no sections");
    let (mut s, lua) = both("minion");
    s.run(Command::SetShowMinion { show: true }).unwrap();
    lua.load("calcsShowMinion(true)").exec().unwrap();
    let cell = find_cell(&lua, "Life");
    let n = assert_breakdown_matches(&mut s, &lua, &cell, "minion Life");
    assert!(n > 0, "minion Life has no sections");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn breakdown_of_vanished_cell_is_empty() {
    let _guard = lock();
    let (mut s, lua) = both("attack");
    let real = find_cell(&lua, "TotalDPS");
    let gone = [
        CellRef {
            section: "nope".into(),
            ..real.clone()
        },
        CellRef {
            sub: 99,
            ..real.clone()
        },
        CellRef {
            row: 999,
            ..real.clone()
        },
        CellRef {
            cell: 99,
            ..real.clone()
        },
    ];
    for cell in gone {
        let view = s.backend.breakdown_view(&cell).unwrap();
        assert_eq!(view.cell, cell);
        assert!(view.sections.is_empty(), "{cell:?}");
    }
    // a real one still has sections afterwards
    let view = s.backend.breakdown_view(&real).unwrap();
    assert!(matches!(
        view.sections.first(),
        Some(BreakdownSection::Text { .. } | BreakdownSection::Table { .. })
    ));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn every_shown_breakdown_matches_pob() {
    let _guard = lock();
    for name in ["attack", "dot", "minion"] {
        let (mut s, lua) = both(name);
        let view = s.backend.calcs_view().unwrap();
        let (mut cells, mut with_sections, mut tooltips, mut nodes) = (0, 0, 0, 0);
        for section in &view.sections {
            for (sub, x) in section.subs.iter().enumerate() {
                for row in &x.rows {
                    for cell in row.cells.iter().filter(|c| c.breakdown) {
                        let cell = CellRef {
                            section: section.id.clone(),
                            sub,
                            row: row.index,
                            cell: cell.index,
                        };
                        let n = assert_breakdown_matches(&mut s, &lua, &cell, &format!("{cell:?}"));
                        cells += 1;
                        with_sections += usize::from(n > 0);
                        for b in &s.backend.breakdown_view(&cell).unwrap().sections {
                            if let BreakdownSection::Table { rows, .. } = b {
                                tooltips += rows.iter().map(|r| r.tooltips.len()).sum::<usize>();
                                nodes += rows.iter().map(|r| r.nodes.len()).sum::<usize>();
                            }
                        }
                    }
                }
            }
        }
        eprintln!(
            "ALLBD {name}: {cells} cells, {with_sections} with sections, {tooltips} tooltips, {nodes} nodes"
        );
        assert!(with_sections > 0, "{name}");
    }
}

// ---- notes ----

fn set_notes(s: &mut Session, text: &str) {
    s.run(Command::SetNotes { text: text.into() }).unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn notes_round_trip() {
    let _guard = lock();
    let (mut s, _lua) = both("attack");
    set_notes(
        &mut s,
        "^xAF6025Unique^7 line\r\n\tTabbed ünïcode ^1red\u{1}\u{7f}",
    );
    let view = s.backend.notes_view().unwrap();
    assert!(!view.text.contains('\r'), "{:?}", view.text);
    assert!(!view.text.contains('\u{1}'), "{:?}", view.text);
    assert!(
        view.text.contains("\n\tTabbed ünïcode ^1red"),
        "{:?}",
        view.text
    );
    assert!(s.backend.build_view().unwrap().unsaved);
    let labels: Vec<_> = view.colours.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "NORMAL",
            "MAGIC",
            "RARE",
            "UNIQUE",
            "FIRE",
            "COLD",
            "LIGHTNING",
            "CHAOS",
            "STRENGTH",
            "DEXTERITY",
            "INTELLIGENCE",
            "DEFAULT"
        ]
    );
    assert_eq!(view.colours[3].code, "^xAF6025");
    assert_eq!(view.colours[11].code, "^7");
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "notes".into(),
        overwrite: false,
    })
    .unwrap();
    assert!(!s.backend.build_view().unwrap().unsaved);
    let saved = fs::read_to_string(s.builds_dir().join("notes.xml")).unwrap();
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &saved, "notes").unwrap();
    let theirs: String = lua
        .load("return build.notesTab.controls.edit.buf")
        .eval()
        .unwrap();
    assert_eq!(view.text, theirs);
    assert_eq!(s.backend.notes_view().unwrap().text, theirs);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn notes_unchanged_text_is_not_unsaved() {
    let _guard = lock();
    let (mut s, _lua) = both("attack");
    assert!(!s.backend.build_view().unwrap().unsaved);
    let text = s.backend.notes_view().unwrap().text;
    set_notes(&mut s, &text);
    assert!(!s.backend.build_view().unwrap().unsaved);
    // a change and then the original text again is no change
    set_notes(&mut s, "something else");
    assert!(s.backend.build_view().unwrap().unsaved);
    set_notes(&mut s, &text);
    assert!(!s.backend.build_view().unwrap().unsaved);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_and_notes_never_write_files() {
    let _guard = lock();
    let (mut s, lua) = both("attack");
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    let cell = find_cell(&lua, "TotalDPS");
    let section = s.backend.calcs_view().unwrap().sections[0].id.clone();
    let commands = vec![
        Command::SetCalcsSelect {
            control: CalcsControl::Mode,
            index: 1,
        },
        Command::SetCalcsCount {
            control: CalcsCount::Stages,
            text: "3".into(),
        },
        Command::SetShowMinion { show: true },
        Command::SetCalcsSearch {
            text: "damage".into(),
        },
        Command::SetCalcsCollapsed {
            section,
            sub: 0,
            collapsed: true,
        },
        Command::Undo(Tab::Calcs),
        Command::Redo(Tab::Calcs),
        Command::Breakdown(cell),
        Command::SetNotes {
            text: "a note ^1red".into(),
        },
    ];
    for command in commands {
        let shown = format!("{command:?}");
        // a command PoB doesn't offer for this build may fail, but it must not write either
        let _ = s.run(command).map_err(|e| eprintln!("{shown}: {e}"));
        assert_eq!(snapshot_dir(&dir), before, "{shown} wrote a file");
    }
    assert!(s.backend.build_view().unwrap().unsaved);
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "after".into(),
        overwrite: false,
    })
    .unwrap();
    let after = snapshot_dir(&dir);
    assert!(after.keys().any(|p| p.ends_with("after.xml")));
    for (path, bytes) in &before {
        assert!(after[path] == *bytes, "{} changed", path.display());
    }
    assert!(
        snapshot_dir(&s.user)
            .keys()
            .all(|p| !p.to_string_lossy().contains("~~temp~~")),
        "a temporary file was left"
    );
}

/// A build saved on the Calcs tab (`viewMode="CALCS"`) shows the same sections and
/// breakdowns as the fixture saved on the tree tab: our frames never draw the Calcs tab.
#[test]
#[ignore = "needs the pinned PoB install"]
fn calcs_view_mode_build_matches_pob() {
    let _guard = lock();
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    assert!(xml.contains("viewMode=\"TREE\""));
    let mut s = Session::start("calcs-view-mode");
    fs::create_dir_all(&s.user).unwrap();
    let path = s.user.join("source.xml");
    fs::write(
        &path,
        xml.replace("viewMode=\"TREE\"", "viewMode=\"CALCS\""),
    )
    .unwrap();
    s.open_fixture("attack", &path);
    let view_mode: String = lua_value(&s.backend.lua, "return pob.build.viewMode");
    assert_eq!(view_mode, "CALCS");
    let lua = upstream_with_calcs_lib(&fixture_xml("attack"), "attack");
    assert_matches_pob(&mut s, &lua, "CALCS viewMode");
    for stat in ["FireResist", "TotalDPS"] {
        let cell = find_cell(&lua, stat);
        let n = assert_breakdown_matches(&mut s, &lua, &cell, stat);
        assert!(n > 0, "{stat}");
        // a recalculation's frame leaves the breakdown alone too
        s.backend.recalculate().unwrap();
        assert_eq!(assert_breakdown_matches(&mut s, &lua, &cell, stat), n);
    }
    assert_matches_pob(&mut s, &lua, "CALCS viewMode after frames");
}
