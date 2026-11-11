//! Gate tests for tattoos and node replacement: the engine's popup session on
//! `ModifyNodePopup` and "Remove All Tattoos" against unmodified upstream PoB, and the
//! no-silent-writes check for every command of the crafting work.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::crafting::{craft_ours, display, ours_popup};
use super::items_harness::{open_popups, settle};
use super::{
    Session, TattooState, attack_session, check_recipe_with, fixtures_dir, load_xml, lock,
    lua_value, our_overrides, pob_dir, snapshot_dir, upstream_lua,
};
use auto_pob::engine::{
    Backend, Command, ControlTooltipQuery, DbSnapshot, ItemDb, ItemDbQuery, PopupInput, PopupKind,
};
use std::fs;

/// The tattoo state our side shows at a checkpoint, compared with what upstream recorded.
fn compare_tattoos(s: &mut Session, theirs: &TattooState, what: &str) {
    let ours: std::collections::BTreeMap<u32, String> = our_overrides(s);
    let expected: std::collections::BTreeMap<u32, String> = theirs
        .overrides
        .iter()
        .map(|o| (o.id, o.dn.clone()))
        .collect();
    assert_eq!(ours, expected, "{what}: tattoos");
    let depth: usize = lua_value(&s.backend.lua, "return #pob.build.spec.undo");
    assert_eq!(depth, theirs.undo_depth, "{what}: undo states");
    assert_eq!(
        s.backend.tree_state_view().unwrap().can_undo,
        theirs.can_undo,
        "{what}: can undo"
    );
    assert_eq!(open_popups(s), 0, "{what}: a popup stayed open");
    let layout = s.backend.tree_layout_view().unwrap();
    for node in &theirs.touched {
        let shown = layout
            .nodes
            .iter()
            .find(|n| n.id == node.id)
            .unwrap_or_else(|| panic!("{what}: node {} is not in the layout", node.id));
        assert_eq!(shown.stats, node.sd, "{what}: stats of node {}", node.id);
    }
    let ids = |pick: fn(&auto_pob::engine::TreeNode) -> bool| {
        layout
            .nodes
            .iter()
            .filter(|n| pick(n))
            .map(|n| n.id)
            .collect::<Vec<u32>>()
    };
    assert_eq!(
        ids(|n| n.replaceable),
        theirs.replaceable,
        "{what}: replaceable nodes"
    );
    assert_eq!(
        ids(|n| n.mastery_effects),
        theirs.mastery_effects,
        "{what}: nodes with mastery effects"
    );
}

/// Reads the saved tattoos of the build in `lua`, the global `BUILD` standing for the build.
const OVERRIDES: &str = "local list = {}
    for id, node in pairs(BUILD.spec.hashOverrides) do
        list[#list + 1] = { id = id, dn = node.dn, sd = node.sd, icon = node.icon,
            activeEffectImage = node.activeEffectImage }
    end
    table.sort(list, function(a, b) return a.id < b.id end)
    return list";

/// Nodes of every shape the right-click rule tells apart, tested with both rules.
const SYNTHETIC: &str = "local nodes = {
    { type = 'Keystone', dn = 'K', sd = { 'x' } },
    { type = 'Keystone', dn = 'K', sd = { 'x' }, expansionSkill = true },
    { type = 'Normal', dn = 'Strength', sd = {} },
    { type = 'Normal', dn = 'Strength', sd = {}, expansionSkill = true },
    { type = 'Normal', dn = 'Life', sd = {} },
    { type = 'Notable', dn = 'N', sd = { '+30 to Dexterity' } },
    { type = 'Notable', dn = 'N', sd = { '+30 to Strength' }, expansionSkill = true },
    { type = 'Notable', dn = 'N', sd = { '+10 to Strength' } },
    { type = 'Notable', dn = 'N', sd = {} },
    { type = 'Notable', dn = 'N', sd = { '+30 to Intelligence' } },
    { type = 'Normal', dn = 'Life', sd = {}, isTattoo = true },
    { type = 'Normal', dn = 'Life', sd = {}, isTattoo = true, expansionSkill = true },
    { type = 'Mastery', dn = 'M', sd = {}, masteryEffects = { {} } },
    { type = 'Mastery', dn = 'M', sd = {} },
    { type = 'Socket', dn = 'S', sd = {} },
}
local out = {}
for i, node in ipairs(nodes) do out[i] = { REPLACEABLE(node), MASTERY(node) } end
return out";

