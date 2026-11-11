//! Gate tests for the crafting popups: the engine's popup sessions against unmodified
//! upstream PoB, which keeps the popup in `main.popups` and acts on its controls.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::items::gear_sessions;
use super::items_harness::{
    ItemsRecipe, assert_same, checkpoints, first_difference, json, json_of, open_popups,
    raws_and_jewels, run_items_recipe, strip_nulls, upstream_json, upstream_with_items_lib,
};
use super::{Session, lock, lua_value};
use auto_pob::engine::{
    Backend, Command, ControlTooltipQuery, DisplayItemView, PopupControlKind, PopupInput,
    PopupKind, PopupView,
};
use mlua::LuaSerdeExt;
use std::collections::BTreeSet;

const AMULET: &str = "Rarity: RARE\nDoom Pendant\nOnyx Amulet\nQuality: 0\nImplicits: 1\n{tags:attribute}+12 to all Attributes\n{tags:life}+70 to maximum Life\n{tags:resistance}+30% to Fire Resistance";
const HELMET: &str = "Rarity: RARE\nDoom Crown\nIron Hat\nImplicits: 0\n+60 to maximum Life";
const UNIQUE_BODY: &str = "Rarity: UNIQUE\nKaom's Test\nGlorious Plate\nImplicits: 0\n+(400-500) to maximum Life\n+(10-20)% to Fire Resistance";
const WEAPON: &str = "Rarity: RARE\nDoom Edge\nRusted Sword\nImplicits: 0\n+10 to Strength";
const TINCTURE: &str = "Rarity: RARE\nDoom Draught\nPrismatic Tincture\nImplicits: 0";

/// Evaluates `code` upstream; none for nil
fn upstream_value(lua: &mlua::Lua, code: &str) -> Option<serde_json::Value> {
    let value: mlua::Value = lua
        .load(code)
        .eval()
        .unwrap_or_else(|e| panic!("{code}: {e}"));
    match value {
        mlua::Value::Nil => None,
        value => Some(strip_nulls(lua.from_value(value).unwrap())),
    }
}

/// Runs `code` upstream, naming the error
fn upstream_run(lua: &mlua::Lua, code: &str) {
    lua.load(code)
        .exec()
        .unwrap_or_else(|e| panic!("{code}: {e}"));
}

fn paste_both(s: &mut Session, lua: &mlua::Lua, text: &str) {
    s.run(Command::PasteItem { text: text.into() }).unwrap();
    lua.globals().set("TEXT", text).unwrap();
    upstream_run(lua, "act.paste(TEXT)");
}

/// The button of the Items tab that opens a window
fn button(kind: &PopupKind) -> String {
    match kind {
        PopupKind::Craft => "craftDisplayItem".into(),
        PopupKind::Enchant { second: false } => "displayItemEnchant".into(),
        PopupKind::Enchant { second: true } => "displayItemEnchant2".into(),
        PopupKind::Anoint { slot: 0 } => "displayItemAnoint".into(),
        PopupKind::Anoint { slot } => format!("displayItemAnoint{}", slot + 1),
        PopupKind::Corrupt => "displayItemCorrupt".into(),
        PopupKind::AddModifier => "displayItemAddCustom".into(),
        PopupKind::AddImplicit => "displayItemAddImplicit".into(),
        PopupKind::AddCrucible => "displayItemAddCrucible".into(),
        other => panic!("no button for {other:?}"),
    }
}

fn ours_popup(s: &mut Session) -> PopupView {
    s.backend
        .popup_view()
        .expect("popup view")
        .expect("a popup is open")
}

/// A popup with Corrupt's implicit dropdowns' choices in a fixed order and the selected
/// choice as its label. PoB builds those lists from `pairs` and sorts them with a comparison
/// that has ties (scourge mods of one level), so two states can order the tied entries
/// differently. Every other control is compared as it is.
fn canonical(mut popup: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    if let Some(controls) = popup["controls"].as_array_mut() {
        for control in controls {
            let name = control["name"].as_str().unwrap_or_default();
            if !["implicit1", "implicit2", "implicit3", "implicit4"].contains(&name) {
                continue;
            }
            if let Some(Value::Array(mut choices)) = control.get("choices").cloned() {
                let selected = control["selected"].as_u64().unwrap_or(0) as usize;
                control["selected"] = choices.get(selected).cloned().unwrap_or(Value::Null);
                choices.sort_by_key(|c| c.to_string());
                control["choices"] = Value::Array(choices);
            }
        }
    }
    popup
}

