//! Commands the UI sends to the engine thread.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::views::{CellRef, ModLineRef};

/// A socket's colour on the display item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketColour {
    R,
    G,
    B,
    W,
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Sub-path relative to the Builds folder; "" is the root.
    ListBuilds {
        folder: String,
    },
    NewBuild,
    OpenBuild {
        path: PathBuf,
    },
    ImportShareCode {
        code: String,
    },
    Save,
    SaveAs {
        folder: String,
        name: String,
        overwrite: bool,
    },
    SetConfig {
        var: String,
        value: ConfigValue,
    },
    SelectConfigSet {
        id: u32,
    },
    NewConfigSet {
        title: String,
    },
    RenameConfigSet {
        id: u32,
        title: String,
    },
    DeleteConfigSet {
        id: u32,
    },
    /// Custom modifier groups; `group` is 0-based.
    SetCustomModText {
        group: usize,
        text: String,
    },
    SetCustomModTitle {
        group: usize,
        title: String,
    },
    SetCustomModEnabled {
        group: usize,
        enabled: bool,
    },
    AddCustomModGroup,
    RemoveCustomModGroup {
        group: usize,
    },

    /// Ask for the tooltip of a tree node; only the latest request of a batch is answered.
    PreviewNode {
        id: u32,
    },
    /// Highlights the nodes that match the text (PoB's tree search); "" clears it. PoB saves the
    /// search with the build, so a changed search marks the build unsaved (Build.lua:1303).
    SetTreeSearch {
        text: String,
    },
    /// Node power: `stat` is one of `TreeState.power_stats`, `None` switches it off.
    /// `max_depth` limits how far from the tree the nodes are calculated; `None` is no limit.
    SetHeatmap {
        stat: Option<String>,
        max_depth: Option<u32>,
    },
    /// Ask for the active tree's link.
    ExportTreeLink,
    /// Shrinks the active tree's link with PoEURL (a network request). The short link comes
    /// as `Snapshot::TreeLink` when the request finishes (R20).
    ShrinkTreeLink,

    /// Allocates the node and the path to it (an ascendancy node switches ascendancy first).
    AllocNode {
        id: u32,
    },
    /// Deallocates the node and what depends on it.
    DeallocNode {
        id: u32,
    },
    /// With unconnected points, `connect` keeps the tree by connecting it to the class;
    /// otherwise the tree is reset.
    SelectClass {
        id: u32,
        connect: bool,
    },
    SelectAscendancy {
        id: u32,
    },
    ResetTree,
    /// "Remove All Tattoos" on the Reset window: presses PoB's button, which adds no undo state
    /// (R18).
    RemoveAllTattoos,
    /// Chooses a mastery's effect, allocating the mastery when it isn't yet.
    SetMastery {
        node: u32,
        effect: u32,
    },

    /// Trees (specs) are 0-based, in the order of `TreeState.specs`.
    SelectSpec {
        index: usize,
    },
    /// Adds an empty tree of the newest version, in the current class; it isn't selected.
    NewSpec {
        title: String,
    },
    RenameSpec {
        index: usize,
        title: String,
    },
    DeleteSpec {
        index: usize,
    },
    /// Compares the active tree with another; `None` stops comparing.
    SetCompareSpec {
        index: Option<usize>,
    },
    /// Converts the active tree to `version`, replacing it or, with `keep_copy`, keeping the old
    /// one next to it.
    ConvertSpec {
        version: String,
        keep_copy: bool,
    },
    ConvertAllSpecs {
        version: String,
    },
    /// Adds the tree of a link as a new tree and selects it.
    ImportTreeLink {
        title: String,
        url: String,
    },

    /// Presses one of the Calcs selector's dropdowns; `index` is 0-based.
    SetCalcsSelect {
        control: CalcsControl,
        index: usize,
    },
    /// Types into the stage or mine count; PoB's filter removes what isn't a digit.
    SetCalcsCount {
        control: CalcsCount,
        text: String,
    },
    /// PoB's right-click on a group in the Skills tab, "Set as main skill". Undoable.
    SetMainSocketGroup {
        group: usize,
    },
    /// Presses one of the sidebar's main-skill dropdowns; `index` is 0-based. `Mode` is an error,
    /// because the sidebar has no mode selector.
    SetMainSkillSelect {
        control: CalcsControl,
        index: usize,
    },
    /// Types into the sidebar's stage or mine count.
    SetMainSkillCount {
        control: CalcsCount,
        text: String,
    },
    SetShowMinion {
        show: bool,
    },
    /// The Calcs search box; "" clears it.
    SetCalcsSearch {
        text: String,
    },
    /// `sub` is the sub-section's 0-based position in its section.
    SetCalcsCollapsed {
        section: String,
        sub: usize,
        collapsed: bool,
    },

    /// Replaces the build's notes, filtered as typing into the notes box would filter them.
    SetNotes {
        text: String,
    },

    /// Socket groups are 0-based, in the order of `SkillsView.groups`.
    AddSocketGroup,
    DeleteSocketGroup {
        group: usize,
    },
    DeleteAllSocketGroups,
    /// `to` is the group's final position.
    MoveSocketGroup {
        from: usize,
        to: usize,
    },
    /// Adds a group from the text PoB's copy writes; text with no gems is an error.
    PasteSocketGroup {
        text: String,
    },
    /// Types into the group's label box; PoB's filter and limit apply.
    SetGroupLabel {
        group: usize,
        text: String,
    },
    /// The group's slot, by PoB's slot name; `None` is "None".
    SetGroupSlot {
        group: usize,
        slot: Option<String>,
    },
    SetGroupEnabled {
        group: usize,
        on: bool,
    },
    SetGroupFullDps {
        group: usize,
        on: bool,
    },
    /// Gem slots are 0-based within the group. Only `SetGem` may use `slot == len`, which
    /// appends; the other gem commands address existing gems.
    SetGem {
        group: usize,
        slot: usize,
        gem_id: String,
    },
    /// The level, quality and count take the text typed into PoB's edit box.
    SetGemLevel {
        group: usize,
        slot: usize,
        text: String,
    },
    SetGemQuality {
        group: usize,
        slot: usize,
        text: String,
    },
    SetGemEnabled {
        group: usize,
        slot: usize,
        on: bool,
    },
    SetGemCount {
        group: usize,
        slot: usize,
        text: String,
    },
    SetGemToggle {
        group: usize,
        slot: usize,
        which: GemToggle,
        on: bool,
    },
    DeleteGem {
        group: usize,
        slot: usize,
    },
    /// Skill sets are addressed by PoB's id (`SkillsView.sets`). Select makes one active.
    SelectSkillSet {
        id: u32,
    },
    /// Adds a set, as the manager's New button and name popup do; a blank title is an error.
    NewSkillSet {
        title: String,
    },
    RenameSkillSet {
        id: u32,
        title: String,
    },
    /// Copies the set under a new title (PoB's Copy button asks for one).
    CopySkillSet {
        id: u32,
        title: String,
    },
    /// The last set can't be deleted.
    DeleteSkillSet {
        id: u32,
    },
    /// One of the Skills tab's gem options. They are build state that Save writes, but not
    /// undoable and not unsaved.
    SetSkillOption(SkillOption),
    /// The Count box of a group an equipped item grants; any other group is an error (R2).
    SetGroupCount {
        group: usize,
        text: String,
    },
    /// The group's Imbued Support box: `Some` picks the gem like Enter in the box, `None` presses
    /// its x button. Refused when PoB's box is disabled or hidden (an item's group).
    SetImbuedSupport {
        group: usize,
        gem_id: Option<String>,
    },
    /// The Optimise Sockets button. It edits the item and records an Items undo state, not a
    /// Skills one (R20).
    OptimiseSockets {
        group: usize,
    },
    /// A query: what hovering the group in the list shows, answered as a `GroupTooltip`; the
    /// latest one in a batch is answered.
    GroupTooltip {
        group: usize,
    },
    /// A query: the text PoB copies for the group, answered as a `SocketGroupText`; the latest
    /// one in a batch is answered. The engine never touches the system clipboard.
    CopySocketGroup {
        group: usize,
    },

    /// A query: what hovering an item shows (R3), answered as an `ItemTooltip`; the latest one
    /// in a batch is answered. A vanished item or slot sends nothing.
    ItemTooltip(ItemTooltipQuery),
    /// A query: the text PoB copies for the item (R14), answered as an `ItemText`; the latest
    /// one in a batch is answered. The engine never touches the system clipboard.
    CopyItem {
        id: u32,
    },

    /// The item list's Sort button.
    SortItems,
    /// Deletes the item (PoB's Delete button, confirmed when the item is in use). An item the
    /// list filter hides is `no such item`.
    DeleteItem {
        id: u32,
        /// The user has been asked about an item in use. PoB's own confirm popup, when it
        /// opens and this is false, is refused (the item may have been equipped since the
        /// UI last looked).
        confirmed: bool,
    },
    /// "Del Unused" has no confirmation in PoB.
    DeleteUnusedItems,
    /// "Del All", confirmed; a no-op with an empty list.
    DeleteAllItems,
    /// Positions are 0-based in `ItemsView.items`; `to` is the item's final position. Refused
    /// while the list is filtered.
    MoveItem {
        from: usize,
        to: usize,
    },
    /// The list's loadout filter; `index` is 0-based in `ItemsView.list_filter`.
    SetItemListFilter {
        index: usize,
    },
    /// The slot's dropdown, by PoB's slot name; `None` is "None".
    EquipItem {
        slot: String,
        id: Option<u32>,
    },
    /// A flask's checkbox.
    SetFlaskActive {
        slot: String,
        on: bool,
    },
    /// The weapon set buttons "I" (`false`) and "II" (`true`).
    SelectWeaponSet {
        second: bool,
    },

    /// Ctrl+V on the Items tab (R10): the text becomes the display item, like PoB's
    /// `CreateDisplayItemFromRaw`. Text with no item base is `PoB couldn't read that item`.
    PasteItem {
        text: String,
    },
    /// The item list's double click: the item goes to the display panel for editing.
    EditItem {
        id: u32,
    },
    /// PoB's "Edit..." / "Create custom..." popup: `rarity` is its dropdown (0 Normal ... 4
    /// Relic), `text` the item without its Rarity line. Refused like a paste when PoB's Save
    /// is disabled.
    SetDisplayItemText {
        rarity: usize,
        text: String,
    },
    /// The panel's Save / Add to build button.
    SaveDisplayItem,
    /// The panel's Cancel button. With `if_id`, it only cancels while the item with that id is
    /// displayed (a clean-up of a deleted item's display must not hit another item); without,
    /// it is the user's own Cancel and an empty panel is an error.
    CancelDisplayItem {
        if_id: Option<u32>,
    },
    /// Ctrl+D (R13): saved with the build, not undoable, not unsaved.
    SetShowStatDifferences {
        on: bool,
    },

    /// One of the six variant dropdowns (`DisplayItemView.variants[..].control`, 0..=5), legacy
    /// or grouped; `index` is 0-based in its list (R11).
    SetItemVariant {
        control: usize,
        index: usize,
    },
    /// The version dropdown of an item with variant groups.
    SetItemVersion {
        index: usize,
    },
    /// The quality box; the box filters digits and takes at most two.
    SetItemQuality {
        text: String,
    },
    /// One of the two influence dropdowns (`which` 0 or 1).
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
    /// Socket `index` (0-based) takes a colour.
    SetItemSocket {
        index: usize,
        colour: SocketColour,
    },
    /// The link after socket `index`.
    SetItemSocketLink {
        index: usize,
        linked: bool,
    },
    /// The "+" button.
    AddItemSocket,
    /// A range slider: `line` indexes `DisplayItemView.range_lines`; `value` is 0..=1.
    SetRangeLine {
        line: usize,
        value: f64,
    },
    /// The Foulborn box next to a line.
    SetRangeLineMutated {
        line: usize,
        on: bool,
    },
    /// Picks entry `index` of affix dropdown `slot` (0..=8, R9). `label` is the entry as the
    /// caller saw it: PoB rebuilds every list after a pick, so a pick against a list that
    /// changed is refused (R8).
    SetAffix {
        slot: usize,
        index: usize,
        label: String,
    },
    /// The slider under affix dropdown `slot`: 0..=1, tier and roll in one.
    SetAffixRange {
        slot: usize,
        value: f64,
    },
    /// "Modifier sorting" (R9, R10).
    SetCraftingSort {
        index: usize,
    },
    /// A cluster jewel's skill dropdown (R11).
    SetClusterSkill {
        index: usize,
    },
    /// A cluster jewel's "Added Passives" slider, as a node count (R11).
    SetClusterNodeCount {
        count: u32,
    },
    /// A custom, crafted or crucible mod's Remove button (R12); `index` is in
    /// `DisplayItemView::custom_mods`.
    RemoveCustomMod {
        index: usize,
    },
    /// A click on a tooltip line of the display item (R4).
    ToggleModLine {
        line: ModLineRef,
    },

    /// Picks the item set in PoB's set dropdown, by PoB's id.
    SelectItemSet {
        id: u32,
    },
    /// The set manager's New button and the name popup. A blank title is refused.
    NewItemSet {
        title: String,
    },
    /// The set manager's Rename button.
    RenameItemSet {
        id: u32,
        title: String,
    },
    /// The set manager's Copy button, which asks for the copy's title.
    CopyItemSet {
        id: u32,
        title: String,
    },
    /// The set manager's Delete button, confirmed; the last set can't go.
    DeleteItemSet {
        id: u32,
    },
    /// Positions are 0-based in `ItemsView.sets`; `to` is the set's final position.
    MoveItemSet {
        from: usize,
        to: usize,
    },

    /// Opens or replaces the gem picker's search (R9); the latest one in a batch is answered
    /// as a `GemSearch`. While it is open PoB's DPS sort runs in the background.
    SearchGems(GemSearchQuery),
    /// Closes the gem picker's search.
    CloseGemSearch,
    /// A query: a gem tooltip (R15), answered as a `GemTooltip`; the latest one in a batch is
    /// answered.
    GemTooltip(GemTooltipQuery),

    /// A query: the breakdown of a Calcs cell; the latest one in a batch is answered.
    Breakdown(CellRef),

    /// Presses the panel's own button for this window and keeps what it opens as the engine's
    /// popup session (R1, R3). Ends an open session. Answered as a `Popup`.
    OpenPopup(PopupKind),
    /// Sets one control of the open popup (R7). Refused for a control PoB doesn't show or
    /// enable, or a pick whose label is no longer at that index (R8).
    SetPopupControl {
        name: String,
        value: PopupInput,
    },
    /// Presses a button of the open popup. A popup PoB closes ends the session.
    PressPopupButton {
        name: String,
    },
    /// Cancel and Escape: drops the session.
    ClosePopup,
    /// A query: the tooltip of a popup entry or button, an affix entry or a DB item (R6); the
    /// latest one in a batch is answered, as a `ControlTooltip`.
    ControlTooltip(ControlTooltipQuery),

    /// Opens or replaces the item database search (R13); the latest one in a batch is answered
    /// as an `ItemDb`. While it is open PoB's DB load and stat sort run in the background.
    SetItemDb(ItemDbQuery),
    /// Closes the item database search.
    CloseItemDb,
    /// The double click on a database row: shows the item on the display panel (R15).
    ViewDbItem {
        db: ItemDb,
        name: String,
    },
    /// Ctrl+click on a database row: adds the item and equips it (R15).
    AddDbItem {
        db: ItemDb,
        name: String,
    },

    Undo(Tab),
    Redo(Tab),
    /// Stops a pending network request: its subscripts are aborted and PoB's own failure path
    /// runs with "Cancelled" (R12).
    CancelRequest {
        id: u64,
    },
    /// Internal: a subscript thread woke the loop. The loop never applies it (R9).
    Wake,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    Bool(bool),
    Number(f64),
    Text(String),
    Choice(String),
    Clear,
}