#[test]
#[ignore = "needs the pinned PoB install"]
fn tattoo_recipe_matches_pob() {
    let _guard = lock();
    let started = std::time::Instant::now();
    let mut initial = String::new();
    let mut keys = Vec::new();
    let mut counts = Vec::new();
    let recipe = check_recipe_with("tattoos", &mut |s, check| {
        let key = s.backend.tree_layout_view().unwrap().key;
        let Some(check) = check else {
            initial = key;
            return;
        };
        let tattoos = check.tattoos.as_ref().expect("a tattoo checkpoint");
        compare_tattoos(s, tattoos, &format!("checkpoint {}", keys.len()));
        counts.push(tattoos.overrides.len());
        keys.push(key);
    });
    assert_eq!(keys.len(), 5);
    // two tattoos, one reset, the undo's two again, then none
    assert_eq!(counts, [1, 2, 1, 2, 0]);
    assert_ne!(
        keys[1], keys[0],
        "the keystone's tattoo left the layout key"
    );
    assert_ne!(keys[2], keys[1], "Reset Node left the layout key");
    assert_eq!(keys[3], keys[1], "the undo didn't restore the layout");
    assert_ne!(keys[4], keys[3], "Remove All Tattoos left the layout key");
    assert_eq!(keys[4], initial, "the tree isn't back as it was");
    // the sidebar was compared at each: the tattooed Strength node changed it
    let sidebars: Vec<_> = recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            super::TreeAction::Check { check } => check.sidebar.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(sidebars.len(), 5);
    assert_ne!(sidebars[0], sidebars[4], "the tattoos changed nothing");
    // Remove All Tattoos adds no undo state, and Reset Node adds one
    let depths: Vec<usize> = recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            super::TreeAction::Check { check } => check.tattoos.as_ref().map(|t| t.undo_depth),
            _ => None,
        })
        .collect();
    assert_eq!(
        depths[4], depths[3],
        "Remove All Tattoos added an undo state"
    );
    assert_eq!(depths[2], depths[1] + 1, "Reset Node adds one");
    eprintln!("tattoo recipe: {:?}", started.elapsed());

    // the right-click rule: the same answers from PoB's own, on nodes of every shape
    let theirs = {
        let lua = upstream_lua(&pob_dir()).unwrap();
        super::load_tree_lib(&lua);
        lua_value::<Vec<[bool; 2]>>(
            &lua,
            &SYNTHETIC
                .replace("REPLACEABLE", "offersReplacement")
                .replace("MASTERY", "offersMastery"),
        )
    };
    let s = Session::start("tattoo-rule");
    let ours: Vec<[bool; 2]> = lua_value(
        &s.backend.lua,
        &SYNTHETIC
            .replace("REPLACEABLE", "glue.node_replaceable")
            .replace("MASTERY", "glue.node_mastery_effects"),
    );
    assert_eq!(ours, theirs);
    let replaceable: Vec<usize> = (0..ours.len()).filter(|&i| ours[i][0]).collect();
    assert_eq!(replaceable, [0, 2, 5, 9, 10]);
    assert_eq!(
        (0..ours.len()).filter(|&i| ours[i][1]).collect::<Vec<_>>(),
        [12]
    );
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn node_replacement_errors_and_saves() {
    let _guard = lock();
    let mut s = attack_session("tattoo-errors");
    let layout = s.backend.tree_layout_view().unwrap();
    let plain = layout
        .nodes
        .iter()
        .find(|n| {
            n.kind == "Normal"
                && n.ascendancy.is_none()
                && !n.replaceable
                && !["Strength", "Dexterity", "Intelligence"].contains(&n.name.as_str())
        })
        .expect("a plain small passive");
    for id in [plain.id, 0] {
        let error = s
            .run(Command::OpenPopup(PopupKind::NodeReplacement { node: id }))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "PoB doesn't offer that node for replacement"
        );
        assert!(s.backend.popup_view().unwrap().is_none());
        assert_eq!(open_popups(&s), 0);
    }
    // an unallocated mastery with effects opens the same window; once allocated it is the
    // effect picker's, and this window is refused
    let state = s.backend.tree_state_view().unwrap();
    let mastery = layout
        .nodes
        .iter()
        .find(|n| {
            n.mastery_effects
                && state
                    .mastery_options
                    .iter()
                    .any(|o| o.node == n.id && o.options.len() > 1)
        })
        .expect("a reachable mastery with effects");
    assert!(!mastery.replaceable);
    s.run(Command::OpenPopup(PopupKind::NodeReplacement {
        node: mastery.id,
    }))
    .unwrap();
    let view = ours_popup(&mut s);
    assert_eq!(view.kind, PopupKind::NodeReplacement { node: mastery.id });
    assert!(view.controls.iter().any(|c| c.name == "modSelect"));
    s.run(Command::ClosePopup).unwrap();
    let effect = state
        .mastery_options
        .iter()
        .find(|o| o.node == mastery.id)
        .unwrap()
        .options[0]
        .id;
    s.run(Command::SetMastery {
        node: mastery.id,
        effect,
    })
    .unwrap();
    let error = s
        .run(Command::OpenPopup(PopupKind::NodeReplacement {
            node: mastery.id,
        }))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "PoB doesn't offer that node for replacement"
    );
    assert_eq!(open_popups(&s), 0);
    let strength = layout
        .nodes
        .iter()
        .find(|n| n.kind == "Normal" && n.name == "Strength" && n.ascendancy.is_none())
        .expect("a Strength node");
    assert!(strength.replaceable);

    // add a tattoo, save, and let upstream load the file
    s.run(Command::OpenPopup(PopupKind::NodeReplacement {
        node: strength.id,
    }))
    .unwrap();
    let view = ours_popup(&mut s);
    assert_eq!(view.kind, PopupKind::NodeReplacement { node: strength.id });
    let modifier = view
        .controls
        .iter()
        .find(|c| c.name == "modSelect")
        .unwrap();
    assert!(modifier.choices.len() >= 2);
    s.run(Command::SetPopupControl {
        name: "modSelect".into(),
        value: PopupInput::Choice {
            index: 1,
            label: modifier.choices[1].clone(),
        },
    })
    .unwrap();
    let affected = s
        .run(Command::PressPopupButton {
            name: "save".into(),
        })
        .unwrap();
    assert!(affected.tree && affected.build && affected.popup && affected.recalc);
    assert!(affected.tree_layout, "the layout didn't change");
    assert!(s.backend.popup_view().unwrap().is_none());
    assert_eq!(our_overrides(&s).len(), 1);
    s.run(Command::SaveAs {
        folder: String::new(),
        name: "tattooed".into(),
        overwrite: false,
    })
    .unwrap();
    let xml = fs::read_to_string(s.builds_dir().join("tattooed.xml")).unwrap();
    let lua = upstream_lua(&pob_dir()).unwrap();
    load_xml(&lua, &xml, "tattooed").unwrap();
    let theirs: serde_json::Value = lua_value(&lua, &OVERRIDES.replace("BUILD", "build"));
    let ours: serde_json::Value =
        lua_value(&s.backend.lua, &OVERRIDES.replace("BUILD", "pob.build"));
    assert_eq!(theirs.as_array().map(Vec::len), Some(1));
    assert_eq!(ours, theirs, "hashOverrides after the reload");

    // a command that ends the session closes the window, and Remove All needs none
    s.run(Command::OpenPopup(PopupKind::NodeReplacement {
        node: strength.id,
    }))
    .unwrap();
    s.run(Command::AllocNode { id: strength.id }).unwrap();
    assert!(s.backend.popup_view().unwrap().is_none());
    // the UI holds the layout with the tattoo; Remove All changes it back
    s.backend.tree_layout_view().unwrap();
    let affected = s.run(Command::RemoveAllTattoos).unwrap();
    assert!(affected.tree && affected.tree_layout);
    assert!(our_overrides(&s).is_empty());
    assert_eq!(open_popups(&s), 0);
    // and again with none left: PoB's button does nothing, and so do we
    s.run(Command::RemoveAllTattoos).unwrap();
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn no_silent_writes_crafting() {
    let _guard = lock();
    let started = std::time::Instant::now();
    let mut s = Session::start("crafting-no-writes");
    let fixture = fixtures_dir().join("attack.xml");
    s.open_fixture("attack", &fixture);
    let dir = s.builds_dir();
    let before = snapshot_dir(&dir);
    let layout = s.backend.tree_layout_view().unwrap();
    let find = |kind: &str, name: Option<&str>| {
        layout
            .nodes
            .iter()
            .find(|n| {
                n.kind == kind
                    && n.ascendancy.is_none()
                    && n.replaceable
                    && name.is_none_or(|name| n.name == name)
            })
            .map(|n| n.id)
            .unwrap()
    };
    let strength = find("Normal", Some("Strength"));
    let keystone = find("Keystone", None);
    let helmet = "Rarity: RARE\nDoom Crown\nIron Hat\nImplicits: 0\n+60 to maximum Life";
    let amulet = "Rarity: RARE\nDoom Pendant\nOnyx Amulet\nQuality: 0\nImplicits: 1\n{tags:attribute}+12 to all Attributes\n{tags:life}+70 to maximum Life\n{tags:resistance}+30% to Fire Resistance";

    let run = |s: &mut Session, command: Command| {
        let shown = format!("{command:?}");
        s.run(command).unwrap_or_else(|e| panic!("{shown}: {e}"));
        assert_eq!(open_popups(s), 0, "{shown}");
    };
    // sets a dropdown of the open window by a part of the label
    let choose = |s: &mut Session, name: &str, part: &str| {
        let view = ours_popup(s);
        let control = view.controls.iter().find(|c| c.name == name).unwrap();
        let index = control
            .choices
            .iter()
            .position(|c| c.contains(part))
            .unwrap_or_else(|| panic!("{name} has no {part}"));
        run(
            s,
            Command::SetPopupControl {
                name: name.into(),
                value: PopupInput::Choice {
                    index,
                    label: control.choices[index].clone(),
                },
            },
        );
    };

    // the affix editor and its custom mods
    craft_ours(&mut s, "Rare", "Quiet Ring", "Ring", "Two-Stone Ring");
    let shown = display(&mut s);
    // the queries answer, and write nothing
    assert!(
        s.backend
            .control_tooltip_view(&ControlTooltipQuery::Affix {
                slot: 0,
                index: Some(1),
            })
            .unwrap()
            .is_some(),
        "no affix tooltip"
    );
    run(
        &mut s,
        Command::SetAffix {
            slot: 0,
            index: 1,
            label: shown.affixes[0].list.labels[1].clone(),
        },
    );
    run(
        &mut s,
        Command::SetAffixRange {
            slot: 0,
            value: 0.5,
        },
    );
    run(&mut s, Command::SetCraftingSort { index: 1 });
    run(&mut s, Command::OpenPopup(PopupKind::AddModifier));
    choose(&mut s, "source", "Custom");
    run(
        &mut s,
        Command::SetPopupControl {
            name: "custom".into(),
            value: PopupInput::Text("+25 to Strength".into()),
        },
    );
    run(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
    );
    assert_eq!(display(&mut s).custom_mods.len(), 1);
    run(&mut s, Command::RemoveCustomMod { index: 0 });
    run(&mut s, Command::SaveDisplayItem);

    // a cluster jewel
    craft_ours(
        &mut s,
        "Rare",
        "Quiet Cluster",
        "Jewel: Cluster",
        "Large Cluster Jewel",
    );
    run(&mut s, Command::SetClusterSkill { index: 0 });
    run(&mut s, Command::SetClusterNodeCount { count: 10 });
    run(&mut s, Command::SaveDisplayItem);

    // enchant, anoint and corrupt
    run(
        &mut s,
        Command::PasteItem {
            text: helmet.into(),
        },
    );
    run(
        &mut s,
        Command::OpenPopup(PopupKind::Enchant { second: false }),
    );
    choose(&mut s, "skill", "Boneshatter");
    choose(&mut s, "enchantment", "");
    run(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
    );
    run(&mut s, Command::SaveDisplayItem);
    run(
        &mut s,
        Command::PasteItem {
            text: amulet.into(),
        },
    );
    run(&mut s, Command::OpenPopup(PopupKind::Anoint { slot: 0 }));
    run(
        &mut s,
        Command::SetPopupControl {
            name: "notableDB.search".into(),
            value: PopupInput::Text("Strength".into()),
        },
    );
    choose(&mut s, "notableDB.sort", "Sort by Life");
    settle(&mut s);
    let key = ours_popup(&mut s)
        .controls
        .iter()
        .find(|c| c.name == "notableDB")
        .and_then(|c| c.rows.first())
        .map(|r| r.key.clone())
        .expect("an anoint row");
    run(
        &mut s,
        Command::SetPopupControl {
            name: "notableDB".into(),
            value: PopupInput::Row(key),
        },
    );
    run(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
    );
    run(&mut s, Command::OpenPopup(PopupKind::Corrupt));
    choose(&mut s, "implicit1", "");
    run(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
    );
    run(&mut s, Command::SaveDisplayItem);

    // the item databases
    for (db, search) in [(ItemDb::Uniques, "Kaom's Heart"), (ItemDb::Rares, "")] {
        run(
            &mut s,
            Command::SetItemDb(ItemDbQuery {
                db,
                slot: 0,
                kind: 0,
                league: 0,
                requirement: 0,
                obtainable: 0,
                sort: 0,
                search: search.into(),
                search_mode: 0,
            }),
        );
        settle(&mut s);
        let DbSnapshot::View(view) = s.backend.item_db_view().unwrap() else {
            panic!("no panel");
        };
        let name = view.rows.first().expect("a database row").name.clone();
        assert!(
            s.backend
                .control_tooltip_view(&ControlTooltipQuery::DbItem {
                    db,
                    name: name.clone(),
                })
                .unwrap()
                .is_some(),
            "no database tooltip"
        );
        if db == ItemDb::Uniques {
            run(&mut s, Command::ViewDbItem { db, name });
            run(&mut s, Command::SaveDisplayItem);
        } else {
            run(&mut s, Command::AddDbItem { db, name });
        }
    }
    run(&mut s, Command::CloseItemDb);

    // tattoos
    run(
        &mut s,
        Command::OpenPopup(PopupKind::NodeReplacement { node: strength }),
    );
    assert!(s.backend.popup_view().unwrap().is_some());
    assert!(
        s.backend
            .control_tooltip_view(&ControlTooltipQuery::Popup {
                control: "modSelect".into(),
                index: Some(0),
            })
            .unwrap()
            .is_some(),
        "no tattoo tooltip"
    );
    choose(&mut s, "modSelect", "");
    run(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
    );
    run(
        &mut s,
        Command::OpenPopup(PopupKind::NodeReplacement { node: keystone }),
    );
    run(
        &mut s,
        Command::SetPopupControl {
            name: "showLegacyTattoo".into(),
            value: PopupInput::Check(true),
        },
    );
    run(&mut s, Command::ClosePopup);
    run(
        &mut s,
        Command::OpenPopup(PopupKind::NodeReplacement { node: strength }),
    );
    run(
        &mut s,
        Command::PressPopupButton {
            name: "reset".into(),
        },
    );
    run(&mut s, Command::RemoveAllTattoos);

    let after = snapshot_dir(&dir);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
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
        assert!(after[path] == *bytes, "{} changed", path.display());
    }
    // only an explicit Save writes
    s.run(Command::Save).unwrap();
    let saved = snapshot_dir(&dir);
    assert!(
        before.iter().any(|(path, bytes)| saved[path] != *bytes),
        "Save didn't write the build"
    );
    eprintln!("no_silent_writes_crafting: {:?}", started.elapsed());
}
