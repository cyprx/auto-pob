//! Task 13b's parity gaps on the Items tab, against upstream PoB pressing the same controls:
//! the databases' Ctrl+Shift+click and drag onto a slot (R23), Ring 3 and the Graft slots
//! (R30), Ctrl+click on the item list, Shift's child tooltips (R34), the stacked range sliders
//! (`showAllItemAffixes`) and the Config tab's Mod Browser (R28).

use super::items::{FIND_NO_SLIDER, NoSlider, display, gear_sessions, items_round_trip, on_both};
use super::items_harness::{
    ItemsAction, ItemsCheckpoint, assert_same, checkpoints, json, json_of, open_popups,
    run_items_recipe, strip_nulls, upstream_json, upstream_with_items_lib,
    upstream_with_items_lib_xml,
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
                    shared: None,
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
    let rows = s
        .backend
        .popup_view()
        .unwrap()
        .expect("open")
        .controls
        .iter()
        .find(|c| c.name == "listControl")
        .map_or(0, |c| c.rows.len());
    eprintln!("mod browser rows: {rows}");
    // the view's rows come as two flat lists, not a table per row: the 6188 rows' tables and
    // the collection they caused were most of the view's cost on every change (M-9)
    let kb: f64 = lua_value(
        &s.backend.lua,
        "collectgarbage('stop') local before = collectgarbage('count') glue.popup_view() \
         local kb = collectgarbage('count') - before collectgarbage('restart') return kb",
    );
    eprintln!("mod browser view: {kb:.0} KB of Lua allocation");
    assert!(kb < 400.0, "the Mod Browser's view allocated {kb:.0} KB");
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

// ---- The Items tab's tree dropdown (specSelect) ----

#[test]
#[ignore = "needs the pinned PoB install"]
fn items_tree_dropdown_matches_pob() {
    let _guard = lock();
    let xml = fixtures_dir().join("attack.xml");
    let lua = upstream_with_items_lib(&xml, "attack");
    let mut s = Session::start("items-tree-dropdown");
    s.open_fixture("attack", &xml);
    let files = snapshot_dir(&s.user);
    let compare = |s: &mut Session, what: &str| -> serde_json::Value {
        let ours = json(&s.backend.items_view().unwrap());
        assert_same(&ours, &upstream_json(&lua), what);
        assert_eq!(open_popups(s), 0, "{what}: a popup was left open");
        ours["specs"].clone()
    };
    let specs = compare(&mut s, "one tree");
    assert_eq!(specs["enabled"], false, "one tree: the dropdown is enabled");
    // a second tree: PoB's own New on the tree manager; the Items view must be sent again
    let affected = s
        .run(Command::NewSpec {
            title: "Second".into(),
        })
        .unwrap();
    assert!(affected.items, "a new tree leaves the Items dropdown stale");
    lua.load(r#"act.new_spec("Second")"#).exec().unwrap();
    let specs = compare(&mut s, "two trees");
    assert_eq!(specs["enabled"], true);
    assert_eq!(specs["labels"].as_array().unwrap().len(), 2);
    // PoB's Items tab dropdown picks each tree in turn
    for index in [0usize, 1] {
        let affected = s.run(Command::SelectSpec { index }).unwrap();
        assert!(
            affected.items,
            "a tree switch leaves the Items dropdown stale"
        );
        lua.load(format!(
            "build.itemsTab.controls.specSelect:SetSel({}) runCallback('OnFrame')",
            index + 1
        ))
        .exec()
        .unwrap();
        let specs = compare(&mut s, &format!("tree {index} picked"));
        assert_eq!(specs["selected"], index);
    }
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}

// ---- Shared items and shared item sets (NF-2, R29 amended) ----

const SHARED_RING: &str = "Rarity: RARE\nShared Loop\nIron Ring\nImplicits: 0\n+20 to maximum Life";
const SHARED_BODY: &str =
    "Rarity: UNIQUE\nKaom's Heart\nGlorious Plate\nImplicits: 0\n+500 to maximum Life";
/// A shared item that grants a skill: Shift shows its gem's tooltip as a child (R34)
const SHARED_CHARM: &str =
    "Rarity: RARE\nShared Charm\nOnyx Amulet\nImplicits: 0\nGrants Level 20 Anger Skill";
const SHARED_SET_RING: &str =
    "Rarity: RARE\nSet Loop\nIron Ring\nImplicits: 0\n+10 to maximum Life";

/// The `<Mode>` element as PoB's writer lays it out (xml.lua:116-158)
fn mode_element(mode: &str, args: &[String]) -> String {
    let mut out = format!("<Mode mode=\"{mode}\">\n");
    for arg in args {
        out.push_str(&format!("\t\t<Arg {arg}/>\n"));
    }
    out.push_str("\t</Mode>");
    out
}

/// A Settings.xml with three shared items and a shared set, `mode` as the last mode and
/// thousands separators off (which the session overrides)
fn shared_seed(mode: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PathOfBuilding>\n\t{mode}\n\
         \t<SharedItems>\n\t\t<Item>{SHARED_RING}</Item>\n\t\t<Item>{SHARED_BODY}</Item>\n\
         \t\t<Item>{SHARED_CHARM}</Item>\n\
         \t\t<ItemSet title=\"Shared Set\"><Item slotName=\"Ring 1\">{SHARED_SET_RING}</Item></ItemSet>\n\
         \t</SharedItems>\n\t<Misc buildSortMode=\"NAME\" showThousandsSeparators=\"false\"/>\n</PathOfBuilding>\n"
    )
}

/// The build list as the last mode, as PoB writes it with nothing selected
fn list_mode() -> String {
    mode_element(
        "LIST",
        &["boolean=\"false\"".to_string(), "string=\"\"".to_string()],
    )
}

/// The same shared lists upstream, which reads no Settings.xml
const SHARED_SEED_UPSTREAM: &str = r#"
table.insert(main.sharedItemList, new("Item", SHARED_RING))
table.insert(main.sharedItemList, new("Item", SHARED_BODY))
table.insert(main.sharedItemList, new("Item", SHARED_CHARM))
table.insert(main.sharedItemSetList, { title = "Shared Set", slots = { ["Ring 1"] = new("Item", SHARED_SET_RING) } })
"#;

/// Upstream with `xml` open and the seeded shared lists
fn shared_upstream(xml: &str, name: &str) -> mlua::Lua {
    let lua = upstream_with_items_lib_xml(xml, name);
    for (name, text) in [
        ("SHARED_RING", SHARED_RING),
        ("SHARED_BODY", SHARED_BODY),
        ("SHARED_CHARM", SHARED_CHARM),
        ("SHARED_SET_RING", SHARED_SET_RING),
    ] {
        lua.globals().set(name, text).unwrap();
    }
    lua.load(SHARED_SEED_UPSTREAM).exec().unwrap();
    lua
}

/// Settings.xml without its `<SharedItems>` element
fn outside_shared(xml: &str) -> String {
    let Some(start) = xml.find("<SharedItems") else {
        return xml.to_string();
    };
    let rest = &xml[start..];
    let end = if rest.starts_with("<SharedItems/>") {
        start + "<SharedItems/>".len()
    } else {
        start
            + rest
                .find("</SharedItems>")
                .expect("SharedItems is not closed")
            + "</SharedItems>".len()
    };
    format!("{}{}", &xml[..start], &xml[end..])
}

/// The `<SharedItems>` element of Settings.xml
fn shared_part(xml: &str) -> String {
    xml.replacen(&outside_shared(xml), "", 1)
}

/// The `<Mode>` element of Settings.xml, as written
fn written_mode(xml: &str) -> &str {
    let start = xml.find("<Mode").expect("no Mode element");
    let end = start + xml[start..].find("</Mode>").expect("Mode is not closed") + "</Mode>".len();
    &xml[start..end]
}

/// Every file under the user folder but Settings.xml
fn other_files(user: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = snapshot_dir(user);
    files.retain(|path, _| !path.ends_with("Settings.xml"));
    files
}

/// Opens the uniques database on both sides searched for `text`; the first row's name
fn db_search(
    s: &mut super::longtail_harness::LongtailSession,
    lua: &mlua::Lua,
    text: &str,
) -> String {
    use auto_pob::engine::ItemDbQuery;
    s.run(Command::SetItemDb(ItemDbQuery {
        db: ItemDb::Uniques,
        slot: 0,
        kind: 0,
        league: 0,
        requirement: 0,
        obtainable: 0,
        sort: 0,
        search: text.into(),
        search_mode: 0,
    }))
    .unwrap();
    while let Some(task) = s.backend.next_background() {
        s.backend.background_step(task, &|_| {}).unwrap();
    }
    lua.load(format!(
        r#"act.db_open("Uniques", {{ search = {text:?} }})"#
    ))
    .exec()
    .unwrap();
    lua_value(lua, r#"return dbDump("Uniques").rows[1].name"#)
}

/// The items view with the items an import added made order-free: PoB adds a set's items in
/// `pairs` order (ItemSetListControl.lua:97), which differs between Lua states. Items with ids
/// above `before` lose their ids and are sorted; the listed ids past `before`'s are sorted.
fn canonical_import(view: &serde_json::Value, before: u64) -> serde_json::Value {
    let mut view = view.clone();
    let items = view["items"].as_array_mut().unwrap();
    let (old, mut new): (Vec<_>, Vec<_>) = items
        .drain(..)
        .partition(|e| e["id"].as_u64().unwrap() <= before);
    for entry in &mut new {
        entry.as_object_mut().unwrap().remove("id");
    }
    new.sort_by_key(|e| e.to_string());
    items.extend(old);
    items.extend(new);
    let listed = view["listed"].as_array_mut().unwrap();
    let (old, mut new): (Vec<_>, Vec<_>) = listed
        .drain(..)
        .partition(|id| id.as_u64().unwrap() <= before);
    new.sort_by_key(|id| id.as_u64().unwrap());
    listed.extend(old);
    listed.extend(new);
    view
}

/// The last item set's slots as slot -> the equipped item's raw text, read the same way on
/// both sides (`TAB` is the Items tab)
const LAST_SET_SLOTS: &str = r#"
local tab = TAB
local set = tab.itemSets[tab.itemSetOrderList[#tab.itemSetOrderList]]
local out = { title = set.title or "", slots = {} }
for slotName, slot in pairs(set) do
	if type(slot) == "table" and slot.selItemId and slot.selItemId ~= 0 then
		local item = tab.items[slot.selItemId]
		out.slots[slotName] = item.name .. "\n" .. item.raw
	end
end
return out
"#;

#[test]
#[ignore = "needs the pinned PoB install"]
fn shared_items_match_pob() {
    use super::longtail_harness::{LongtailSession, settings_folder};
    use auto_pob::engine::SharedItemSource;
    use std::collections::BTreeMap;
    let _guard = lock();
    let xml = fixtures_dir().join("attack.xml");
    // the last mode is a build classic PoB had open, not the one opened here: a shared save
    // must write it back as it was, args and all (I-3)
    let seeded_mode = std::cell::RefCell::new(String::new());
    let mut s = LongtailSession::start("shared-items", &|user| {
        let other = settings_folder(user).join("Builds/Other.xml");
        std::fs::copy(&xml, &other).unwrap();
        let mode = mode_element(
            "BUILD",
            &[
                format!("string=\"{}\"", other.display()),
                "string=\"Other\"".to_string(),
            ],
        );
        *seeded_mode.borrow_mut() = mode.clone();
        shared_seed(&mode)
    });
    let seeded_mode = seeded_mode.into_inner();
    let dest = settings_folder(&s.user).join("Builds/attack.xml");
    std::fs::copy(&xml, &dest).unwrap();
    s.run(Command::OpenBuild { path: dest }).unwrap();
    // an override the shared saves must keep out of the file (R12)
    s.run(Command::SetOverrides {
        values: BTreeMap::from([("showThousandsSeparators".to_string(), "true".to_string())]),
    })
    .unwrap();
    let files = other_files(&s.user);
    let lua = shared_upstream(&std::fs::read_to_string(&xml).unwrap(), "attack");
    let text = |s: &LongtailSession| String::from_utf8(s.settings_bytes()).unwrap();
    let compare = |s: &mut LongtailSession, what: &str| -> serde_json::Value {
        let ours = json(&s.backend.items_view().unwrap());
        assert_same(&ours, &upstream_json(&lua), what);
        assert_eq!(s.open_popups(), 0, "{what}: a popup was left open");
        ours
    };
    let act = |code: &str| {
        lua.load(code)
            .exec()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    };

    // PoB's own LoadSharedItems read the seed
    let seeded = compare(&mut s, "seeded");
    assert_eq!(seeded["shared_items"].as_array().unwrap().len(), 3);
    assert_eq!(seeded["shared_sets"][0]["title"], "Shared Set");
    assert_eq!(seeded["shared_sets"][0]["raw_title"], "Shared Set");
    assert_eq!(
        seeded["shared_sets"][0]["lines"].as_array().unwrap().len(),
        1
    );

    // viewing writes nothing: the view, a shared item's tooltip with and without Shift, and a
    // database row's (M-2)
    let before = s.settings_bytes();
    let children =
        |v: &serde_json::Value| v.get("children").map_or(0, |c| c.as_array().unwrap().len());
    for shift in [false, true] {
        let ours = s
            .backend
            .item_tooltip_view(&ItemTooltipQuery {
                id: 0,
                slot: None,
                theirs: false,
                shift,
                shared: Some(2),
            })
            .unwrap()
            .expect("a tooltip");
        let theirs: serde_json::Value = lua_value(
            &lua,
            &format!(
                "local saved = IsKeyDown IsKeyDown = function(k) return {shift} and k == 'SHIFT' end \
                 local tt = new('Tooltip') \
                 local ok, err = pcall(function() build.itemsTab.controls.sharedItemList:AddValueTooltip(tt, 3, main.sharedItemList[3]) end) \
                 IsKeyDown = saved assert(ok, err) return tooltipDump(tt)"
            ),
        );
        let ours = json_of(&ours.tooltip);
        assert_same(
            &ours,
            &strip_nulls(theirs),
            &format!("shared tooltip, shift {shift}"),
        );
        assert_eq!(
            children(&ours),
            usize::from(shift),
            "shared tooltip children, shift {shift}"
        );
    }
    let skill_unique = db_search(&mut s, &lua, "Blood of Corruption");
    for shift in [false, true] {
        let ours = s
            .backend
            .control_tooltip_view(&auto_pob::engine::ControlTooltipQuery::DbItem {
                db: ItemDb::Uniques,
                name: skill_unique.clone(),
                shift,
            })
            .unwrap()
            .expect("a database tooltip");
        let theirs: serde_json::Value = lua_value(
            &lua,
            &format!(
                "local saved = IsKeyDown IsKeyDown = function(k) return {shift} and k == 'SHIFT' end \
                 local ok, out = pcall(dbTooltip, 'Uniques', {skill_unique:?}) \
                 IsKeyDown = saved assert(ok, out) return out"
            ),
        );
        let ours = json_of(&ours.tooltip);
        assert_same(
            &ours,
            &strip_nulls(theirs),
            &format!("database tooltip, shift {shift}"),
        );
        assert_eq!(
            children(&ours),
            usize::from(shift),
            "database tooltip children, shift {shift}"
        );
    }
    let shift: bool = s.eval("return IsKeyDown('SHIFT') and true or false");
    assert!(!shift, "IsKeyDown was left swapped");
    s.backend.items_view().unwrap();
    assert_eq!(s.settings_bytes(), before, "a view wrote Settings.xml");

    // Ctrl+C on a shared row copies what PoB copies (I-4)
    let affected = s.run(Command::CopySharedItem { index: 1 }).unwrap();
    assert_eq!(affected.shared_text, Some(1));
    let ours = s.backend.shared_item_text(1).unwrap();
    let theirs: String = lua_value(&lua, "return act.shared_copy(1)");
    assert!(ours.shared && ours.id == 1);
    assert_eq!(ours.text, theirs, "the shared copy text");
    assert!(ours.text.contains("Kaom's Heart\r\n"), "{}", ours.text);
    compare(&mut s, "copied");

    // a build item dragged onto row 1 of the shared list: saved at once, the last mode kept
    // as the file has it and the override out of the file
    let id = seeded["items"][0]["id"].as_u64().unwrap() as u32;
    let affected = s
        .run(Command::AddSharedItem {
            source: SharedItemSource::Item { id },
            index: 1,
        })
        .unwrap();
    assert!(affected.items, "the Items view is stale");
    act(&format!("act.shared_add_item({id}, 1)"));
    compare(&mut s, "build item shared");
    let first = text(&s);
    assert_ne!(first.as_bytes(), before.as_slice(), "nothing was saved");
    assert_eq!(written_mode(&first), seeded_mode, "the last mode changed");
    assert!(
        first.contains("showThousandsSeparators=\"false\""),
        "the override reached Settings.xml"
    );
    assert_eq!(shared_part(&first).matches("<Item>").count(), 4, "{first}");

    // every later edit of the shared lists changes only <SharedItems>
    let mut last = first;
    let mut edit = |s: &mut LongtailSession, command: Command, code: &str, what: &str| {
        let affected = s.run(command).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(affected.items, "{what}: the Items view is stale");
        act(code);
        compare(s, what);
        let now = text(s);
        assert_eq!(
            outside_shared(&now),
            outside_shared(&last),
            "{what}: more than the shared items changed"
        );
        assert_ne!(shared_part(&now), shared_part(&last), "{what}: not saved");
        last = now;
    };

    // a database row onto row 0
    let name = db_search(&mut s, &lua, "Kalandra");
    edit(
        &mut s,
        Command::AddSharedItem {
            source: SharedItemSource::Db {
                db: ItemDb::Uniques,
                name: name.clone(),
            },
            index: 0,
        },
        &format!("act.shared_add_db('Uniques', {name:?}, 0)"),
        "database row shared",
    );
    edit(
        &mut s,
        Command::MoveSharedItem { from: 0, to: 2 },
        "act.shared_move(0, 2)",
        "moved",
    );
    edit(
        &mut s,
        Command::DeleteSharedItem { index: 2 },
        "act.shared_delete(2)",
        "deleted",
    );

    // into the build: the shared list stays, nothing is saved. On the item list a drop lands
    // at the row under the cursor (I-4), and a database row dragged there is added unequipped.
    let saved = s.settings_bytes();
    for (index, slot, at, code) in [
        (0, Some("Ring 2"), None, "act.shared_drop(0, 'Ring 2')"),
        (2, None, Some(1), "act.shared_drop(2, nil, 1)"),
        (3, None, None, "act.shared_drop(3, nil)"),
    ] {
        let affected = s
            .run(Command::DropSharedItem {
                index,
                slot: slot.map(str::to_string),
                at,
            })
            .unwrap();
        assert!(affected.items && affected.build && affected.recalc);
        act(code);
        let view = compare(&mut s, &format!("dropped {index} on {slot:?} at {at:?}"));
        if let Some(at) = at {
            let landed = view["listed"][at].as_u64().unwrap();
            let entry = view["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"].as_u64() == Some(landed))
                .unwrap();
            assert!(
                entry["name"].as_str().unwrap().starts_with("Kaom's Heart"),
                "the drop didn't land at row {at}: {entry}"
            );
        }
    }
    let affected = s
        .run(Command::DropDbItemOnList {
            db: ItemDb::Uniques,
            name: name.clone(),
            at: Some(0),
        })
        .unwrap();
    assert!(affected.items && affected.build && affected.recalc);
    act(&format!("act.db_drop_on_list('Uniques', {name:?}, 0)"));
    let view = compare(&mut s, "database row dropped on the list");
    let first_listed = view["listed"][0].as_u64().unwrap();
    assert!(
        view["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"].as_u64() == Some(first_listed) && e.get("equipped").is_none()),
        "the database row isn't first and unequipped"
    );
    s.run(Command::ViewSharedItem { index: 1 }).unwrap();
    act("act.shared_view(1)");
    let theirs: serde_json::Value = lua_value(&lua, "return displayDump()");
    let ours = json_of(&s.backend.display_item_view().unwrap().expect("displayed"));
    assert_same(&ours, &strip_nulls(theirs), "shared item displayed");
    assert_eq!(
        s.settings_bytes(),
        saved,
        "a build change wrote Settings.xml"
    );
    // PoB hides the shared list under the displayed item: nothing reaches it
    assert!(!s.backend.items_view().unwrap().shared_shown);
    let error = s
        .run(Command::DeleteSharedItem { index: 0 })
        .expect_err("the list is hidden");
    assert!(
        error.to_string().contains("PoB doesn't offer that here"),
        "{error}"
    );
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
    act("act.cancel()");
    assert!(compare(&mut s, "display cancelled")["shared_shown"] == true);

    // refusals change nothing and write nothing
    for (command, message) in [
        (
            Command::DeleteSharedItem { index: 9 },
            "no such shared item",
        ),
        (
            Command::DropSharedItem {
                index: 0,
                slot: Some("Helmet".into()),
                at: None,
            },
            "that item can't go in that slot",
        ),
        (
            Command::DropSharedItem {
                index: 0,
                slot: None,
                at: Some(999),
            },
            "no such row",
        ),
        (
            Command::AddSharedItem {
                source: SharedItemSource::Item { id: 9999 },
                index: 0,
            },
            "no such item",
        ),
        (
            Command::AddSharedItem {
                source: SharedItemSource::Item { id },
                index: 99,
            },
            "no such row",
        ),
        (Command::CopySharedItem { index: 99 }, ""),
        // the main skill has no minion: the sidebar's minion dropdown is hidden (M-10)
        (
            Command::DropOnMinion {
                source: auto_pob::engine::ItemDragSource::Shared { index: 1 },
            },
            "PoB doesn't offer that here",
        ),
    ] {
        let view = s.backend.items_view().unwrap();
        if let Command::CopySharedItem { index } = command {
            assert!(s.backend.shared_item_text(index).is_err());
            continue;
        }
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert!(error.to_string().contains(message), "{command:?}: {error}");
        assert_eq!(
            s.backend.items_view().unwrap(),
            view,
            "{command:?} changed something"
        );
        assert_eq!(s.open_popups(), 0);
    }
    assert_eq!(s.settings_bytes(), saved, "a refusal wrote Settings.xml");

    // shared item sets, through PoB's Manage Item Sets window
    let set = s.backend.items_view().unwrap().active_set;
    edit(
        &mut s,
        Command::AddSharedItemSet { id: set },
        &format!("act.shared_set_add({set})"),
        "set shared",
    );
    edit(
        &mut s,
        Command::RenameSharedItemSet {
            index: 1,
            title: "Renamed".into(),
        },
        "act.shared_set_rename(1, 'Renamed')",
        "set renamed",
    );
    let error = s
        .run(Command::RenameSharedItemSet {
            index: 0,
            title: "  ".into(),
        })
        .expect_err("a blank title");
    assert!(
        error.to_string().contains("that name isn't allowed"),
        "{error}"
    );
    edit(
        &mut s,
        Command::MoveSharedItemSet { from: 0, to: 1 },
        "act.shared_set_move(0, 1)",
        "set moved",
    );
    let saved = s.settings_bytes();
    // the seeded one-slot set compares exactly
    let affected = s.run(Command::ImportSharedItemSet { index: 1 }).unwrap();
    assert!(affected.items && affected.build);
    act("act.shared_set_import(1)");
    let imported = compare(&mut s, "set imported");
    assert_eq!(imported["sets"].as_array().unwrap().len(), 2);
    assert_eq!(s.settings_bytes(), saved, "an import wrote Settings.xml");
    edit(
        &mut s,
        Command::DeleteSharedItemSet { index: 1 },
        "act.shared_set_delete(1)",
        "set deleted",
    );
    // the many-slot set, last: PoB adds its items in `pairs` order, so the new items, their ids
    // and the slot they fill are compared order-free (M-1)
    let saved = s.settings_bytes();
    let before_ids = imported["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_u64().unwrap())
        .max()
        .unwrap();
    let affected = s.run(Command::ImportSharedItemSet { index: 0 }).unwrap();
    assert!(affected.items && affected.build);
    act("act.shared_set_import(0)");
    let ours = json(&s.backend.items_view().unwrap());
    let theirs = upstream_json(&lua);
    assert!(
        ours["items"].as_array().unwrap().len() > imported["items"].as_array().unwrap().len() + 1,
        "the many-slot set added too few items"
    );
    assert_same(
        &canonical_import(&ours, before_ids),
        &canonical_import(&theirs, before_ids),
        "many-slot set imported",
    );
    let slots_ours: serde_json::Value = lua_value(
        &s.backend.lua,
        &LAST_SET_SLOTS.replace("TAB", "pob.build.itemsTab"),
    );
    let slots_theirs: serde_json::Value =
        lua_value(&lua, &LAST_SET_SLOTS.replace("TAB", "build.itemsTab"));
    assert_eq!(slots_ours, slots_theirs, "the imported set's slots");
    assert!(
        slots_ours["slots"].as_object().unwrap().len() > 1,
        "{slots_ours}"
    );
    assert_eq!(slots_ours["title"], "Renamed");
    assert_eq!(s.settings_bytes(), saved, "an import wrote Settings.xml");
    assert_eq!(s.open_popups(), 0);
    // no build file, cache or other file was written (M-7)
    assert_eq!(other_files(&s.user), files, "a file was written");
}

/// Restores a file's permissions however the test ends
struct Writable(std::path::PathBuf);

impl Drop for Writable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644));
    }
}