/// The gem options of the Skills tab. Indices are 0-based positions in the dropdown's list.
#[derive(Debug, Clone, PartialEq)]
pub enum SkillOption {
    SortByDps(bool),
    SortField(usize),
    DefaultLevel(usize),
    /// The text typed into the default quality box
    DefaultQuality(String),
    SupportTypes(usize),
    ShowLegacy(bool),
}

/// The gem box a search or a gem tooltip is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemTarget {
    /// The name box of the group's gem in this 0-based slot; `len` is the empty row.
    Slot(usize),
    /// The group's Imbued Support box.
    Imbued,
}

/// PoB's "S" and "A" overlay buttons on a gem box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemFilter {
    All,
    Support,
    Active,
}

impl GemFilter {
    /// The gem tag PoB filters by (`sortGemsBy`, GemSelectControl.lua:732-741).
    pub(crate) fn pob_tag(self) -> Option<&'static str> {
        match self {
            GemFilter::All => None,
            GemFilter::Support => Some("support"),
            GemFilter::Active => Some("grants_active_skill"),
        }
    }
}

/// What the gem picker searches: `text` is typed into the box of `target` in `group`.
#[derive(Debug, Clone, PartialEq)]
pub struct GemSearchQuery {
    pub group: usize,
    pub target: GemTarget,
    pub text: String,
    pub filter: GemFilter,
}

