//! Gate tests for the Items tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{
    Session, fixtures, fixtures_dir, load_tree_lib, load_xml, lock, lua_value, plain_notes,
    pob_dir, recipe_setting, upstream_lua,
};
use auto_pob::engine::upstream::{SidebarLine, golden_of};
use auto_pob::engine::{
    Backend, Command, ItemTooltipLine, ItemTooltipQuery, ItemsView, SlotKind, Tab,
};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;

/// Upstream PoB with a build loaded and `tree_lib.lua` and `items_lib.lua` defined.
fn upstream_with_items_lib(xml: &std::path::Path, name: &str) -> mlua::Lua {
    upstream_with_items_lib_xml(&fs::read_to_string(xml).unwrap(), name)
}

/// `upstream_with_items_lib` for a build given as XML text.
fn upstream_with_items_lib_xml(xml: &str, name: &str) -> mlua::Lua {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, xml, name).unwrap();
    load_tree_lib(&lua);
    lua.load(fs::read_to_string(fixtures_dir().join("items_lib.lua")).unwrap())
        .set_name("=items_lib")
        .exec()
        .expect("items_lib.lua");
    lua
}

/// What upstream's Items tab shows now (`itemsDump()`), as JSON with the nulls left out:
/// a Lua table has no nil fields.
fn upstream_json(lua: &mlua::Lua) -> serde_json::Value {
    let value: mlua::Value = lua.load("return itemsDump()").eval().unwrap();
    strip_nulls(lua.from_value(value).unwrap())
}

/// Our view as JSON, without the `None` fields (they are nulls here and absent upstream).
fn json(view: &ItemsView) -> serde_json::Value {
    strip_nulls(serde_json::to_value(view).unwrap())
}

/// The path of the first difference between two JSON values.
fn first_difference(path: &str, a: &serde_json::Value, b: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<_> = x.keys().chain(y.keys()).collect();
            keys.into_iter().find_map(|k| match (x.get(k), y.get(k)) {
                (Some(p), Some(q)) => first_difference(&format!("{path}.{k}"), p, q),
                (p, q) => Some(format!("{path}.{k}: {p:?} vs {q:?}")),
            })
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (p, q))| first_difference(&format!("{path}[{i}]"), p, q)),
        _ if a == b => None,
        _ => Some(format!("{path}: {a} vs {b}")),
    }
}

/// Asserts equality, naming the first difference.
fn assert_same(ours: &serde_json::Value, theirs: &serde_json::Value, what: &str) {
    if let Some(difference) = first_difference("", ours, theirs) {
        panic!("{what}: ours vs upstream at {difference}");
    }
}

