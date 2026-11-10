//! Gate tests for the Skills tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{
    Session, fixtures, fixtures_dir, load_xml, lock, lua_value, pob_dir, recipe_setting,
    upstream_lua,
};
use auto_pob::engine::upstream::{SidebarLine, golden_of};
use auto_pob::engine::{
    Backend, Command, GemToggle, SkillSetEntry, SkillsView, SocketGroupView, Tab,
};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
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

/// What upstream's Skills tab shows now (`skillsDump()`).
fn upstream_dump(lua: &mlua::Lua) -> SkillsView {
    let value: mlua::Value = lua.load("return skillsDump()").eval().unwrap();
    lua.from_value(value).unwrap()
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
        let theirs = upstream_dump(&lua);
        assert_eq!(ours, theirs, "{name}");
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

// ---- Recorded recipes ----

/// `GemToggle` as the recipe library names it
#[derive(Debug, Clone, Copy, serde::Deserialize)]
enum RecordedToggle {
    First,
    Second,
}

/// One step of a recipe as `skills_lib.lua`'s `act.*` recorded it, with 0-based indices.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "cmd")]
enum SkillsAction {
    AddSocketGroup,
    DeleteSocketGroup {
        group: usize,
    },
    DeleteAllSocketGroups,
    MoveSocketGroup {
        from: usize,
        to: usize,
    },
    PasteSocketGroup {
        text: String,
    },
    SetGroupLabel {
        group: usize,
        text: String,
    },
    SetGroupSlot {
        group: usize,
        slot: Option<String>,
    },
    SetGroupEnabled {
        group: usize,
        on: bool,
    },
    SetGroupFullDps {
        group: usize,
        on: bool,
    },
    SetGem {
        group: usize,
        slot: usize,
        gem_id: String,
    },
    SetGemLevel {
        group: usize,
        slot: usize,
        text: String,
    },
    SetGemQuality {
        group: usize,
        slot: usize,
        text: String,
    },
    SetGemCount {
        group: usize,
        slot: usize,
        text: String,
    },
    SetGemEnabled {
        group: usize,
        slot: usize,
        on: bool,
    },
    SetGemToggle {
        group: usize,
        slot: usize,
        which: RecordedToggle,
        on: bool,
    },
    DeleteGem {
        group: usize,
        slot: usize,
    },
    Undo,
    Redo,
    /// Built from the recorded checkpoint table, which can hold empty lists
    #[serde(skip_deserializing)]
    Check {
        check: SkillsCheckpoint,
    },
}

impl SkillsAction {
    /// The command that replays this action; none for a checkpoint.
    fn command(&self) -> Option<Command> {
        Some(match self.clone() {
            SkillsAction::AddSocketGroup => Command::AddSocketGroup,
            SkillsAction::DeleteSocketGroup { group } => Command::DeleteSocketGroup { group },
            SkillsAction::DeleteAllSocketGroups => Command::DeleteAllSocketGroups,
            SkillsAction::MoveSocketGroup { from, to } => Command::MoveSocketGroup { from, to },
            SkillsAction::PasteSocketGroup { text } => Command::PasteSocketGroup { text },
            SkillsAction::SetGroupLabel { group, text } => Command::SetGroupLabel { group, text },
            SkillsAction::SetGroupSlot { group, slot } => Command::SetGroupSlot { group, slot },
            SkillsAction::SetGroupEnabled { group, on } => Command::SetGroupEnabled { group, on },
            SkillsAction::SetGroupFullDps { group, on } => Command::SetGroupFullDps { group, on },
            SkillsAction::SetGem {
                group,
                slot,
                gem_id,
            } => Command::SetGem {
                group,
                slot,
                gem_id,
            },
            SkillsAction::SetGemLevel { group, slot, text } => {
                Command::SetGemLevel { group, slot, text }
            }
            SkillsAction::SetGemQuality { group, slot, text } => {
                Command::SetGemQuality { group, slot, text }
            }
            SkillsAction::SetGemCount { group, slot, text } => {
                Command::SetGemCount { group, slot, text }
            }
            SkillsAction::SetGemEnabled { group, slot, on } => {
                Command::SetGemEnabled { group, slot, on }
            }
            SkillsAction::SetGemToggle {
                group,
                slot,
                which,
                on,
            } => Command::SetGemToggle {
                group,
                slot,
                which: match which {
                    RecordedToggle::First => GemToggle::First,
                    RecordedToggle::Second => GemToggle::Second,
                },
                on,
            },
            SkillsAction::DeleteGem { group, slot } => Command::DeleteGem { group, slot },
            SkillsAction::Undo => Command::Undo(Tab::Skills),
            SkillsAction::Redo => Command::Redo(Tab::Skills),
            SkillsAction::Check { .. } => return None,
        })
    }
}

/// The Skills tab at one point of a recipe: the model's part of `skillsDump()` (not the
/// options and the slot choices, which no command here changes) and the sidebar.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct SkillsCheckpoint {
    sets: Vec<SkillSetEntry>,
    active_set: u32,
    groups: Vec<SocketGroupView>,
    can_undo: bool,
    can_redo: bool,
    sidebar: Vec<SidebarLine>,
}