/// A Settings.xml shared edits can't save (I-1), and one another program changed (I-2)
#[test]
#[ignore = "needs the pinned PoB install"]
fn shared_items_save_failures() {
    use super::longtail_harness::{LongtailSession, settings_folder};
    use auto_pob::engine::{BuildSort, SharedItemSource};
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock();
    let mut s = LongtailSession::start("shared-failures", &|_| shared_seed(&list_mode()));
    // our own writer refreshes what we last wrote: a shared edit after it is saved
    s.run(Command::SetBuildSort {
        sort: BuildSort::Level,
    })
    .unwrap();
    let xml = fixtures_dir().join("attack.xml");
    let dest = settings_folder(&s.user).join("Builds/attack.xml");
    std::fs::copy(&xml, &dest).unwrap();
    s.run(Command::OpenBuild { path: dest }).unwrap();
    let id = s.backend.items_view().unwrap().items[0].id;
    let add = Command::AddSharedItem {
        source: SharedItemSource::Item { id },
        index: 0,
    };
    let sorted = s.settings_bytes();
    s.run(add.clone())
        .expect("a shared edit after our own write");
    assert_ne!(s.settings_bytes(), sorted, "nothing was saved");

    // a write PoB can't make fails the same command, and the edit is undone
    let path = s.settings_path();
    let _restore = Writable(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let before = (s.settings_bytes(), s.backend.items_view().unwrap());
    let error = s.run(add.clone()).expect_err("an unwritable Settings.xml");
    assert!(
        error.to_string().contains("Settings.xml"),
        "{error}: not the save's reason"
    );
    assert_eq!(
        (s.settings_bytes(), s.backend.items_view().unwrap()),
        before,
        "a failed save changed something"
    );
    let prompt: bool = s.eval("return pob.main.promptMsg ~= nil");
    assert!(!prompt, "the save's prompt was left for a later command");
    assert_eq!(s.open_popups(), 0);
    // the next, unrelated command is clean
    s.run(Command::SortItems).expect("the next command");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    // another program wrote the file: the edit is refused and the file left as it wrote it
    let mut theirs = String::from_utf8(s.settings_bytes()).unwrap();
    theirs = theirs.replace("buildSortMode=\"LEVEL\"", "buildSortMode=\"CLASS\"");
    assert!(theirs.contains("CLASS"), "{theirs}");
    std::fs::write(&path, &theirs).unwrap();
    let view = s.backend.items_view().unwrap();
    let error = s.run(add).expect_err("an outside write");
    assert_eq!(
        error.to_string(),
        "Settings.xml was changed by another program; restart auto-pob to edit shared items"
    );
    assert_eq!(
        s.settings_bytes(),
        theirs.as_bytes(),
        "Settings.xml was written"
    );
    assert_eq!(
        s.backend.items_view().unwrap(),
        view,
        "the refusal changed something"
    );
    assert_eq!(s.open_popups(), 0);
}

/// The writers the user starts besides the shared edits (engine I-1): the build list's sort and
/// Options' Save refuse a Settings.xml another program wrote, as a shared edit does, and a token
/// save, which PoB makes unasked and which can't be refused, leaves that change on record so
/// the user's writers stay refused. A sort PoB couldn't save is undone and reported.
#[test]
#[ignore = "needs the pinned PoB install"]
fn settings_writers_refuse_outside_changes() {
    use super::longtail_harness::{LongtailSession, settings_folder};
    use auto_pob::engine::{BuildSort, SharedItemSource};
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock();
    let mut s = LongtailSession::start("writers-outside", &|_| shared_seed(&list_mode()));
    let path = s.settings_path();
    let sort = |s: &mut LongtailSession| s.backend.build_list_view().unwrap().sort;
    assert_eq!(sort(&mut s), BuildSort::Name);

    // a sort PoB can't save is undone, with its reason
    {
        let _restore = Writable(path.clone());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        let before = s.settings_bytes();
        let error = s
            .run(Command::SetBuildSort {
                sort: BuildSort::Level,
            })
            .expect_err("an unwritable Settings.xml");
        assert!(
            error.to_string().contains("Settings.xml"),
            "{error}: not the save's reason"
        );
        assert_eq!(sort(&mut s), BuildSort::Name, "the unsaved sort stayed");
        assert!(s.eval::<bool>("return pob.main.main.buildSortMode == 'NAME'"));
        assert_eq!(s.settings_bytes(), before);
        let prompt: bool = s.eval("return pob.main.promptMsg ~= nil");
        assert!(!prompt, "the save's prompt was left for a later command");
    }

    // classic PoB writes the file: the sort and Options' Save are refused, and its bytes stay
    let theirs = String::from_utf8(s.settings_bytes()).unwrap().replace(
        "showThousandsSeparators=\"false\"",
        "showThousandsSeparators=\"true\"",
    );
    assert!(
        theirs.contains("showThousandsSeparators=\"true\""),
        "{theirs}"
    );
    std::fs::write(&path, &theirs).unwrap();
    let error = s
        .run(Command::SetBuildSort {
            sort: BuildSort::Level,
        })
        .expect_err("an outside write");
    assert_eq!(
        error.to_string(),
        "Settings.xml was changed by another program; restart auto-pob to change the build sort"
    );
    assert_eq!(
        sort(&mut s),
        BuildSort::Name,
        "the refused sort was applied"
    );
    assert_eq!(
        s.settings_bytes(),
        theirs.as_bytes(),
        "Settings.xml was written"
    );

    s.run(Command::OpenPopup(PopupKind::Options)).unwrap();
    s.run(Command::SetPopupControl {
        name: "showAnimations".into(),
        value: PopupInput::Check(false),
    })
    .unwrap();
    let edited = s.popup().expect("the Options window");
    let error = s
        .run(Command::PressPopupButton {
            name: "save".into(),
        })
        .expect_err("an outside write");
    assert_eq!(
        error.to_string(),
        "Settings.xml was changed by another program; restart auto-pob to save Options"
    );
    assert_eq!(
        s.settings_bytes(),
        theirs.as_bytes(),
        "Settings.xml was written"
    );
    assert_eq!(
        s.popup(),
        Some(edited),
        "the window and its edits stay for the user"
    );
    assert_eq!(s.open_popups(), 0);
    s.run(Command::ClosePopup).unwrap();

    // a token save PoB makes unasked still writes, but doesn't take the file for ours
    s.eval::<()>("pob.main.main.lastToken = 'token'; pob.main.main:SaveSettings()");
    assert_ne!(
        s.settings_bytes(),
        theirs.as_bytes(),
        "the token save wrote"
    );
    let error = s
        .run(Command::SetBuildSort {
            sort: BuildSort::Level,
        })
        .expect_err("the outside write is still on record");
    assert!(error.to_string().contains("another program"), "{error}");
    drop(s);

    // PoB couldn't read the file at start: its saves would be silent no-ops, so ours refuse
    let mut s = LongtailSession::start_unreadable("writers-unread");
    assert!(s.eval::<bool>("return pob.main.main.errorReadingSettings == true"));
    let xml = fixtures_dir().join("attack.xml");
    let dest = settings_folder(&s.user).join("Builds/attack.xml");
    std::fs::copy(&xml, &dest).unwrap();
    let unread = snapshot_dir(&s.user);
    let refused = "Settings.xml couldn't be read when auto-pob started; restart auto-pob to ";
    let error = s
        .run(Command::SetBuildSort {
            sort: BuildSort::Level,
        })
        .expect_err("an unread Settings.xml");
    assert_eq!(error.to_string(), format!("{refused}change the build sort"));
    s.run(Command::OpenPopup(PopupKind::Options)).unwrap();
    let error = s
        .run(Command::PressPopupButton {
            name: "save".into(),
        })
        .expect_err("an unread Settings.xml");
    assert_eq!(error.to_string(), format!("{refused}save Options"));
    s.run(Command::ClosePopup).unwrap();
    s.run(Command::OpenBuild { path: dest }).unwrap();
    let id = s.backend.items_view().unwrap().items[0].id;
    let view = s.backend.items_view().unwrap();
    let error = s
        .run(Command::AddSharedItem {
            source: SharedItemSource::Item { id },
            index: 0,
        })
        .expect_err("an unread Settings.xml");
    assert_eq!(error.to_string(), format!("{refused}edit shared items"));
    assert_eq!(
        s.backend.items_view().unwrap(),
        view,
        "the refusal changed something"
    );
    assert_eq!(snapshot_dir(&s.user), unread, "something was written");
}

/// An Animate Guardian build: its minion uses an item set, which the sidebar's minion dropdown
/// lists (Build.lua:1610-1620)
fn guardian_xml() -> String {
    let xml = std::fs::read_to_string(fixtures_dir().join("attack.xml")).unwrap();
    let guardian = xml.replacen(
        "skillId=\"Boneshatter\" skillPartCalcs=\"1\" skillPart=\"1\" enableGlobal1=\"true\" enableGlobal2=\"true\" nameSpec=\"Boneshatter\" gemId=\"Metadata/Items/Gems/SkillGemBoneshatter\" enabled=\"true\" variantId=\"Boneshatter\"",
        "skillId=\"AnimateGuardian\" enableGlobal1=\"true\" enableGlobal2=\"true\" nameSpec=\"Animate Guardian\" gemId=\"Metadata/Items/Gems/SkillGemAnimateGuardian\" enabled=\"true\" variantId=\"AnimateGuardian\"",
        1,
    );
    assert_ne!(guardian, xml, "the main skill wasn't replaced");
    guardian
}

/// Items dragged onto the sidebar's minion dropdown from the item list, a database and the
/// shared list (Build.lua:534-543, ItemsTab.lua:1182-1191; M-10)
#[test]
#[ignore = "needs the pinned PoB install"]
fn minion_drop_matches_pob() {
    use super::longtail_harness::{LongtailSession, settings_folder};
    use auto_pob::engine::ItemDragSource;
    let _guard = lock();
    let mut s = LongtailSession::start("minion-drop", &|_| shared_seed(&list_mode()));
    let xml = guardian_xml();
    let dest = settings_folder(&s.user).join("Builds/guardian.xml");
    std::fs::write(&dest, &xml).unwrap();
    s.run(Command::OpenBuild { path: dest }).unwrap();
    let lua = shared_upstream(&xml, "guardian");
    let files = snapshot_dir(&s.user);
    let compare = |s: &mut LongtailSession, what: &str| {
        let ours = json(&s.backend.items_view().unwrap());
        assert_same(&ours, &upstream_json(&lua), what);
        assert_eq!(s.open_popups(), 0, "{what}: a popup was left open");
        ours
    };
    compare(&mut s, "opened");
    let helmet = db_search(&mut s, &lua, "Goldrim");
    // the build's weapon: the guardian uses Weapon 1
    let weapon = compare(&mut s, "searched")["items"][0]["id"]
        .as_u64()
        .unwrap() as u32;
    let drops = vec![
        (
            ItemDragSource::Item { id: weapon },
            format!("act.minion_drop('item', {weapon})"),
        ),
        (
            ItemDragSource::Db {
                db: ItemDb::Uniques,
                name: helmet.clone(),
            },
            format!("act.minion_drop('db', 'Uniques', {helmet:?})"),
        ),
        // Kaom's Heart, a body armour
        (
            ItemDragSource::Shared { index: 1 },
            "act.minion_drop('shared', 1)".to_string(),
        ),
    ];
    for (source, code) in drops {
        let affected = s
            .run(Command::DropOnMinion {
                source: source.clone(),
            })
            .unwrap_or_else(|e| panic!("{source:?}: {e}"));
        assert!(affected.items && affected.build && affected.recalc);
        lua.load(&code)
            .exec()
            .unwrap_or_else(|e| panic!("{code}: {e}"));
        compare(&mut s, &format!("{source:?} dropped on the minion"));
    }
    // the guardian doesn't use rings: the dropdown refuses it
    let view = s.backend.items_view().unwrap();
    let error = s
        .run(Command::DropOnMinion {
            source: ItemDragSource::Shared { index: 0 },
        })
        .expect_err("a ring");
    assert!(
        error.to_string().contains("the minion can't use that item"),
        "{error}"
    );
    assert_eq!(
        s.backend.items_view().unwrap(),
        view,
        "the refusal changed something"
    );
    assert_eq!(s.open_popups(), 0);
    assert_eq!(snapshot_dir(&s.user), files, "a file was written");
}
