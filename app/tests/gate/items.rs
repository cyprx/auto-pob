//! Gate tests for the Items tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{
    Session, fixtures, fixtures_dir, load_tree_lib, load_xml, lock, lua_value, pob_dir,
    recipe_setting, upstream_lua,
};
use auto_pob::engine::{Backend, ItemTooltipLine, ItemTooltipQuery, ItemsView, SlotKind};
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
        .load(&format!(
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
