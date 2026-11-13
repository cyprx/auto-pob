//! Gate tests for the Items tab: our snapshot against unmodified upstream PoB.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::items_harness::{
    ItemsCheckpoint, ItemsRecipe, assert_same, check_items_recipe, checkpoints, json, json_of,
    open_popups, raws_and_jewels, run_items_recipe, setup_xml, strip_nulls, upstream_json,
    upstream_with_items_lib,
};
use super::{Session, fixtures, lock, lua_value, snapshot_dir};
use auto_pob::engine::{
    Backend, Command, DisplayItemView, ItemTooltipLine, ItemTooltipQuery, ItemsView, ModLineList,
    ModLineRef, SlotKind, SocketColour, Tab,
};
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;

/// Our session with the `gear` setup open, and upstream PoB with the same build.
pub(crate) fn gear_sessions(test: &str) -> (Session, mlua::Lua) {
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
pub(crate) fn on_both(s: &Session, lua: &mlua::Lua, code: &str) {
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
            let query = ItemTooltipQuery {
                id: item.id,
                slot,
                theirs: false,
                shift: false,
                shared: None,
            };
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
                theirs: false,
                shift: false,
                shared: None,
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
            theirs: false,
            shift: false,
            shared: None,
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
    // an item in use is deleted only once the user has been asked: PoB's own popup is refused
    // and closed, and the item stays
    refuse(
        &mut s,
        Command::DeleteItem {
            id: kaom,
            confirmed: false,
        },
        "confirm the deletion",
    );
    assert_eq!(open_popups(&s), 0, "the refused delete left a popup open");
    assert!(
        s.backend
            .items_view()
            .unwrap()
            .items
            .iter()
            .any(|i| i.id == kaom)
    );
    refuse(
        &mut s,
        Command::DeleteItem {
            id: 9999,
            confirmed: true,
        },
        "no such item",
    );
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
    refuse(
        &mut s,
        Command::DeleteItem {
            id: kaom,
            confirmed: true,
        },
        "no such item",
    );
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

pub(crate) fn display(s: &mut Session) -> Option<DisplayItemView> {
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
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
    for command in [
        Command::SaveDisplayItem,
        Command::CancelDisplayItem { if_id: None },
    ] {
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
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
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

#[test]
#[ignore = "needs the pinned PoB install"]
fn display_edit_recipes_match_pob() {
    let _guard = lock();
    let variants = check_items_recipe("variants");
    let checks = checkpoints(&variants);
    assert_eq!(checks.len(), 5);
    let display = |i: usize| checks[i].display.as_ref().expect("a displayed item");
    // a legacy unique: variant 2 on its first dropdown, alternate variant 3 on its second
    let legacy = display(0)["variants"].as_array().unwrap();
    assert_eq!(legacy[0]["control"], 0);
    assert_eq!(legacy[0]["list"]["selected"], 1);
    assert_eq!(legacy[1]["control"], 1);
    assert_eq!(legacy[1]["list"]["selected"], 2);
    assert!(checks[1].display.is_none());
    // variant groups: the group controls are the first ones, and the second took its second choice
    let grouped = display(2)["variants"].as_array().unwrap();
    assert!(grouped.len() >= 2, "{grouped:?}");
    assert_eq!(grouped[1]["list"]["selected"], 1);
    // versions: the dropdown, and the lines of the version picked
    assert_eq!(display(3)["version"]["selected"], 0);
    let lines = |d: &serde_json::Value| -> String { d["tooltip"]["lines"].to_string() };
    assert!(lines(display(3)).contains("+20 to maximum Life"));
    assert!(!lines(display(3)).contains("+60 to maximum Life"));
    assert!(checks[4].display.is_none());
    let edits = check_items_recipe("display_edits");
    let checks = checkpoints(&edits);
    assert_eq!(checks.len(), 8);
    let display = |i: usize| checks[i].display.as_ref().expect("a displayed item");
    // the amulet
    let amulet = display(0);
    assert_eq!(amulet["quality"], "20");
    assert_eq!(amulet["influences"][0]["selected"], 1);
    assert_eq!(amulet["influences"][1]["selected"], 2);
    assert_eq!(amulet["catalyst"]["selected"], 4);
    assert_eq!(amulet["catalyst_quality"], "15");
    assert!(checks[1].display.is_none());
    // the armour: socket 1 green, no link after socket 2, a fifth socket
    let sockets = display(2)["sockets"].as_array().unwrap();
    assert_eq!(sockets.len(), 5);
    assert_eq!(sockets[0]["colour"], "G");
    assert_eq!(sockets[1]["linked"], false);
    assert_eq!(sockets[0]["linked"], true);
    // the unique: the second line at 0.25, which the re-paste then drops
    let range = |d: &serde_json::Value, i: usize| d["range_lines"][i]["range"].as_f64();
    assert_eq!(range(display(3), 1), Some(0.25));
    // the first line at 1.0 on the re-pasted item, though its slider held a stale 1.0
    assert_eq!(range(display(4), 0), Some(1.0));
    assert_ne!(range(display(4), 1), Some(0.25));
    // a mutated line, then a mod line off
    let mutated = |d: &serde_json::Value| {
        d["range_lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["mutated"] == true)
    };
    assert!(!mutated(display(4)));
    assert!(mutated(display(5)), "no line mutated");
    assert_ne!(display(5)["tooltip"], display(6)["tooltip"]);
    assert!(checks[7].display.is_none());
    // the other layout of the range controls
    range_dropdown_matches_pob();
}

/// A unique jewel: PoB shows no influence, catalyst, variant or socket control for it.
const UNIQUE_JEWEL: &str = "Rarity: UNIQUE\nTester's Jewel\nCobalt Jewel\nQuality: 0\nImplicits: 0\n+(10-20) to maximum Life\n+7 to maximum Mana";

/// The first Foulborn unique (by name) with a mutable line that has no slider, with that line's
/// 0-based index, and a line of the same item that has a slider but can't mutate.
pub(crate) const FIND_NO_SLIDER: &str = r#"
local titles = {}
for title in pairs(data.foulbornMap) do titles[#titles + 1] = title end
table.sort(titles)
local texts = {}
for _, list in pairs(data.uniques) do
	for _, text in ipairs(list) do texts[text:match("^[^\n]+")] = text end
end
for _, title in ipairs(titles) do
	local text = texts[title]
	if text then
		local item = new("Item", text)
		local bare, plain
		for i, m in ipairs(item.rangeLineList) do
			if m.modId and m.newModId and not m.showSlider and not bare then bare = i - 1 end
			if m.showSlider and not (m.modId and m.newModId) and not plain then plain = i - 1 end
		end
		if bare and plain then return { text = text, bare = bare, plain = plain } end
	end
end
error("no such unique")
"#;

/// Variant groups and versions; group 1 has the one variant, so its dropdown is disabled.
const GROUPED: &str = "Rarity: UNIQUE\nGrouped Charm\nPrismatic Ring\nVariant: A\nVariant: B\nVariant: C\nVersion: Old\nVersion: New\nImplicits: 0\n{version:1,2}{variant:1}{group:1}+10 to maximum Life\n{version:1,2}{variant:2}{group:2}+20 to maximum Mana\n{version:1,2}{variant:3}{group:2}+30 to maximum Mana";

#[derive(serde::Deserialize)]
pub(crate) struct NoSlider {
    pub(crate) text: String,
    pub(crate) bare: usize,
    pub(crate) plain: usize,
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn display_edit_errors() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("display-edit-errors");
    // every display command, with the error each must give
    let refuse = |s: &mut Session, command: Command, message: &str| {
        let before = (display(s), s.backend.items_view().unwrap());
        let error = s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert!(
            error.to_string().contains(message),
            "{command:?}: {error} (wanted {message})"
        );
        assert_eq!(
            (display(s), s.backend.items_view().unwrap()),
            before,
            "{command:?} changed something"
        );
        assert_eq!(open_popups(s), 0, "{command:?}: a popup was left open");
    };
    let hidden = "PoB doesn't show that control for this item";
    let line = |list, index| ModLineRef { list, index };

    // no item is displayed
    let every_command = |s: &mut Session| {
        let commands = [
            Command::SetItemVariant {
                control: 0,
                index: 0,
            },
            Command::SetItemVersion { index: 0 },
            Command::SetItemQuality { text: "5".into() },
            Command::SetItemInfluence { which: 0, index: 1 },
            Command::SetItemCatalyst { index: 1 },
            Command::SetItemCatalystQuality { text: "5".into() },
            Command::SetItemSocket {
                index: 0,
                colour: SocketColour::R,
            },
            Command::SetItemSocketLink {
                index: 0,
                linked: true,
            },
            Command::AddItemSocket,
            Command::SetRangeLine {
                line: 0,
                value: 0.5,
            },
            Command::SetRangeLineMutated { line: 0, on: true },
            Command::ToggleModLine {
                line: line(ModLineList::Explicit, 0),
            },
        ];
        for command in commands {
            refuse(s, command, "no item is being viewed");
        }
    };
    every_command(&mut s);

    // a jewel with none of the controls
    s.run(Command::PasteItem {
        text: UNIQUE_JEWEL.into(),
    })
    .unwrap();
    for command in [
        Command::SetItemInfluence { which: 0, index: 1 },
        Command::SetItemInfluence { which: 1, index: 1 },
        Command::SetItemInfluence { which: 2, index: 1 },
        Command::SetItemCatalyst { index: 1 },
        Command::SetItemCatalystQuality { text: "5".into() },
        Command::SetItemVariant {
            control: 5,
            index: 0,
        },
        Command::SetItemVariant {
            control: 0,
            index: 0,
        },
        Command::SetItemVersion { index: 0 },
        Command::SetItemSocket {
            index: 0,
            colour: SocketColour::R,
        },
        Command::SetItemSocketLink {
            index: 0,
            linked: true,
        },
        Command::AddItemSocket,
        // the second line has no slider and is no range line at all
        Command::SetRangeLineMutated { line: 0, on: true },
    ] {
        refuse(&mut s, command, hidden);
    }
    refuse(
        &mut s,
        Command::SetRangeLine {
            line: 1,
            value: 0.5,
        },
        "no such range line",
    );
    refuse(
        &mut s,
        Command::SetRangeLineMutated { line: 1, on: true },
        "no such range line",
    );
    // the quality box takes two digits
    refuse(
        &mut s,
        Command::SetItemQuality { text: "123".into() },
        "too long",
    );
    // a mod line the item doesn't have
    refuse(
        &mut s,
        Command::ToggleModLine {
            line: line(ModLineList::Explicit, 9),
        },
        "no such mod line",
    );
    refuse(
        &mut s,
        Command::ToggleModLine {
            line: line(ModLineList::Implicit, 0),
        },
        "no such mod line",
    );
    // text the box filters away changes nothing and is no error
    s.run(Command::SetItemQuality { text: "abc".into() })
        .unwrap();
    assert_eq!(display(&mut s).unwrap().quality.as_deref(), Some("0"));

    // a mod line of a variant the item doesn't use is in the item but not on its tooltip
    s.run(Command::PasteItem {
        text: "Rarity: UNIQUE\nTester's Charm\nIron Ring\nVariant: Old\nVariant: New\nImplicits: 0\n{variant:1}+10 to maximum Life\n{variant:2}+20 to maximum Mana".into(),
    })
    .unwrap();
    refuse(
        &mut s,
        Command::ToggleModLine {
            line: line(ModLineList::Explicit, 0),
        },
        "isn't on the tooltip",
    );
    // the line that is on the tooltip toggles, and says so
    let affected = s
        .run(Command::ToggleModLine {
            line: line(ModLineList::Explicit, 1),
        })
        .unwrap();
    assert!(affected.display_item && affected.recalc);

    // a mutable line without a slider, and a slider line that can't mutate
    let found: NoSlider = s
        .backend
        .lua
        .from_value(s.backend.lua.load(FIND_NO_SLIDER).eval().unwrap())
        .unwrap();
    s.run(Command::PasteItem { text: found.text }).unwrap();
    refuse(
        &mut s,
        Command::SetRangeLine {
            line: found.bare,
            value: 0.5,
        },
        hidden,
    );
    refuse(
        &mut s,
        Command::SetRangeLineMutated {
            line: found.plain,
            on: true,
        },
        hidden,
    );

    // a dropdown PoB shows but disables: group 1 has one variant to pick from
    s.run(Command::PasteItem {
        text: GROUPED.into(),
    })
    .unwrap();
    let shown = display(&mut s).unwrap();
    assert!(shown.version.is_some());
    assert_eq!(shown.variants[0].control, 0);
    assert!(!shown.variants[0].list.enabled, "group 1 is not disabled");
    assert!(shown.variants[1].list.enabled);
    refuse(
        &mut s,
        Command::SetItemVariant {
            control: 0,
            index: 0,
        },
        hidden,
    );
    // the version dropdown beside it takes a pick
    s.run(Command::SetItemVersion { index: 0 }).unwrap();

    // and with the item gone again
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
    every_command(&mut s);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn a_conditional_cancel_only_cancels_the_item_it_names() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("display-conditional-cancel");
    let items = s.backend.items_view().unwrap().items;
    let (first, other) = (items[0].id, items[1].id);
    s.run(Command::EditItem { id: first }).unwrap();
    assert_eq!(display(&mut s).unwrap().id, Some(first));
    // another item's clean-up leaves it alone, with no error
    s.run(Command::CancelDisplayItem { if_id: Some(other) })
        .unwrap();
    assert_eq!(display(&mut s).unwrap().id, Some(first));
    // a pasted item has no id, so a clean-up for a listed item leaves it alone too
    s.run(Command::PasteItem {
        text: "Rarity: RARE\nDoom Plate\nGlorious Plate\nImplicits: 0\n+50 to maximum Life".into(),
    })
    .unwrap();
    assert_eq!(display(&mut s).unwrap().id, None);
    s.run(Command::CancelDisplayItem { if_id: Some(first) })
        .unwrap();
    assert!(display(&mut s).is_some(), "the pasted item was cancelled");
    // the user's own Cancel is unconditional, and the named one cancels what it names
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
    assert!(display(&mut s).is_none());
    s.run(Command::EditItem { id: first }).unwrap();
    s.run(Command::CancelDisplayItem { if_id: Some(first) })
        .unwrap();
    assert!(display(&mut s).is_none());
    // and with nothing displayed it is no error, unlike the user's Cancel
    s.run(Command::CancelDisplayItem { if_id: Some(first) })
        .unwrap();
    assert!(
        s.run(Command::CancelDisplayItem { if_id: None }).is_err(),
        "an empty panel's own Cancel is refused"
    );
}

/// One recipe saved, reloaded by upstream and compared with the recipe's result and ours.
pub(crate) fn items_round_trip(name: &str) {
    let _guard = lock();
    let (recipe, mut s) = run_items_recipe(name);
    round_trip_checks(name, &recipe, &mut s);
}

/// `items_round_trip`'s checks on a session where `recipe` ended, for a test that also checks
/// that session in other ways (one replay of the recipe for both)
pub(crate) fn round_trip_checks(name: &str, recipe: &ItemsRecipe, s: &mut Session) {
    s.run(Command::Save).unwrap();
    let saved = s.backend.build_view().unwrap().path.expect("a saved build");
    // what upstream PoB makes of the file we wrote
    let lua = upstream_with_items_lib(&saved, &recipe.base);
    let reloaded = upstream_json(&lua);
    // the undo history and what the list shows are not saved
    // `flasks_jewels` leaves the live slots apart from the saved file's: PoB's undo
    // restores only the sets' flask copies (R12), and a socket whose node went is
    // labelled from the live tree. Its slots are not compared; the saved sets' flasks are,
    // below, and the jewels and item texts as for every recipe.
    let live_slots = name != "flasks_jewels";
    let saved_part = |mut value: serde_json::Value| {
        for key in ["can_undo", "can_redo", "list_filter", "shared_shown"] {
            value.as_object_mut().unwrap().remove(key);
        }
        if !live_slots {
            value.as_object_mut().unwrap().remove("slots");
        }
        value
    };
    let ours = saved_part(json(&s.backend.items_view().unwrap()));
    let expected = saved_part(recipe.result.clone());
    let reloaded = saved_part(reloaded);
    assert_same(&reloaded, &expected, &format!("{name}: reloaded vs recipe"));
    assert_same(&ours, &reloaded, &format!("{name}: ours vs reloaded"));
    let (raws, jewels) = raws_and_jewels(&lua, "build");
    assert_eq!(raws, recipe.raws, "{name}: item text after reload");
    assert_eq!(jewels, recipe.jewels, "{name}: jewels after reload");
    let (ours_raws, ours_jewels) = raws_and_jewels(&s.backend.lua, "pob.build");
    assert_eq!(ours_raws, raws, "{name}: our item text");
    assert_eq!(ours_jewels, jewels, "{name}: our jewels");
    assert!(!raws.is_empty(), "{name}: no items");
    // the flasks that are on, by set (an untouched set holds no slot entries yet)
    let flasks = |lua: &mlua::Lua, build: &str| -> BTreeMap<String, bool> {
        lua_value(
            lua,
            &format!(
                "local out = {{}} for id, set in pairs({build}.itemsTab.itemSets) do for slot, entry in pairs(set) do if type(entry) == 'table' and entry.active then out[id .. '/' .. slot] = true end end end return out"
            ),
        )
    };
    assert_eq!(
        flasks(&s.backend.lua, "pob.build"),
        flasks(&lua, "build"),
        "{name}: the saved sets' flasks"
    );
}

// One test per recipe, so the gate runner can spread the slowest one (72 s as a single test).
macro_rules! round_trip_tests {
    ($($test:ident: $recipe:literal,)*) => {$(
        #[test]
        #[ignore = "needs the pinned PoB install"]
        fn $test() {
            items_round_trip($recipe);
        }
    )*};
}

round_trip_tests! {
    items_round_trip_paste_equip: "paste_equip",
    items_round_trip_sets: "sets",
    items_round_trip_display_edits: "display_edits",
    items_round_trip_flasks_jewels: "flasks_jewels",
    items_round_trip_variants: "variants",
    items_round_trip_delete_sort: "delete_sort",
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn no_silent_writes_items() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("items-no-writes");
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    let found: NoSlider = s
        .backend
        .lua
        .from_value(s.backend.lua.load(FIND_NO_SLIDER).eval().unwrap())
        .unwrap();
    let view = s.backend.items_view().unwrap();
    let slot = |name: &str| view.slots.iter().find(|slot| slot.name == name).unwrap();
    let spare = slot("Ring 2").choices[0];
    let first = view.items[0].id;
    let second_set = view.sets[0].id;
    let amulet = "Rarity: RARE\nDoom Pendant\nOnyx Amulet\nQuality: 0\nImplicits: 1\n{tags:attribute}+12 to all Attributes\n{tags:life}+70 to maximum Life\n{tags:resistance}+30% to Fire Resistance";
    let armour = "Rarity: RARE\nDoom Plate\nGlorious Plate\nSockets: R-R-R-R\nImplicits: 0\n+50 to maximum Life";
    let versioned = "Rarity: UNIQUE\nVeil of Versions\nPrismatic Ring\nVersion: Old\nVersion: New\nImplicits: 0\n{version:1}+20 to maximum Life\n{version:2}+60 to maximum Life";
    let variant = "Rarity: UNIQUE\nTester's Charm\nIron Ring\nVariant: Old\nVariant: New\nImplicits: 0\n{variant:1}+10 to maximum Life\n{variant:2}+20 to maximum Mana";
    let commands = vec![
        // the list and the slots (Tasks 3-4)
        Command::SetItemListFilter { index: 1 },
        Command::SetItemListFilter { index: 0 },
        Command::SortItems,
        Command::MoveItem { from: 0, to: 1 },
        Command::EquipItem {
            slot: "Ring 2".into(),
            id: Some(spare),
        },
        Command::EquipItem {
            slot: "Ring 2".into(),
            id: None,
        },
        Command::SetFlaskActive {
            slot: "Flask 1".into(),
            on: false,
        },
        Command::SelectWeaponSet { second: true },
        Command::SelectWeaponSet { second: false },
        Command::Undo(Tab::Items),
        Command::Redo(Tab::Items),
        // item sets
        Command::NewItemSet {
            title: "Mapping".into(),
        },
        Command::CopyItemSet {
            id: second_set,
            title: "Bossing".into(),
        },
        Command::RenameItemSet {
            id: second_set,
            title: "Leveling".into(),
        },
        Command::MoveItemSet { from: 0, to: 1 },
        Command::SelectItemSet { id: second_set },
        // the display item (Task 5) and its editors (this task)
        Command::SetShowStatDifferences { on: false },
        Command::PasteItem {
            text: amulet.into(),
        },
        Command::SetItemQuality { text: "20".into() },
        Command::SetItemInfluence { which: 0, index: 1 },
        Command::SetItemInfluence { which: 1, index: 2 },
        Command::SetItemCatalyst { index: 4 },
        Command::SetItemCatalystQuality { text: "15".into() },
        Command::ToggleModLine {
            line: ModLineRef {
                list: ModLineList::Explicit,
                index: 0,
            },
        },
        Command::SaveDisplayItem,
        Command::PasteItem {
            text: armour.into(),
        },
        Command::SetItemSocket {
            index: 0,
            colour: SocketColour::G,
        },
        Command::SetItemSocketLink {
            index: 1,
            linked: false,
        },
        Command::AddItemSocket,
        Command::SetDisplayItemText {
            rarity: 2,
            text: armour.replace("Rarity: RARE\n", ""),
        },
        Command::CancelDisplayItem { if_id: None },
        Command::PasteItem {
            text: variant.into(),
        },
        Command::SetItemVariant {
            control: 0,
            index: 0,
        },
        Command::PasteItem {
            text: versioned.into(),
        },
        Command::SetItemVersion { index: 0 },
        Command::PasteItem { text: found.text },
        Command::SetRangeLine {
            line: found.plain,
            value: 0.25,
        },
        Command::SetRangeLineMutated {
            line: found.bare,
            on: true,
        },
        Command::SaveDisplayItem,
        Command::EditItem { id: first },
        Command::CancelDisplayItem { if_id: None },
        // deleting
        Command::DeleteItem {
            id: spare,
            confirmed: true,
        },
        Command::DeleteUnusedItems,
        Command::DeleteItemSet { id: second_set },
        Command::DeleteAllItems,
    ];
    for command in commands {
        let shown = format!("{command:?}");
        s.run(command).unwrap_or_else(|e| panic!("{shown}: {e}"));
    }
    // the queries
    let id = s.backend.items_view().unwrap().items.first().map(|i| i.id);
    if let Some(id) = id {
        s.backend.item_text(id).unwrap();
        s.backend
            .item_tooltip_view(&ItemTooltipQuery {
                id,
                slot: None,
                theirs: false,
                shift: false,
                shared: None,
            })
            .unwrap();
    }
    assert_eq!(
        snapshot_dir(&dir).keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "files appeared or went"
    );
    assert!(
        snapshot_dir(&s.user)
            .keys()
            .all(|p| !p.to_string_lossy().contains("~~temp~~")),
        "a temporary file was left"
    );
    for (path, bytes) in &before {
        assert!(
            snapshot_dir(&dir)[path] == *bytes,
            "{} changed",
            path.display()
        );
    }
    // only an explicit Save writes
    s.run(Command::Save).unwrap();
    let after = snapshot_dir(&dir);
    assert!(
        before.iter().any(|(path, bytes)| after[path] != *bytes),
        "Save didn't write the build"
    );
}

/// With "show all item affixes sliders" off, a unique has the single slider and Foulborn box
/// after the range line dropdown instead of one pair per line.
fn range_dropdown_matches_pob() {
    let (mut s, lua) = gear_sessions("display-range-dropdown");
    let found: NoSlider = s
        .backend
        .lua
        .from_value(s.backend.lua.load(FIND_NO_SLIDER).eval().unwrap())
        .unwrap();
    on_both(&s, &lua, "main.showAllItemAffixes = false");
    s.backend
        .lua
        .load("pob.main.main.showAllItemAffixes = false")
        .exec()
        .unwrap();
    lua.globals().set("text", found.text.clone()).unwrap();
    lua.load("act.paste(text)").exec().unwrap();
    s.run(Command::PasteItem { text: found.text }).unwrap();
    let compare = |s: &mut Session, what: &str| {
        let theirs: serde_json::Value = lua_value(&lua, "return displayDump()");
        let ours = json_of(&display(s).expect("displayed"));
        assert_same(&ours, &strip_nulls(theirs), what);
    };
    compare(&mut s, "pasted");
    // PoB's selection of the range line dropdown and its slider's value, through Lua
    let selection = |s: &Session| -> (u32, Option<f64>) {
        lua_value::<(u32, Option<f64>)>(
            &s.backend.lua,
            "local c = pob.build.itemsTab.controls return {c.displayItemRangeLine.selIndex, c.displayItemRangeSlider.val}",
        )
    };
    let refuse_untouched = |s: &mut Session, command: Command| {
        let at = selection(s);
        s.run(command.clone()).expect_err(&format!("{command:?}"));
        assert_eq!(selection(s), at, "{command:?} moved PoB's selection");
    };
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
        if what == "slider" {
            // the selection is on the slider line now; a refused line leaves it there
            assert_eq!(selection(&s).0 as usize, found.plain + 1);
            refuse_untouched(
                &mut s,
                Command::SetRangeLine {
                    line: found.bare,
                    value: 0.75,
                },
            );
            refuse_untouched(
                &mut s,
                Command::SetRangeLine {
                    line: 99,
                    value: 0.75,
                },
            );
        }
    }
    // the box line is selected now; the line that can't mutate is refused without moving it
    assert_eq!(selection(&s).0 as usize, found.bare + 1);
    refuse_untouched(
        &mut s,
        Command::SetRangeLineMutated {
            line: found.plain,
            on: true,
        },
    );
    // a second slider line after the first, when the item has one
    let before = display(&mut s);
    s.run(Command::SetRangeLine {
        line: found.plain,
        value: 0.75,
    })
    .unwrap();
    lua.load(format!("act.range({}, 0.75)", found.plain))
        .exec()
        .unwrap();
    compare(&mut s, "slider again");
    assert_ne!(display(&mut s), before);
}
