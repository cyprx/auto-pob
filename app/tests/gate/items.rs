//! Gate tests for the Items tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::{
    Session, fixtures, fixtures_dir, load_tree_lib, load_xml, lock, lua_value, plain_notes,
    pob_dir, recipe_setting, upstream_lua,
};
use auto_pob::engine::upstream::{SidebarLine, golden_of};
use auto_pob::engine::{
    Backend, Command, DisplayItemView, ItemTooltipLine, ItemTooltipQuery, ItemsView, SlotKind, Tab,
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
        // Lua has one number type: 1 and 1.0 are the same value
        (Value::Number(x), Value::Number(y)) if x.as_f64() == y.as_f64() => None,
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

/// The text of items to add on top of the `gear` build: a multi-variant unique and a unique
/// flask from PoB's data (as the Items tab's database adds them), a rare weapon, a rare jewel
/// and a rare abyss jewel, and rings and a body armour with each header field.
const EXTRA_TEXTS: &str = r#"
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
        -- linked sockets, quality, and an enchant, a scourge, an implicit, an explicit and a
        -- crucible line
        "Rarity: RARE\nDread Shell\nGlorious Plate\nQuality: 20\nSockets: R-G-B B\nImplicits: 3\n{enchant}+3% to Fire Resistance\n{scourge}+4% to Cold Resistance\n+5% to Lightning Resistance\n{crucible}+10 to maximum Life\n+120 to maximum Life",
        -- a catalyst (shown for a ring with tagged mods) and a ranged line with a slider
        "Rarity: RARE\nCat Loop\nIron Ring\nCatalyst: Abrasive\nCatalystQuality: 20\nImplicits: 0\n{tags:life}+30 to maximum Life\n{range:0.5}+(10-20) to maximum Mana",
        -- a unique with versions and grouped variants (no unique of PoB's data uses them)
        "Rarity: UNIQUE\nGrouped Test\nSapphire Ring\nVersion: Release\nVersion: Rework\nVariant: Alpha\nVariant: Beta\nVariant: Gamma\nSelected Version: 2\nImplicits: 0\n{variant:1,2}{group:1}{version:1,2}+10 to Strength\n{variant:3}{group:1}{version:2}+20 to Strength",
    }
    -- a unique with Foulborn lines, as it drops and with one line transformed
    local howl = unique("Alpha's Howl")
    texts[#texts + 1] = howl
    texts[#texts + 1] = (howl:gsub("%+2 to Level of Socketed Aura Gems", "+2 to Level of Socketed Minion Gems"))
    return texts
"#;

/// `EXTRA_TEXTS`, then each added to the build at model level (`BUILD` is the build).
fn extra_items_code() -> String {
    format!(
        r#"
        local texts = (function() {EXTRA_TEXTS} end)()
        local t = BUILD.itemsTab
        for _, text in ipairs(texts) do
            t:AddItem(new("Item", text), true)
        end
        t:PopulateSlots()
        "#
    )
}

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

/// Upstream's `displayDump()`, with the nulls left out; none without a displayed item.
fn upstream_display(lua: &mlua::Lua) -> Option<serde_json::Value> {
    upstream_tooltip(lua, "return displayDump()")
}

/// What the pasted items made the display panel show: each part must be non-default at least
/// once, or the comparison would only cover the defaults.
#[derive(Default)]
struct DisplaySeen {
    version: bool,
    variants: bool,
    linked_sockets: bool,
    shown_socket: bool,
    can_add_socket: bool,
    quality: bool,
    catalyst: bool,
    catalyst_quality: bool,
    slider: bool,
    influence: bool,
    mutable: bool,
    mutated: bool,
    mod_lists: std::collections::BTreeSet<String>,
}

impl DisplaySeen {
    fn note(&mut self, view: &serde_json::Value) {
        let any = |key: &str, test: &dyn Fn(&serde_json::Value) -> bool| {
            view[key].as_array().is_some_and(|a| a.iter().any(test))
        };
        self.version |= view.get("version").is_some();
        self.variants |= !view["variants"].as_array().unwrap().is_empty();
        self.linked_sockets |= any("sockets", &|s| s["linked"] == true);
        self.shown_socket |= any("sockets", &|s| s["shown"] == true);
        self.can_add_socket |= view["can_add_socket"] == true;
        self.quality |= view.get("quality").is_some_and(|q| q != "0");
        self.catalyst |= view["catalyst"]["selected"].as_u64().unwrap_or(0) > 0;
        self.catalyst_quality |= view.get("catalyst_quality").is_some_and(|q| q != "0");
        self.slider |= any("range_lines", &|l| l["slider"] == true);
        self.influence |= any("influences", &|d| d["selected"].as_u64().unwrap_or(0) > 0);
        self.mutable |= any("range_lines", &|l| l["mutable"] == true);
        self.mutated |= any("range_lines", &|l| l["mutated"] == true);
        for line in view["tooltip"]["lines"].as_array().unwrap() {
            if let Some(list) = line["mod_line"]["list"].as_str() {
                self.mod_lists.insert(list.to_string());
            }
        }
    }

    fn assert_all(&self) {
        let flags = [
            ("version", self.version),
            ("variants", self.variants),
            ("linked sockets", self.linked_sockets),
            ("shown socket", self.shown_socket),
            ("can_add_socket", self.can_add_socket),
            ("quality", self.quality),
            ("catalyst", self.catalyst),
            ("catalyst_quality", self.catalyst_quality),
            ("range slider", self.slider),
            ("influence selection", self.influence),
            ("mutable line", self.mutable),
            ("mutated line", self.mutated),
        ];
        for (name, seen) in flags {
            assert!(seen, "no pasted item made the display show: {name}");
        }
        for list in ["enchant", "scourge", "implicit", "explicit", "crucible"] {
            assert!(
                self.mod_lists.contains(list),
                "no {list} mod_line: {:?}",
                self.mod_lists
            );
        }
    }
}

/// Every `mod_line` of the display tooltip names a line of the item whose words are in the
/// tooltip line: a check on the list and index that doesn't repeat the glue's search.
fn check_mod_lines(s: &Session, view: &serde_json::Value, what: &str) {
    let letters = |text: &str| -> String {
        let mut plain = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '^' {
                // a colour code: ^7 or ^xRRGGBB
                if chars.peek() == Some(&'x') {
                    chars.next();
                    for _ in 0..6 {
                        chars.next();
                    }
                } else {
                    chars.next();
                }
            } else if c.is_alphabetic() {
                plain.push(c);
            }
        }
        plain
    };
    for line in view["tooltip"]["lines"].as_array().unwrap() {
        let Some(list) = line["mod_line"]["list"].as_str() else {
            continue;
        };
        let index = line["mod_line"]["index"].as_u64().unwrap() + 1;
        let own: String = lua_value(
            &s.backend.lua,
            &format!("return pob.build.itemsTab.displayItem.{list}ModLines[{index}].line"),
        );
        let shown = letters(line["text"].as_str().unwrap());
        assert!(
            shown.contains(&letters(&own)),
            "{what}: mod_line {list}[{index}] is {own:?}, the tooltip line is {:?}",
            line["text"]
        );
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_tooltips_match_pob() {
    let _guard = lock();
    let (mut s, lua) = gear_sessions("items-tooltips");
    // the items are added as the Items tab adds them: pasted, then saved (which equips what
    // fits an empty slot), on both sides
    let texts: Vec<String> = lua_value(&lua, EXTRA_TEXTS);
    let mut shown = DisplaySeen::default();
    for text in &texts {
        s.run(Command::PasteItem { text: text.clone() }).unwrap();
        lua.globals().set("pasted", text.as_str()).unwrap();
        lua.load("act.paste(pasted)").exec().unwrap();
        let ours = s.backend.display_item_view().unwrap();
        // the display panel as PoB shows it for this item
        let theirs = upstream_display(&lua).expect("upstream displays the paste");
        let ours = json_of(&ours.expect("we display the paste"));
        let label = format!("display of {}", text.lines().nth(1).unwrap());
        assert_same(&ours, &theirs, &label);
        shown.note(&ours);
        check_mod_lines(&s, &ours, &label);
        s.run(Command::SaveDisplayItem).unwrap();
        lua.load("act.save()").exec().unwrap();
    }
    shown.assert_all();
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
    s.run(Command::SetShowStatDifferences { on: false })
        .unwrap();
    lua.load("act.stat_differences(false)").exec().unwrap();
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
    on_both(&s, &lua, &extra_items_code());
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
    PasteItem {
        text: String,
    },
    EditItem {
        id: u32,
    },
    SetDisplayItemText {
        rarity: usize,
        text: String,
    },
    SaveDisplayItem,
    CancelDisplayItem,
    SetShowStatDifferences {
        on: bool,
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
            ItemsAction::PasteItem { text } => Command::PasteItem { text },
            ItemsAction::EditItem { id } => Command::EditItem { id },
            ItemsAction::SetDisplayItemText { rarity, text } => {
                Command::SetDisplayItemText { rarity, text }
            }
            ItemsAction::SaveDisplayItem => Command::SaveDisplayItem,
            ItemsAction::CancelDisplayItem => Command::CancelDisplayItem,
            ItemsAction::SetShowStatDifferences { on } => Command::SetShowStatDifferences { on },
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
    /// The display panel (`displayDump()`); none while nothing is displayed
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
                check.display = check.display.map(strip_nulls);
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
        display: s
            .backend
            .display_item_view()
            .expect("display item view")
            .map(|view| json_of(&view)),
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
                let affected = s
                    .run(command)
                    .unwrap_or_else(|e| panic!("{name}: {action:?}: {e}"));
                if matches!(
                    action,
                    ItemsAction::PasteItem { .. }
                        | ItemsAction::EditItem { .. }
                        | ItemsAction::SetDisplayItemText { .. }
                        | ItemsAction::SaveDisplayItem
                        | ItemsAction::CancelDisplayItem
                ) {
                    assert!(
                        affected.display_item,
                        "{name}: {action:?} doesn't flag the display item"
                    );
                }
                assert_eq!(open_popups(&s), 0, "{name}: {action:?} left a popup open");
                // the loop sends the display item after a batch that flags it or recalculates,
                // which is what fills the tooltip PoB would otherwise rebuild only on an edit
                // (R4)
                if affected.display_item || affected.recalc {
                    s.backend.display_item_view().expect("display item view");
                }
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
                match (&ours.display, &check.display) {
                    (Some(ours), Some(theirs)) => assert_same(
                        ours,
                        theirs,
                        &format!("{name}: checkpoint {checked} display"),
                    ),
                    (ours, theirs) => {
                        assert_eq!(ours, theirs, "{name}: checkpoint {checked} display")
                    }
                }
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
    assert_eq!(open_popups(&s), 0, "{name}: a popup was left open");
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
    let ids = |c: &ItemsCheckpoint| -> Vec<u64> {
        c.items["sets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_u64().unwrap())
            .collect()
    };
    let bossing = ids(checks[0])[3];
    assert_eq!(checks[1].items["active_set"], bossing);
    // deleting the active set (the last one) leaves the one before it active, and unlists it
    let after_delete = ids(checks[3]);
    assert_eq!(after_delete.len(), 3);
    assert!(!after_delete.contains(&bossing));
    assert_eq!(checks[3].items["active_set"], after_delete[2]);
    assert_eq!(after_delete, ids(checks[2])[..3]);
    // Undo brings it back as the active set, in the order from before the move
    assert_eq!(checks[4].items["active_set"], bossing);
    // (the move added no undo state, so Undo restores the order from before it)
    assert_eq!(ids(checks[4]), ids(checks[1]));
    assert_ne!(ids(checks[4]), ids(checks[2]));
}

/// The number of popups PoB has open
fn open_popups(s: &Session) -> u32 {
    lua_value(&s.backend.lua, "return #pob.main.main.popups")
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
        assert_eq!(open_popups(s), 0, "{command:?} left a popup open");
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
    // every command that goes through renders no popup behind it
    let titled = |s: &mut Session, title: &str| {
        s.backend
            .items_view()
            .unwrap()
            .sets
            .iter()
            .find(|set| set.title == title)
            .unwrap_or_else(|| panic!("no set {title}"))
            .id
    };
    s.run(Command::NewItemSet {
        title: "New".into(),
    })
    .unwrap();
    assert_eq!(open_popups(&s), 0, "New");
    let made = titled(&mut s, "New");
    s.run(Command::RenameItemSet {
        id: made,
        title: "Renamed".into(),
    })
    .unwrap();
    assert_eq!(open_popups(&s), 0, "Rename");
    s.run(Command::CopyItemSet {
        id: made,
        title: "Copied".into(),
    })
    .unwrap();
    assert_eq!(open_popups(&s), 0, "Copy");
    // a move to the same place changes nothing and doesn't touch the build
    let moved = s.run(Command::MoveItemSet { from: 1, to: 1 }).unwrap();
    assert!(!moved.build && !moved.items, "{moved:?}");
    assert_eq!(open_popups(&s), 0, "Move to the same place");
    s.run(Command::MoveItemSet { from: 0, to: 2 }).unwrap();
    assert_eq!(open_popups(&s), 0, "Move");
    refuse(
        &mut s,
        Command::MoveItemSet {
            from: usize::MAX,
            to: 0,
        },
        "no such item set",
    );
    let copied = titled(&mut s, "Copied");
    s.run(Command::DeleteItemSet { id: copied }).unwrap();
    assert_eq!(open_popups(&s), 0, "Delete");
    s.run(Command::DeleteItemSet { id: made }).unwrap();
    // the last set stays
    s.run(Command::DeleteItemSet { id: second }).unwrap();
    assert_eq!(item_set_count(&s), 1);
    refuse(
        &mut s,
        Command::DeleteItemSet { id: first },
        "last item set",
    );
}

// ---- The display item ----

const RING: &str = "Rarity: RARE\nDoom Loop\nIron Ring\nImplicits: 0\n+30 to maximum Life";

fn display(s: &mut Session) -> Option<DisplayItemView> {
    s.backend.display_item_view().expect("display item view")
}

/// `unsaved` as PoB's next frame sees it (Build.lua:1303)
fn unsaved_after_frame(s: &mut Session) -> bool {
    s.backend.lua.load("glue.frame()").exec().unwrap();
    s.backend.build_view().unwrap().unsaved
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn display_recipe_matches_pob() {
    let _guard = lock();
    let recipe = check_items_recipe("paste_equip");
    let checks = checkpoints(&recipe);
    assert_eq!(checks.len(), 7);
    let display = |i: usize| checks[i].display.as_ref();
    // a new item: no id, "Add to build"
    let pasted = display(0).expect("the pasted ring is displayed");
    assert_eq!(pasted["save_label"], "Add to build");
    assert!(pasted.get("id").is_none(), "{pasted}");
    // the equipped armour was replaced and nothing is displayed
    assert!(display(1).is_none());
    // an edited item: its id and "Save"
    let edited = display(2).expect("the ring is displayed for editing");
    assert_eq!(edited["save_label"], "Save");
    assert!(edited["id"].is_u64(), "{edited}");
    assert!(
        edited["edit_text"]
            .as_str()
            .unwrap()
            .contains("+15 to maximum Mana")
    );
    // saved in place: the order is unchanged and nothing is displayed
    assert!(display(3).is_none());
    let order = |i: usize| -> Vec<u64> {
        checks[i].items["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(order(2), order(3));
    // the item displayed again while another ring is equipped, then cancelled
    assert!(display(4).is_some());
    assert!(display(5).is_some());
    assert!(display(6).is_none());
    // the display tooltip follows the outputs: equipping a ring while it is shown changes
    // its comparison (R4)
    assert_ne!(
        display(2).unwrap()["tooltip"],
        display(4).unwrap()["tooltip"],
        "the comparison didn't follow the new outputs"
    );
    // and Ctrl+D with the item displayed drops the comparison
    assert_ne!(
        display(4).unwrap()["tooltip"],
        display(5).unwrap()["tooltip"],
        "the comparison didn't follow the toggle"
    );
    assert_eq!(checks[4].items["show_stat_differences"], true);
    assert_eq!(checks[5].items["show_stat_differences"], false);
    assert_eq!(checks[6].items["show_stat_differences"], false);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn bad_item_text_is_an_error() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("display-bad-text");
    let refuse = |s: &mut Session, command: Command| {
        let before = (display(s), s.backend.items_view().unwrap());
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert!(
            error.to_string().contains("PoB couldn't read that item"),
            "{command:?}: {error}"
        );
        assert_eq!((display(s), s.backend.items_view().unwrap()), before);
        assert_eq!(open_popups(s), 0, "{command:?}: a popup was left open");
    };
    // nothing displayed
    refuse(
        &mut s,
        Command::PasteItem {
            text: "hello".into(),
        },
    );
    refuse(
        &mut s,
        Command::SetDisplayItemText {
            rarity: 2,
            text: "hello".into(),
        },
    );
    assert!(display(&mut s).is_none());
    // an item being edited stays as it is
    s.run(Command::PasteItem { text: RING.into() }).unwrap();
    let shown = display(&mut s).expect("the ring");
    assert_eq!(shown.name, "Doom Loop, Iron Ring");
    refuse(
        &mut s,
        Command::PasteItem {
            text: "hello".into(),
        },
    );
    refuse(
        &mut s,
        Command::SetDisplayItemText {
            rarity: 2,
            text: "hello".into(),
        },
    );
    // a rarity PoB doesn't list
    let error = s
        .run(Command::SetDisplayItemText {
            rarity: 5,
            text: RING.into(),
        })
        .unwrap_err();
    assert!(error.to_string().contains("no such rarity"), "{error}");
    assert_eq!(display(&mut s), Some(shown));
    // no item: Save and Cancel are refused
    s.run(Command::CancelDisplayItem).unwrap();
    for command in [Command::SaveDisplayItem, Command::CancelDisplayItem] {
        let error = s.run(command).unwrap_err();
        assert!(
            error.to_string().contains("no item is being viewed"),
            "{error}"
        );
    }
    let error = s.run(Command::EditItem { id: 9999 }).unwrap_err();
    assert!(error.to_string().contains("no such item"), "{error}");
}

/// The `showStatDifferences` attribute of the `<Items>` element the build was saved with
fn saved_differences(s: &Session, name: &str) -> String {
    let xml = fs::read_to_string(s.builds_dir().join(format!("{name}.xml"))).unwrap();
    let items = &xml[xml.find("<Items").expect("an Items element")..];
    let tag = &items[..items.find('>').unwrap()];
    let key = "showStatDifferences=\"";
    let at = tag
        .find(key)
        .unwrap_or_else(|| panic!("no showStatDifferences in {tag}"));
    tag[at + key.len()..].split('"').next().unwrap().to_string()
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn display_edits_are_not_unsaved() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("display-unsaved");
    s.run(Command::Save).unwrap();
    assert!(!unsaved_after_frame(&mut s));
    let undo_depth =
        |s: &Session| -> u32 { lua_value(&s.backend.lua, "return #pob.build.itemsTab.undo") };
    let depth = undo_depth(&s);
    // paste and cancel
    s.run(Command::PasteItem { text: RING.into() }).unwrap();
    assert!(display(&mut s).is_some());
    assert!(!unsaved_after_frame(&mut s), "a paste is not an edit");
    s.run(Command::SetDisplayItemText {
        rarity: 2,
        text: format!("{RING}\n+9 to maximum Mana").replace("Rarity: RARE\n", ""),
    })
    .unwrap();
    assert!(
        !unsaved_after_frame(&mut s),
        "editing the text is not an edit"
    );
    s.run(Command::CancelDisplayItem).unwrap();
    assert!(display(&mut s).is_none());
    assert!(!unsaved_after_frame(&mut s), "cancel is not an edit");
    // Ctrl+D is saved with the build but doesn't mark it
    assert_eq!(saved_differences(&s, "gear"), "true");
    let affected = s
        .run(Command::SetShowStatDifferences { on: false })
        .unwrap();
    assert!(affected.items && affected.display_item && affected.recalc);
    assert!(!unsaved_after_frame(&mut s));
    assert!(!s.backend.items_view().unwrap().show_stat_differences);
    s.run(Command::Save).unwrap();
    assert_eq!(saved_differences(&s, "gear"), "false");
    // already off: nothing to recalculate
    let affected = s
        .run(Command::SetShowStatDifferences { on: false })
        .unwrap();
    assert!(!affected.recalc);
    s.run(Command::SetShowStatDifferences { on: true }).unwrap();
    assert!(!unsaved_after_frame(&mut s));
    s.run(Command::Save).unwrap();
    assert_eq!(saved_differences(&s, "gear"), "true");
    assert_eq!(undo_depth(&s), depth, "display edits add no undo state");
    // Saving the item is an edit
    s.run(Command::PasteItem { text: RING.into() }).unwrap();
    s.run(Command::SaveDisplayItem).unwrap();
    assert!(unsaved_after_frame(&mut s), "adding an item is an edit");
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn display_item_view_is_pure() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("display-pure");
    s.run(Command::PasteItem { text: RING.into() }).unwrap();
    let state = |s: &mut Session| (pure_state(s), unsaved_after_frame(s));
    let before = state(&mut s);
    let first = display(&mut s).expect("displayed");
    let second = display(&mut s).expect("displayed");
    assert_eq!(first, second);
    assert_eq!(state(&mut s), before);
    assert_eq!(open_popups(&s), 0);
    // what the view reads from PoB's Edit popup is the item
    assert_eq!(first.edit_rarity, 2);
    assert!(first.edit_text.contains("+30 to maximum Life"));
    assert!(!first.edit_text.contains("Rarity:"));
    // the same for an item being edited
    let id = s.backend.items_view().unwrap().items[0].id;
    s.run(Command::EditItem { id }).unwrap();
    let before = state(&mut s);
    assert_eq!(display(&mut s), display(&mut s));
    assert_eq!(state(&mut s), before);
}
