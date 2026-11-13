//! Gate tests for the Skills tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{
    Session, fixtures, fixtures_dir, load_xml, lock, lua_value, plain_notes, pob_dir,
    recipe_setting, snapshot_dir, upstream_lua,
};
use auto_pob::engine::upstream::{SidebarLine, golden_of};
use auto_pob::engine::{
    Backend, BackgroundTask, CalcsControl, CalcsCount, Command, DpsMark, GemFilter, GemSearchQuery,
    GemSearchView, GemTarget, GemToggle, GemTooltipQuery, MainSkillSelector, SkillOption,
    SkillSetEntry, SkillsView, SocketGroupView, Tab,
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
    SetGroupCount {
        group: usize,
        text: String,
    },
    SetImbuedSupport {
        group: usize,
        /// Missing for the x button
        gem_id: Option<String>,
    },
    OptimiseSockets {
        group: usize,
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
    SelectSkillSet {
        id: u32,
    },
    NewSkillSet {
        title: String,
    },
    RenameSkillSet {
        id: u32,
        title: String,
    },
    CopySkillSet {
        id: u32,
        title: String,
    },
    DeleteSkillSet {
        id: u32,
    },
    /// `option` is the name `act.option` takes; `value` is a bool, an index or the quality text
    SetSkillOption {
        option: String,
        value: serde_json::Value,
    },
    SetMainSocketGroup {
        group: usize,
    },
    SetMainSkillSelect {
        control: String,
        index: usize,
    },
    SetMainSkillCount {
        control: String,
        text: String,
    },
    Undo,
    Redo,
    /// Built from the recorded checkpoint table, which can hold empty lists
    #[serde(skip_deserializing)]
    Check {
        check: Box<SkillsCheckpoint>,
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
            SkillsAction::SetGroupCount { group, text } => Command::SetGroupCount { group, text },
            SkillsAction::SetImbuedSupport { group, gem_id } => {
                Command::SetImbuedSupport { group, gem_id }
            }
            SkillsAction::OptimiseSockets { group } => Command::OptimiseSockets { group },
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
            SkillsAction::SelectSkillSet { id } => Command::SelectSkillSet { id },
            SkillsAction::NewSkillSet { title } => Command::NewSkillSet { title },
            SkillsAction::RenameSkillSet { id, title } => Command::RenameSkillSet { id, title },
            SkillsAction::CopySkillSet { id, title } => Command::CopySkillSet { id, title },
            SkillsAction::DeleteSkillSet { id } => Command::DeleteSkillSet { id },
            SkillsAction::SetSkillOption { option, value } => {
                Command::SetSkillOption(skill_option(&option, &value))
            }
            SkillsAction::SetMainSocketGroup { group } => Command::SetMainSocketGroup { group },
            SkillsAction::SetMainSkillSelect { control, index } => Command::SetMainSkillSelect {
                control: calcs_control(&control),
                index,
            },
            SkillsAction::SetMainSkillCount { control, text } => Command::SetMainSkillCount {
                control: match control.as_str() {
                    "Stages" => CalcsCount::Stages,
                    "Mines" => CalcsCount::Mines,
                    other => panic!("unknown count {other}"),
                },
                text,
            },
            SkillsAction::Undo => Command::Undo(Tab::Skills),
            SkillsAction::Redo => Command::Redo(Tab::Skills),
            SkillsAction::Check { .. } => return None,
        })
    }
}

/// The sidebar dropdown `act.sidebar` pressed, as our control.
fn calcs_control(name: &str) -> CalcsControl {
    match name {
        "Group" => CalcsControl::Group,
        "Skill" => CalcsControl::Skill,
        "Part" => CalcsControl::Part,
        "Minion" => CalcsControl::Minion,
        "MinionSkill" => CalcsControl::MinionSkill,
        other => panic!("unknown control {other}"),
    }
}