/// A recipe run in upstream PoB: the steps it took and where they ended.
struct SkillsRecipe {
    base: String,
    /// The build to start from when the recipe prepared one
    setup_xml: Option<String>,
    actions: Vec<SkillsAction>,
    result: SkillsView,
}

/// Runs `skills/<name>.lua` in upstream PoB on its base fixture.
fn upstream_skills_recipe(name: &str) -> SkillsRecipe {
    let source = fs::read_to_string(fixtures_dir().join(format!("skills/{name}.lua"))).unwrap();
    let base = recipe_setting(&source, "base").unwrap_or_else(|| panic!("{name}: no base"));
    let lua = upstream_with_skills_lib(&fixtures_dir().join(format!("{base}.xml")), &base);
    // a checkpoint records PoB's sidebar lines, which the golden extraction reads
    let version = Game::Poe1.pob_version().to_string();
    lua.globals()
        .set(
            "sidebar_lines",
            lua.create_function(move |lua, ()| {
                let golden = golden_of(lua, &version).map_err(mlua::Error::external)?;
                lua.to_value(&golden.sidebar)
            })
            .unwrap(),
        )
        .unwrap();
    lua.load(source)
        .set_name(format!("={name}"))
        .exec()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    lua.load("recipe()")
        .exec()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let recorded: mlua::Table = lua.globals().get("actions").unwrap();
    let actions = recorded
        .sequence_values::<mlua::Table>()
        .map(|entry| {
            let entry = entry.unwrap();
            if let Ok(check) = entry.get::<mlua::Table>("check") {
                let mut check: SkillsCheckpoint = lua
                    .from_value(mlua::Value::Table(check))
                    .unwrap_or_else(|e| panic!("{name}: checkpoint: {e}"));
                without_skill_dps(&mut check.sidebar);
                return SkillsAction::Check { check };
            }
            let json: serde_json::Value = lua.from_value(mlua::Value::Table(entry)).unwrap();
            serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{name}: {json}: {e}"))
        })
        .collect();
    SkillsRecipe {
        base,
        setup_xml: lua.globals().get("setup_xml").unwrap(),
        actions,
        result: upstream_dump(&lua),
    }
}

/// PoB lists one sidebar line per skill after "Full DPS" (the `SkillDPS` stat) once a group is in
/// Full DPS. Our stats view doesn't list them yet (a gap from sub-project 1, not the Skills
/// tab's), so checkpoints compare the sidebar up to the "Full DPS" line.
fn without_skill_dps(lines: &mut Vec<SidebarLine>) {
    if let Some(at) = lines.iter().position(|l| l.label == "Full DPS") {
        lines.truncate(at + 1);
    }
}

/// Our Skills tab and sidebar in the shape of a recipe's `checkpoint()`.
fn our_skills_checkpoint(s: &mut Session) -> SkillsCheckpoint {
    let view = s.backend.skills_view().expect("skills view");
    let mut sidebar: Vec<SidebarLine> = s
        .stats()
        .stats
        .iter()
        .map(|r| SidebarLine {
            label: r.label.clone(),
            text: r.text.clone(),
        })
        .collect();
    without_skill_dps(&mut sidebar);
    SkillsCheckpoint {
        sets: view.sets,
        active_set: view.active_set,
        groups: view.groups,
        can_undo: view.can_undo,
        can_redo: view.can_redo,
        sidebar,
    }
}