/// Equal views; where the order of tied entries differs, equal in `canonical` form. Returns
/// whether the order was the same too.
fn same_popup(ours: &serde_json::Value, theirs: &serde_json::Value, what: &str) -> bool {
    if first_difference("", ours, theirs).is_none() {
        return true;
    }
    assert_same(&canonical(ours.clone()), &canonical(theirs.clone()), what);
    false
}

/// The position of the entry `label` in upstream's dropdown `name`
fn upstream_index(lua: &mlua::Lua, name: &str, label: &str) -> usize {
    let theirs = upstream_value(lua, "return popupDump()").expect("upstream has no popup open");
    theirs["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .and_then(|c| c["choices"].as_array())
        .and_then(|choices| choices.iter().position(|c| c == label))
        .unwrap_or_else(|| panic!("upstream's {name} has no entry {label}"))
}

/// Both sides' open popup, equal
fn check_popup(s: &mut Session, lua: &mlua::Lua, what: &str) -> PopupView {
    let view = ours_popup(s);
    let theirs = upstream_value(lua, "return popupDump()").expect("upstream has no popup open");
    if !same_popup(&json_of(&view), &theirs, what) {
        eprintln!("{what}: tied entries are in another order upstream");
    }
    assert_eq!(open_popups(s), 0, "{what}: a popup is left in PoB's stack");
    view
}

/// What the compared popups held, so a comparison of defaults can't pass for the real thing
#[derive(Default)]
struct Seen {
    kinds: BTreeSet<String>,
    hidden: bool,
    disabled_dropdown: bool,
    tooltip_text: bool,
    tooltips: usize,
    rows: bool,
    row_selected: bool,
    note: bool,
    slider_moved: bool,
    divisions: bool,
    checked: bool,
    text: bool,
    selected: bool,
}

impl Seen {
    fn note(&mut self, view: &PopupView) {
        for control in &view.controls {
            self.kinds.insert(format!("{:?}", control.kind));
            self.hidden |= !control.shown;
            self.disabled_dropdown |= control.name == "source"
                && control.kind == PopupControlKind::Dropdown
                && control.shown
                && !control.enabled;
            self.tooltip_text |= control.tooltip_text.is_some();
            self.rows |= !control.rows.is_empty();
            self.row_selected |= control.row_selected.is_some();
            self.note |= control.note.is_some();
            self.slider_moved |= control.kind == PopupControlKind::Slider && control.value > 0.0;
            self.divisions |= control.divisions.is_some();
            self.checked |= control.checked;
            self.text |= !control.text.is_empty();
            self.selected |= control.selected > 0;
        }
    }
}

/// An input for a control of `kind` that the view shows usable, and that changes it
fn pick_input(view: &PopupView, kind: PopupControlKind) -> Option<(String, PopupInput)> {
    view.controls
        .iter()
        .filter(|c| c.kind == kind && c.shown && c.enabled)
        .find_map(|c| {
            let input = match kind {
                // a stat sort calculates every entry; it is not what this test is about
                PopupControlKind::Dropdown if !c.name.ends_with("sort") && c.choices.len() >= 2 => {
                    let index = if c.selected + 1 == c.choices.len() {
                        0
                    } else {
                        c.choices.len() - 1
                    };
                    PopupInput::Choice {
                        index,
                        label: c.choices[index].clone(),
                    }
                }
                PopupControlKind::Check => PopupInput::Check(!c.checked),
                PopupControlKind::Edit => PopupInput::Text("a typed line".into()),
                PopupControlKind::Slider => PopupInput::Number(0.75),
                PopupControlKind::List => PopupInput::Row(c.rows.first()?.key.clone()),
                _ => return None,
            };
            Some((c.name.clone(), input))
        })
}

/// Calls upstream's `act.<name>(args)`
fn upstream_act(lua: &mlua::Lua, name: &str, args: impl mlua::IntoLuaMulti) {
    let act: mlua::Table = lua.globals().get("act").unwrap();
    let function: mlua::Function = act.get(name).unwrap();
    function
        .call::<()>(args)
        .unwrap_or_else(|e| panic!("act.{name}: {e}"));
}

fn set_both(s: &mut Session, lua: &mlua::Lua, name: &str, value: PopupInput) {
    s.run(Command::SetPopupControl {
        name: name.into(),
        value: value.clone(),
    })
    .unwrap_or_else(|e| panic!("set {name} to {value:?}: {e}"));
    // upstream picks by its own index of the label shown (see `canonical`)
    let theirs = match &value {
        PopupInput::Choice { label, .. } => PopupInput::Choice {
            index: upstream_index(lua, name, label),
            label: label.clone(),
        },
        other => other.clone(),
    };
    upstream_act(lua, "popup_set", (name, lua.to_value(&theirs).unwrap()));
}

fn press_both(s: &mut Session, lua: &mlua::Lua, name: &str) {
    s.run(Command::PressPopupButton { name: name.into() })
        .unwrap_or_else(|e| panic!("press {name}: {e}"));
    upstream_act(lua, "popup_press", name);
}

/// The tooltips of the first, middle and last entry of every dropdown that builds one, and of
/// the buttons that do, equal upstream's
fn check_tooltips(s: &mut Session, lua: &mlua::Lua, view: &PopupView, seen: &mut Seen, what: &str) {
    for control in view.controls.iter().filter(|c| c.shown && c.hover_tooltip) {
        let indexes: Vec<Option<usize>> = match control.kind {
            PopupControlKind::Dropdown if !control.choices.is_empty() => {
                let last = control.choices.len() - 1;
                [0, last / 2, last]
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(Some)
                    .collect()
            }
            PopupControlKind::Button => vec![None],
            _ => continue,
        };
        for index in indexes {
            let query = ControlTooltipQuery::Popup {
                control: control.name.clone(),
                index,
            };
            let ours = s.backend.control_tooltip_view(&query).unwrap();
            let theirs = upstream_value(
                lua,
                &match index {
                    Some(i) => format!(
                        "return popupTooltip({:?}, {})",
                        control.name,
                        upstream_index(lua, &control.name, &control.choices[i])
                    ),
                    None => format!("return popupTooltip({:?})", control.name),
                },
            );
            let label = format!("{what}: tooltip {} {index:?}", control.name);
            match (ours, theirs) {
                (Some(ours), Some(theirs)) => {
                    assert_eq!(ours.query, query);
                    assert_same(&json_of(&ours.tooltip), &theirs, &label);
                    seen.tooltips += usize::from(!ours.tooltip.lines.is_empty());
                }
                (ours, theirs) => panic!("{label}: ours {ours:?}, upstream {theirs:?}"),
            }
        }
    }
}

/// Opens `kind` on both sides and compares the views: as opened, after one setter per control
/// kind that is usable, and the tooltips. The popup is closed on both sides at the end.
fn compare_opener(s: &mut Session, lua: &mlua::Lua, kind: PopupKind, seen: &mut Seen, what: &str) {
    let what = format!("{what}: {kind:?}");
    s.run(Command::OpenPopup(kind.clone()))
        .unwrap_or_else(|e| panic!("{what}: open: {e}"));
    upstream_act(lua, "open_popup", button(&kind));
    let view = check_popup(s, lua, &format!("{what} as opened"));
    assert_eq!(view.kind, kind);
    seen.note(&view);
    // a unique's Corrupt popup has two pages; look at the roll ranges as well
    let mut pages = vec![String::new()];
    if kind == PopupKind::Corrupt && view.controls.iter().any(|c| c.name == "rolls" && c.shown) {
        pages.push("rolls".into());
        pages.push("implicits".into());
    }
    for page in pages {
        if !page.is_empty() {
            press_both(s, lua, &page);
            let view = check_popup(s, lua, &format!("{what} {page} page"));
            seen.note(&view);
        }
        for control_kind in [
            PopupControlKind::Dropdown,
            PopupControlKind::Check,
            PopupControlKind::Slider,
            PopupControlKind::List,
            PopupControlKind::Edit,
        ] {
            let current = ours_popup(s);
            let Some((name, input)) = pick_input(&current, control_kind) else {
                continue;
            };
            set_both(s, lua, &name, input.clone());
            let label = format!("{what} {page} after {name} = {input:?}");
            let after = check_popup(s, lua, &label);
            assert_ne!(after, current, "{label}: the view didn't change");
            seen.note(&after);
        }
        let view = ours_popup(s);
        check_tooltips(s, lua, &view, seen, &format!("{what} {page}"));
    }
    s.run(Command::ClosePopup).unwrap();
    upstream_act(lua, "popup_close", ());
    assert!(s.backend.popup_view().unwrap().is_none());
    assert_eq!(open_popups(s), 0);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn popup_views_match_pob() {
    let _guard = lock();
    let (mut s, lua) = gear_sessions("popup-views");
    let mut seen = Seen::default();

    // no item is displayed: the Craft window
    assert!(s.backend.display_item_view().unwrap().is_none());
    compare_opener(&mut s, &lua, PopupKind::Craft, &mut seen, "no item");

    let mut offered = BTreeSet::new();
    for (name, text) in [
        ("rare amulet", AMULET),
        ("rare helmet", HELMET),
        ("unique body armour", UNIQUE_BODY),
        ("rare weapon", WEAPON),
        ("rare tincture", TINCTURE),
    ] {
        paste_both(&mut s, &lua, text);
        let view = s.backend.display_item_view().unwrap().expect("displayed");
        let theirs = upstream_value(&lua, "return displayDump().popups").unwrap();
        assert_same(&json_of(&view.popups), &theirs, &format!("{name}: popups"));
        assert!(!view.popups.is_empty(), "{name}: PoB offers no window");
        for kind in view.popups.clone() {
            offered.insert(format!("{kind:?}"));
            compare_opener(&mut s, &lua, kind, &mut seen, name);
        }
    }
    eprintln!("windows compared: {offered:?}");

    for kind in [
        "Label", "Button", "Dropdown", "Slider", "Check", "Edit", "List",
    ] {
        assert!(seen.kinds.contains(kind), "no {kind} control was compared");
    }
    assert!(seen.hidden, "no hidden control was compared");
    assert!(seen.disabled_dropdown, "no disabled dropdown was compared");
    assert!(seen.tooltip_text, "no static tooltip text was compared");
    assert!(seen.tooltips > 0, "no popup tooltip had lines");
    assert!(seen.rows, "no list had rows");
    assert!(seen.row_selected, "no list row was selected");
    assert!(seen.note, "no list had a note");
    assert!(seen.slider_moved, "no slider left its start");
    assert!(seen.checked, "no check box was on");
    assert!(seen.text, "no edit box held text");
    assert!(seen.selected, "no dropdown left its first entry");
    // none of these popups sets a slider's division count
    eprintln!("slider divisions compared: {}", seen.divisions);
    // the windows the four items and the tincture offer between them
    for kind in [
        "Enchant",
        "Anoint",
        "Corrupt",
        "AddModifier",
        "AddImplicit",
        "AddCrucible",
    ] {
        assert!(
            offered.iter().any(|o| o.starts_with(kind)),
            "no item offered {kind}: {offered:?}"
        );
    }
}

/// Runs `command`, which must fail with a message containing `expected`, and checks that the
/// popup and the display item are as they were.
fn refused(s: &mut Session, command: Command, expected: &str) {
    let popup = s.backend.popup_view().unwrap().map(|v| json_of(&v));
    let display = s.backend.display_item_view().unwrap().map(|v| json_of(&v));
    let what = format!("{command:?}");
    let error = s
        .run(command)
        .expect_err(&format!("{what} should be refused"));
    assert!(
        error.to_string().contains(expected),
        "{what}: {error} doesn't say {expected:?}"
    );
    assert_eq!(
        s.backend.popup_view().unwrap().map(|v| json_of(&v)),
        popup,
        "{what} changed the popup"
    );
    assert_eq!(
        s.backend.display_item_view().unwrap().map(|v| json_of(&v)),
        display,
        "{what} changed the display item"
    );
    assert_eq!(open_popups(s), 0);
}

fn open(s: &mut Session, kind: PopupKind) -> PopupView {
    s.run(Command::OpenPopup(kind)).unwrap();
    ours_popup(s)
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn popup_session_lifecycle() {
    let _guard = lock();
    let (mut s, _lua) = gear_sessions("popup-lifecycle");
    let gear = s.builds_dir().join("gear.xml");
    let call =
        |s: &Session, code: &str| -> mlua::Value { s.backend.lua.load(code).eval().unwrap() };

    // an Enchant session is no popup of PoB's: tooltips that PoB blanks behind a popup work,
    // and a frame draws nothing (Review Focus 1)
    s.run(Command::PasteItem {
        text: HELMET.into(),
    })
    .unwrap();
    let view = open(&mut s, PopupKind::Enchant { second: false });
    assert_eq!(view.kind, PopupKind::Enchant { second: false });
    let items = s.backend.items_view().unwrap();
    let id = items.items[0].id;
    let tooltip = s
        .backend
        .item_tooltip_view(&auto_pob::engine::ItemTooltipQuery { id, slot: None })
        .unwrap()
        .expect("the item is listed");
    assert!(
        !tooltip.tooltip.lines.is_empty(),
        "the item list's tooltip is blank behind the session"
    );
    assert!(s.backend.popup_view().unwrap().is_some());
    let drawn: (i64, i64) = s
        .backend
        .lua
        .load(
            r#"
            local class = common.classes.PopupDialog
            local draw = class.Draw
            local draws = 0
            class.Draw = function(...) draws = draws + 1 return draw(...) end
            glue.frame()
            class.Draw = draw
            return draws, #pob.main.main.popups
            "#,
        )
        .eval()
        .unwrap();
    assert_eq!(drawn, (0, 0), "a frame drew a popup or left one open");
    assert_eq!(open_popups(&s), 0);
    assert!(
        s.backend.popup_view().unwrap().is_some(),
        "a frame ended the session"
    );

    // a command that is neither a session command nor a query ends the session
    let affected = s.run(Command::SortItems).unwrap();
    assert!(affected.popup);
    assert!(s.backend.popup_view().unwrap().is_none());
    // and so does opening another build
    open(&mut s, PopupKind::Enchant { second: false });
    let affected = s.run(Command::OpenBuild { path: gear }).unwrap();
    assert!(affected.popup);
    assert!(s.backend.popup_view().unwrap().is_none());
    assert!(s.backend.display_item_view().unwrap().is_none());

    // Craft's Create makes an Unique, whose text editor takes the session over (R2)
    let view = open(&mut s, PopupKind::Craft);
    let rarity = view
        .controls
        .iter()
        .find(|c| c.name == "rarity")
        .expect("a rarity dropdown");
    let unique = rarity
        .choices
        .iter()
        .position(|c| c.ends_with("Unique"))
        .expect("Unique is offered");
    s.run(Command::SetPopupControl {
        name: "rarity".into(),
        value: PopupInput::Choice {
            index: unique,
            label: rarity.choices[unique].clone(),
        },
    })
    .unwrap();
    let affected = s
        .run(Command::PressPopupButton {
            name: "save".into(),
        })
        .unwrap();
    assert!(affected.popup && affected.display_item);
    let view = ours_popup(&mut s);
    assert_eq!(view.kind, PopupKind::EditText);
    assert!(view.controls.iter().any(|c| c.name == "edit"));
    assert_eq!(open_popups(&s), 0);
    s.run(Command::ClosePopup).unwrap();
    assert!(s.backend.popup_view().unwrap().is_none());
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();

    // a press that raises keeps the session (spec 6)
    s.run(Command::PasteItem {
        text: HELMET.into(),
    })
    .unwrap();
    let before = open(&mut s, PopupKind::Enchant { second: false });
    call(
        &s,
        "pob.build.itemsTab.SetDisplayItem = function() error('forced', 0) end",
    );
    refused(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
        "forced",
    );
    call(&s, "pob.build.itemsTab.SetDisplayItem = nil");
    assert_eq!(
        ours_popup(&mut s),
        before,
        "the failed press changed the popup"
    );
    s.run(Command::ClosePopup).unwrap();

    // refusals leave the popup and the item alone (Review Focus 3)
    s.run(Command::PasteItem {
        text: AMULET.into(),
    })
    .unwrap();
    let view = open(&mut s, PopupKind::AddModifier);
    let source = view.controls.iter().find(|c| c.name == "source").unwrap();
    assert!(source.enabled && source.choices.len() > 2);
    let custom = view.controls.iter().find(|c| c.name == "custom").unwrap();
    assert!(
        !custom.shown,
        "the custom box shows before Custom is picked"
    );
    let set = |name: &str, value| Command::SetPopupControl {
        name: name.into(),
        value,
    };
    refused(
        &mut s,
        set("nothing", PopupInput::Check(true)),
        "no such control",
    );
    refused(
        &mut s,
        set("custom", PopupInput::Text("+1 to Life".into())),
        "doesn't show that control",
    );
    refused(
        &mut s,
        set(
            "source",
            PopupInput::Choice {
                index: 1,
                label: "not what is listed".into(),
            },
        ),
        "the list changed; pick again",
    );
    refused(
        &mut s,
        Command::OpenPopup(PopupKind::Craft),
        "doesn't show that control",
    );
    refused(
        &mut s,
        Command::OpenPopup(PopupKind::Anoint { slot: 1 }),
        "doesn't show that control",
    );
    // the session survived all of that
    assert_eq!(ours_popup(&mut s), view);
    s.run(Command::ClosePopup).unwrap();
    refused(
        &mut s,
        Command::PressPopupButton {
            name: "save".into(),
        },
        "no popup is open",
    );

    // a dropdown PoB disables (a tincture can only take custom lines)
    s.run(Command::PasteItem {
        text: TINCTURE.into(),
    })
    .unwrap();
    let view = open(&mut s, PopupKind::AddModifier);
    let source = view.controls.iter().find(|c| c.name == "source").unwrap();
    assert!(!source.enabled && source.choices.len() == 1);
    refused(
        &mut s,
        set(
            "source",
            PopupInput::Choice {
                index: 0,
                label: source.choices[0].clone(),
            },
        ),
        "doesn't show that control",
    );
    s.run(Command::ClosePopup).unwrap();
    assert_eq!(open_popups(&s), 0);
}

// ---- The crafter ----

/// Ends a recipe's test: saves the build, loads the file in upstream PoB and compares what it
/// holds with the recipe's end state and ours (R24). The undo history and the list filter are
/// not saved.
pub(crate) fn crafting_round_trip(name: &str, recipe: &ItemsRecipe, s: &mut Session) {
    s.run(Command::Save).unwrap();
    let saved = s.backend.build_view().unwrap().path.expect("a saved build");
    let lua = upstream_with_items_lib(&saved, &recipe.base);
    let saved_part = |mut value: serde_json::Value| {
        for key in ["can_undo", "can_redo", "list_filter"] {
            value.as_object_mut().unwrap().remove(key);
        }
        value
    };
    let ours = saved_part(json(&s.backend.items_view().unwrap()));
    let reloaded = saved_part(upstream_json(&lua));
    assert_same(
        &reloaded,
        &saved_part(recipe.result.clone()),
        &format!("{name}: reloaded vs recipe"),
    );
    assert_same(&ours, &reloaded, &format!("{name}: ours vs reloaded"));
    let (raws, jewels) = raws_and_jewels(&lua, "build");
    assert_eq!(raws, recipe.raws, "{name}: item text after reload");
    assert_eq!(jewels, recipe.jewels, "{name}: jewels after reload");
    let (ours_raws, ours_jewels) = raws_and_jewels(&s.backend.lua, "pob.build");
    assert_eq!(ours_raws, raws, "{name}: our item text");
    assert_eq!(ours_jewels, jewels, "{name}: our jewels");
    assert!(!raws.is_empty(), "{name}: no items");
    let overrides = |lua: &mlua::Lua, build: &str| -> serde_json::Value {
        strip_nulls(lua_value(
            lua,
            &format!("return {build}.spec.hashOverrides or {{}}"),
        ))
    };
    assert_eq!(
        overrides(&s.backend.lua, "pob.build"),
        overrides(&lua, "build"),
        "{name}: hash overrides"
    );
}

/// Crafts an item through the Craft window, as the panel's button does
fn craft_ours(s: &mut Session, rarity: &str, title: &str, kind: &str, base: &str) {
    s.run(Command::OpenPopup(PopupKind::Craft)).unwrap();
    for (name, wanted) in [("rarity", rarity), ("type", kind)] {
        let view = ours_popup(s);
        let control = view.controls.iter().find(|c| c.name == name).unwrap();
        let index = control
            .choices
            .iter()
            .position(|c| c.ends_with(wanted))
            .unwrap_or_else(|| panic!("no {wanted}"));
        s.run(Command::SetPopupControl {
            name: name.into(),
            value: PopupInput::Choice {
                index,
                label: control.choices[index].clone(),
            },
        })
        .unwrap();
    }
    s.run(Command::SetPopupControl {
        name: "title".into(),
        value: PopupInput::Text(title.into()),
    })
    .unwrap();
    let view = ours_popup(s);
    let control = view.controls.iter().find(|c| c.name == "base").unwrap();
    let index = control
        .choices
        .iter()
        .position(|c| c.contains(base))
        .unwrap_or_else(|| panic!("no base {base}"));
    s.run(Command::SetPopupControl {
        name: "base".into(),
        value: PopupInput::Choice {
            index,
            label: control.choices[index].clone(),
        },
    })
    .unwrap();
    s.run(Command::PressPopupButton {
        name: "save".into(),
    })
    .unwrap();
}

fn display(s: &mut Session) -> DisplayItemView {
    s.backend
        .display_item_view()
        .unwrap()
        .expect("an item is displayed")
}

/// Runs `command`, which must fail with a message containing `expected`, and checks that
/// the display item is as it was.
fn refused_display(s: &mut Session, command: Command, expected: &str) {
    let before = s.backend.display_item_view().unwrap().map(|v| json_of(&v));
    let what = format!("{command:?}");
    let error = s
        .run(command)
        .expect_err(&format!("{what} should be refused"));
    assert!(
        error.to_string().contains(expected),
        "{what}: {error} doesn't say {expected:?}"
    );
    assert_eq!(
        s.backend.display_item_view().unwrap().map(|v| json_of(&v)),
        before,
        "{what} changed the display item"
    );
    assert_eq!(open_popups(s), 0);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn craft_recipes_match_pob() {
    let _guard = lock();
    let mut tiers = false;
    let mut ranges = false;
    let mut sorted = false;
    let mut counted = false;
    let mut tooltips = 0;
    let mut slider_tooltips = 0;
    let mut suffix = false;
    let mut kept = None;
    for name in ["craft_rare", "cluster"] {
        let (recipe, mut s) = run_items_recipe(name);
        for check in checkpoints(&recipe) {
            // the checkpoint after Save has no display item
            let Some(display) = check.display.as_ref() else {
                continue;
            };
            for affix in display["affixes"].as_array().unwrap() {
                tiers |= affix.get("tiers").is_some();
                ranges |= affix.get("range").is_some();
                suffix |= affix["label"] == "Suffix:";
            }
            sorted |= display["crafting_sort"]["selected"]
                .as_u64()
                .is_some_and(|i| i > 0);
            counted |= display["cluster"]["count"].as_u64().is_some_and(|c| c > 0);
            for recorded in &check.affix_tooltips {
                if recorded.tooltip["lines"]
                    .as_array()
                    .is_some_and(|l| !l.is_empty())
                {
                    tooltips += 1;
                    slider_tooltips += usize::from(recorded.index.is_none());
                }
            }
        }
        crafting_round_trip(name, &recipe, &mut s);
        if name == "craft_rare" {
            kept = Some(s);
        }
    }
    assert!(
        tiers && ranges && suffix,
        "no affix had a slider or a suffix label"
    );
    assert!(sorted, "the modifier sorting never left its default");
    assert!(counted, "no cluster jewel was compared");
    assert!(tooltips >= 6 && slider_tooltips > 0, "{tooltips} tooltips");

    // the errors (Review Focus 3), in craft_rare's session
    let mut s = kept.unwrap();
    assert!(
        s.backend
            .lua
            .load("return pob.build.itemsTab.selControl == nil")
            .eval::<bool>()
            .unwrap(),
        "selControl is set"
    );
    craft_ours(&mut s, "Rare", "Error Ring", "Ring", "Two-Stone Ring");
    let before = display(&mut s);
    assert_eq!(before.affixes.len(), 6);
    assert!(before.crafting_sort.is_some() && before.cluster.is_none());
    let old = before.affixes[1].list.labels.clone();
    s.run(Command::SetAffix {
        slot: 0,
        index: 1,
        label: before.affixes[0].list.labels[1].clone(),
    })
    .unwrap();
    let after = display(&mut s);
    assert_eq!(after.affixes[0].list.selected, 1);
    // two picks queued in one batch: the second's list was rebuilt by the first
    let moved = old
        .iter()
        .zip(&after.affixes[1].list.labels)
        .position(|(a, b)| a != b)
        .expect("the first pick changed slot 1's list");
    refused_display(
        &mut s,
        Command::SetAffix {
            slot: 1,
            index: moved,
            label: old[moved].clone(),
        },
        "the list changed; pick again",
    );
    refused_display(
        &mut s,
        Command::SetAffix {
            slot: 8,
            index: 0,
            label: "None".into(),
        },
        "doesn't show that control",
    );
    refused_display(
        &mut s,
        Command::SetAffixRange {
            slot: 8,
            value: 0.5,
        },
        "doesn't show that control",
    );
    // slot 5 is empty: its slider is hidden
    assert!(after.affixes[5].range.is_none());
    refused_display(
        &mut s,
        Command::SetAffixRange {
            slot: 5,
            value: 0.5,
        },
        "doesn't show that control",
    );
    refused_display(
        &mut s,
        Command::SetAffix {
            slot: 1,
            index: 999,
            label: "None".into(),
        },
        "no such choice",
    );
    refused_display(
        &mut s,
        Command::SetCraftingSort { index: 999 },
        "no such choice",
    );
    refused_display(
        &mut s,
        Command::SetClusterSkill { index: 0 },
        "doesn't show that control",
    );
    refused_display(
        &mut s,
        Command::SetClusterNodeCount { count: 10 },
        "doesn't show that control",
    );
    // a tooltip of a vanished control answers nothing
    for query in [
        ControlTooltipQuery::Affix {
            slot: 8,
            index: Some(0),
        },
        ControlTooltipQuery::Affix {
            slot: 5,
            index: None,
        },
        ControlTooltipQuery::Affix {
            slot: 1,
            index: Some(999),
        },
    ] {
        assert!(s.backend.control_tooltip_view(&query).unwrap().is_none());
    }
    // no display item: every command is refused
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
    for command in [
        Command::SetAffix {
            slot: 0,
            index: 0,
            label: "None".into(),
        },
        Command::SetAffixRange {
            slot: 0,
            value: 0.5,
        },
        Command::SetCraftingSort { index: 1 },
        Command::SetClusterSkill { index: 0 },
        Command::SetClusterNodeCount { count: 10 },
    ] {
        refused_display(&mut s, command, "no item is being viewed");
    }
    assert!(
        s.backend
            .control_tooltip_view(&ControlTooltipQuery::Affix {
                slot: 0,
                index: Some(0)
            })
            .unwrap()
            .is_none()
    );
    assert_eq!(open_popups(&s), 0);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn mods_recipe_matches_pob() {
    let _guard = lock();
    let (recipe, mut s) = run_items_recipe("mods");
    let mut kinds = BTreeSet::new();
    let mut most = 0;
    for check in checkpoints(&recipe) {
        let Some(display) = check.display.as_ref() else {
            continue;
        };
        let mods = display["custom_mods"].as_array().unwrap();
        most = most.max(mods.len());
        for m in mods {
            assert!(
                !m["line"].as_str().unwrap().contains("..."),
                "a label was cut: {m}"
            );
            kinds.insert(m["kind"].as_str().unwrap().to_string());
        }
    }
    assert_eq!(
        kinds,
        ["Crafted", "Crucible", "Custom"]
            .map(String::from)
            .into_iter()
            .collect(),
        "every kind is compared"
    );
    assert_eq!(most, 3);
    crafting_round_trip("mods", &recipe, &mut s);

    // the errors: no such row, and no item
    craft_ours(&mut s, "Rare", "Mod Ring", "Ring", "Two-Stone Ring");
    s.run(Command::OpenPopup(PopupKind::AddModifier)).unwrap();
    // a session is open: the affix tooltips PoB blanks stay blank
    assert!(
        s.backend
            .control_tooltip_view(&ControlTooltipQuery::Affix {
                slot: 0,
                index: Some(0)
            })
            .unwrap()
            .is_none()
    );
    s.run(Command::ClosePopup).unwrap();
    assert!(
        s.backend
            .control_tooltip_view(&ControlTooltipQuery::Affix {
                slot: 0,
                index: Some(0)
            })
            .unwrap()
            .is_some()
    );
    refused_display(
        &mut s,
        Command::RemoveCustomMod { index: 0 },
        "doesn't show that control",
    );
    s.run(Command::CancelDisplayItem { if_id: None }).unwrap();
    refused_display(
        &mut s,
        Command::RemoveCustomMod { index: 0 },
        "no item is being viewed",
    );
    assert_eq!(open_popups(&s), 0);
}
