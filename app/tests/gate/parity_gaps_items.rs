//! Task 13b's parity gaps on the Items tab, against upstream PoB pressing the same controls:
//! the databases' Ctrl+Shift+click and drag onto a slot (R23), Ring 3 and the Graft slots
//! (R30), Ctrl+click on the item list, Shift's child tooltips (R34), the stacked range sliders
//! (`showAllItemAffixes`) and the Config tab's Mod Browser (R28).

use super::items::{FIND_NO_SLIDER, NoSlider, display, gear_sessions, items_round_trip, on_both};
use super::items_harness::{
    ItemsAction, ItemsCheckpoint, assert_same, checkpoints, json_of, open_popups, run_items_recipe,
    strip_nulls, upstream_with_items_lib,
};
use super::{Session, fixtures_dir, lock, lua_value, snapshot_dir};
use auto_pob::engine::{Backend, Command, ItemDb, ItemTooltipQuery, PopupInput, PopupKind, Tab};
use mlua::LuaSerdeExt;

/// The slot `name` as a checkpoint has it
fn slot(check: &ItemsCheckpoint, name: &str) -> serde_json::Value {
    check.items["slots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no slot {name}"))
}

/// The id of the item named `name` in our Items view
fn item_id(s: &mut Session, name: &str) -> u32 {
    s.backend
        .items_view()
        .unwrap()
        .items
        .iter()
        .find(|e| e.name == name || e.name.starts_with(&format!("{name},")))
        .unwrap_or_else(|| panic!("no item {name}"))
        .id
}

/// Runs `command`, which must fail with `message` and change neither the Items view nor the
/// display panel
fn refused(s: &mut Session, command: Command, message: &str) {
    let before = (s.backend.items_view().unwrap(), display(s));
    let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
    assert!(
        error.to_string().contains(message),
        "{command:?}: {error} (wanted {message})"
    );
    assert_eq!(
        (s.backend.items_view().unwrap(), display(s)),
        before,
        "{command:?} changed something"
    );
    assert_eq!(open_popups(s), 0, "{command:?} left a popup open");
}

// ---- Databases (R23) ----

