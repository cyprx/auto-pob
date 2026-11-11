//! The recorded-recipe harness for the Items tab: upstream recipes from `fixtures/items/*.lua`
//! replayed as our commands. Included from `gate.rs`, shared by the Items and crafting tests.

use super::{
    Session, fixtures_dir, load_tree_lib, load_xml, lua_value, plain_notes, pob_dir,
    recipe_setting, upstream_lua,
};
use auto_pob::engine::upstream::{SidebarLine, golden_of};
use auto_pob::engine::{
    Backend, Command, ControlTooltipQuery, ItemsView, ModLineRef, PopupInput, PopupKind,
    SocketColour, Tab,
};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;

/// Upstream PoB with a build loaded and `tree_lib.lua` and `items_lib.lua` defined.
pub(crate) fn upstream_with_items_lib(xml: &std::path::Path, name: &str) -> mlua::Lua {
    upstream_with_items_lib_xml(&fs::read_to_string(xml).unwrap(), name)
}

/// `upstream_with_items_lib` for a build given as XML text.
pub(crate) fn upstream_with_items_lib_xml(xml: &str, name: &str) -> mlua::Lua {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    load_xml(&lua, xml, name).unwrap();
    load_tree_lib(&lua);
    lua.load(fs::read_to_string(fixtures_dir().join("items_lib.lua")).unwrap())
        .set_name("=items_lib")
        .exec()
        .expect("items_lib.lua");
    lua.load(fs::read_to_string(fixtures_dir().join("popup_lib.lua")).unwrap())
        .set_name("=popup_lib")
        .exec()
        .expect("popup_lib.lua");
    lua
}

/// What upstream's Items tab shows now (`itemsDump()`), as JSON with the nulls left out:
/// a Lua table has no nil fields.
pub(crate) fn upstream_json(lua: &mlua::Lua) -> serde_json::Value {
    let value: mlua::Value = lua.load("return itemsDump()").eval().unwrap();
    strip_nulls(lua.from_value(value).unwrap())
}

/// Our view as JSON, without the `None` fields (they are nulls here and absent upstream).
pub(crate) fn json(view: &ItemsView) -> serde_json::Value {
    strip_nulls(serde_json::to_value(view).unwrap())
}

