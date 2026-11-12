//! Commands the UI sends to the engine thread.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::views::{BuildSort, CellRef, ModLineRef};

/// A socket's colour on the display item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketColour {
    R,
    G,
    B,
    W,
}

/// Where an import from a link or code goes (R16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportMode {
    /// Replaces the open build; only for a build with a file
    ThisBuild,
    /// Opens a new, unsaved build
    NewBuild,
    /// Adds the build as a comparison: PoB's "Import as comparison" (R5). PoB offers it only
    /// for a build with a file.
    Comparison,
}

/// Where a comparison build comes from (R5)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareSource {
    /// A build file, read by the engine; the comparison is named after the file
    File(PathBuf),
    /// A share code
    Code(String),
}

/// A platform of PoB's account import (PoB's ids `"PC"`, `"XBOX"`, `"SONY"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Realm {
    Pc,
    Xbox,
    Sony,
}

impl Realm {
    /// PoB's id for the realm drop-down
    pub fn pob_id(self) -> &'static str {
        match self {
            Realm::Pc => "PC",
            Realm::Xbox => "XBOX",
            Realm::Sony => "SONY",
        }
    }
}

/// What an account import takes from a character: PoB's two buttons and their options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CharacterPart {
    /// "Passive Tree and Jewels"
    Tree { delete_jewels: bool },
    /// "Items and Skills"
    ItemsAndSkills {
        clear_items: bool,
        clear_skills: bool,
        ignore_weapon_swap: bool,
    },
}