/// The option `act.option(name, value)` pressed, as our command.
fn skill_option(name: &str, value: &serde_json::Value) -> SkillOption {
    let on = || value.as_bool().expect("a bool");
    let index = || value.as_u64().expect("an index") as usize;
    match name {
        "sort_by_dps" => SkillOption::SortByDps(on()),
        "sort_field" => SkillOption::SortField(index()),
        "default_level" => SkillOption::DefaultLevel(index()),
        "default_quality" => SkillOption::DefaultQuality(value.as_str().expect("text").into()),
        "support_types" => SkillOption::SupportTypes(index()),
        "show_legacy" => SkillOption::ShowLegacy(on()),
        _ => panic!("unknown option {name}"),
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
    /// The sidebar's main-skill selector
    main_skill: MainSkillSelector,
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
                let check: SkillsCheckpoint = lua
                    .from_value(mlua::Value::Table(check))
                    .unwrap_or_else(|e| panic!("{name}: checkpoint: {e}"));
                return SkillsAction::Check {
                    check: Box::new(check),
                };
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

/// Our Skills tab and sidebar in the shape of a recipe's `checkpoint()`.
fn our_skills_checkpoint(s: &mut Session) -> SkillsCheckpoint {
    let view = s.backend.skills_view().expect("skills view");
    let stats = s.stats();
    let sidebar: Vec<SidebarLine> = stats
        .stats
        .iter()
        .map(|r| SidebarLine {
            label: r.label.clone(),
            text: r.text.clone(),
            notes: plain_notes(&r.notes),
        })
        .collect();
    SkillsCheckpoint {
        sets: view.sets,
        active_set: view.active_set,
        groups: view.groups,
        can_undo: view.can_undo,
        can_redo: view.can_redo,
        sidebar,
        main_skill: stats.main_skill,
    }
}

/// Replays a recipe as our commands and checks every checkpoint and the end state against
/// upstream.
fn check_skills_recipe(name: &str) -> SkillsRecipe {
    run_skills_recipe(name).0
}

/// `check_skills_recipe`, keeping the session our commands ran in.
fn run_skills_recipe(name: &str) -> (SkillsRecipe, Session) {
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
                assert_eq!(&ours, &**check, "{name}: checkpoint {checked}");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "{name}: no checkpoint");
    let ours = s.backend.skills_view().unwrap();
    assert_ne!(ours, before, "{name}: the recipe changes the Skills tab");
    assert_eq!(ours, recipe.result, "{name}");
    (recipe, s)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn group_recipes_match_pob() {
    let _guard = lock();
    let recipe = check_skills_recipe("groups");
    // the sidebar lists the Full DPS skills after "Full DPS", and the whole sidebar is compared
    assert!(
        checkpoints(&recipe).iter().any(|c| {
            let at = c.sidebar.iter().position(|l| l.label == "Full DPS");
            at.is_some_and(|at| c.sidebar.len() > at + 1)
        }),
        "a Full DPS skill line"
    );
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
    s.run(Command::SetMainSocketGroup { group: 2 }).unwrap();
    lua.load("act.main_group(2)").exec().unwrap();
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
            SkillsAction::Check { check } => Some(&**check),
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
    let gem_checks = checks.clone();
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
    // the two toggles are told apart: one off at a time, each at its own checkpoint
    let vaal = check_skills_recipe("vaal");
    let checks = checkpoints(&vaal);
    let state = |i: usize| {
        let gem = &checks[i].groups.last().unwrap().gems[0];
        (
            gem.toggle1.as_ref().expect("toggle 1").on,
            gem.toggle2.as_ref().expect("toggle 2").on,
        )
    };
    assert_eq!(state(0), (true, false));
    assert_eq!(state(1), (false, true));
    // the empty-text fallbacks are visible: Melee Splash goes from level 18 to its natural max
    let level = |i: usize| gem_checks[i].groups.last().unwrap().gems[1].level;
    assert_eq!(level(0), 18);
    assert_ne!(level(1), 18);
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
fn unknown_gem_without_a_level_still_gives_a_view() {
    let _guard = lock();
    let mut s = Session::start("unknown-gem-no-level");
    let attack = fs::read_to_string(fixtures_dir().join("attack.xml")).unwrap();
    // PoB keeps the nil level of a gem it doesn't know
    let first = attack.find("<Gem ").expect("a gem");
    let xml = format!(
        "{}<Gem nameSpec=\"Fireblal\" enabled=\"true\" count=\"1\"/>\n{}",
        &attack[..first],
        &attack[first..]
    );
    let path = s.user.join("no-level.xml");
    fs::create_dir_all(&s.user).unwrap();
    fs::write(&path, xml).unwrap();
    s.open_fixture("no-level", &path);
    let view = s.backend.skills_view().expect("the view deserialises");
    let gem = &view.groups[0].gems[0];
    assert_eq!(gem.name_spec, "Fireblal");
    assert_eq!((gem.level, gem.quality), (0, 0));
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
    assert!(err.to_string().contains("isn't in PoB's data"), "{err}");
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
    // text the filter removes entirely does nothing, as EditControl:Insert does
    s.run(Command::SetGemLevel {
        group: 0,
        slot: 0,
        text: "abc".into(),
    })
    .unwrap();
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

/// A step goes past the box's limit, as PoB's SetText does (EditControl.lua:686-699): 99 then
/// + on a 2-character level box sets 100, where typed text that long is refused
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_step_sets_a_level_past_the_box_limit() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "step-level");
    // PoB clamps the level to the gem's own maximum afterwards; the quality it keeps
    s.run(Command::SetGemQuality {
        group: 0,
        slot: 0,
        text: "99".into(),
    })
    .unwrap();
    s.run(Command::Step(Box::new(Command::SetGemQuality {
        group: 0,
        slot: 0,
        text: "100".into(),
    })))
    .expect("a step past the limit");
    let view = s.backend.skills_view().unwrap();
    assert_eq!(view.groups[0].gems[0].quality, 100);
    // a level step past the limit goes through too
    s.run(Command::Step(Box::new(Command::SetGemLevel {
        group: 0,
        slot: 0,
        text: "100".into(),
    })))
    .expect("a level step past the limit");
    // the step is over: typing that long is refused again
    let err = s
        .run(Command::SetGemLevel {
            group: 0,
            slot: 0,
            text: "101".into(),
        })
        .expect_err("typed past the limit");
    assert!(err.to_string().contains("too long"), "{err}");
}

// ---- Skill sets and gem options ----

#[test]
#[ignore = "needs the pinned PoB install"]
fn set_recipes_match_pob() {
    let _guard = lock();
    let recipe = check_skills_recipe("sets");
    let checks = checkpoints(&recipe);
    // coverage: the untitled set shows "Default", a set other than the first is active, a set was
    // renamed and a copy holds the group pasted into it
    assert!(
        checks
            .iter()
            .any(|c| c.sets.iter().any(|s| s.title == "Default")),
        "the Default title"
    );
    assert!(
        checks.iter().any(|c| c.active_set != 1),
        "a non-default active set"
    );
    let titles = |i: usize| {
        checks[i]
            .sets
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(0), ["Default", "Mapping", "Bossing"]);
    assert_eq!(titles(1), ["Default", "Clear", "Bossing"]);
    // deleting the active set (3) moves to the one before it (2); undo brings it back
    assert_eq!(checks[2].active_set, 2);
    assert_eq!(titles(2), ["Default", "Clear"]);
    assert_eq!(checks[3].active_set, 3);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn last_set_delete_is_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "last-set");
    let before = s.backend.skills_view().unwrap();
    assert_eq!(before.sets.len(), 1);
    let err = s
        .run(Command::DeleteSkillSet {
            id: before.sets[0].id,
        })
        .unwrap_err();
    assert!(err.to_string().contains("last skill set"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
    // an id no set has
    let err = s.run(Command::DeleteSkillSet { id: 99 }).unwrap_err();
    assert!(err.to_string().contains("no such skill set"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn blank_set_title_is_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "blank-title");
    s.run(Command::NewSkillSet {
        title: "Second".into(),
    })
    .unwrap();
    let before = s.backend.skills_view().unwrap();
    // the last two are emptied by the box's filter (control characters)
    for title in ["", "   ", "\t\r\n", "\u{1}\u{2}", "\u{7f}"] {
        for command in [
            Command::NewSkillSet {
                title: title.into(),
            },
            Command::CopySkillSet {
                id: 1,
                title: title.into(),
            },
            Command::RenameSkillSet {
                id: 1,
                title: title.into(),
            },
        ] {
            let shown = format!("{command:?}");
            let err = s.run(command).expect_err(&shown);
            assert!(err.to_string().contains("needs a name"), "{shown}: {err}");
        }
    }
    // a title the box's limit refuses leaves no half-made set behind
    for command in [
        Command::NewSkillSet {
            title: "x".repeat(101),
        },
        Command::CopySkillSet {
            id: 1,
            title: "x".repeat(101),
        },
        Command::RenameSkillSet {
            id: 1,
            title: "x".repeat(101),
        },
    ] {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(err.to_string().contains("too long"), "{shown}: {err}");
    }
    assert_eq!(s.backend.skills_view().unwrap(), before);
    let open: usize = lua_value(&s.backend.lua, "return #pob.main.main.popups");
    assert_eq!(open, 0, "a popup was left open");
    let sets: usize = lua_value(
        &s.backend.lua,
        "local n = 0 for _ in pairs(pob.build.skillsTab.skillSets) do n = n + 1 end return n",
    );
    assert_eq!(sets, before.sets.len(), "a set was left in the table");
}

/// Every option pressed to a non-default value, as `act.option` calls and our commands. Counts
/// follow PoB's lists: 3 support types, several sort fields and five default levels.
fn option_presses() -> Vec<(&'static str, serde_json::Value, SkillOption)> {
    use serde_json::json;
    vec![
        ("sort_by_dps", json!(false), SkillOption::SortByDps(false)),
        ("sort_field", json!(3), SkillOption::SortField(3)),
        ("default_level", json!(3), SkillOption::DefaultLevel(3)),
        (
            "default_quality",
            json!("20"),
            SkillOption::DefaultQuality("20".into()),
        ),
        ("support_types", json!(2), SkillOption::SupportTypes(2)),
        ("show_legacy", json!(true), SkillOption::ShowLegacy(true)),
    ]
}

/// The attributes of a saved build's `<Skills>` element (PoB writes them in no fixed order).
fn skills_tag(xml: &str) -> std::collections::BTreeMap<String, String> {
    let start = xml.find("<Skills").expect("a Skills element") + "<Skills".len();
    let tag = &xml[start..start + xml[start..].find('>').unwrap()];
    let mut attributes = std::collections::BTreeMap::new();
    let mut rest = tag.trim();
    while let Some((key, after)) = rest.split_once("=\"") {
        let (value, next) = after.split_once('"').expect("a closing quote");
        attributes.insert(key.trim().to_string(), value.to_string());
        rest = next.trim();
    }
    attributes
}

fn upstream_flags(lua: &mlua::Lua) -> String {
    lua.load(
        r#"runCallback('OnFrame')
        local b = build
        return table.concat({ tostring(b.modFlag), tostring(b.notesTab.modFlag), tostring(b.partyTab.modFlag),
            tostring(b.configTab.modFlag), tostring(b.treeTab.modFlag), tostring(b.treeTab.searchFlag),
            tostring(b.spec.modFlag), tostring(b.skillsTab.modFlag), tostring(b.itemsTab.modFlag),
            tostring(b.calcsTab.modFlag) }, ",")"#,
    )
    .eval()
    .unwrap()
}

fn upstream_unsaved(lua: &mlua::Lua) -> bool {
    lua.load("runCallback('OnFrame') return build.unsaved and true or false")
        .eval()
        .unwrap()
}

/// Presses every option on both sides, comparing after each, then saves both and compares the
/// `<Skills>` attributes.
fn press_options_on_both(s: &mut Session, lua: &mlua::Lua, name: &str) {
    assert!(!s.backend.build_view().unwrap().unsaved, "{name}: start");
    // upstream's loader leaves `build.modFlag` set (our OpenBuild resets it); a save would clear it
    lua.load("build.modFlag = false").exec().unwrap();
    assert!(
        !upstream_unsaved(lua),
        "{name}: upstream start: {}",
        upstream_flags(lua)
    );
    for (option, value, command) in option_presses() {
        let before = s.backend.skills_view().unwrap().options;
        let affected = s.run(Command::SetSkillOption(command.clone())).unwrap();
        assert!(affected.skills && !affected.build, "{name}: {command:?}");
        lua.globals()
            .set("pressed", lua.to_value(&value).unwrap())
            .unwrap();
        lua.globals().set("option", option).unwrap();
        lua.load("act.option(option, pressed)").exec().unwrap();
        let ours = s.backend.skills_view().unwrap().options;
        assert_eq!(ours, upstream_dump(lua).options, "{name}: {command:?}");
        assert_ne!(ours, before, "{name}: {command:?} changes the options");
        assert!(
            !s.backend.build_view().unwrap().unsaved,
            "{name}: {command:?} marks the build unsaved"
        );
        assert!(
            !upstream_unsaved(lua),
            "{name}: upstream {option}: {}",
            upstream_flags(lua)
        );
    }
    s.run(Command::SaveAs {
        folder: String::new(),
        name: format!("options-{name}"),
        overwrite: false,
    })
    .unwrap();
    let saved = fs::read_to_string(s.builds_dir().join(format!("options-{name}.xml"))).unwrap();
    let theirs: String = lua.load("return build:SaveDB('x')").eval().unwrap();
    assert_eq!(skills_tag(&saved), skills_tag(&theirs), "{name}");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn skill_options_match_pob() {
    let _guard = lock();
    let (mut s, lua) = session_on("attack", "options");
    let start = s.backend.skills_view().unwrap().options;
    assert!(start.sort_by_dps && !start.show_legacy);
    press_options_on_both(&mut s, &lua, "attack");
    let end = s.backend.skills_view().unwrap().options;
    assert!(!end.sort_by_dps && end.show_legacy);
    assert_eq!(end.default_quality, "20");
    assert_eq!(end.sort_field.selected, 3);
    assert_eq!(end.default_level.selected, 3);
    assert_eq!(end.support_types.selected, 2);

    // a new build never went through Load: the sort field dropdown still shows "Full DPS" in
    // PoB's control, but the model, and so the view, says CombinedDPS (R18)
    let mut s = Session::start("skills-options-new");
    s.run(Command::NewBuild).unwrap();
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    lua.load("newBuild()").exec().unwrap();
    lua.load(fs::read_to_string(fixtures_dir().join("skills_lib.lua")).unwrap())
        .set_name("=skills_lib")
        .exec()
        .unwrap();
    let ours = s.backend.skills_view().unwrap().options;
    assert_eq!(ours, upstream_dump(&lua).options);
    assert_eq!(ours.sort_field.selected, 1, "CombinedDPS, not entry 1");
    assert_eq!(ours.sort_field.labels[1], "Combined DPS");
    press_options_on_both(&mut s, &lua, "new");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn bad_skill_options_are_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "bad-options");
    let before = s.backend.skills_view().unwrap();
    for command in [
        SkillOption::SortField(99),
        SkillOption::DefaultLevel(99),
        SkillOption::SupportTypes(99),
        SkillOption::DefaultQuality("123".into()),
    ] {
        let shown = format!("{command:?}");
        s.run(Command::SetSkillOption(command)).expect_err(&shown);
    }
    // text the filter empties does nothing, as typing it would
    s.run(Command::SetSkillOption(SkillOption::DefaultQuality(
        "abc".into(),
    )))
    .unwrap();
    assert_eq!(s.backend.skills_view().unwrap(), before);
    // over 23 is capped by PoB's callback
    s.run(Command::SetSkillOption(SkillOption::DefaultQuality(
        "99".into(),
    )))
    .unwrap();
    assert_eq!(
        s.backend.skills_view().unwrap().options.default_quality,
        "23"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn skills_round_trip() {
    let _guard = lock();
    for name in ["groups", "sets"] {
        let (_, mut s) = run_skills_recipe(name);
        if name == "sets" {
            for (_, _, option) in option_presses() {
                s.run(Command::SetSkillOption(option)).unwrap();
            }
        }
        s.run(Command::SaveAs {
            folder: String::new(),
            name: format!("round-{name}"),
            overwrite: false,
        })
        .unwrap();
        assert!(!s.backend.build_view().unwrap().unsaved);
        let mut ours = s.backend.skills_view().unwrap();
        let saved = fs::read_to_string(s.builds_dir().join(format!("round-{name}.xml"))).unwrap();
        let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
        load_xml(&lua, &saved, name).unwrap();
        lua.load(fs::read_to_string(fixtures_dir().join("skills_lib.lua")).unwrap())
            .set_name("=skills_lib")
            .exec()
            .unwrap();
        let mut theirs = upstream_dump(&lua);
        // undo history is not saved
        for view in [&mut ours, &mut theirs] {
            view.can_undo = false;
            view.can_redo = false;
        }
        assert_eq!(ours, theirs, "{name}");
        if name == "sets" {
            assert!(theirs.active_set != 1 && theirs.sets.len() == 3);
            assert_eq!(theirs.options.default_quality, "20");
        }
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn no_silent_writes_skills() {
    let _guard = lock();
    let (mut s, _lua) = session_on("attack", "no-writes");
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    let view = s.backend.skills_view().unwrap();
    let gem_id = view.groups[0].gems[0].gem_id.clone().expect("a gem id");
    let text = |t: &str| t.to_string();
    let mut commands = vec![
        Command::AddSocketGroup,
        Command::PasteSocketGroup {
            text: "Label: Auras\r\nHatred 20/0  1\r\nWrath 20/0  1\r\n".into(),
        },
        Command::SetGroupLabel {
            group: 0,
            text: text("Edited"),
        },
        Command::SetGroupSlot {
            group: 0,
            slot: Some("Helmet".into()),
        },
        Command::SetGroupEnabled {
            group: 0,
            on: false,
        },
        Command::SetGroupFullDps { group: 0, on: true },
        Command::MoveSocketGroup { from: 0, to: 1 },
        Command::CopySocketGroup { group: 0 },
        Command::SetGem {
            group: 0,
            slot: 0,
            gem_id,
        },
        Command::SetGemLevel {
            group: 0,
            slot: 0,
            text: text("5"),
        },
        Command::SetGemQuality {
            group: 0,
            slot: 0,
            text: text("5"),
        },
        Command::SetGemCount {
            group: 0,
            slot: 0,
            text: text("2"),
        },
        Command::SetGemEnabled {
            group: 0,
            slot: 0,
            on: false,
        },
        Command::SetGemToggle {
            group: 0,
            slot: 0,
            which: GemToggle::First,
            on: false,
        },
        Command::SearchGems(gem_query(0, GemTarget::Slot(0), "", GemFilter::All)),
        Command::GemTooltip(GemTooltipQuery {
            group: 0,
            target: GemTarget::Slot(0),
            gem_id: None,
        }),
        Command::SearchGems(gem_query(0, GemTarget::Slot(1), "", GemFilter::Support)),
        Command::CloseGemSearch,
        Command::DeleteGem { group: 0, slot: 0 },
        Command::NewSkillSet {
            title: text("Mapping"),
        },
        Command::CopySkillSet {
            id: 1,
            title: text("Bossing"),
        },
        Command::SelectSkillSet { id: 3 },
        Command::RenameSkillSet {
            id: 2,
            title: text("Clear"),
        },
        Command::DeleteSkillSet { id: 3 },
    ];
    commands.extend(
        option_presses()
            .into_iter()
            .map(|(_, _, option)| Command::SetSkillOption(option)),
    );
    commands.extend([
        Command::Undo(Tab::Skills),
        Command::Redo(Tab::Skills),
        Command::DeleteSocketGroup { group: 0 },
        Command::DeleteAllSocketGroups,
    ]);
    let mut ran = 0;
    let mut dps_steps = 0;
    for command in commands {
        let shown = format!("{command:?}");
        // a command PoB doesn't offer for this build may fail, but it must not write either
        match s.run(command) {
            Err(e) => eprintln!("{shown}: {e}"),
            Ok(affected) => {
                ran += 1;
                // a search's whole DPS sort runs before the next command
                dps_steps += finish_gem_dps(&mut s).0;
                if affected.gem_search {
                    s.backend.gem_search_view().unwrap();
                }
                if let Some(query) = &affected.gem_tooltip {
                    s.backend.gem_tooltip_view(query).unwrap();
                }
            }
        }
        assert_eq!(snapshot_dir(&dir), before, "{shown} wrote a file");
    }
    assert!(ran > 25, "only {ran} commands ran");
    assert!(dps_steps > 1, "no DPS sort ran");
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

/// The `item_group` recipe's build (a helmet that grants Icestorm) opened in our engine, and
/// the index of the group the item made.
fn item_group_session(test: &str) -> (Session, usize) {
    let recipe = upstream_skills_recipe("item_group");
    let xml = recipe.setup_xml.expect("the recipe prepares a build");
    let mut s = Session::start(&format!("skills-{test}-item"));
    let path = s.user.join("setup.xml");
    fs::create_dir_all(&s.user).unwrap();
    fs::write(&path, xml).unwrap();
    s.open_fixture(&recipe.base, &path);
    let view = s.backend.skills_view().unwrap();
    let item = view
        .groups
        .iter()
        .position(|g| g.source.is_some())
        .expect("an item group");
    (s, item)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_and_imbued_recipes_match_pob() {
    let _guard = lock();
    let recipe = check_skills_recipe("item_group");
    let checks: Vec<_> = checkpoints(&recipe);
    // the item's source shows under its Full DPS line, and the notes are compared
    assert!(
        checks
            .iter()
            .any(|c| c.sidebar.iter().any(|l| !l.notes.is_empty())),
        "a sidebar note"
    );
    let first = checks[0];
    let item = first
        .groups
        .iter()
        .find(|g| g.source.is_some())
        .expect("an item group");
    assert_eq!(item.count.as_deref(), Some("3"));
    assert_eq!(item.label, "Granted");
    assert_eq!(item.slot.as_deref(), Some("Helmet"));
    assert!(item.source_note.as_deref().is_some_and(|n| !n.is_empty()));
    assert!(item.sockets_line.is_some());
    assert!(item.can_optimise);
    assert!(item.imbued.is_none());
    assert!(item.enabled && item.full_dps);
    let last = checks[1]
        .groups
        .iter()
        .find(|g| g.source.is_some())
        .unwrap();
    assert!(!last.enabled);
    // the count moved Full DPS: the sidebar line is part of the checkpoint
    assert!(first.sidebar.iter().any(|l| l.label == "Full DPS"));

    let recipe = check_skills_recipe("imbued_optimise");
    let checks = checkpoints(&recipe);
    assert_eq!(checks.len(), 3);
    assert_eq!(
        checks[0].groups[0].imbued.as_ref().unwrap().name.as_deref(),
        Some("Added Fire Damage")
    );
    assert!(checks[0].groups[0].sockets_line.is_some());
    assert_ne!(
        checks[0].groups[0].sockets_line, checks[1].groups[0].sockets_line,
        "Optimise Sockets changes the item's sockets"
    );
    assert!(checks[2].groups[0].imbued.as_ref().unwrap().name.is_none());
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_group_refuses_what_pob_hides() {
    let _guard = lock();
    let (mut s, item) = item_group_session("refusals");
    let before = s.backend.skills_view().unwrap();
    let gem_id = before.groups[0].gems[0].gem_id.clone().expect("a gem id");
    assert!(!before.groups[item].gems.is_empty());
    let cases = [
        (
            Command::SetGroupSlot {
                group: item,
                slot: None,
            },
            "slot of an item's socket group",
        ),
        (
            Command::SetGem {
                group: item,
                slot: 0,
                gem_id: gem_id.clone(),
            },
            "can't be edited",
        ),
        (
            Command::SetGem {
                group: item,
                slot: 1,
                gem_id: gem_id.clone(),
            },
            "can't be edited",
        ),
        (
            Command::DeleteGem {
                group: item,
                slot: 0,
            },
            "can't be edited",
        ),
        (
            Command::SetGemLevel {
                group: item,
                slot: 0,
                text: "5".into(),
            },
            "can't be edited",
        ),
        (
            Command::DeleteSocketGroup { group: item },
            "can't be deleted",
        ),
        (
            Command::SetImbuedSupport {
                group: item,
                gem_id: Some(gem_id.clone()),
            },
            "no imbued support",
        ),
        (
            Command::SetImbuedSupport {
                group: item,
                gem_id: None,
            },
            "no imbued support",
        ),
        (
            Command::SetGroupCount {
                group: 0,
                text: "3".into(),
            },
            "has a count",
        ),
    ];
    for (command, fragment) in cases {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(err.to_string().contains(fragment), "{shown}: {err}");
        assert_eq!(s.backend.skills_view().unwrap(), before, "{shown}");
    }
    // copying is a query: its answer is the error
    let err = s.backend.socket_group_text(item).unwrap_err();
    assert!(err.to_string().contains("can't be copied"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
    // what PoB does allow: the count, the label and the flags
    s.run(Command::SetGroupCount {
        group: item,
        text: "2".into(),
    })
    .unwrap();
    let after = s.backend.skills_view().unwrap();
    assert_eq!(after.groups[item].count.as_deref(), Some("2"));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn bad_imbued_and_optimise_are_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "imbued-refusals");
    let before = s.backend.skills_view().unwrap();
    // no slot: the box is disabled and no item is chosen
    for (command, fragment) in [
        (
            Command::SetImbuedSupport {
                group: 0,
                gem_id: Some("Metadata/Items/Gems/SkillGemSupportAddedFireDamage".into()),
            },
            "needs an item slot",
        ),
        (
            Command::OptimiseSockets { group: 0 },
            "no item with sockets",
        ),
    ] {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(err.to_string().contains(fragment), "{shown}: {err}");
    }
    assert_eq!(s.backend.skills_view().unwrap(), before);
    s.run(Command::SetGroupSlot {
        group: 0,
        slot: Some("Weapon 1".into()),
    })
    .unwrap();
    let before = s.backend.skills_view().unwrap();
    // a gem that can't support the group's skill isn't offered
    let err = s
        .run(Command::SetImbuedSupport {
            group: 0,
            gem_id: Some("Metadata/Items/Gems/SkillGemCleave".into()),
        })
        .unwrap_err();
    assert!(err.to_string().contains("isn't offered"), "{err}");
    assert_eq!(s.backend.skills_view().unwrap(), before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn optimise_records_items_undo_not_skills() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "optimise-undo");
    s.run(Command::SetGroupSlot {
        group: 0,
        slot: Some("Weapon 1".into()),
    })
    .unwrap();
    let item = |s: &Session| -> (String, usize) {
        let raw: String = lua_value(
            &s.backend.lua,
            r#"local tab = pob.build.itemsTab
            return tab.items[tab.slots["Weapon 1"].selItemId].raw"#,
        );
        let depth: usize = lua_value(&s.backend.lua, "return #pob.build.itemsTab.undo");
        (raw, depth)
    };
    let (raw_before, depth_before) = item(&s);
    let skills_depth =
        |s: &Session| -> usize { lua_value(&s.backend.lua, "return #pob.build.skillsTab.undo") };
    let skills_before = skills_depth(&s);
    let affected = s.run(Command::OptimiseSockets { group: 0 }).unwrap();
    assert!(affected.skills && affected.build && affected.recalc);
    let (raw_after, depth_after) = item(&s);
    assert_ne!(raw_before, raw_after, "the sockets changed");
    assert_eq!(depth_after, depth_before + 1, "an Items undo state");
    assert_eq!(skills_depth(&s), skills_before, "no Skills undo state");
    // Skills undo reverts the slot change, not the sockets
    s.run(Command::Undo(Tab::Skills)).unwrap();
    assert_eq!(item(&s).0, raw_after);
    assert_eq!(s.backend.skills_view().unwrap().groups[0].slot, None);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn group_tooltip_matches_pob() {
    let _guard = lock();
    let mut checked = 0;
    for fixture in ["attack", "minion", "item_group"] {
        let (mut s, xml) = if fixture == "item_group" {
            let (s, _) = item_group_session("tooltip");
            let xml = s.user.join("setup.xml");
            (s, xml)
        } else {
            let (s, _) = session_on(fixture, "tooltip");
            (s, fixtures_dir().join(format!("{fixture}.xml")))
        };
        let lua = upstream_with_skills_lib(&xml, fixture);
        lua.load(include_str!("../../src/engine/glue/tooltip.lua"))
            .set_name("=recorder")
            .exec()
            .unwrap();
        let theirs: Vec<auto_pob::engine::GroupTooltipView> = lua_value(
            &lua,
            r#"
            local list = {}
            local lister = build.skillsTab.controls.groupList
            for i, group in ipairs(build.skillsTab.socketGroupList) do
                local tooltip = glue.tooltip()
                lister:AddValueTooltip(tooltip, i, group)
                list[i] = { group = i - 1, lines = tooltip:lines() }
            end
            return list
            "#,
        );
        assert!(!theirs.is_empty(), "{fixture}");
        for expected in &theirs {
            let affected = s
                .run(Command::GroupTooltip {
                    group: expected.group,
                })
                .unwrap();
            assert_eq!(affected.group_tooltip, Some(expected.group));
            let ours = s.backend.group_tooltip_view(expected.group).unwrap();
            assert_eq!(&ours, expected, "{fixture}: group {}", expected.group);
            if !expected.lines.is_empty() {
                checked += 1;
            }
        }
    }
    assert!(checked >= 3, "only {checked} groups had a tooltip");
}

/// The write paths added with item groups, imbued supports and Optimise Sockets, in states
/// where they succeed: each must return Ok and write no file.
#[test]
#[ignore = "needs the pinned PoB install"]
fn no_silent_writes_item_and_imbued() {
    let _guard = lock();
    let (mut s, _lua) = session_on("attack", "no-writes-imbued");
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    let fire = "Metadata/Items/Gems/SkillGemSupportAddedFireDamage";
    for command in [
        Command::SetGroupSlot {
            group: 0,
            slot: Some("Weapon 1".into()),
        },
        Command::SetImbuedSupport {
            group: 0,
            gem_id: Some(fire.into()),
        },
        Command::OptimiseSockets { group: 0 },
        Command::GroupTooltip { group: 0 },
        Command::SetImbuedSupport {
            group: 0,
            gem_id: None,
        },
    ] {
        let shown = format!("{command:?}");
        s.run(command).unwrap_or_else(|e| panic!("{shown}: {e}"));
        assert_eq!(snapshot_dir(&dir), before, "{shown} wrote a file");
    }
    let view = s.backend.skills_view().unwrap();
    assert_eq!(view.groups[0].slot.as_deref(), Some("Weapon 1"));
    drop(s);

    let (mut s, item) = item_group_session("no-writes");
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    for command in [
        Command::SetGroupCount {
            group: item,
            text: "2".into(),
        },
        Command::GroupTooltip { group: item },
        Command::OptimiseSockets { group: item },
    ] {
        let shown = format!("{command:?}");
        s.run(command).unwrap_or_else(|e| panic!("{shown}: {e}"));
        assert_eq!(snapshot_dir(&dir), before, "{shown} wrote a file");
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn main_skill_recipe_matches_pob() {
    let _guard = lock();
    let recipe = check_skills_recipe("main_skill");
    let checks = checkpoints(&recipe);
    // coverage: the right-click moves the main group, and the part dropdown reaches the
    // second part of Boneshatter
    assert!(!checks[0].groups[1].is_main && checks[1].groups[1].is_main);
    assert!(checks[0].groups[0].is_main && !checks[1].groups[0].is_main);
    assert_eq!(checks[2].main_skill.group.selected, 0);
    assert_eq!(checks[2].main_skill.part.as_ref().unwrap().selected, 1);
    assert_ne!(
        checks[1].sidebar, checks[2].sidebar,
        "the part changes the stats"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn main_skill_selector_matches_pob() {
    let _guard = lock();
    for (name, xml) in fixtures() {
        let (mut s, lua) = {
            let mut s = Session::start(&format!("main-selector-{name}"));
            s.open_fixture(&name, &xml);
            (s, upstream_with_skills_lib(&xml, &name))
        };
        lua.load("runCallback('OnFrame')").exec().unwrap();
        let theirs: MainSkillSelector = {
            let value: mlua::Value = lua.load("return mainSkillDump()").eval().unwrap();
            lua.from_value(value).unwrap()
        };
        assert_eq!(s.stats().main_skill, theirs, "{name}");
        assert!(!theirs.group.labels.is_empty(), "{name}: group list");
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn main_skill_mode_is_an_error() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "main-mode");
    let before = s.stats().main_skill;
    let err = s
        .run(Command::SetMainSkillSelect {
            control: CalcsControl::Mode,
            index: 0,
        })
        .unwrap_err();
    assert!(err.to_string().contains("Mode"), "{err}");
    let err = s
        .run(Command::SetMainSkillSelect {
            control: CalcsControl::Group,
            index: 99,
        })
        .unwrap_err();
    assert!(err.to_string().contains("no such choice"), "{err}");
    let err = s
        .run(Command::SetMainSocketGroup { group: 99 })
        .unwrap_err();
    assert!(err.to_string().contains("no such socket group"), "{err}");
    assert_eq!(s.stats().main_skill, before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn main_skill_count_matches_pob() {
    let _guard = lock();
    let recipe = check_skills_recipe("main_skill_counts");
    let checks = checkpoints(&recipe);
    assert!(checks[0].main_skill.stages.is_some(), "a Stages box");
    assert_eq!(checks[1].main_skill.stages.as_deref(), Some("56"));
    assert_eq!(checks[2].main_skill.stages.as_deref(), Some("3"));
    assert_ne!(
        checks[1].sidebar, checks[2].sidebar,
        "the stages change the stats"
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn hidden_sidebar_controls_are_refused() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "main-hidden");
    // Cleave has one part, no stages and no mines
    s.run(Command::PasteSocketGroup {
        text: "Cleave 20/0  1\r\n".into(),
    })
    .unwrap();
    s.run(Command::SetMainSocketGroup { group: 1 }).unwrap();
    let selector = s.stats().main_skill;
    assert!(selector.part.is_none() && selector.stages.is_none() && selector.mines.is_none());
    let before = selector;
    let err = s
        .run(Command::SetMainSkillSelect {
            control: CalcsControl::Part,
            index: 0,
        })
        .unwrap_err();
    assert!(err.to_string().contains("Part isn't available"), "{err}");
    for control in [CalcsCount::Stages, CalcsCount::Mines] {
        let err = s
            .run(Command::SetMainSkillCount {
                control,
                text: "4".into(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("isn't available"), "{err}");
    }
    assert_eq!(s.stats().main_skill, before);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn main_skill_dropdowns_match_pob() {
    let _guard = lock();
    let minion = check_skills_recipe("main_skill_minion");
    let checks = checkpoints(&minion);
    assert!(checks[0].main_skill.minion.is_some(), "a minion dropdown");
    assert_eq!(
        checks[2].main_skill.minion_skill.as_ref().unwrap().selected,
        1
    );
    assert_ne!(
        checks[1].main_skill, checks[2].main_skill,
        "the minion skill moves the selector"
    );
    let dot = check_skills_recipe("main_skill_dot");
    let checks = checkpoints(&dot);
    assert_eq!(checks[1].main_skill.skill.as_ref().unwrap().selected, 1);
    assert_ne!(
        checks[0].sidebar, checks[1].sidebar,
        "the skill changes the stats"
    );
}

// ---- Gem search, DPS sort and gem tooltips (Task 7) ----

fn gem_query(group: usize, target: GemTarget, text: &str, filter: GemFilter) -> GemSearchQuery {
    GemSearchQuery {
        group,
        target,
        text: text.into(),
        filter,
    }
}

/// Runs the background work to its end, as the engine loop does when idle; returns the number
/// of steps and the time they took. Only the DPS sort may run.
fn finish_gem_dps(s: &mut Session) -> (usize, std::time::Duration) {
    let start = std::time::Instant::now();
    let mut steps = 0;
    while let Some(task) = s.backend.next_background() {
        assert_eq!(task, BackgroundTask::GemDps);
        s.backend.background_step(task, &|_| {}).unwrap();
        steps += 1;
        assert!(steps < 100_000, "the DPS sort never ends");
    }
    (steps, start.elapsed())
}

/// Opens the search in our engine, runs its DPS sort to the end and returns the list.
fn our_search(s: &mut Session, query: GemSearchQuery) -> GemSearchView {
    let affected = s.run(Command::SearchGems(query.clone())).unwrap();
    assert!(affected.gem_search);
    finish_gem_dps(s);
    let view = s
        .backend
        .gem_search_view()
        .unwrap()
        .expect("an open search");
    assert_eq!(view.query, query);
    assert!(!view.sorting, "{query:?} still sorting");
    assert!(!view.ended);
    view
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamSearch {
    #[serde(default)]
    ids: Vec<String>,
    #[serde(default)]
    dps: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    colours: std::collections::BTreeMap<String, String>,
    base_dps: f64,
}

fn lua_target(lua: &mlua::Lua, target: GemTarget) -> mlua::Value {
    match target {
        GemTarget::Slot(slot) => mlua::Value::Integer(slot as i64),
        GemTarget::Imbued => mlua::Value::String(lua.create_string("imbued").unwrap()),
    }
}

/// Upstream's `gemSearch`, which must leave the Skills tab as it was.
fn upstream_search(lua: &mlua::Lua, query: &GemSearchQuery) -> UpstreamSearch {
    let globals = lua.globals();
    globals.set("q_group", query.group).unwrap();
    globals
        .set("q_target", lua_target(lua, query.target))
        .unwrap();
    globals.set("q_text", query.text.as_str()).unwrap();
    let filter = match query.filter {
        GemFilter::All => None,
        GemFilter::Support => Some("support"),
        GemFilter::Active => Some("grants_active_skill"),
    };
    globals.set("q_filter", filter).unwrap();
    let before = upstream_dump(lua);
    let found = lua_value(lua, "return gemSearch(q_group, q_target, q_text, q_filter)");
    assert_eq!(
        upstream_dump(lua),
        before,
        "{query:?} changed the Skills tab"
    );
    found
}

fn ids(view: &GemSearchView) -> Vec<&str> {
    view.rows.iter().map(|r| r.gem_id.as_str()).collect()
}

fn our_dps(view: &GemSearchView) -> std::collections::BTreeMap<String, f64> {
    view.rows
        .iter()
        .filter_map(|r| r.dps.map(|d| (r.gem_id.clone(), d)))
        .collect()
}

fn set_sort_by_dps(s: &mut Session, lua: &mlua::Lua, on: bool) {
    s.run(Command::SetSkillOption(SkillOption::SortByDps(on)))
        .unwrap();
    lua.load(format!("act.option('sort_by_dps', {on})"))
        .exec()
        .unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn search_matches_pob() {
    let _guard = lock();
    let cases = [
        ("", GemFilter::All),
        ("ad", GemFilter::All),
        ("cold", GemFilter::All),
        (":fire:-support", GemFilter::All),
        ("", GemFilter::Support),
        ("", GemFilter::Active),
    ];
    for (fixture, slot) in [("attack", 3), ("minion", 1)] {
        let (mut s, lua) = session_on(fixture, "search");
        for sort in [false, true] {
            set_sort_by_dps(&mut s, &lua, sort);
            for (text, filter) in cases {
                let query = gem_query(0, GemTarget::Slot(slot), text, filter);
                let ours = our_search(&mut s, query.clone());
                let theirs = upstream_search(&lua, &query);
                assert_eq!(ids(&ours), theirs.ids, "{fixture} sort {sort}: {query:?}");
                if text.is_empty() {
                    assert!(ours.rows.len() > 50, "{fixture}: {query:?}");
                } else {
                    assert!(!ours.rows.is_empty(), "{fixture}: {query:?}");
                }
            }
        }
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_dps_matches_pob() {
    let _guard = lock();
    for (fixture, slot) in [("attack", 3), ("minion", 1)] {
        let (mut s, lua) = session_on(fixture, "gem-dps");
        let query = gem_query(0, GemTarget::Slot(slot), "", GemFilter::All);
        s.run(Command::SearchGems(query.clone())).unwrap();
        let first = s.backend.gem_search_view().unwrap().unwrap();
        assert!(first.sorting, "{fixture}: the sort hasn't started");
        let (steps, took) = finish_gem_dps(&mut s);
        let ours = s.backend.gem_search_view().unwrap().unwrap();
        let theirs = upstream_search(&lua, &query);
        let calculated = our_dps(&ours);
        eprintln!(
            "{fixture}: DPS sort of {} gems ({} calculated) took {took:?} in {steps} steps",
            ours.rows.len(),
            calculated.len()
        );
        assert!(calculated.len() > 20, "{fixture}: {}", calculated.len());
        assert_eq!(calculated, theirs.dps, "{fixture}");
        let colours: std::collections::BTreeMap<_, _> = ours
            .rows
            .iter()
            .filter_map(|r| r.dps_colour.clone().map(|c| (r.gem_id.clone(), c)))
            .collect();
        assert_eq!(colours, theirs.colours, "{fixture}");
        assert_eq!(ours.base_dps, theirs.base_dps, "{fixture}");
        assert!(ours.base_dps > 0.0, "{fixture}");
        // the marks follow Draw: a check for supports that can support, + for global effects
        for row in &ours.rows {
            match row.mark {
                Some(DpsMark::Check) => assert!(row.support && row.can_support, "{row:?}"),
                Some(DpsMark::Plus) => assert!(!row.support, "{row:?}"),
                None => {}
            }
        }
        assert!(ours.rows.iter().any(|r| r.mark == Some(DpsMark::Check)));
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn imbued_search_matches_pob() {
    let _guard = lock();
    let (mut s, lua) = session_on("attack", "imbued-search");
    s.run(Command::SetGroupSlot {
        group: 0,
        slot: Some("Weapon 1".into()),
    })
    .unwrap();
    lua.load("act.slot(0, 'Weapon 1')").exec().unwrap();
    let query = gem_query(0, GemTarget::Imbued, "", GemFilter::All);
    let ours = our_search(&mut s, query.clone());
    let theirs = upstream_search(&lua, &query);
    assert!(!ours.rows.is_empty());
    assert!(ours.rows.iter().all(|r| r.support && r.can_support));
    assert_eq!(ids(&ours), theirs.ids);
    assert_eq!(our_dps(&ours), theirs.dps);
    // the box has no S or A buttons
    let err = s
        .run(Command::SearchGems(gem_query(
            0,
            GemTarget::Imbued,
            "",
            GemFilter::Support,
        )))
        .unwrap_err();
    assert!(err.to_string().contains("filter"), "{err}");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_tooltip_matches_pob() {
    let _guard = lock();
    let (mut s, lua) = session_on("attack", "gem-tooltip");
    lua.load(include_str!("../../src/engine/glue/tooltip.lua"))
        .set_name("=recorder")
        .exec()
        .unwrap();
    let melee = "Metadata/Items/Gems/SkillGemSupportMeleePhysicalDamage";
    for (slot, gem_id) in [(3, Some(melee)), (0, None)] {
        let query = GemTooltipQuery {
            group: 0,
            target: GemTarget::Slot(slot),
            gem_id: gem_id.map(String::from),
        };
        let affected = s.run(Command::GemTooltip(query.clone())).unwrap();
        assert_eq!(affected.gem_tooltip.as_ref(), Some(&query));
        let ours = s.backend.gem_tooltip_view(&query).unwrap();
        assert_eq!(ours.query, query);
        lua.globals().set("q_slot", slot).unwrap();
        lua.globals().set("q_id", gem_id).unwrap();
        let theirs: Vec<auto_pob::engine::TooltipLine> =
            lua_value(&lua, "return gemTooltip(0, q_slot, q_id)");
        assert!(theirs.len() > 3, "{query:?}: {theirs:?}");
        assert_eq!(ours.lines, theirs, "{query:?}");
    }
    // a failing tooltip is answered with no lines
    let bad = GemTooltipQuery {
        group: 0,
        target: GemTarget::Slot(3),
        gem_id: Some("Metadata/Items/Gems/NoSuchGem".into()),
    };
    assert!(s.backend.gem_tooltip_view(&bad).is_err());
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_dps_survives_interleaved_commands() {
    let _guard = lock();
    let query = gem_query(0, GemTarget::Slot(3), "", GemFilter::All);
    let label = || Command::SetGroupLabel {
        group: 1,
        text: "x".into(),
    };
    // the fixture has one group; a second one is the other group a command shows
    let second = || Command::PasteSocketGroup {
        text: "Label: Auras\r\nHatred 20/0  1\r\n".into(),
    };
    // uninterrupted, on the same build with the same label
    let (mut plain, _) = session_on("attack", "interleaved-plain");
    warm_stat_maps(&plain.backend.lua);
    plain.run(second()).unwrap();
    plain.run(label()).unwrap();
    let expected = our_search(&mut plain, query.clone());

    let (mut s, _) = session_on("attack", "interleaved");
    warm_stat_maps(&s.backend.lua);
    s.run(second()).unwrap();
    s.run(Command::SearchGems(query.clone())).unwrap();
    for _ in 0..2 {
        s.backend
            .background_step(BackgroundTask::GemDps, &|_| {})
            .unwrap();
    }
    assert_eq!(
        s.backend.next_background(),
        Some(BackgroundTask::GemDps),
        "the sort ended within two steps"
    );
    let affected = s.run(Command::GroupTooltip { group: 1 }).unwrap();
    assert_eq!(affected.group_tooltip, Some(1));
    assert!(!s.backend.group_tooltip_view(1).unwrap().lines.is_empty());
    s.run(label()).unwrap();
    finish_gem_dps(&mut s);
    let ours = s.backend.gem_search_view().unwrap().unwrap();
    assert!(!ours.sorting);
    assert_eq!(our_dps(&ours), our_dps(&expected));
    assert_eq!(ours.base_dps, expected.base_dps);
    assert_eq!(ids(&ours), ids(&expected));
    assert_eq!(s.backend.skills_view().unwrap().groups[1].label, "x");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn set_gem_after_filtered_search() {
    let _guard = lock();
    let (mut s, lua) = session_on("attack", "filtered-pick");
    let support = gem_query(0, GemTarget::Slot(3), "", GemFilter::Support);
    our_search(&mut s, support.clone());
    upstream_search(&lua, &support);
    // an active gem, which the S filter hid, picked in the same box
    let view = s.backend.skills_view().unwrap();
    let active = &view.groups[0].gems[0];
    assert!(!active.support);
    let gem_id = active.gem_id.clone().unwrap();
    s.run(Command::SetGem {
        group: 0,
        slot: 3,
        gem_id: gem_id.clone(),
    })
    .expect("a stale S filter refused the gem");
    lua.globals()
        .set("q_name", active.name_spec.as_str())
        .unwrap();
    lua.load("act.gem(0, 3, q_name)").exec().unwrap();
    assert_eq!(
        s.backend.skills_view().unwrap().groups[0].gems[3].gem_id,
        Some(gem_id)
    );
    // the same filtered search again lists only supports, as upstream's does
    let again = our_search(&mut s, support.clone());
    let theirs = upstream_search(&lua, &support);
    assert!(again.rows.iter().all(|r| r.support), "the S filter is lost");
    assert_eq!(ids(&again), theirs.ids);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_search_ends_and_refuses() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "search-ends");
    // a box PoB doesn't have is an error, and opens no job (slot 3 is the empty row)
    for target in [GemTarget::Slot(4), GemTarget::Slot(99)] {
        assert!(
            s.run(Command::SearchGems(gem_query(
                0,
                target,
                "",
                GemFilter::All
            )))
            .is_err()
        );
        assert_eq!(s.backend.next_background(), None);
        assert!(s.backend.gem_search_view().unwrap().is_none());
    }
    assert!(
        s.run(Command::SearchGems(gem_query(
            42,
            GemTarget::Slot(0),
            "",
            GemFilter::All
        )))
        .is_err()
    );
    assert_eq!(s.backend.next_background(), None);
    // structural commands end the search
    let query = gem_query(0, GemTarget::Slot(0), "fire", GemFilter::All);
    s.run(Command::SearchGems(query.clone())).unwrap();
    let affected = s.run(Command::AddSocketGroup).unwrap();
    // the picker is told: one last view, then nothing
    assert!(affected.gem_search);
    let last = s.backend.gem_search_view().unwrap().expect("an ended view");
    assert!(last.ended && last.rows.is_empty() && last.query == query);
    assert!(s.backend.gem_search_view().unwrap().is_none());
    assert_eq!(s.backend.next_background(), None);
    s.run(Command::SearchGems(query.clone())).unwrap();
    // closing is the picker's own doing: no last view
    let affected = s.run(Command::CloseGemSearch).unwrap();
    assert!(!affected.gem_search);
    assert!(s.backend.gem_search_view().unwrap().is_none());
    // a delete during a search ends it the same way
    s.run(Command::SearchGems(query.clone())).unwrap();
    s.run(Command::DeleteGem { group: 0, slot: 0 }).unwrap();
    let last = s.backend.gem_search_view().unwrap().expect("an ended view");
    assert!(last.ended);
    assert!(s.backend.gem_search_view().unwrap().is_none());
    // an option PoB's sort cache checks restarts the sort; it is asked of PoB's control, so
    // choosing the field already chosen starts nothing
    our_search(&mut s, query.clone());
    assert_eq!(s.backend.next_background(), None);
    let field = s.backend.skills_view().unwrap().options.sort_field.selected;
    s.run(Command::SetSkillOption(SkillOption::SortField(field)))
        .unwrap();
    assert_eq!(s.backend.next_background(), None);
    s.run(Command::SetSkillOption(SkillOption::SortField(
        if field == 0 { 1 } else { 0 },
    )))
    .unwrap();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::GemDps));
    finish_gem_dps(&mut s);
    // a group table that is no longer at its index (as when a recalculation adds an item's
    // group) ends the search with a last view
    s.backend
        .lua
        .load(
            r#"local list = pob.build.skillsTab.socketGroupList
            local copy = {}
            for k, v in pairs(list[1]) do copy[k] = v end
            list[1] = copy"#,
        )
        .exec()
        .unwrap();
    s.backend.recalculate().unwrap();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::GemDps));
    let affected = s
        .backend
        .background_step(BackgroundTask::GemDps, &|_| {})
        .unwrap();
    assert!(affected.gem_search);
    let last = s.backend.gem_search_view().unwrap().expect("a last view");
    assert!(last.ended && !last.sorting && last.rows.is_empty());
    assert!(s.backend.gem_search_view().unwrap().is_none());
    assert_eq!(s.backend.next_background(), None);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_dps_retry_restarts_a_failed_sort() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "dps-retry");
    let query = gem_query(0, GemTarget::Slot(0), "", GemFilter::All);
    // break the sort on our side only
    let box_lua = "pob.build.skillsTab.gemSlots[1].nameSpec";
    s.backend
        .lua
        .load(format!(
            "{box_lua}.DPSBuilder = function() error('broken', 0) end"
        ))
        .exec()
        .unwrap();
    s.run(Command::SearchGems(query.clone())).unwrap();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::GemDps));
    assert!(
        s.backend
            .background_step(BackgroundTask::GemDps, &|_| {})
            .is_err()
    );
    assert_eq!(
        s.backend.next_background(),
        None,
        "a failed job stays stopped"
    );
    // undo the breakage and send the same search again (Retry)
    s.backend
        .lua
        .load(format!("{box_lua}.DPSBuilder = nil"))
        .exec()
        .unwrap();
    s.run(Command::SearchGems(query.clone())).unwrap();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::GemDps));
    finish_gem_dps(&mut s);
    let view = s.backend.gem_search_view().unwrap().unwrap();
    assert!(!view.sorting && view.rows.iter().any(|r| r.dps.is_some()));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn commands_answered_during_gem_dps() {
    use auto_pob::engine::{Engine, Event, Snapshot};
    use std::time::{Duration, Instant};
    let _guard = lock();
    pob_dir();
    let user = std::env::temp_dir().join(format!("auto-pob-gate-gem-dps-{}", std::process::id()));
    let _ = fs::remove_dir_all(&user);
    fs::create_dir_all(&user).unwrap();
    let build = user.join("attack.xml");
    fs::copy(fixtures_dir().join("attack.xml"), &build).unwrap();
    let engine = Engine::spawn(Game::Poe1, std::env::temp_dir(), user.clone(), || {});
    engine.send(Command::OpenBuild { path: build });
    engine.send(Command::SearchGems(gem_query(
        0,
        GemTarget::Slot(3),
        "",
        GemFilter::All,
    )));
    let deadline = Instant::now() + Duration::from_secs(300);
    let next = || {
        let wait = deadline.saturating_duration_since(Instant::now());
        engine.events.recv_timeout(wait).expect("an event")
    };
    // the sort has started when a search view says so
    loop {
        match next() {
            Event::Snapshot {
                snapshot: Snapshot::GemSearch(view),
                ..
            } => {
                assert!(view.sorting, "the sort ended before the label was sent");
                break;
            }
            Event::Error { message, .. } | Event::StartupFailed { message } => {
                panic!("{message}")
            }
            _ => {}
        }
    }
    let label_seq = engine.send(Command::SetGroupLabel {
        group: 0,
        text: "Mid sort".into(),
    });
    let mut labelled = false;
    let mut progress = 0;
    loop {
        match next() {
            Event::Snapshot {
                seq,
                snapshot: Snapshot::Skills(view),
            } if seq >= label_seq => {
                assert_eq!(view.groups[0].label, "Mid sort");
                labelled = true;
            }
            Event::Snapshot {
                snapshot: Snapshot::GemSearch(view),
                ..
            } if !view.sorting => {
                assert!(labelled, "the sort ended before the label was answered");
                assert!(!view.rows.is_empty());
                break;
            }
            Event::Progress {
                task: BackgroundTask::GemDps,
                ..
            } => progress += 1,
            Event::Error { message, .. } | Event::BackgroundFailed { message, .. } => {
                panic!("{message}")
            }
            _ => {}
        }
    }
    assert!(progress > 0, "the sort reported no progress");
    engine.send(Command::Shutdown);
    while engine.events.recv_timeout(Duration::from_secs(30)).is_ok() {}
    assert!(!engine.is_alive());
    let _ = fs::remove_dir_all(&user);
}

/// attack.xml as PoB saves it with another tab open (`viewMode`), opened in our engine.
fn attack_in_view_mode(test: &str, mode: &str) -> Session {
    let xml = fs::read_to_string(fixtures_dir().join("attack.xml")).unwrap();
    assert!(xml.contains("viewMode=\"TREE\""));
    let xml = xml.replace("viewMode=\"TREE\"", &format!("viewMode=\"{mode}\""));
    let mut s = Session::start(&format!("skills-{test}"));
    fs::create_dir_all(&s.user).unwrap();
    let path = s.user.join("source.xml");
    fs::write(&path, xml).unwrap();
    s.open_fixture("attack", &path);
    s
}

/// PoB draws the tab a build was saved on every frame (Build.lua:1276-1294). A frame on the
/// Skills tab would draw the gem boxes, and `GemSelectControl:Draw` resumes the DPS sort
/// against whatever group is shown (R13).
#[test]
#[ignore = "needs the pinned PoB install"]
fn frames_on_skills_view_mode_leave_dps_alone() {
    let _guard = lock();
    let mut s = attack_in_view_mode("view-mode", "SKILLS");
    let query = gem_query(0, GemTarget::Slot(3), "", GemFilter::All);
    s.run(Command::SearchGems(query.clone())).unwrap();
    for _ in 0..2 {
        s.backend
            .background_step(BackgroundTask::GemDps, &|_| {})
            .unwrap();
    }
    let done = |s: &Session| -> usize {
        lua_value(
            &s.backend.lua,
            r#"local n = 0
            for _, colour in pairs(pob.build.skillsTab.gemSlots[4].nameSpec.sortCache.dpsColor) do
                if colour ~= "" then n = n + 1 end
            end
            return n"#,
        )
    };
    let before = done(&s);
    assert!(before > 0);
    for _ in 0..5 {
        s.backend.lua.load("glue.frame()").exec().unwrap();
    }
    assert_eq!(done(&s), before, "a frame ran the DPS sort");
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::GemDps));
    // the build still says which tab it was saved on
    let view_mode: String = lua_value(&s.backend.lua, "return pob.build.viewMode");
    assert_eq!(view_mode, "SKILLS");
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "saved".into(),
        overwrite: false,
    })
    .unwrap();
    let saved = fs::read_to_string(s.builds_dir().join("saved.xml")).unwrap();
    assert!(saved.contains("viewMode=\"SKILLS\""), "viewMode not kept");
}

/// PoB processes a skill's stat map lazily, the first time a calculation looks a stat up
/// (Data.lua:1003-1013), and that is where `hasGlobalEffect` gets set. So which gems a DPS sort
/// calculates depends on which calculations ran before it, in PoB as here. Tests that compare
/// a restarted sort with a single one look up every stat a calculation can reach first
/// (CalcTools.lua:183-241: stats, qualityStats, constantStats), on every side.
fn warm_stat_maps(lua: &mlua::Lua) {
    lua.load(
        r#"for _, effect in pairs(data.skills) do
            local map = effect.statMap
            if map then
                for _, stat in ipairs(effect.stats or {}) do local _ = map[stat] end
                for _, stat in ipairs(effect.qualityStats or {}) do local _ = map[stat[1]] end
                for _, stat in ipairs(effect.constantStats or {}) do local _ = map[stat[1]] end
            end
        end"#,
    )
    .exec()
    .unwrap();
}

/// A DPS-relevant config change between slices restarts the sort; the finished values are
/// upstream's under the same config.
#[test]
#[ignore = "needs the pinned PoB install"]
fn gem_dps_restarts_on_config_change() {
    let _guard = lock();
    let (mut s, lua) = session_on("attack", "config-restart");
    warm_stat_maps(&s.backend.lua);
    warm_stat_maps(&lua);
    let query = gem_query(0, GemTarget::Slot(3), "", GemFilter::All);
    s.run(Command::SearchGems(query.clone())).unwrap();
    for _ in 0..2 {
        s.backend
            .background_step(BackgroundTask::GemDps, &|_| {})
            .unwrap();
    }
    let unbuffed = s.backend.gem_search_view().unwrap().unwrap().base_dps;
    s.run(Command::SetConfig {
        var: "buffAdrenaline".into(),
        value: auto_pob::engine::ConfigValue::Bool(true),
    })
    .unwrap();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::GemDps));
    finish_gem_dps(&mut s);
    let ours = s.backend.gem_search_view().unwrap().unwrap();
    assert!(!ours.sorting);
    lua.load(
        r#"local check = build.configTab.varControls.buffAdrenaline
        check.state = true
        check.changeFunc(true)
        build.buildFlag = true
        runCallback("OnFrame")"#,
    )
    .exec()
    .unwrap();
    let theirs = upstream_search(&lua, &query);
    assert!(ours.base_dps > unbuffed, "{} <= {unbuffed}", ours.base_dps);
    assert_eq!(ours.base_dps, theirs.base_dps);
    assert_eq!(our_dps(&ours), theirs.dps);
    assert_eq!(ids(&ours), theirs.ids);
}

/// A structural command that fails changes nothing, so the search stays open.
#[test]
#[ignore = "needs the pinned PoB install"]
fn failed_structural_command_keeps_the_search() {
    let _guard = lock();
    let (mut s, _) = session_on("attack", "failed-structural");
    let query = gem_query(0, GemTarget::Slot(3), "fire", GemFilter::All);
    s.run(Command::SearchGems(query.clone())).unwrap();
    for failing in [
        Command::DeleteSocketGroup { group: 42 },
        Command::MoveSocketGroup { from: 0, to: 42 },
        Command::DeleteGem { group: 0, slot: 42 },
        Command::SelectSkillSet { id: 42 },
    ] {
        let shown = format!("{failing:?}");
        assert!(s.run(failing).is_err(), "{shown}");
        let view = s.backend.gem_search_view().unwrap();
        assert_eq!(view.map(|v| v.query), Some(query.clone()), "{shown}");
    }
}