fn strip_nulls(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        // an empty Lua table is an object here, whatever it holds
        Value::Object(map) if map.is_empty() => Value::Array(Vec::new()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        other => other,
    }
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
    assert_same(&json(&ours), &upstream_json(&lua), "attack");
    drop(s);

    // the gear setup, with the second set active and the "Unused Items" filter on, so that the
    // list, the swap and abyssal slots, the flask and the jewel socket are all in states other
    // than the default ones
    let (mut s, lua) = gear_sessions("items-gear");
    let before = s.backend.items_view().unwrap();
    assert_same(
        &json(&before),
        &upstream_json(&lua),
        "gear, before the set and the filter change",
    );
    // the finders' fields in a state where each is set (lesson 8)
    let item = |view: &ItemsView, name: &str| -> auto_pob::engine::ItemEntry {
        view.items
            .iter()
            .find(|item| item.name.starts_with(name))
            .unwrap_or_else(|| panic!("no item {name}"))
            .clone()
    };
    let paua = item(&before, "Gale Band");
    let equipped = paua.equipped.expect("Gale Band is equipped in set 2");
    assert_eq!(
        (equipped.slot.as_str(), equipped.set.as_deref()),
        ("Ring 2", Some("Swap"))
    );
    assert!(paua.display_label.contains("(Used in 'Swap')"));
    assert_eq!(item(&before, "Dusk Eye").abyss_set.as_deref(), Some("Swap"));
    assert_eq!(
        item(&before, "Eldritch Knowledge").tree.as_deref(),
        Some("Other tree")
    );
    assert!(
        item(&before, "Eldritch Knowledge")
            .display_label
            .contains("(Used in 'Other tree')")
    );
    on_both(
        &s,
        &lua,
        "local t = BUILD.itemsTab\n\
         t:SetActiveItemSet(2)\n\
         t.controls.itemList.controls.loadoutFilter:SetSel(3)",
    );
    let ours = s.backend.items_view().unwrap();
    assert_eq!(
        json(&ours),
        upstream_json(&lua),
        "gear, set 2 and the filter"
    );
    // with set 2 active, set 1's belt jewel is the one another set holds
    assert_eq!(
        item(&ours, "Gloom Eye").abyss_set.as_deref(),
        Some("Default")
    );

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

// ---- Tooltips and copy ----

/// Items on top of the `gear` build, added at model level on both sides with the same code: a
/// multi-variant unique and a unique flask from PoB's data (as the Items tab's database adds
/// them), a rare weapon, a rare jewel and a rare abyss jewel. Nothing is equipped, so each is
/// compared against what is in the slots it could go in.
const EXTRA_ITEMS: &str = r#"
    local t = BUILD.itemsTab
    local function unique(name)
        for _, list in pairs(data.uniques) do
            for _, text in ipairs(list) do
                if text:match("^" .. name .. "\n") then return "Rarity: UNIQUE\n" .. text end
            end
        end
        error("no unique " .. name)
    end
    local texts = {
        unique("The Anvil"),
        unique("Blood of the Karui"),
        "Rarity: RARE\nGrim Bow\nCrude Bow\nImplicits: 0\nAdds 3 to 9 Physical Damage\n+22 to maximum Life",
        "Rarity: RARE\nDoom Ward\nCobalt Jewel\nImplicits: 0\n+7 to Intelligence\n+9 to maximum Life",
        "Rarity: RARE\nSoul Eye\nGhastly Eye Jewel\nImplicits: 0\n+9 to maximum Life",
        -- two influences, one influence, synthesised, foil, and Memory Strands (PoB gives
        -- its line a background image, ItemsTab.lua:4610)
        "Rarity: RARE\nDusk Loop\nIron Ring\nShaper Item\nElder Item\nImplicits: 0\n+31 to maximum Life",
        "Rarity: RARE\nHunter Loop\nIron Ring\nHunter Item\nImplicits: 0\n+32 to maximum Life",
        "Rarity: RARE\nSynth Loop\nIron Ring\nSynthesised Item\nImplicits: 0\n+33 to maximum Life",
        "Rarity: UNIQUE\nFoil Heart\nGlorious Plate\nFoil Unique (Gold)\nImplicits: 0\n+101 to maximum Life",
        "Rarity: RARE\nMemory Loop\nIron Ring\nMemory Strands: 40\nImplicits: 0\n+34 to maximum Life",
    }
    for _, text in ipairs(texts) do
        local item = new("Item", text)
        t:AddItem(item, true)
    end
    t:PopulateSlots()
"#;

fn json_of<T: serde::Serialize>(value: &T) -> serde_json::Value {
    strip_nulls(serde_json::to_value(value).unwrap())
}

/// Upstream's answer from `listTooltip` or `slotTooltip`; `None` for a nil.
fn upstream_tooltip(lua: &mlua::Lua, code: &str) -> Option<serde_json::Value> {
    let value: mlua::Value = lua.load(code).eval().unwrap();
    match value {
        mlua::Value::Nil => None,
        value => Some(strip_nulls(lua.from_value(value).unwrap())),
    }
}

/// What the compared tooltips held.
#[derive(Default)]
struct Seen {
    compared: usize,
    /// with an "Equipping this item in" line
    equipping: usize,
    /// with at least one influence
    influences: usize,
    foil: usize,
    /// with a line that has a background
    background: usize,
}

/// Compares every item's list tooltip and the tooltip of every slot that lists it.
fn compare_tooltips(s: &mut Session, lua: &mlua::Lua, what: &str) -> Seen {
    let view = s.backend.items_view().unwrap();
    let mut seen = Seen::default();
    for item in &view.items {
        let mut queries = vec![(None, format!("listTooltip({})", item.id))];
        for slot in view
            .slots
            .iter()
            .filter(|slot| slot.choices.contains(&item.id))
        {
            queries.push((
                Some(slot.name.clone()),
                format!("slotTooltip({:?}, {})", slot.name, item.id),
            ));
        }
        for (slot, code) in queries {
            let query = ItemTooltipQuery { id: item.id, slot };
            let ours = s.backend.item_tooltip_view(&query).unwrap();
            let theirs = upstream_tooltip(lua, &format!("return {code}"));
            let label = format!("{what}: {} {:?}", item.name, query.slot);
            match (ours, theirs) {
                (Some(ours), Some(theirs)) => {
                    assert_eq!(ours.query, query);
                    assert_same(&json_of(&ours.tooltip), &theirs, &label);
                    let lines = &ours.tooltip.lines;
                    assert!(
                        !lines.windows(2).any(|pair| matches!(
                            pair,
                            [
                                ItemTooltipLine::Separator { .. },
                                ItemTooltipLine::Separator { .. }
                            ]
                        )),
                        "{label}: two separators in a row"
                    );
                    let has_equipping = lines.iter().any(|line| {
                        matches!(line, ItemTooltipLine::Line { text, .. }
                            if text.contains("Equipping this item in"))
                    });
                    seen.equipping += usize::from(has_equipping);
                    seen.influences += usize::from(!ours.tooltip.influences.is_empty());
                    seen.foil += usize::from(ours.tooltip.foil.is_some());
                    seen.background += usize::from(lines.iter().any(|line| {
                        matches!(
                            line,
                            ItemTooltipLine::Line {
                                background: Some(_),
                                ..
                            }
                        )
                    }));
                    seen.compared += 1;
                }
                (None, None) => {}
                (ours, theirs) => panic!("{label}: ours {ours:?}, upstream {theirs:?}"),
            }
        }
    }
    seen
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_tooltips_match_pob() {
    let _guard = lock();
    let (mut s, lua) = gear_sessions("items-tooltips");
    on_both(&s, &lua, EXTRA_ITEMS);
    let view = s.backend.items_view().unwrap();
    for name in [
        "The Anvil",
        "Blood of the Karui",
        "Grim Bow",
        "Doom Ward",
        "Soul Eye",
    ] {
        assert!(
            view.items.iter().any(|item| item.name.starts_with(name)),
            "no {name}"
        );
    }

    let started = std::time::Instant::now();
    let on = compare_tooltips(&mut s, &lua, "stat differences on");
    eprintln!(
        "{} tooltips compared in {:?}",
        on.compared,
        started.elapsed()
    );
    assert!(on.compared > 40, "only {} tooltips", on.compared);
    let equipping = on.equipping;
    assert!(equipping > 0, "no tooltip compares with the equipped item");
    // the header fields and line backgrounds are compared in non-default states too
    assert!(on.influences > 0, "no tooltip has an influence");
    assert!(on.foil > 0, "no tooltip has a foil type");
    assert!(on.background > 0, "no tooltip has a line background");

    // PoB's Ctrl+D body: the toggle only
    on_both(&s, &lua, "BUILD.itemsTab.showStatDifferences = false");
    let off = compare_tooltips(&mut s, &lua, "stat differences off");
    let equipping_off = off.equipping;
    assert!(off.compared > 40);
    assert!(
        equipping_off < equipping,
        "the comparison should go with the toggle"
    );

    // a vanished item, slot or entry sends nothing; so does an item the filter hides
    let ask = |s: &mut Session, id: u32, slot: Option<&str>| {
        s.backend
            .item_tooltip_view(&ItemTooltipQuery {
                id,
                slot: slot.map(str::to_string),
            })
            .unwrap()
    };
    assert!(ask(&mut s, 9999, None).is_none());
    assert!(ask(&mut s, view.items[0].id, Some("No Such Slot")).is_none());
    // a body armour is not in Ring 1's list
    let kaom = view
        .items
        .iter()
        .find(|i| i.name.starts_with("Kaom"))
        .unwrap()
        .id;
    assert!(ask(&mut s, kaom, Some("Ring 1")).is_none());
    assert!(ask(&mut s, kaom, Some("Body Armour")).is_some());

    // an item that exists but is filtered out of the item list has no list tooltip (R3)
    on_both(
        &s,
        &lua,
        "BUILD.itemsTab.controls.itemList.controls.loadoutFilter:SetSel(3)",
    );
    let filtered = s.backend.items_view().unwrap();
    let hidden = filtered
        .items
        .iter()
        .find(|item| !filtered.listed.contains(&item.id))
        .expect("the filter hides an item");
    assert!(ask(&mut s, hidden.id, None).is_none());
    assert!(upstream_tooltip(&lua, &format!("return listTooltip({})", hidden.id)).is_none());
    // while a listed one still has it, and the same item still has its slot tooltip
    assert!(ask(&mut s, filtered.listed[0], None).is_some());
    let slot = filtered
        .slots
        .iter()
        .find(|slot| slot.choices.contains(&hidden.id));
    if let Some(slot) = slot {
        assert!(ask(&mut s, hidden.id, Some(&slot.name)).is_some());
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn tooltip_of_the_unique_jewel() {
    // the measurement of Task 2: one comparison tooltip for the unique jewel
    let _guard = lock();
    let (mut s, lua) = gear_sessions("items-jewel-tooltip");
    let view = s.backend.items_view().unwrap();
    let jewel = view
        .items
        .iter()
        .find(|i| i.name.starts_with("Eldritch"))
        .unwrap();
    let sockets: Vec<_> = view
        .slots
        .iter()
        .filter(|slot| slot.kind == SlotKind::Jewel && slot.choices.contains(&jewel.id))
        .collect();
    assert!(!sockets.is_empty());
    let (mut times, mut compared) = (Vec::new(), 0);
    for slot in sockets {
        let query = ItemTooltipQuery {
            id: jewel.id,
            slot: Some(slot.name.clone()),
        };
        let start = std::time::Instant::now();
        let ours = s
            .backend
            .item_tooltip_view(&query)
            .unwrap()
            .expect("tooltip");
        times.push(start.elapsed());
        let theirs = upstream_tooltip(
            &lua,
            &format!("return slotTooltip({:?}, {})", slot.name, jewel.id),
        );
        assert_same(&json_of(&ours.tooltip), &theirs.unwrap(), &slot.name);
        let text: Vec<String> = ours
            .tooltip
            .lines
            .iter()
            .map(|line| match line {
                ItemTooltipLine::Line { text, .. } => text.clone(),
                ItemTooltipLine::Separator { .. } => "----".into(),
            })
            .collect();
        if !text
            .iter()
            .any(|line| line.contains("Equipping this item in"))
        {
            continue;
        }
        compared += 1;
        let bytes = serde_json::to_string(&ours).unwrap().len();
        eprintln!(
            "{} ({} lines, {bytes} bytes, {:?}):\n{}",
            slot.name,
            text.len(),
            times.last().unwrap(),
            text.join("\n")
        );
    }
    assert_eq!(compared, 1, "exactly the allocated socket compares");
    eprintln!(
        "{} sockets queried, {:?} each at most",
        times.len(),
        times.iter().max().unwrap()
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn copy_item_matches_pob() {
    let _guard = lock();
    let (mut s, lua) = gear_sessions("items-copy");
    on_both(&s, &lua, EXTRA_ITEMS);
    s.backend.lua.load("saved_copy = Copy").exec().unwrap();
    let copy_is_saved = |s: &Session| -> bool {
        s.backend
            .lua
            .load("return rawequal(Copy, saved_copy)")
            .eval()
            .unwrap()
    };
    let view = s.backend.items_view().unwrap();
    assert!(view.items.len() > 10);
    let copy_item: mlua::Function = lua.globals().get("copyItem").unwrap();
    for item in &view.items {
        let ours = s.backend.item_text(item.id).unwrap();
        assert_eq!(ours.id, item.id);
        let theirs: Option<String> = copy_item.call(item.id).unwrap();
        let theirs = theirs.unwrap_or_else(|| panic!("upstream copied nothing for {}", item.name));
        assert!(theirs.contains("\r\n"), "{}", item.name);
        assert_eq!(ours.text, theirs, "{}", item.name);
        assert!(copy_is_saved(&s), "Copy not restored after {}", item.name);
    }
    // an unknown id fails before Copy is swapped
    assert!(s.backend.item_text(9999).is_err());
    assert!(copy_is_saved(&s), "Copy not restored after an unknown id");
    // an item whose copy fails after Copy is swapped (BuildRaw raises): Copy is put back and
    // the error comes out
    s.backend
        .lua
        .load(format!(
            "pob.build.itemsTab.items[{}].BuildRaw = function() error('boom', 0) end",
            view.items[0].id
        ))
        .exec()
        .unwrap();
    let error = s.backend.item_text(view.items[0].id).unwrap_err();
    assert!(error.to_string().contains("boom"), "{error}");
    assert!(copy_is_saved(&s), "Copy not restored after a failing copy");
    // and the item list is untouched by copying
    assert_eq!(s.backend.items_view().unwrap(), view);
}

// ---- The recorded-recipe harness ----

/// One step of a recipe as `items_lib.lua`'s `act.*` recorded it, with 0-based positions.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "cmd")]
enum ItemsAction {
    SortItems,
    DeleteItem {
        id: u32,
    },
    DeleteUnusedItems,
    DeleteAllItems,
    MoveItem {
        from: usize,
        to: usize,
    },
    SetItemListFilter {
        index: usize,
    },
    EquipItem {
        slot: String,
        id: Option<u32>,
    },
    SetFlaskActive {
        slot: String,
        on: bool,
    },
    AddSocketGroup,
    SetGroupSlot {
        group: usize,
        slot: Option<String>,
    },
    SelectWeaponSet {
        second: bool,
    },
    /// Recorded by tree_lib's `act.alloc` and `act.dealloc`
    AllocNode {
        id: u32,
    },
    DeallocNode {
        id: u32,
    },
    SelectItemSet {
        id: u32,
    },
    NewItemSet {
        title: String,
    },
    RenameItemSet {
        id: u32,
        title: String,
    },
    CopyItemSet {
        id: u32,
        title: String,
    },
    DeleteItemSet {
        id: u32,
    },
    MoveItemSet {
        from: usize,
        to: usize,
    },
    Undo,
    Redo,
    /// Built from the recorded checkpoint table
    #[serde(skip_deserializing)]
    Check {
        check: Box<ItemsCheckpoint>,
    },
}

impl ItemsAction {
    /// The command that replays this action; none for a checkpoint.
    fn command(&self) -> Option<Command> {
        Some(match self.clone() {
            ItemsAction::SortItems => Command::SortItems,
            ItemsAction::DeleteItem { id } => Command::DeleteItem { id },
            ItemsAction::DeleteUnusedItems => Command::DeleteUnusedItems,
            ItemsAction::DeleteAllItems => Command::DeleteAllItems,
            ItemsAction::MoveItem { from, to } => Command::MoveItem { from, to },
            ItemsAction::SetItemListFilter { index } => Command::SetItemListFilter { index },
            ItemsAction::EquipItem { slot, id } => Command::EquipItem { slot, id },
            ItemsAction::SetFlaskActive { slot, on } => Command::SetFlaskActive { slot, on },
            ItemsAction::AddSocketGroup => Command::AddSocketGroup,
            ItemsAction::SetGroupSlot { group, slot } => Command::SetGroupSlot { group, slot },
            ItemsAction::SelectWeaponSet { second } => Command::SelectWeaponSet { second },
            ItemsAction::AllocNode { id } => Command::AllocNode { id },
            ItemsAction::DeallocNode { id } => Command::DeallocNode { id },
            ItemsAction::SelectItemSet { id } => Command::SelectItemSet { id },
            ItemsAction::NewItemSet { title } => Command::NewItemSet { title },
            ItemsAction::RenameItemSet { id, title } => Command::RenameItemSet { id, title },
            ItemsAction::CopyItemSet { id, title } => Command::CopyItemSet { id, title },
            ItemsAction::DeleteItemSet { id } => Command::DeleteItemSet { id },
            ItemsAction::MoveItemSet { from, to } => Command::MoveItemSet { from, to },
            ItemsAction::Undo => Command::Undo(Tab::Items),
            ItemsAction::Redo => Command::Redo(Tab::Items),
            ItemsAction::Check { .. } => return None,
        })
    }
}

/// The Items tab at one point of a recipe: `itemsDump()`, the jewels each socket holds and the
/// sidebar.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct ItemsCheckpoint {
    items: serde_json::Value,
    /// The display item; always `None` until the display panel exists (Task 5)
    #[serde(default)]
    display: Option<serde_json::Value>,
    /// `spec.jewels` without the empty sockets
    #[serde(default)]
    jewels: BTreeMap<u32, u32>,
    sidebar: Vec<SidebarLine>,
    /// `mainSocketGroup`, 1-based as PoB has it: a weapon swap moves it (R6)
    main_group: u32,
}

/// A recipe run in upstream PoB: the steps it took and where they ended.
struct ItemsRecipe {
    base: String,
    /// The build to start from when the recipe prepared one, or names a `setup`
    setup_xml: Option<String>,
    actions: Vec<ItemsAction>,
    result: serde_json::Value,
}

/// Runs `items/<name>.lua` in upstream PoB on its base fixture, or on its `setup` build.
fn upstream_items_recipe(name: &str) -> ItemsRecipe {
    let source = fs::read_to_string(fixtures_dir().join(format!("items/{name}.lua"))).unwrap();
    let base = recipe_setting(&source, "base").unwrap_or_else(|| panic!("{name}: no base"));
    let setup = recipe_setting(&source, "setup").map(|setup| setup_xml(&setup));
    let lua = match &setup {
        Some(xml) => upstream_with_items_lib_xml(xml, &base),
        None => upstream_with_items_lib(&fixtures_dir().join(format!("{base}.xml")), &base),
    };
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
                let mut check: ItemsCheckpoint = lua
                    .from_value(mlua::Value::Table(check))
                    .unwrap_or_else(|e| panic!("{name}: checkpoint: {e}"));
                check.items = strip_nulls(check.items);
                return ItemsAction::Check {
                    check: Box::new(check),
                };
            }
            let json: serde_json::Value = lua.from_value(mlua::Value::Table(entry)).unwrap();
            serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{name}: {json}: {e}"))
        })
        .collect();
    ItemsRecipe {
        base,
        setup_xml: setup,
        actions,
        result: upstream_json(&lua),
    }
}

/// Our Items tab and sidebar in the shape of a recipe's `checkpoint()`.
fn our_items_checkpoint(s: &mut Session) -> ItemsCheckpoint {
    let view = s.backend.items_view().expect("items view");
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
    let jewels: BTreeMap<u32, u32> = s
        .backend
        .lua
        .from_value(
            s.backend
                .lua
                .load("return pob.build.spec.jewels")
                .eval()
                .unwrap(),
        )
        .unwrap();
    ItemsCheckpoint {
        items: json(&view),
        display: None,
        jewels: jewels.into_iter().filter(|(_, id)| *id != 0).collect(),
        sidebar,
        main_group: s
            .backend
            .lua
            .load("return pob.build.mainSocketGroup")
            .eval()
            .unwrap(),
    }
}

/// Replays a recipe as our commands and checks every checkpoint and the end state against
/// upstream.
fn check_items_recipe(name: &str) -> ItemsRecipe {
    let recipe = upstream_items_recipe(name);
    assert!(!recipe.actions.is_empty(), "{name}: no actions recorded");
    let mut s = Session::start(&format!("items-recipe-{name}"));
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
    let before = json(&s.backend.items_view().unwrap());
    let mut checked = 0;
    for action in &recipe.actions {
        match action.command() {
            Some(command) => {
                s.run(command)
                    .unwrap_or_else(|e| panic!("{name}: {action:?}: {e}"));
            }
            None => {
                let ItemsAction::Check { check } = action else {
                    unreachable!()
                };
                let ours = our_items_checkpoint(&mut s);
                assert_same(
                    &ours.items,
                    &check.items,
                    &format!("{name}: checkpoint {checked}"),
                );
                assert_eq!(ours.display, check.display, "{name}: checkpoint {checked}");
                assert_eq!(
                    ours.jewels, check.jewels,
                    "{name}: checkpoint {checked} jewels"
                );
                assert_eq!(
                    ours.sidebar, check.sidebar,
                    "{name}: checkpoint {checked} sidebar"
                );
                assert_eq!(
                    ours.main_group, check.main_group,
                    "{name}: checkpoint {checked} main group"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "{name}: no checkpoint");
    let ours = json(&s.backend.items_view().unwrap());
    assert_ne!(ours, before, "{name}: the recipe changes the Items tab");
    assert_same(&ours, &recipe.result, name);
    recipe
}

fn checkpoints(recipe: &ItemsRecipe) -> Vec<&ItemsCheckpoint> {
    recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            ItemsAction::Check { check } => Some(&**check),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn list_slot_recipes_match_pob() {
    let _guard = lock();
    for name in ["delete_sort", "flasks_jewels"] {
        let recipe = check_items_recipe(name);
        let checks = checkpoints(&recipe);
        let slot = |c: &ItemsCheckpoint, name: &str| {
            c.items["slots"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("{name}: no slot {name}"))
        };
        if name == "delete_sort" {
            // the filtered list is read-only
            assert_eq!(checks[1].items["can_reorder"], false);
            assert_eq!(checks[0].items["can_reorder"], true);
        } else {
            // the flask toggled off and undone: the checkpoint pins what PoB shows (R12)
            // PoB's undo restores the sets' copies, not the live flask: it stays off (R12)
            assert_eq!(slot(checks[0], "Flask 1")["flask_active"], false);
            // a second jewel socket, then its node goes: the jewel stays, the slot hides
            assert_eq!(checks[1].jewels.len(), 2, "{:?}", checks[1].jewels);
            assert_eq!(checks[2].jewels.len(), 2, "{:?}", checks[2].jewels);
            // the sockets are labelled in tree order, among the shown ones
            let labels = |c: &ItemsCheckpoint| -> Vec<(String, bool)> {
                c.items["slots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|s| {
                        s["kind"] == "Jewel"
                            && c.jewels.contains_key(&(s["node"].as_u64().unwrap() as u32))
                    })
                    .map(|s| (s["label"].as_str().unwrap().to_string(), s["shown"] == true))
                    .collect()
            };
            assert_eq!(
                labels(checks[1]),
                [
                    ("Socket #1".to_string(), true),
                    ("Socket #2".to_string(), true)
                ]
            );
            assert_eq!(
                labels(checks[2]),
                [
                    ("Socket #1".to_string(), false),
                    ("Socket #1".to_string(), true)
                ]
            );
            // the second weapon set: swap slots shown, Weapon 1 hidden
            assert_eq!(checks[3].items["second_weapon_set"], true);
            assert_eq!(slot(checks[3], "Weapon 1")["shown"], false);
            assert_eq!(slot(checks[3], "Weapon 1 Swap")["shown"], true);
            // the switch moved the main group to the one in the swap set (R6)
            assert_eq!(checks[2].main_group, 1);
            assert_eq!(checks[3].main_group, 2);
        }
    }
}

// ---- Undo and errors ----

/// The raw text of the item named `name`, as ours copies it.
fn our_raw(s: &mut Session, name: &str) -> String {
    let view = s.backend.items_view().unwrap();
    let id = view
        .items
        .iter()
        .find(|i| i.name.starts_with(name))
        .unwrap_or_else(|| panic!("no item {name}"))
        .id;
    s.backend.item_text(id).unwrap().text
}

/// The raw text of the item named `name` in upstream.
fn upstream_raw(lua: &mlua::Lua, name: &str) -> String {
    lua.globals().set("wanted", name).unwrap();
    lua_value(
        lua,
        r#"
        for id, item in pairs(build.itemsTab.items) do
            if item.name:sub(1, #wanted) == wanted then return copyItem(id) end
        end
        error("no item " .. wanted)
        "#,
    )
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn items_undo_reverts_optimise() {
    let _guard = lock();
    // the gear build is attack plus items, so the list has something to move
    let (mut s, lua) = gear_sessions("items-undo-optimise");
    const MAUL: &str = "Driftwood Maul";
    let original = our_raw(&mut s, MAUL);
    // our Items tab and the Maul's text against upstream's, at every step
    let same = |s: &mut Session, what: &str| -> (String, serde_json::Value) {
        let ours = json(&s.backend.items_view().unwrap());
        assert_same(&ours, &upstream_json(&lua), what);
        let raw = our_raw(s, MAUL);
        assert_eq!(raw, upstream_raw(&lua, MAUL), "{what}: raw text");
        (raw, ours)
    };
    let (_, start) = same(&mut s, "start");
    // the list's order; the second Undo also clears the tree jewel (the load's first undo
    // state predates it), so the whole view isn't compared with the start
    let order = |view: &serde_json::Value| -> Vec<u64> {
        view["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_u64().unwrap())
            .collect()
    };

    // an Items edit first, so the stack is [S0, E, load]: the move is E's change
    s.run(Command::MoveItem { from: 0, to: 1 }).unwrap();
    lua.load("act.move(0, 1)").exec().unwrap();
    let (_, moved) = same(&mut s, "moved");
    assert_ne!(order(&moved), order(&start), "the move changes the list");

    // Optimise Sockets is the Skills tab's button, which adds the Items undo state first (R12)
    s.run(Command::SetGroupSlot {
        group: 0,
        slot: Some("Weapon 1".into()),
    })
    .unwrap();
    s.run(Command::OptimiseSockets { group: 0 }).unwrap();
    lua.load(
        r#"
        local tab = build.skillsTab
        tab:SetDisplayGroup(tab.socketGroupList[1])
        local c = tab.controls
        for i, choice in ipairs(c.groupSlot.list) do
            if choice.slotName == "Weapon 1" then c.groupSlot:SetSel(i) end
        end
        runCallback("OnFrame")
        c.optimiseSockets.onClick()
        runCallback("OnFrame")
        "#,
    )
    .exec()
    .unwrap();
    let (optimised, _) = same(&mut s, "optimised");
    assert_ne!(optimised, original, "Optimise Sockets edits the Maul");

    let undo = |s: &mut Session| {
        s.run(Command::Undo(Tab::Items)).unwrap();
        lua.load("build.itemsTab:Undo() build.buildFlag = true runCallback('OnFrame')")
            .exec()
            .unwrap();
    };
    let redo = |s: &mut Session| {
        s.run(Command::Redo(Tab::Items)).unwrap();
        lua.load("build.itemsTab:Redo() build.buildFlag = true runCallback('OnFrame')")
            .exec()
            .unwrap();
    };
    // one Undo reverts the Optimise and nothing else: the list is still moved
    undo(&mut s);
    let (raw, view) = same(&mut s, "undo 1");
    assert_eq!(raw, original, "one Undo reverts the optimise");
    assert_eq!(order(&view), order(&moved), "and keeps the move");
    // the second goes on to revert the move
    undo(&mut s);
    let (_, view) = same(&mut s, "undo 2");
    assert_eq!(
        order(&view),
        order(&start),
        "the second Undo reverts the move"
    );
    // Redo brings the move back, with the Maul as it was
    redo(&mut s);
    let (raw, view) = same(&mut s, "redo");
    assert_eq!(order(&view), order(&moved));
    assert_eq!(raw, original, "Redo doesn't re-apply the optimise (R12)");
    // and a following Undo goes as upstream does
    undo(&mut s);
    same(&mut s, "undo after redo");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_slot_errors() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("items-slot-errors");
    let view = s.backend.items_view().unwrap();
    let id_of = |name: &str| {
        view.items
            .iter()
            .find(|i| i.name.starts_with(name))
            .unwrap_or_else(|| panic!("no item {name}"))
            .id
    };
    let (ring, kaom, viridian, maul) = (
        id_of("Gale Band"),
        id_of("Kaom"),
        id_of("Viridian"),
        id_of("Driftwood"),
    );
    let hidden_socket = view
        .slots
        .iter()
        .find(|slot| slot.kind == SlotKind::Jewel && !slot.shown)
        .expect("an unallocated socket")
        .name
        .clone();
    let state = |s: &mut Session| {
        let jewels: serde_json::Value = lua_value(
            &s.backend.lua,
            "local out = {} for n, id in pairs(pob.build.spec.jewels) do out[#out + 1] = n .. '=' .. id end table.sort(out) return table.concat(out, ',')",
        );
        (s.backend.items_view().unwrap(), jewels)
    };
    let refuse = |s: &mut Session, command: Command, message: &str| {
        let before = state(s);
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert!(
            error.to_string().contains(message),
            "{command:?}: {error} (wanted {message})"
        );
        assert_eq!(state(s), before, "{command:?} changed something");
    };
    let equip = |slot: &str, id: u32| Command::EquipItem {
        slot: slot.into(),
        id: Some(id),
    };

    refuse(
        &mut s,
        equip("Helmet", ring),
        "that item can't go in that slot",
    );
    refuse(&mut s, equip(&hidden_socket, viridian), "no such slot");
    refuse(&mut s, equip("Nowhere", ring), "no such slot");
    refuse(&mut s, equip("Ring 1", 9999), "no such item");
    refuse(
        &mut s,
        Command::SetFlaskActive {
            slot: "Flask 2".into(),
            on: true,
        },
        "empty",
    );
    refuse(&mut s, Command::DeleteItem { id: 9999 }, "no such item");
    refuse(
        &mut s,
        Command::SetItemListFilter { index: 99 },
        "no such filter",
    );

    // a filtered list is read-only, and hides what it doesn't list
    s.run(Command::SetItemListFilter { index: 2 }).unwrap();
    refuse(
        &mut s,
        Command::MoveItem { from: 0, to: 1 },
        "can't be reordered",
    );
    refuse(&mut s, Command::DeleteItem { id: kaom }, "no such item");
    s.run(Command::SetItemListFilter { index: 0 }).unwrap();

    refuse(
        &mut s,
        Command::MoveItem { from: 0, to: 999 },
        "no such item",
    );
    refuse(
        &mut s,
        Command::MoveItem { from: 999, to: 0 },
        "no such item",
    );
    refuse(
        &mut s,
        Command::SetFlaskActive {
            slot: "Helmet".into(),
            on: true,
        },
        "not a flask slot",
    );

    // the second weapon set hides "Weapon 1"
    s.run(Command::SelectWeaponSet { second: true }).unwrap();
    refuse(&mut s, equip("Weapon 1", maul), "no such slot");

    // with nothing in the list, Del Unused and Del All do nothing (PoB's buttons are disabled)
    s.run(Command::SelectWeaponSet { second: false }).unwrap();
    s.run(Command::DeleteAllItems).unwrap();
    assert!(s.backend.items_view().unwrap().items.is_empty());
    let before = state(&mut s);
    s.run(Command::DeleteUnusedItems).unwrap();
    s.run(Command::DeleteAllItems).unwrap();
    assert_eq!(state(&mut s), before);
}

// ---- Item sets ----

#[test]
#[ignore = "needs the pinned PoB install"]
fn set_recipe_matches_pob() {
    let _guard = lock();
    let recipe = check_items_recipe("sets");
    let checks = checkpoints(&recipe);
    let titles = |c: &ItemsCheckpoint| -> Vec<String> {
        c.items["sets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap().to_string())
            .collect()
    };
    let t0 = titles(checks[0]);
    assert_eq!(&t0[2..], ["Mapping", "Bossing"]);
    // renamed and moved to the front, with no undo state (R9)
    let t2 = titles(checks[2]);
    assert_eq!(t2[0], "Clear");
    assert_eq!(t2.len(), 4);
    assert!(!t2.contains(&"Mapping".to_string()));
    // the second weapon set of the new set is on at checkpoint 1
    assert_eq!(checks[1].items["second_weapon_set"], true);
}

/// The number of entries in PoB's `itemSets` (orphans from a refused name show here)
fn item_set_count(s: &Session) -> u32 {
    lua_value(
        &s.backend.lua,
        "local n = 0 for _ in pairs(pob.build.itemsTab.itemSets) do n = n + 1 end return n",
    )
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_set_errors() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("item-set-errors");
    let view = s.backend.items_view().unwrap();
    let first = view.sets[0].id;
    let second = view.sets[1].id;
    let refuse = |s: &mut Session, command: Command, message: &str| {
        let before = (s.backend.items_view().unwrap(), item_set_count(s));
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert!(
            error.to_string().contains(message),
            "{command:?}: {error} (wanted {message})"
        );
        let after = (s.backend.items_view().unwrap(), item_set_count(s));
        assert_eq!(after, before, "{command:?} changed something");
    };
    let long = "x".repeat(101);
    for title in ["", "   ", long.as_str()] {
        let message = if title.len() > 100 {
            "too long"
        } else {
            "needs a name"
        };
        refuse(
            &mut s,
            Command::NewItemSet {
                title: title.into(),
            },
            message,
        );
        refuse(
            &mut s,
            Command::CopyItemSet {
                id: first,
                title: title.into(),
            },
            message,
        );
        refuse(
            &mut s,
            Command::RenameItemSet {
                id: first,
                title: title.into(),
            },
            message,
        );
    }
    for command in [
        Command::SelectItemSet { id: 999 },
        Command::CopyItemSet {
            id: 999,
            title: "A".into(),
        },
        Command::RenameItemSet {
            id: 999,
            title: "A".into(),
        },
        Command::DeleteItemSet { id: 999 },
    ] {
        refuse(&mut s, command, "no such item set");
    }
    refuse(
        &mut s,
        Command::MoveItemSet { from: 0, to: 9 },
        "no such item set",
    );
    refuse(
        &mut s,
        Command::MoveItemSet { from: 9, to: 0 },
        "no such item set",
    );
    // the last set stays
    s.run(Command::DeleteItemSet { id: second }).unwrap();
    assert_eq!(item_set_count(&s), 1);
    refuse(
        &mut s,
        Command::DeleteItemSet { id: first },
        "last item set",
    );
}