/// The path of the first difference between two JSON values.
pub(crate) fn first_difference(
    path: &str,
    a: &serde_json::Value,
    b: &serde_json::Value,
) -> Option<String> {
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
pub(crate) fn assert_same(ours: &serde_json::Value, theirs: &serde_json::Value, what: &str) {
    if let Some(difference) = first_difference("", ours, theirs) {
        panic!("{what}: ours vs upstream at {difference}");
    }
}

pub(crate) fn strip_nulls(value: serde_json::Value) -> serde_json::Value {
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
pub(crate) fn setup_xml(name: &str) -> String {
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

pub(crate) fn json_of<T: serde::Serialize>(value: &T) -> serde_json::Value {
    strip_nulls(serde_json::to_value(value).unwrap())
}

/// The number of popups PoB has open
pub(crate) fn open_popups(s: &Session) -> u32 {
    lua_value(&s.backend.lua, "return #pob.main.main.popups")
}

// ---- The recorded-recipe harness ----

/// One step of a recipe as `items_lib.lua`'s `act.*` recorded it, with 0-based positions.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "cmd")]
pub(crate) enum ItemsAction {
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
    SetItemVariant {
        control: usize,
        index: usize,
    },
    SetItemVersion {
        index: usize,
    },
    SetItemQuality {
        text: String,
    },
    SetItemInfluence {
        which: usize,
        index: usize,
    },
    SetItemCatalyst {
        index: usize,
    },
    SetItemCatalystQuality {
        text: String,
    },
    SetItemSocket {
        index: usize,
        colour: String,
    },
    SetItemSocketLink {
        index: usize,
        linked: bool,
    },
    AddItemSocket,
    SetRangeLine {
        line: usize,
        value: f64,
    },
    SetRangeLineMutated {
        line: usize,
        on: bool,
    },
    ToggleModLine {
        line: ModLineRef,
    },
    /// Recorded by items_lib's `act.affix`, `act.affix_range`, `act.crafting_sort`,
    /// `act.cluster_skill` and `act.cluster_count`
    SetAffix {
        slot: usize,
        index: usize,
        label: String,
    },
    SetAffixRange {
        slot: usize,
        value: f64,
    },
    SetCraftingSort {
        index: usize,
    },
    SetClusterSkill {
        index: usize,
    },
    SetClusterNodeCount {
        count: u32,
    },
    /// Recorded by items_lib's `act.remove_custom`
    RemoveCustomMod {
        index: usize,
    },
    Undo,
    Redo,
    /// Recorded by popup_lib's `act.open_popup`, `act.popup_set`, `act.popup_press` and
    /// `act.popup_close`
    OpenPopup {
        kind: PopupKind,
    },
    SetPopupControl {
        name: String,
        value: PopupInput,
    },
    PressPopupButton {
        name: String,
    },
    ClosePopup,
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
            ItemsAction::DeleteItem { id } => Command::DeleteItem {
                id,
                confirmed: true,
            },
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
            ItemsAction::CancelDisplayItem => Command::CancelDisplayItem { if_id: None },
            ItemsAction::SetShowStatDifferences { on } => Command::SetShowStatDifferences { on },
            ItemsAction::SetItemVariant { control, index } => {
                Command::SetItemVariant { control, index }
            }
            ItemsAction::SetItemVersion { index } => Command::SetItemVersion { index },
            ItemsAction::SetItemQuality { text } => Command::SetItemQuality { text },
            ItemsAction::SetItemInfluence { which, index } => {
                Command::SetItemInfluence { which, index }
            }
            ItemsAction::SetItemCatalyst { index } => Command::SetItemCatalyst { index },
            ItemsAction::SetItemCatalystQuality { text } => {
                Command::SetItemCatalystQuality { text }
            }
            ItemsAction::SetItemSocket { index, colour } => Command::SetItemSocket {
                index,
                colour: socket_colour(&colour),
            },
            ItemsAction::SetItemSocketLink { index, linked } => {
                Command::SetItemSocketLink { index, linked }
            }
            ItemsAction::AddItemSocket => Command::AddItemSocket,
            ItemsAction::SetRangeLine { line, value } => Command::SetRangeLine { line, value },
            ItemsAction::SetRangeLineMutated { line, on } => {
                Command::SetRangeLineMutated { line, on }
            }
            ItemsAction::ToggleModLine { line } => Command::ToggleModLine { line },
            ItemsAction::SetAffix { slot, index, label } => {
                Command::SetAffix { slot, index, label }
            }
            ItemsAction::SetAffixRange { slot, value } => Command::SetAffixRange { slot, value },
            ItemsAction::SetCraftingSort { index } => Command::SetCraftingSort { index },
            ItemsAction::SetClusterSkill { index } => Command::SetClusterSkill { index },
            ItemsAction::SetClusterNodeCount { count } => Command::SetClusterNodeCount { count },
            ItemsAction::RemoveCustomMod { index } => Command::RemoveCustomMod { index },
            ItemsAction::OpenPopup { kind } => Command::OpenPopup(kind),
            ItemsAction::SetPopupControl { name, value } => {
                Command::SetPopupControl { name, value }
            }
            ItemsAction::PressPopupButton { name } => Command::PressPopupButton { name },
            ItemsAction::ClosePopup => Command::ClosePopup,
            ItemsAction::Undo => Command::Undo(Tab::Items),
            ItemsAction::Redo => Command::Redo(Tab::Items),
            ItemsAction::Check { .. } => return None,
        })
    }
}

pub(crate) fn socket_colour(colour: &str) -> SocketColour {
    match colour {
        "R" => SocketColour::R,
        "G" => SocketColour::G,
        "B" => SocketColour::B,
        "W" => SocketColour::W,
        other => panic!("no socket colour {other}"),
    }
}

