//! Gate tests for the Items tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{
    Session, fixtures, fixtures_dir, load_tree_lib, load_xml, lock, lua_value, pob_dir,
    recipe_setting, upstream_lua,
};
use auto_pob::engine::{Backend, ItemsView, SlotKind};
use mlua::LuaSerdeExt;
use std::fs;

/// Upstream PoB with a build loaded and `tree_lib.lua` and `items_lib.lua` defined.
fn upstream_with_items_lib(xml: &std::path::Path, name: &str) -> mlua::Lua {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, &fs::read_to_string(xml).unwrap(), name).unwrap();
    load_tree_lib(&lua);
    lua.load(fs::read_to_string(fixtures_dir().join("items_lib.lua")).unwrap())
        .set_name("=items_lib")
        .exec()
        .expect("items_lib.lua");
    lua
}

/// What upstream's Items tab shows now (`itemsDump()`).
fn upstream_dump(lua: &mlua::Lua) -> ItemsView {
    let value: mlua::Value = lua.load("return itemsDump()").eval().unwrap();
    lua.from_value(value).unwrap()
}

/// The build `items/<name>.lua` prepares, saved as XML. The script runs in upstream PoB on its
/// base fixture and sets `setup_xml`.
fn setup_xml(name: &str) -> String {
    let source = fs::read_to_string(fixtures_dir().join(format!("items/{name}.lua"))).unwrap();
    let base = recipe_setting(&source, "base").unwrap_or_else(|| panic!("{name}: no base"));
    let lua = upstream_with_items_lib(&fixtures_dir().join(format!("{base}.xml")), &base);
    lua.load(source)
        .set_name(format!("={name}"))
        .exec()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    lua.load("recipe()")
        .exec()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    lua.globals()
        .get::<Option<String>>("setup_xml")
        .unwrap()
        .unwrap_or_else(|| panic!("{name}: no setup_xml"))
}

/// Our session with the `gear` setup open, and upstream PoB with the same build.
fn gear_sessions(test: &str) -> (Session, mlua::Lua) {
    let xml = setup_xml("gear");
    let mut s = Session::start(test);
    let path = s.user.join("gear.xml");
    fs::create_dir_all(&s.user).unwrap();
    fs::write(&path, &xml).unwrap();
    s.open_fixture("gear", &path);
    let lua = upstream_with_items_lib(&path, "gear");
    (s, lua)
}