/// Replays a recipe as our commands and checks every checkpoint and the end state against
/// upstream.
fn check_skills_recipe(name: &str) -> SkillsRecipe {
    let recipe = upstream_skills_recipe(name);
    assert!(!recipe.actions.is_empty(), "{name}: no actions recorded");
    let mut s = Session::start(&format!("skills-recipe-{name}"));
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
    let before = s.backend.skills_view().unwrap();
    let mut checked = 0;
    for action in &recipe.actions {
        match action.command() {
            Some(command) => {
                s.run(command)
                    .unwrap_or_else(|e| panic!("{name}: {action:?}: {e}"));
            }
            None => {
                let SkillsAction::Check { check } = action else {
                    unreachable!()
                };
                let ours = our_skills_checkpoint(&mut s);
                assert_eq!(&ours, check, "{name}: checkpoint {checked}");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "{name}: no checkpoint");
    let ours = s.backend.skills_view().unwrap();
    assert_ne!(ours, before, "{name}: the recipe changes the Skills tab");
    assert_eq!(ours, recipe.result, "{name}");
    recipe
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn group_recipes_match_pob() {
    let _guard = lock();
    let recipe = check_skills_recipe("groups");
    // coverage the first review asked for: a group that isn't main, and a disabled one
    let mid = recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            SkillsAction::Check { check } => Some(check),
            _ => None,
        })
        .nth(1)
        .unwrap();
    assert!(mid.groups.iter().any(|g| !g.is_main));
    assert!(mid.groups.iter().any(|g| !g.enabled));
}

fn session_on(fixture: &str, test: &str) -> (Session, mlua::Lua) {
    let xml = fixtures_dir().join(format!("{fixture}.xml"));
    let mut s = Session::start(&format!("skills-{test}-{fixture}"));
    s.open_fixture(fixture, &xml);
    let lua = upstream_with_skills_lib(&xml, fixture);
    (s, lua)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn copy_matches_pob() {
    let _guard = lock();
    for fixture in ["attack", "minion"] {
        let (mut s, lua) = session_on(fixture, "copy");
        let ours = s.backend.socket_group_text(0).unwrap();
        assert_eq!(ours.group, 0);
        let theirs: String = lua.load("return copyText(0)").eval().unwrap();
        assert!(theirs.contains("\r\n"), "{fixture}: {theirs:?}");
        assert_eq!(ours.text, theirs, "{fixture}");
        // pasting it back adds the same group on both sides
        s.run(Command::PasteSocketGroup {
            text: ours.text.clone(),
        })
        .unwrap();
        lua.globals().set("pasted", theirs).unwrap();
        lua.load("act.paste(pasted)").exec().unwrap();
        assert_eq!(
            s.backend.skills_view().unwrap(),
            upstream_dump(&lua),
            "{fixture}"
        );
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn copy_restores_copy_global() {
    let _guard = lock();
    let (mut s, _lua) = session_on("attack", "copy-global");
    let lua = &s.backend.lua;
    lua.load("saved_copy = Copy").exec().unwrap();
    let is_function: bool = lua_value(lua, "return type(saved_copy) == 'function'");
    assert!(is_function, "the bootstrap defines Copy");
    let same =
        |s: &Session| -> bool { lua_value(&s.backend.lua, "return rawequal(Copy, saved_copy)") };
    s.backend.socket_group_text(0).unwrap();
    assert!(same(&s), "after a copy");
    // a group PoB can't copy makes the call fail; PoB's own error path restores Copy too
    assert!(s.backend.socket_group_text(99).is_err());
    assert!(same(&s), "after a failing copy");
    s.backend
        .lua
        .load(
            r#"
            local tab = pob.build.skillsTab
            function tab:CopySocketGroup() error("boom") end
            "#,
        )
        .exec()
        .unwrap();
    assert!(s.backend.socket_group_text(0).is_err());
    assert!(same(&s), "after PoB's copy raised");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn delete_before_main_shifts_main() {
    let _guard = lock();
    let (mut s, lua) = session_on("attack", "main-shift");
    let pasted = "Hatred 20/0  1\r\n";
    for _ in 0..2 {
        s.run(Command::PasteSocketGroup {
            text: pasted.into(),
        })
        .unwrap();
        lua.globals().set("pasted", pasted).unwrap();
        lua.load("act.paste(pasted)").exec().unwrap();
    }
    // Task 6 replaces this with SetMainSocketGroup; the Calcs selector follows the same group
    let set_main = "local b = pob and pob.build or build; b.mainSocketGroup = 3; b.calcsTab.input.skill_number = 3";
    s.backend.lua.load(set_main).exec().unwrap();
    lua.load(set_main).exec().unwrap();
    s.run(Command::DeleteSocketGroup { group: 0 }).unwrap();
    lua.load("act.delete_group(0)").exec().unwrap();
    s.run(Command::MoveSocketGroup { from: 1, to: 0 }).unwrap();
    lua.load("act.move_group(1, 0)").exec().unwrap();
    let ours = s.backend.skills_view().unwrap();
    let theirs = upstream_dump(&lua);
    let mains = |v: &SkillsView| v.groups.iter().map(|g| g.is_main).collect::<Vec<_>>();
    assert_eq!(mains(&ours), mains(&theirs));
    assert!(mains(&ours).iter().any(|m| *m), "a group is main");
    let skill_number = |lua: &mlua::Lua| -> u32 {
        lua_value(
            lua,
            "return (pob and pob.build or build).calcsTab.input.skill_number",
        )
    };
    assert_eq!(skill_number(&s.backend.lua), skill_number(&lua));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn paste_without_gems_is_an_error() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "paste-error");
    let before = s.backend.skills_view().unwrap();
    let err = s
        .run(Command::PasteSocketGroup {
            text: "hello".into(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("no gems"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn undo_redo_skills() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "undo");
    let original = s.backend.skills_view().unwrap();
    assert!(!original.can_undo && !original.can_redo);
    s.run(Command::AddSocketGroup).unwrap();
    s.run(Command::PasteSocketGroup {
        text: "Hatred 20/0  1\r\n".into(),
    })
    .unwrap();
    s.run(Command::SetGroupLabel {
        group: 0,
        text: "Edited".into(),
    })
    .unwrap();
    let edited = s.backend.skills_view().unwrap();
    assert_eq!(edited.groups.len(), original.groups.len() + 2);
    assert_eq!(edited.groups[0].label, "Edited");
    assert!(edited.can_undo && !edited.can_redo);
    for _ in 0..3 {
        s.run(Command::Undo(Tab::Skills)).unwrap();
    }
    let undone = s.backend.skills_view().unwrap();
    assert_eq!(undone.groups, original.groups);
    assert!(!undone.can_undo && undone.can_redo);
    for _ in 0..3 {
        s.run(Command::Redo(Tab::Skills)).unwrap();
    }
    let redone = s.backend.skills_view().unwrap();
    assert_eq!(redone.groups, edited.groups);
    assert!(redone.can_undo && !redone.can_redo);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn out_of_range_group_is_an_error() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "range");
    let before = s.backend.skills_view().unwrap();
    let missing = before.groups.len() + 5;
    let no_group = "no such socket group";
    let commands = [
        (Command::DeleteSocketGroup { group: missing }, no_group),
        (
            Command::MoveSocketGroup {
                from: missing,
                to: 0,
            },
            no_group,
        ),
        (
            Command::MoveSocketGroup {
                from: 0,
                to: missing,
            },
            no_group,
        ),
        (
            Command::SetGroupLabel {
                group: missing,
                text: "x".into(),
            },
            no_group,
        ),
        (
            Command::SetGroupSlot {
                group: missing,
                slot: None,
            },
            no_group,
        ),
        (
            Command::SetGroupEnabled {
                group: missing,
                on: false,
            },
            no_group,
        ),
        (
            Command::SetGroupFullDps {
                group: missing,
                on: true,
            },
            no_group,
        ),
        // a slot PoB doesn't offer, and a label its box would refuse
        (
            Command::SetGroupSlot {
                group: 0,
                slot: Some("Left Pocket".into()),
            },
            "unknown slot Left Pocket",
        ),
        (
            Command::SetGroupLabel {
                group: 0,
                text: "x".repeat(51),
            },
            "too long",
        ),
    ];
    for (command, fragment) in commands {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(err.to_string().contains(fragment), "{shown}: {err}");
    }
    let err = s.backend.socket_group_text(missing).unwrap_err();
    assert!(err.to_string().contains(no_group), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

/// The recorded checkpoints of a recipe
fn checkpoints(recipe: &SkillsRecipe) -> Vec<&SkillsCheckpoint> {
    recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            SkillsAction::Check { check } => Some(check),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_recipes_match_pob() {
    let _guard = lock();
    let gems = check_skills_recipe("gems");
    let checks = checkpoints(&gems);
    let all_gems = || {
        checks
            .iter()
            .flat_map(|c| c.groups.iter().flat_map(|g| &g.gems))
    };
    // coverage the first review asked for: a disabled gem, a gem with an error, quality
    // other than 0, a count above 1
    assert!(all_gems().any(|g| !g.enabled), "a disabled gem");
    assert!(all_gems().any(|g| g.error.is_some()), "a gem with an error");
    assert!(all_gems().any(|g| g.quality == 20), "quality other than 0");
    assert!(
        all_gems().any(|g| g.count.is_some_and(|c| c > 1)),
        "a count above 1"
    );
    let vaal = check_skills_recipe("vaal");
    let checks = checkpoints(&vaal);
    let toggled = &checks[0].groups.last().unwrap().gems[0];
    let (t1, t2) = (toggled.toggle1.as_ref(), toggled.toggle2.as_ref());
    assert!(t1.is_some_and(|t| !t.on), "{toggled:?}");
    assert!(t2.is_some_and(|t| !t.on), "{toggled:?}");
}

/// The attack fixture with the `Fireblal` group (a gem PoB doesn't know) pasted as the last group.
fn session_with_unknown_gem(test: &str) -> (Session, usize) {
    let (mut s, _) = session_on("attack", test);
    s.run(Command::PasteSocketGroup {
        text: "Fireblal 20/0  1\r\n".into(),
    })
    .unwrap();
    let group = s.backend.skills_view().unwrap().groups.len() - 1;
    (s, group)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn set_gem_hidden_by_options_is_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "hidden-gem");
    s.backend
        .lua
        .load("pob.build.skillsTab.showLegacyGems = false")
        .exec()
        .unwrap();
    let legacy: String = lua_value(
        &s.backend.lua,
        r#"for id, gem in pairs(pob.build.data.gems) do
            if gem.grantedEffect.legacy and not gem.grantedEffect.hideFromGemList
                and not (gem.grantedEffect.plusVersionOf) and not gem.tagString:match("Exceptional") then
                return (id:gsub("^Default:", ""))
            end
        end"#,
    );
    let before = s.backend.skills_view().unwrap();
    let err = s
        .run(Command::SetGem {
            group: 0,
            slot: 0,
            gem_id: legacy,
        })
        .unwrap_err();
    assert!(err.to_string().contains("isn't offered"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
    assert!(!before.groups[0].gems.is_empty());
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_edit_on_empty_row_is_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "empty-row");
    let before = s.backend.skills_view().unwrap();
    let slot = before.groups[0].gems.len();
    let text = || "5".to_string();
    let commands = [
        Command::SetGemLevel {
            group: 0,
            slot,
            text: text(),
        },
        Command::SetGemQuality {
            group: 0,
            slot,
            text: text(),
        },
        Command::SetGemCount {
            group: 0,
            slot,
            text: text(),
        },
        Command::SetGemEnabled {
            group: 0,
            slot,
            on: false,
        },
        Command::SetGemToggle {
            group: 0,
            slot,
            which: GemToggle::First,
            on: false,
        },
        Command::DeleteGem { group: 0, slot },
    ];
    for command in commands {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(err.to_string().contains("no such gem"), "{shown}: {err}");
    }
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn enabled_on_unknown_gem_is_refused() {
    let _guard = lock();
    let (mut s, group) = session_with_unknown_gem("unknown-gem");
    let before = s.backend.skills_view().unwrap();
    assert!(before.groups[group].gems[0].error.is_some());
    let err = s
        .run(Command::SetGemEnabled {
            group,
            slot: 0,
            on: false,
        })
        .unwrap_err();
    assert!(err.to_string().contains("unknown"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn overlong_level_is_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "long-level");
    let before = s.backend.skills_view().unwrap();
    for command in [
        Command::SetGemLevel {
            group: 0,
            slot: 0,
            text: "123".into(),
        },
        Command::SetGemQuality {
            group: 0,
            slot: 0,
            text: "123".into(),
        },
        Command::SetGemCount {
            group: 0,
            slot: 0,
            text: "123".into(),
        },
    ] {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(err.to_string().contains("too long"), "{shown}: {err}");
    }
    assert_eq!(s.backend.skills_view().unwrap(), before);
}