/// The Items tab at one point of a recipe: `itemsDump()`, the jewels each socket holds and the
/// sidebar.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct ItemsCheckpoint {
    pub(crate) items: serde_json::Value,
    /// The display panel (`displayDump()`); none while nothing is displayed
    #[serde(default)]
    pub(crate) display: Option<serde_json::Value>,
    /// `spec.jewels` without the empty sockets
    #[serde(default)]
    pub(crate) jewels: BTreeMap<u32, u32>,
    pub(crate) sidebar: Vec<SidebarLine>,
    /// `mainSocketGroup`, 1-based as PoB has it: a weapon swap moves it (R6)
    pub(crate) main_group: u32,
    /// The open popup (`popupDump()`); none while no window is open
    #[serde(default)]
    pub(crate) popup: Option<serde_json::Value>,
    /// Slot 0's affix tooltips as hovering shows them (`affixTooltips()`)
    #[serde(default)]
    pub(crate) affix_tooltips: Vec<RecordedAffixTooltip>,
    /// The tree layout's text (`layoutDigest()`), when the recipe asked for it
    #[serde(default)]
    pub(crate) layout: Option<String>,
}

/// An affix dropdown entry's tooltip (or, without an index, the slider's) at a checkpoint
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct RecordedAffixTooltip {
    pub(crate) slot: usize,
    #[serde(default)]
    pub(crate) index: Option<usize>,
    pub(crate) tooltip: serde_json::Value,
}

/// A recipe run in upstream PoB: the steps it took and where they ended.
pub(crate) struct ItemsRecipe {
    pub(crate) base: String,
    /// The build to start from when the recipe prepared one, or names a `setup`
    pub(crate) setup_xml: Option<String>,
    pub(crate) actions: Vec<ItemsAction>,
    pub(crate) result: serde_json::Value,
    /// Every item's `BuildRaw()` and `spec.jewels` where the recipe ended
    pub(crate) raws: BTreeMap<u32, String>,
    pub(crate) jewels: BTreeMap<u32, u32>,
}

/// Every item's `BuildRaw()` and the jewels each socket holds, read with `code` in a Lua state
/// where `BUILD` is the build.
pub(crate) fn raws_and_jewels(
    lua: &mlua::Lua,
    build: &str,
) -> (BTreeMap<u32, String>, BTreeMap<u32, u32>) {
    let raws = lua_value(
        lua,
        &format!(
            "local out = {{}} for id, item in pairs({build}.itemsTab.items) do out[id] = item:BuildRaw() end return out"
        ),
    );
    let jewels: BTreeMap<u32, u32> = lua_value(lua, &format!("return {build}.spec.jewels"));
    (
        raws,
        jewels.into_iter().filter(|(_, id)| *id != 0).collect(),
    )
}

/// Runs `items/<name>.lua` in upstream PoB on its base fixture, or on its `setup` build.
pub(crate) fn upstream_items_recipe(name: &str) -> ItemsRecipe {
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
                check.popup = check.popup.map(strip_nulls);
                for recorded in &mut check.affix_tooltips {
                    recorded.tooltip = strip_nulls(recorded.tooltip.take());
                }
                return ItemsAction::Check {
                    check: Box::new(check),
                };
            }
            let json: serde_json::Value = lua.from_value(mlua::Value::Table(entry)).unwrap();
            serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{name}: {json}: {e}"))
        })
        .collect();
    let (raws, jewels) = raws_and_jewels(&lua, "build");
    ItemsRecipe {
        base,
        setup_xml: setup,
        actions,
        result: upstream_json(&lua),
        raws,
        jewels,
    }
}

/// Our Items tab and sidebar in the shape of a recipe's `checkpoint()`.
pub(crate) fn our_items_checkpoint(s: &mut Session) -> ItemsCheckpoint {
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
        popup: s
            .backend
            .popup_view()
            .expect("popup view")
            .map(|view| json_of(&view)),
        affix_tooltips: Vec::new(),
        layout: None,
    }
}

/// Replays a recipe as our commands and checks every checkpoint and the end state against
/// upstream.
pub(crate) fn check_items_recipe(name: &str) -> ItemsRecipe {
    run_items_recipe(name).0
}