/// Runs `code` on both sides; `BUILD` stands for the build, `pob.build` here and `build` there.
fn on_both(s: &Session, lua: &mlua::Lua, code: &str) {
    s.backend
        .lua
        .load(code.replace("BUILD", "pob.build") + "\nglue.frame()")
        .exec()
        .unwrap();
    lua.load(code.replace("BUILD", "build") + "\nrunCallback('OnFrame')")
        .exec()
        .unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn items_view_matches_pob() {
    let _guard = lock();
    // the attack fixture, as it comes
    let (_, xml) = fixtures()
        .into_iter()
        .find(|(name, _)| name == "attack")
        .expect("attack fixture");
    let mut s = Session::start("items-attack");
    s.open_fixture("attack", &xml);
    let lua = upstream_with_items_lib(&xml, "attack");
    let ours = s.backend.items_view().unwrap();
    assert!(!ours.items.is_empty(), "attack: no items");
    assert_eq!(ours, upstream_dump(&lua), "attack");
    drop(s);

    // the gear setup, with the second set active and the "Unused Items" filter on, so that the
    // list, the swap and abyssal slots, the flask and the jewel socket are all in states other
    // than the default ones
    let (mut s, lua) = gear_sessions("items-gear");
    let before = s.backend.items_view().unwrap();
    assert_eq!(
        before,
        upstream_dump(&lua),
        "gear, before the set and the filter change"
    );
    on_both(
        &s,
        &lua,
        "local t = BUILD.itemsTab\n\
         t:SetActiveItemSet(2)\n\
         t.controls.itemList.controls.loadoutFilter:SetSel(3)",
    );
    let ours = s.backend.items_view().unwrap();
    assert_eq!(ours, upstream_dump(&lua), "gear, set 2 and the filter");

    assert!(ours.second_weapon_set);
    assert!(!ours.can_reorder);
    assert!(ours.listed.len() < ours.items.len() && !ours.listed.is_empty());
    assert_eq!(ours.list_filter.selected, 2);
    let slot = |name: &str| {
        ours.slots
            .iter()
            .find(|slot| slot.name == name)
            .unwrap_or_else(|| panic!("no slot {name}"))
    };
    let swap = slot("Weapon 1 Swap");
    assert!(swap.shown && swap.kind == SlotKind::Swap && swap.selected != 0);
    assert!(!slot("Weapon 1").shown);
    let abyssal = slot("Belt Abyssal Socket 1");
    assert!(abyssal.shown && abyssal.kind == SlotKind::Abyssal && abyssal.selected != 0);
    assert_eq!(abyssal.parent.as_deref(), Some("Belt"));
    assert_eq!(slot("Flask 1").flask_active, Some(true));
    assert_eq!(slot("Flask 1").kind, SlotKind::Flask);
    let socket = ours
        .slots
        .iter()
        .find(|slot| slot.kind == SlotKind::Jewel && slot.shown)
        .expect("a shown jewel socket");
    assert_eq!(socket.label, "Socket #1");
    assert!(socket.selected != 0 && socket.node.is_some());
    assert!(ours.items.iter().any(|item| item.equipped.is_some()));
}

/// The values a pure view must leave alone, read from PoB.
fn pure_state(s: &Session) -> serde_json::Value {
    lua_value(
        &s.backend.lua,
        r#"
        local t = pob.build.itemsTab
        local slots, jewels = {}, {}
        for name, slot in pairs(t.slots) do slots[#slots + 1] = name .. "=" .. slot.selItemId end
        table.sort(slots)
        for node, id in pairs(pob.build.spec.jewels) do jewels[#jewels + 1] = node .. "=" .. id end
        table.sort(jewels)
        return {
            slots = table.concat(slots, ","),
            jewels = table.concat(jewels, ","),
            undo = #t.undo,
            redo = #t.redo,
            popups = #pob.main.main.popups,
        }
        "#,
    )
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn items_view_is_pure() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("items-pure");
    // a selection PopulateSlots would reset: the body armour in Ring 2
    s.backend
        .lua
        .load(
            r#"
            local t = pob.build.itemsTab
            for id, item in pairs(t.items) do
                if item.name:find("Kaom") then t.slots["Ring 2"].selItemId = id end
            end
            "#,
        )
        .exec()
        .unwrap();
    let unsaved = s.backend.build_view().unwrap().unsaved;
    let before = pure_state(&s);
    assert!(
        !before["slots"].as_str().unwrap().contains("Ring 2=0,"),
        "{before}"
    );
    assert!(!before["jewels"].as_str().unwrap().is_empty(), "{before}");
    let first = s.backend.items_view().unwrap();
    let second = s.backend.items_view().unwrap();
    assert_eq!(first, second);
    assert_eq!(pure_state(&s), before);
    assert_eq!(s.backend.build_view().unwrap().unsaved, unsaved);
    assert_eq!(before["popups"], 0);

    // what a view costs, for the task report
    let mut times: Vec<std::time::Duration> = (0..20)
        .map(|_| {
            let start = std::time::Instant::now();
            s.backend.items_view().unwrap();
            start.elapsed()
        })
        .collect();
    times.sort();
    let json = serde_json::to_string(&s.backend.items_view().unwrap()).unwrap();
    eprintln!("items_view median {:?}, {} bytes", times[10], json.len());
    assert!(times[10] < std::time::Duration::from_millis(100));
}