/// The question a delete was confirmed under
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteKind {
    /// A build file
    Build,
    /// A folder PoB deletes without asking, because it was empty
    EmptyFolder,
    /// A folder with things in it, deleted with them
    Folder,
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Sub-path relative to the Builds folder; "" is the root.
    ListBuilds {
        folder: String,
    },
    NewBuild,
    /// PoB's Rename button on a build or a folder of the Builds folder (R16). `name` is the new
    /// name without `.xml`. Refuses the open build (or a folder holding it).
    RenameBuild {
        path: PathBuf,
        name: String,
    },
    /// PoB's Copy button: the entry copied under `name` in its own folder (R16)
    CopyBuild {
        path: PathBuf,
        name: String,
    },
    /// PoB's permanent delete (R16). The UI asks first, with `BuildListEntry::delete_prompt`;
    /// a folder goes with everything in it. Refuses the open build (or a folder holding it).
    DeleteBuild {
        path: PathBuf,
        /// What the UI asked about, from the entry's `delete_prompt`. The engine deletes only
        /// when PoB asks the same question (R16).
        kind: DeleteKind,
    },
    /// PoB's Ctrl+X, Ctrl+V into `folder`, a sub-path of the Builds folder ("" is the root)
    MoveBuild {
        path: PathBuf,
        folder: String,
    },
    /// PoB's Ctrl+C on an entry, then Ctrl+V in `folder` (a sub-path of the Builds folder, not
    /// the entry's own: there PoB asks for a name, which is `CopyBuild`)
    CopyBuildTo {
        path: PathBuf,
        folder: String,
    },
    /// PoB's New Folder in the listed folder
    NewFolder {
        name: String,
    },
    /// PoB's New Folder in the Save As picker: the folder is named by `folder`, whatever the
    /// build list shows
    NewFolderIn {
        folder: String,
        name: String,
    },
    /// PoB's sort dropdown; the choice is saved to `Settings.xml` at once (R16)
    SetBuildSort {
        sort: BuildSort,
    },
    /// Types into PoB's build list search box; not saved (R16)
    SetBuildFilter {
        text: String,
    },
    OpenBuild {
        path: PathBuf,
    },
    ImportShareCode {
        code: String,
    },
    /// Pastes into PoB's import box, whose validator runs live; the verdict is in
    /// `ShareView.import` (R16).
    SetImportCode {
        text: String,
    },
    /// Imports from a build site's link, a `pob://` link or a share code (R15, R16). A link is
    /// a network request; a code imports at once.
    ImportFromUrl {
        url: String,
        mode: ImportMode,
    },
    /// Uploads the build to a build site (a network request); the new link comes in
    /// `ShareView.link` (R17).
    ShareBuild {
        site: String,
    },
    /// Lists an account's public characters: PoB's "Import By Account Name" Start button (R19).
    /// A request; `AccountView.public` follows it.
    ListPublicCharacters {
        account: String,
        realm: Realm,
    },
    /// Imports a part of a listed character (R18, R19): a request. `league` `None` is PoB's
    /// "All" entry, which `PublicView.leagues` lists last as the label "All".
    ImportPublicCharacter {
        league: Option<String>,
        name: String,
        part: CharacterPart,
    },
    /// PoB's Close button: back to the account box (R19)
    ClosePublicCharacters,
    /// PoB's "Authorize with Path of Exile" (shown only when signed out): a request whose
    /// sign-in link comes in `OAuthView.auth_url`, for the UI to open (R1).
    SignIn,
    /// PoB's "Logout from Path of Exile API": PoB forgets the tokens and saves its settings.
    SignOut,
    /// Selects the realm and presses Fetch when PoB offers it, once per realm (R18). A request,
    /// unless the realm's list is there already or PoB's rate limiter answers at once.
    ListCharacters {
        realm: Realm,
    },
    /// Imports a part of a signed-in account's character (R18): a request, unless PoB's rate
    /// limiter answers at once. `league` `None` is PoB's "Any".
    ImportCharacter {
        realm: Realm,
        league: Option<String>,
        name: String,
        part: CharacterPart,
    },
    /// Ask for the build's share code: PoB's Generate button, no network (R17).
    ExportShareCode,
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
    /// Types the level into the top bar's level box (`characterLevel`, R21). PoB clamps it to
    /// 1..=100 and switches to Manual.
    SetCharacterLevel {
        level: u32,
    },
    /// Presses the top bar's Auto/Manual button when the mode differs (R21).
    SetLevelMode {
        auto: bool,
    },
    /// Picks the bloodline (`secondaryAscendDrop`, R21) by its id; 0 is "None".
    SelectSecondaryAscendancy {
        id: u32,
    },
    /// Picks a loadout in the top bar's dropdown (R21): a real entry of `BuildView.loadouts`,
    /// not one of PoB's headers or actions.
    SelectLoadout {
        label: String,
    },
    /// The loadouts dropdown's "New Loadout" and its name popup: a tree, item set, skill set and
    /// config set with this title (R21).
    NewLoadout {
        name: String,
    },
    /// The loadouts dropdown's "Sync" (R21).
    SyncLoadouts,
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
    /// Cancel and Escape: drops the session. On Options it presses PoB's own Cancel first.
    ClosePopup,
    /// The app's own values for PoB's settings (R12), by `Settings.xml` key: replaces the whole
    /// map, so a key left out goes back to its shared value. Needs no build. Sent after `Ready`
    /// and after each Options Save. The engine never writes `auto-pob.toml`; the UI does.
    SetOverrides {
        values: BTreeMap<String, String>,
    },
    /// Stops the running timeless search (R17). PoB's previous results stay: it replaces them
    /// only when a search finishes. A no-op with none running.
    CancelTimelessSearch,
    /// The app's trade realm ("PC", "PS4" or "Xbox") and league, from `auto-pob.toml` (R10,
    /// R17): the trade windows take them instead of PoB's league fetch. Kept in the glue,
    /// needs no build, answers nothing. Sent after `Ready` and after each Options Save.
    SetTradeDefaults {
        realm: String,
        league: String,
    },
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
    /// Ctrl+click on a database row: adds the item and equips it (R15). `second_slot` is
    /// Ctrl+Shift+click, which equips into the second slot where the item fits it (R23).
    AddDbItem {
        db: ItemDb,
        name: String,
        second_slot: bool,
    },

    /// Adds a comparison build (R5); it becomes the active one. A build PoB can't load is
    /// PoB's `Failed to import build for comparison.`
    LoadCompare {
        source: CompareSource,
    },
    /// PoB's "Compare with:" dropdown; `index` is 0-based in `CompareView.entries`
    SelectCompare {
        index: usize,
    },
    /// Selects the comparison, then presses PoB's Remove, which removes the active one (R5)
    RemoveCompare {
        index: usize,
    },
    /// Sets one of PoB's Compare controls, from R6's list (`CompareView.controls`)
    SetCompareControl {
        name: String,
        value: PopupInput,
    },
    /// PoB's "Copy tree" (`and_use: false`) and "Copy and use" buttons (R8)
    CopyCompareTree {
        and_use: bool,
    },
    /// PoB's Copy (`and_use: false`) and Equip buttons on an Items row of the Compare tab: the
    /// comparison's item in `slot` (`CompareItemRow.slot`) is added, and equipped (R8). An Items
    /// undo state.
    CopyCompareItem {
        slot: String,
        and_use: bool,
    },
    /// PoB's "Copy Config from Compare Build", plus a Config undo state of ours (R8)
    CopyCompareConfig,
    /// UI state, sent on entering (`on`) and leaving the Compare tab's Tree view: while on,
    /// `TreeState.compare` colours the tree against the active comparison (R7). No undo, not
    /// unsaved. A build change drops it with the comparisons.
    SetCompareOverlay {
        on: bool,
    },
    /// PoB's tree version dropdowns on the Compare tab's Tree view: `leftVersionSelect` (this
    /// build's tree) or, with `theirs`, `rightVersionSelect` (the comparison's). The pick
    /// opens PoB's "Convert to Version" popup, whose "Convert" (`keep_copy: false`) or
    /// "Copy + Convert" is pressed. `index` is 0-based in the control's choices.
    ConvertCompareTree {
        theirs: bool,
        index: usize,
        label: String,
        keep_copy: bool,
    },
    /// A query: the builds in `folder` (below the Builds folder, "" or "a/b/"), for the Compare
    /// tab's saved-build picker. Read in Rust, no links followed, nothing outside the Builds
    /// folder; unlike `ListBuilds` it doesn't leave the build or change anything.
    BrowseBuilds {
        folder: String,
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
    /// A query: PoB's help, as its About window lists it (`Snapshot::Help`). It needs no build
    /// and changes nothing.
    Help,
    /// P on the tree (PassiveTreeView.lua:204-205): shows or hides the heat map, keeping its
    /// stat. With no stat chosen yet PoB shows damage against defence.
    ToggleHeatmap,
    /// Ctrl+D on the tree (PassiveTreeView.lua:206-207): whether a node's tooltip lists what
    /// allocating it changes (PassiveTreeView.lua:1659)
    SetTreeStatDifferences {
        on: bool,
    },
    /// A click with Shift held on the last node of the traced path: PoB allocates the path
    /// traced through the hovered nodes instead of the node's own (PassiveTreeView.lua:504).
    /// `path` runs from the tree to `id`, which is its last.
    AllocTracePath {
        id: u32,
        path: Vec<u32>,
    },
    /// A query, as `PreviewNode`, while Shift traces a path: the tooltip PoB shows then
    /// (PassiveTreeView.lua:1662, 1715-1716)
    PreviewTrace {
        id: u32,
        path: Vec<u32>,
    },

    /// Drags a skill set's row in the manager to a new place (R31). Positions are 0-based in
    /// `SkillsView.sets`; `to` is the set's final position. PoB only marks the tab changed.
    MoveSkillSet {
        from: usize,
        to: usize,
    },
    /// A query: what hovering this control shows, PoB's `tooltipFunc` with its "what if" stat
    /// comparison (R22). The latest one in a batch is answered, as a `HoverTooltip`.
    HoverTooltip(HoverTarget),
    /// The config set manager's Copy button and its name popup (NF-4).
    CopyConfigSet {
        id: u32,
        title: String,
    },
    /// Drags a config set's row in the manager (NF-4); 0-based positions in `ConfigView.sets`,
    /// `to` the final one.
    MoveConfigSet {
        from: usize,
        to: usize,
    },
    /// The tree manager's Copy button and its "Copy Tree" name popup (NF-3); 0-based.
    CopySpec {
        index: usize,
        title: String,
    },
    /// Drags a tree's row in the tree manager (NF-3); 0-based positions in `TreeState.specs`,
    /// `to` the final one.
    MoveSpec {
        from: usize,
        to: usize,
    },
    /// The power report's filter dropdown (0-based: Show Unallocated, Show Unallocated &
    /// Clusters, Show Allocated) and its Show Masteries box (NF-5). UI state of PoB's list,
    /// not saved.
    SetPowerReportFilter {
        filter: usize,
        masteries: bool,
    },
    /// The Import tab's X next to the account history: removes the account from the list
    /// and from PoB's remembered accounts (NF-8)
    RemoveAccountHistory {
        account: String,
    },
    /// PoB's "Convert to <latest>" button under an older tree: the tree is copied into the
    /// latest version beside it, and PoB's "Tree Converted" message comes as `Event::Message`
    /// (TreeTab:610)
    ConvertSpecLatest,
    /// A click on another class's ascendancy node while the tree doesn't connect to that
    /// class: PoB's "Class Change" question, answered with Continue (the tree is reset) or,
    /// with `connect`, Connect Path (PassiveTreeView.lua:464-492)
    AllocNodeChangingClass {
        id: u32,
        connect: bool,
    },
    /// A database row dragged onto a slot (R23): PoB's `CanReceiveDrag`, then `ReceiveDrag`,
    /// which adds the item, equips it and records an undo state. A slot that can't take the
    /// item is `that item can't go in that slot`.
    DropDbItem {
        db: ItemDb,
        name: String,
        slot: String,
    },
    /// Ctrl+click on an item list row (`ItemListControl:OnSelClick`): equips the item into its
    /// slot, or takes it out when it is already there; `second_slot` is Ctrl+Shift+click,
    /// the second slot where the item fits it.
    QuickEquipItem {
        id: u32,
        second_slot: bool,
    },
    /// PoB's shared items (NF-2), kept in Settings.xml across builds. An edit of the shared
    /// lists saves Settings.xml through PoB's `main:SaveSettings` (R29 amended by the
    /// controller: shared item list edits), overrides kept out (R12).
    ///
    /// A build item or a database row dragged onto the shared list, dropped at 0-based row
    /// `index` (`index` = the length appends).
    AddSharedItem {
        source: SharedItemSource,
        index: usize,
    },
    /// Shared item `index` dragged onto `slot`, or onto the build's item list (`slot` `None`):
    /// a copy is added to the build. On the item list it lands at 0-based row `at`, the row
    /// under the cursor, as PoB's list marks it (ItemListControl.lua:275-283); `None`, or a list
    /// PoB doesn't let reorder (a loadout filter), appends. The shared list is unchanged and
    /// nothing is saved.
    DropSharedItem {
        index: usize,
        slot: Option<String>,
        at: Option<usize>,
    },
    /// The shared list's Delete, confirmed in the UI (PoB's "Delete Item" question)
    DeleteSharedItem {
        index: usize,
    },
    /// A shared item dragged within the shared list; `to` is its final position
    MoveSharedItem {
        from: usize,
        to: usize,
    },
    /// The double click on a shared item: it goes to the display panel
    ViewSharedItem {
        index: usize,
    },
    /// The build's item set `id` dragged onto the Manage Item Sets window's shared set list
    AddSharedItemSet {
        id: u32,
    },
    /// Shared set `index` dragged onto the build's set list: a new item set with copies
    ImportSharedItemSet {
        index: usize,
    },
    /// The shared set list's Delete, confirmed in the UI (PoB's "Delete Item Set" question)
    DeleteSharedItemSet {
        index: usize,
    },
    /// The shared set list's Rename (or F2); a blank title is `that name isn't allowed`
    RenameSharedItemSet {
        index: usize,
        title: String,
    },
    MoveSharedItemSet {
        from: usize,
        to: usize,
    },
    /// A database row dragged onto the build's item list: added at 0-based row `at` (as
    /// `DropSharedItem`), not equipped (ItemsTab.lua:1184-1187, ItemListControl.lua:275-283)
    DropDbItemOnList {
        db: ItemDb,
        name: String,
        at: Option<usize>,
    },
    /// Ctrl+C on a shared item row (SharedItemListControl.lua:62-64): a query, answered with
    /// the text PoB copies
    CopySharedItem {
        index: usize,
    },
    /// An item dragged onto the sidebar's minion dropdown while it shows an item set (Animate
    /// Guardian): PoB equips it into that set for the minion (Build.lua:534-543)
    DropOnMinion {
        source: ItemDragSource,
    },
}

/// A row being dragged from one of the Items tab's lists
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemDragSource {
    /// A build item, by id
    Item { id: u32 },
    /// A database row
    Db { db: ItemDb, name: String },
    /// A shared item, by 0-based index
    Shared { index: usize },
}

/// What is dragged onto the shared item list
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharedItemSource {
    /// A build item, by id
    Item { id: u32 },
    /// A database row
    Db { db: ItemDb, name: String },
}

/// A control whose hover tooltip PoB computes (R22). Gem positions are 0-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoverTarget {
    /// A gem's Quality box: its quality lines and "Setting to 20 quality will give you:"
    GemQuality { group: usize, slot: usize },
    /// A gem's enable box: "Disabling/Enabling this gem will give you:"
    GemEnabled { group: usize, slot: usize },
    /// A gem's Count box: PoB's note on what the count does
    GemCount { group: usize, slot: usize },
    /// A Config option's control, as hovering its box shows it
    ConfigOption { var: String },
    /// A custom modifier group's enable box (0-based): "Disabling/Enabling this group..."
    CustomModEnabled { group: usize },
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
/// slot or one of its dropdown entries; `theirs` asks for the comparison's item in the slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTooltipQuery {
    pub id: u32,
    pub slot: Option<String>,
    /// The active comparison's item in `slot` (`id` is not used): what hovering it on the
    /// Compare tab's Items view shows
    #[serde(default)]
    pub theirs: bool,
    /// Shift is held: PoB's tooltip then fills the granted skills' child tooltips, and a list
    /// hover reads the item's second-slot mods (R34)
    #[serde(default)]
    pub shift: bool,
    /// Hovering shared item `shared` (0-based; `id` and `slot` are not used)
    #[serde(default)]
    pub shared: Option<usize>,
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
    /// PoB's Options window, run as a main-owned session (R13, R19): it needs no build
    Options,
    /// PoB's "Buy Similar" window (`CompareBuySimilar.openPopup`, R10), a build-owned session;
    /// its realm and league are the app's trade defaults (`SetTradeDefaults`)
    BuySimilar {
        source: BuySource,
    },
    /// The Tree tab's "Find Timeless Jewel" window (`TreeTab:FindTimelessJewel`, R17), a
    /// build-owned session. Its `searchButton` press starts `BackgroundTask::Timeless`.
    TimelessJewel,
    /// PoB's "Spectre Library" (`build:OpenSpectreLibrary`, R18), a build-owned session opened
    /// by the sidebar's or the Calcs selector's "Manage Spectres..." button; its Save commits
    SpectreLibrary,
    /// The Config tab's "Add Mod" on custom modifier group `group` (0-based):
    /// `configTab:OpenAddModPopup(blockData)` as a build-owned session (R28). Its "Add" appends
    /// the picked modifier to the group's text.
    ModBrowser {
        group: usize,
    },
}

/// Where a "Buy similar" search starts from (R10): PoB's own three entry points
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuySource {
    /// The display panel's "Buy similar" button
    DisplayItem,
    /// A slot's context menu, with the slot's item
    Slot(String),
    /// The Compare tab's Items row: the comparison's item in this slot (`CompareItemRow.slot`)
    CompareSlot(String),
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
    /// The second click of a double click on a list row (the timeless results add that jewel,
    /// R17)
    RowDoubleClick(String),
    /// A list row clicked with Shift held: the other end of a range (the timeless results'
    /// trade search, R17)
    RowShiftClick(String),
    /// A row of a mutable list dragged to `to` (0-based, its place after the drop), as PoB's
    /// list reorders on release (the Spectre Library's "Spectres in Build")
    RowMove {
        key: String,
        to: usize,
    },
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
        /// Shift held: PoB's child tooltips (R34)
        shift: bool,
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