/// `check_items_recipe`, keeping our session where the recipe ended.
pub(crate) fn run_items_recipe(name: &str) -> (ItemsRecipe, Session) {
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
                let command = relabel_pick(&mut s, command);
                let started = std::time::Instant::now();
                let affected = s
                    .run(command)
                    .unwrap_or_else(|e| panic!("{name}: {action:?}: {e}"));
                if matches!(action, ItemsAction::SetCraftingSort { .. }) {
                    eprintln!("{name}: SetCraftingSort took {:?}", started.elapsed());
                }
                if matches!(
                    action,
                    ItemsAction::PasteItem { .. }
                        | ItemsAction::EditItem { .. }
                        | ItemsAction::SetDisplayItemText { .. }
                        | ItemsAction::SaveDisplayItem
                        | ItemsAction::CancelDisplayItem
                        | ItemsAction::SetItemVariant { .. }
                        | ItemsAction::SetItemVersion { .. }
                        | ItemsAction::SetItemQuality { .. }
                        | ItemsAction::SetItemInfluence { .. }
                        | ItemsAction::SetItemCatalyst { .. }
                        | ItemsAction::SetItemCatalystQuality { .. }
                        | ItemsAction::SetItemSocket { .. }
                        | ItemsAction::SetItemSocketLink { .. }
                        | ItemsAction::AddItemSocket
                        | ItemsAction::SetRangeLine { .. }
                        | ItemsAction::SetRangeLineMutated { .. }
                        | ItemsAction::ToggleModLine { .. }
                        | ItemsAction::SetAffix { .. }
                        | ItemsAction::SetAffixRange { .. }
                        | ItemsAction::SetCraftingSort { .. }
                        | ItemsAction::SetClusterSkill { .. }
                        | ItemsAction::SetClusterNodeCount { .. }
                        | ItemsAction::RemoveCustomMod { .. }
                ) {
                    assert!(
                        affected.display_item,
                        "{name}: {action:?} doesn't flag the display item"
                    );
                }
                // the UI's list, slots and sets go stale unless these are flagged
                if matches!(
                    action,
                    ItemsAction::SortItems
                        | ItemsAction::DeleteItem { .. }
                        | ItemsAction::DeleteUnusedItems
                        | ItemsAction::DeleteAllItems
                        | ItemsAction::MoveItem { .. }
                        | ItemsAction::SetItemListFilter { .. }
                        | ItemsAction::EquipItem { .. }
                        | ItemsAction::SetFlaskActive { .. }
                        | ItemsAction::SelectWeaponSet { .. }
                        | ItemsAction::SelectItemSet { .. }
                        | ItemsAction::NewItemSet { .. }
                        | ItemsAction::RenameItemSet { .. }
                        | ItemsAction::CopyItemSet { .. }
                        | ItemsAction::DeleteItemSet { .. }
                        | ItemsAction::MoveItemSet { .. }
                ) && !matches!(action, ItemsAction::MoveItemSet { from, to } if from == to)
                {
                    assert!(
                        affected.items,
                        "{name}: {action:?} doesn't flag the Items view"
                    );
                    // everything but the list filter marks the build unsaved
                    if !matches!(action, ItemsAction::SetItemListFilter { .. }) {
                        assert!(
                            affected.build,
                            "{name}: {action:?} doesn't flag the build view"
                        );
                    }
                }
                assert_eq!(open_popups(&s), 0, "{name}: {action:?} left a popup open");
                if matches!(
                    action,
                    ItemsAction::OpenPopup { .. }
                        | ItemsAction::SetPopupControl { .. }
                        | ItemsAction::PressPopupButton { .. }
                        | ItemsAction::ClosePopup
                ) {
                    assert!(affected.popup, "{name}: {action:?} doesn't flag the popup");
                }
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
                settle(&mut s);
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
                match (&ours.popup, &check.popup) {
                    (Some(ours), Some(theirs)) => {
                        assert_same(ours, theirs, &format!("{name}: checkpoint {checked} popup"))
                    }
                    (ours, theirs) => {
                        assert_eq!(ours, theirs, "{name}: checkpoint {checked} popup")
                    }
                }
                // the affix tooltips, hovered now on our side
                for recorded in &check.affix_tooltips {
                    let query = ControlTooltipQuery::Affix {
                        slot: recorded.slot,
                        index: recorded.index,
                    };
                    let view = s
                        .backend
                        .control_tooltip_view(&query)
                        .unwrap()
                        .unwrap_or_else(|| panic!("{name}: no tooltip for {query:?}"));
                    let what = format!("{name}: checkpoint {checked} tooltip {query:?}");
                    let ours = json_of(&view.tooltip);
                    if first_difference("", &ours, &recorded.tooltip).is_some() {
                        // see `canonical_tooltip`
                        eprintln!("{what}: the \"can appear on\" order differs");
                        assert_same(
                            &canonical_tooltip(ours),
                            &canonical_tooltip(recorded.tooltip.clone()),
                            &what,
                        );
                    }
                    assert!(!view.tooltip.lines.is_empty() || recorded.index == Some(0));
                }
                if let Some(theirs) = &check.layout {
                    let ours: String =
                        lua_value(&s.backend.lua, "return glue.tree_layout_digest()");
                    assert_eq!(&ours, theirs, "{name}: checkpoint {checked} tree layout");
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "{name}: no checkpoint");
    assert_eq!(open_popups(&s), 0, "{name}: a popup was left open");
    let ours = json(&s.backend.items_view().unwrap());
    assert_ne!(ours, before, "{name}: the recipe changes the Items tab");
    assert_same(&ours, &recipe.result, name);
    (recipe, s)
}