/// A gem tooltip: `Some(id)` is a picker row (what selecting the gem would give), `None` is
/// the gem already in the box.
#[derive(Debug, Clone, PartialEq)]
pub struct GemTooltipQuery {
    pub group: usize,
    pub target: GemTarget,
    pub gem_id: Option<String>,
}

/// An item tooltip: `slot: None` is a hover in the item list, `Some(name)` a hover over that
/// slot or one of its dropdown entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTooltipQuery {
    pub id: u32,
    pub slot: Option<String>,
}

/// A window PoB opens from the Items tab's buttons (R3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PopupKind {
    /// "Craft item...", shown while no item is displayed
    Craft,
    /// The item text editor; only reached from `Craft` (R2)
    EditText,
    Enchant {
        second: bool,
    },
    /// "Anoint...", "Anoint 2..." and so on: `slot` is 0..=3
    Anoint {
        slot: usize,
    },
    Corrupt,
    AddModifier,
    AddImplicit,
    AddCrucible,
    NodeReplacement {
        node: u32,
    },
}

/// What a popup control is set to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PopupInput {
    /// A dropdown entry, with the label that was shown (R8)
    Choice {
        index: usize,
        label: String,
    },
    Number(f64),
    Check(bool),
    Text(String),
    /// A list row, by its key
    Row(String),
}