#[test]
#[ignore = "needs the pinned PoB install"]
fn db_second_slot_and_drop_match_pob() {
    let _guard = lock();
    let (recipe, mut s) = run_items_recipe("db_drop");
    let checks = checkpoints(&recipe);
    // Ctrl+Shift+click put the ring in Ring 2; Ring 1 stayed as it was
    assert_ne!(slot(checks[0], "Ring 2")["selected"], 0);
    assert_eq!(slot(checks[0], "Ring 1")["selected"], 0);
    // the drop equipped Ring 1, and its undo took it out again
    assert_ne!(slot(checks[1], "Ring 1")["selected"], 0);
    assert_eq!(slot(checks[2], "Ring 1")["selected"], 0);
    let dropped = recipe
        .actions
        .iter()
        .find_map(|a| match a {
            ItemsAction::DropDbItem { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("a drop was recorded");
    let files = snapshot_dir(&s.user);
    // a ring can't go on the head; Graft 1 is hidden on a 3_29 tree; there is no such slot
    for (slot, message) in [
        ("Helmet", "that item can't go in that slot"),
        ("Graft 1", "PoB doesn't offer that here"),
        ("Ring 9", "no such slot"),
    ] {
        refused(
            &mut s,
            Command::DropDbItem {
                db: ItemDb::Uniques,
                name: dropped.clone(),
                slot: slot.into(),
            },
            message,
        );
    }
    refused(
        &mut s,
        Command::DropDbItem {
            db: ItemDb::Uniques,
            name: "No Such Ring, Iron Ring".into(),
            slot: "Ring 1".into(),
        },
        "no such item in the database",
    );
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}

// ---- Ring 3, grafts and Ctrl+click on the list (R30) ----

#[test]
#[ignore = "needs the pinned PoB install"]
fn ring3_graft_match_pob() {
    let _guard = lock();
    let (recipe, mut s) = run_items_recipe("ring3_graft");
    let checks = checkpoints(&recipe);
    // "+1 Ring Slot" shows Ring 3, the 3_27 tree the grafts
    for name in ["Ring 3", "Graft 1", "Graft 2"] {
        assert_eq!(slot(checks[0], name)["shown"], true, "{name}");
    }
    let mut id = |name: &str| item_id(&mut s, name);
    assert_eq!(slot(checks[1], "Ring 3")["selected"], id("Third Loop"));
    assert_eq!(
        slot(checks[1], "Graft 1")["selected"],
        id("Battering Uulgraft")
    );
    assert_eq!(
        slot(checks[1], "Graft 2")["selected"],
        id("Spiking Uulgraft")
    );
    // Ctrl+Shift+click: Ring 2; Ctrl+click: Ring 1
    assert_eq!(slot(checks[2], "Ring 2")["selected"], id("Fourth Band"));
    assert_eq!(slot(checks[2], "Ring 1")["selected"], id("Fifth Circle"));
    // the second Ctrl+click took it out, and the undo put it back
    assert_eq!(slot(checks[3], "Ring 1")["selected"], id("Fifth Circle"));
    // the node gone, Ring 3 hides
    assert_eq!(slot(checks[4], "Ring 3")["shown"], false);
    let files = snapshot_dir(&s.user);
    refused(
        &mut s,
        Command::QuickEquipItem {
            id: 9999,
            second_slot: false,
        },
        "no such item",
    );
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn items_round_trip_ring3_graft() {
    items_round_trip("ring3_graft");
}

// ---- Shift's child tooltips (R34) ----

/// An amulet that grants a skill: its tooltip has a child tooltip, filled under Shift
const SKILL_AMULET: &str =
    "Rarity: RARE\nSkill Charm\nOnyx Amulet\nImplicits: 0\nGrants Level 20 Anger Skill";
/// A ring whose mods differ by ring slot: a list hover under Shift reads the second slot's mod
/// list (ItemsTab.lua:4447), which only weapon, armour and flask lines show
const SIDED_RING: &str = "Rarity: RARE\nSided Loop\nIron Ring\nImplicits: 0\nLeft ring slot: +30 to maximum Life\nRight ring slot: +40 to maximum Mana";

#[test]
#[ignore = "needs the pinned PoB install"]
fn shift_tooltip_matches_pob() {
    let _guard = lock();
    let (mut s, lua) = gear_sessions("shift-tooltip");
    for text in [SKILL_AMULET, SIDED_RING] {
        lua.globals().set("text", text).unwrap();
        lua.load("act.paste(text) act.save()").exec().unwrap();
        s.run(Command::PasteItem { text: text.into() }).unwrap();
        s.run(Command::SaveDisplayItem).unwrap();
    }
    let files = snapshot_dir(&s.user);
    for (name, slot) in [
        ("Skill Charm", None),
        ("Sided Loop", None),
        ("Skill Charm", Some("Amulet")),
        ("Sided Loop", Some("Ring 2")),
    ] {
        let id = item_id(&mut s, name);
        let theirs_id: u32 = lua_value(&lua, &format!("return pick.item({name:?})"));
        assert_eq!(id, theirs_id, "{name}: ids differ");
        let mut seen = Vec::new();
        for shift in [false, true] {
            let ours = s
                .backend
                .item_tooltip_view(&ItemTooltipQuery {
                    id,
                    slot: slot.map(str::to_string),
                    theirs: false,
                    shift,
                })
                .unwrap()
                .unwrap_or_else(|| panic!("{name}: no tooltip"));
            let code = match slot {
                None => format!("return listTooltip({id})"),
                Some(slot) => format!("return slotTooltip({slot:?}, {id})"),
            };
            let code = format!(
                "local saved = IsKeyDown IsKeyDown = function(k) return {shift} and k == 'SHIFT' end \
                 local ok, out = pcall(function() {code} end) IsKeyDown = saved assert(ok, out) return out"
            );
            let theirs: serde_json::Value = lua_value(&lua, &code);
            let ours = json_of(&ours.tooltip);
            assert_same(
                &ours,
                &strip_nulls(theirs),
                &format!("{name} in {slot:?}, shift {shift}"),
            );
            seen.push(ours);
        }
        let children =
            |v: &serde_json::Value| v.get("children").map_or(0, |c| c.as_array().unwrap().len());
        assert_eq!(children(&seen[0]), 0, "{name}: children without Shift");
        if name == "Skill Charm" {
            assert_eq!(
                children(&seen[1]),
                1,
                "{name}: no child tooltip under Shift"
            );
        }
    }
    // the key state is put back
    let shift: bool = lua_value(
        &s.backend.lua,
        "return IsKeyDown('SHIFT') and true or false",
    );
    assert!(!shift, "IsKeyDown was left swapped");
    assert_eq!(open_popups(&s), 0);
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}

// ---- The stacked range sliders (displayItemStacked*) ----

#[test]
#[ignore = "needs the pinned PoB install"]
fn stacked_range_lines_match_pob() {
    let _guard = lock();
    let (mut s, lua) = gear_sessions("stacked-range");
    let found: NoSlider = s
        .backend
        .lua
        .from_value(s.backend.lua.load(FIND_NO_SLIDER).eval().unwrap())
        .unwrap();
    // PoB's default (Main.lua:121): one slider and box per line on a unique
    let on: bool = lua_value(&lua, "return main.showAllItemAffixes");
    assert!(on, "showAllItemAffixes is off upstream");
    lua.globals().set("text", found.text.clone()).unwrap();
    lua.load("act.paste(text)").exec().unwrap();
    s.run(Command::PasteItem { text: found.text }).unwrap();
    let compare = |s: &mut Session, what: &str| {
        let theirs: serde_json::Value = lua_value(&lua, "return displayDump()");
        let ours = json_of(&display(s).expect("displayed"));
        assert_same(&ours, &strip_nulls(theirs), what);
    };
    compare(&mut s, "pasted");
    assert!(display(&mut s).unwrap().stacked, "not stacked");
    let files = snapshot_dir(&s.user);
    for (what, command, code) in [
        (
            "slider",
            Command::SetRangeLine {
                line: found.plain,
                value: 0.25,
            },
            format!("act.range({}, 0.25)", found.plain),
        ),
        (
            "foulborn",
            Command::SetRangeLineMutated {
                line: found.bare,
                on: true,
            },
            format!("act.mutated({}, true)", found.bare),
        ),
    ] {
        let before = display(&mut s);
        s.run(command).unwrap();
        lua.load(code).exec().unwrap();
        compare(&mut s, what);
        assert_ne!(display(&mut s), before, "{what}: nothing changed");
        // the stacked controls took it: the dropdown's selection didn't move
        let selected: u32 = lua_value(
            &s.backend.lua,
            "return pob.build.itemsTab.controls.displayItemRangeLine.selIndex",
        );
        assert_eq!(selected, 1, "{what}: the line dropdown moved");
    }
    let lines = display(&mut s).unwrap().range_lines.len();
    assert!(
        s.run(Command::SetRangeLine {
            line: lines,
            value: 0.5
        })
        .is_err()
    );
    // with the option off, the line dropdown is back
    on_both(&s, &lua, "main.showAllItemAffixes = false");
    s.backend
        .lua
        .load("pob.main.main.showAllItemAffixes = false")
        .exec()
        .unwrap();
    compare(&mut s, "option off");
    assert!(!display(&mut s).unwrap().stacked, "still stacked");
    assert_eq!(open_popups(&s), 0);
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}

// ---- The Mod Browser (R28) ----

const OPEN_MOD_BROWSER: &str = "act.mod_browser_open(1)";

#[test]
#[ignore = "needs the pinned PoB install"]
fn mod_browser_matches_pob() {
    let _guard = lock();
    let xml = fixtures_dir().join("attack.xml");
    let lua = upstream_with_items_lib(&xml, "attack");
    let mut s = Session::start("mod-browser");
    s.open_fixture("attack", &xml);
    let files = snapshot_dir(&s.user);
    let compare = |s: &mut Session, what: &str| {
        let theirs: serde_json::Value = lua_value(&lua, "return popupDump()");
        let ours = s.backend.popup_view().unwrap().map(|v| json_of(&v));
        match ours {
            Some(ours) => assert_same(&ours, &strip_nulls(theirs), what),
            None => assert!(theirs.is_null(), "{what}: ours closed, upstream's open"),
        }
        assert_eq!(open_popups(s), 0, "{what}: a popup was left in PoB's stack");
    };
    let text_of = |s: &mut Session, lua: &mlua::Lua| -> (String, String) {
        let theirs: String = lua_value(
            lua,
            "local t = build.configTab return t.configSets[t.activeConfigSetId].customModsList[1].text or ''",
        );
        (s.config().custom_mods[0].text.clone(), theirs)
    };
    let row = |s: &mut Session, index: usize| -> String {
        let view = s.backend.popup_view().unwrap().expect("open");
        let list = view
            .controls
            .iter()
            .find(|c| c.name == "listControl")
            .expect("the list");
        list.rows[index].key.clone()
    };

    // search, pick, Add
    let affected = s
        .run(Command::OpenPopup(PopupKind::ModBrowser { group: 0 }))
        .unwrap();
    assert!(affected.popup);
    lua.load(OPEN_MOD_BROWSER).exec().unwrap();
    compare(&mut s, "opened");
    s.run(Command::SetPopupControl {
        name: "search".into(),
        value: PopupInput::Text("life regen".into()),
    })
    .unwrap();
    lua.load(r#"act.popup_set("search", { Text = "life regen" })"#)
        .exec()
        .unwrap();
    compare(&mut s, "searched");
    let key = row(&mut s, 2);
    s.run(Command::SetPopupControl {
        name: "listControl".into(),
        value: PopupInput::Row(key.clone()),
    })
    .unwrap();
    lua.globals().set("key", key.clone()).unwrap();
    lua.load(r#"act.popup_set("listControl", { Row = key })"#)
        .exec()
        .unwrap();
    compare(&mut s, "picked");
    let affected = s
        .run(Command::PressPopupButton {
            name: "save".into(),
        })
        .unwrap();
    assert!(
        affected.config && affected.recalc,
        "Add didn't flag the config"
    );
    lua.load(r#"act.popup_press("save")"#).exec().unwrap();
    compare(&mut s, "added");
    let (ours, theirs) = text_of(&mut s, &lua);
    assert_eq!(ours, theirs, "the group's text after Add");
    assert!(
        ours.contains(&key),
        "the picked mod {key:?} is not in {ours:?}"
    );

    // the double click is the Add
    s.run(Command::OpenPopup(PopupKind::ModBrowser { group: 0 }))
        .unwrap();
    lua.load(OPEN_MOD_BROWSER).exec().unwrap();
    let key = row(&mut s, 0);
    let affected = s
        .run(Command::SetPopupControl {
            name: "listControl".into(),
            value: PopupInput::RowDoubleClick(key.clone()),
        })
        .unwrap();
    assert!(
        affected.config && affected.recalc,
        "the double click didn't flag the config"
    );
    lua.globals().set("key", key.clone()).unwrap();
    lua.load(
        r#"local c = main.popups[1].controls.listControl
        for i, entry in ipairs(c.list) do
            if entry == key then c:SelectIndex(i) c:OnSelClick(i, entry, true) end
        end"#,
    )
    .exec()
    .unwrap();
    compare(&mut s, "double clicked");
    let (ours, theirs) = text_of(&mut s, &lua);
    assert_eq!(ours, theirs, "the group's text after the double click");
    assert_eq!(ours.lines().count(), 2, "{ours:?}");

    // an undo replaces the group's table: the window is over, as PoB's modal one never sees it
    s.run(Command::OpenPopup(PopupKind::ModBrowser { group: 0 }))
        .unwrap();
    let affected = s.run(Command::Undo(Tab::Config)).unwrap();
    assert!(affected.popup, "the undo didn't end the window");
    assert!(s.backend.popup_view().unwrap().is_none());
    let error = s
        .run(Command::PressPopupButton {
            name: "save".into(),
        })
        .expect_err("pressed a window that is over");
    assert!(error.to_string().contains("no popup is open"), "{error}");
    let error = s
        .run(Command::OpenPopup(PopupKind::ModBrowser { group: 5 }))
        .expect_err("a group that isn't there");
    assert!(
        error.to_string().contains("no custom modifier group"),
        "{error}"
    );
    assert_eq!(open_popups(&s), 0);
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}