/// A recorded pick names the entry it chose by index and label. Where PoB builds a list from
/// `pairs`, entries that tie sort in an order that differs per Lua state, so the index can
/// land on another entry in ours: the label says which was meant.
fn relabel_pick(s: &mut Session, command: Command) -> Command {
    let Command::SetPopupControl {
        name,
        value: PopupInput::Choice { index, label },
    } = command
    else {
        return command;
    };
    // only the lists PoB builds from `pairs`
    if ![
        "modSelect",
        "modGroupSelect",
        "implicit1",
        "implicit2",
        "implicit3",
        "implicit4",
    ]
    .contains(&name.as_str())
    {
        return Command::SetPopupControl {
            name,
            value: PopupInput::Choice { index, label },
        };
    }
    let view = s.backend.popup_view().ok().flatten();
    let index = view
        .and_then(|v| v.controls.into_iter().find(|c| c.name == name))
        .map(|c| match c.choices.get(index) {
            Some(here) if *here == label => index,
            _ => {
                let found = c.choices.iter().position(|c| *c == label).unwrap_or(index);
                eprintln!("{name}: pick {label:?} moved from entry {index} to {found}");
                found
            }
        })
        .unwrap_or(index);
    Command::SetPopupControl {
        name,
        value: PopupInput::Choice { index, label },
    }
}

pub(crate) fn checkpoints(recipe: &ItemsRecipe) -> Vec<&ItemsCheckpoint> {
    recipe
        .actions
        .iter()
        .filter_map(|a| match a {
            ItemsAction::Check { check } => Some(&**check),
            _ => None,
        })
        .collect()
}