/// An item database (Task 6)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemDb {
    Uniques,
    Rares,
}

/// What an item database panel searches: PoB's controls, each as a 0-based index into its list.
/// The rares database has only slot, type, search and search mode (R13): the rest must be 0.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDbQuery {
    pub db: ItemDb,
    pub slot: usize,
    pub kind: usize,
    pub league: usize,
    pub requirement: usize,
    pub obtainable: usize,
    pub sort: usize,
    pub search: String,
    pub search_mode: usize,
}

/// A tooltip behind a control (R6): `index` is the dropdown entry, or none for the control
/// itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlTooltipQuery {
    Popup {
        control: String,
        index: Option<usize>,
    },
    Affix {
        slot: usize,
        index: Option<usize>,
    },
    DbItem {
        db: ItemDb,
        name: String,
    },
}

/// A Vaal gem's two "Enable <skill>" toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GemToggle {
    First,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Config,
    Tree,
    Calcs,
    Skills,
    Items,
}

/// The dropdowns of the Calcs selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcsControl {
    Group,
    Skill,
    Part,
    Minion,
    MinionSkill,
    Mode,
}

impl CalcsControl {
    /// The name of PoB's control.
    pub(crate) fn pob_name(self) -> &'static str {
        match self {
            CalcsControl::Group => "mainSocketGroup",
            CalcsControl::Skill => "mainSkill",
            CalcsControl::Part => "mainSkillPart",
            CalcsControl::Minion => "mainSkillMinion",
            CalcsControl::MinionSkill => "mainSkillMinionSkill",
            CalcsControl::Mode => "mode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcsCount {
    Stages,
    Mines,
}

impl CalcsCount {
    pub(crate) fn pob_name(self) -> &'static str {
        match self {
            CalcsCount::Stages => "mainSkillStageCount",
            CalcsCount::Mines => "mainSkillMineCount",
        }
    }
}