/// How many Anoint sort steps `settle` has run, so a test can see the sort was a background job
pub(crate) static ANOINT_SORT_STEPS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Runs the background steps the engine would run between commands, until there are none.
pub(crate) fn settle(s: &mut Session) {
    while let Some(task) = s.backend.next_background() {
        if task == auto_pob::engine::BackgroundTask::AnointSort {
            ANOINT_SORT_STEPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        s.backend
            .background_step(task, &|_| {})
            .unwrap_or_else(|e| panic!("{task:?}: {e}"));
    }
}

/// A notable's tooltip ends with the cluster jewels it can appear on: a group per size, with
/// the skills that carry it. PoB builds both lists from `pairs`, so two Lua states order the
/// groups and the skills differently; the blank line that follows every group but the first
/// moves with them. In canonical form the groups are sorted by header, the skills within each,
/// and the blanks come last. Every other line is compared as it is.
fn canonical_tooltip(mut tooltip: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    let Some(lines) = tooltip["lines"].as_array_mut() else {
        return tooltip;
    };
    let text = |line: &Value| line["text"].as_str().unwrap_or_default().to_string();
    let Some(start) = lines
        .iter()
        .position(|l| text(l).ends_with(" can appear on:"))
    else {
        return tooltip;
    };
    let tail: Vec<Value> = lines.split_off(start + 1);
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut blanks = Vec::new();
    for line in tail {
        let t = text(&line);
        if line["size"] == 18 {
            groups.push((line.to_string(), Vec::new()));
        } else if t.is_empty() {
            blanks.push(line);
        } else {
            groups
                .last_mut()
                .expect("a skill before its size")
                .1
                .push(line.to_string());
        }
    }
    groups.sort();
    for (header, mut skills) in groups {
        skills.sort();
        lines.push(serde_json::from_str(&header).unwrap());
        lines.extend(
            skills
                .iter()
                .map(|l| serde_json::from_str::<Value>(l).unwrap()),
        );
    }
    lines.extend(blanks);
    tooltip
}

#[cfg(test)]
mod canonical_tests {
    use super::canonical_tooltip;
    use serde_json::{Value, json};

    fn line(text: &str, size: u32) -> Value {
        json!({ "text": text, "size": size })
    }

    /// Two size groups, each with skills, after a lead-in line
    fn tooltip(groups: &[(&str, &[&str])]) -> Value {
        let mut lines = vec![line("Notable", 16), line("Notable can appear on:", 16)];
        for (i, (size, skills)) in groups.iter().enumerate() {
            if i > 0 {
                lines.push(line("", 16));
            }
            lines.push(line(size, 18));
            lines.extend(skills.iter().map(|s| line(s, 16)));
        }
        json!({ "lines": lines })
    }

    const BASE: &[(&str, &[&str])] = &[
        ("Large", &["Fire", "Cold", "Life"]),
        ("Medium", &["Fire", "Mana"]),
    ];

    #[test]
    fn a_reordered_tail_compares_equal() {
        let reordered: &[(&str, &[&str])] = &[
            ("Medium", &["Mana", "Fire"]),
            ("Large", &["Life", "Fire", "Cold"]),
        ];
        assert_eq!(
            canonical_tooltip(tooltip(BASE)),
            canonical_tooltip(tooltip(reordered))
        );
    }

    #[test]
    fn a_missing_extra_or_moved_skill_differs() {
        let canon = |groups: &[(&str, &[&str])]| canonical_tooltip(tooltip(groups));
        let base = canon(BASE);
        let missing: &[(&str, &[&str])] =
            &[("Large", &["Fire", "Cold"]), ("Medium", &["Fire", "Mana"])];
        let extra: &[(&str, &[&str])] = &[
            ("Large", &["Fire", "Cold", "Life", "Chaos"]),
            ("Medium", &["Fire", "Mana"]),
        ];
        let moved: &[(&str, &[&str])] = &[
            ("Large", &["Fire", "Cold"]),
            ("Medium", &["Fire", "Mana", "Life"]),
        ];
        assert_ne!(base, canon(missing), "a missing skill");
        assert_ne!(base, canon(extra), "an extra skill");
        assert_ne!(base, canon(moved), "a skill moved between groups");
    }

    #[test]
    fn a_missing_blank_separator_differs() {
        let mut short = tooltip(BASE);
        let lines = short["lines"].as_array_mut().unwrap();
        let blank = lines.iter().position(|l| l["text"] == "").unwrap();
        lines.remove(blank);
        assert_ne!(canonical_tooltip(tooltip(BASE)), canonical_tooltip(short));
    }

    #[test]
    fn lines_before_the_tail_stay_exact() {
        let mut other = tooltip(BASE);
        other["lines"][0] = line("Another notable", 16);
        assert_ne!(canonical_tooltip(tooltip(BASE)), canonical_tooltip(other));
    }
}
