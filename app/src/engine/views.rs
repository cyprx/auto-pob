//! Typed snapshots of PoB state, deserialised from Lua tables.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::commands::{
    ControlTooltipQuery, GemSearchQuery, GemTooltipQuery, ItemDb, ItemTooltipQuery, PopupKind,
    Realm,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildView {
    pub name: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
    pub class: String,
    #[serde(default)]
    pub ascendancy: Option<String>,
    pub level: u32,
    pub unsaved: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildListView {
    pub folder: String,
    #[serde(default)]
    pub entries: Vec<BuildListEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildListEntry {
    pub name: String,
    pub is_folder: bool,
    pub path: PathBuf,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub ascendancy: Option<String>,
    #[serde(default)]
    pub level: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct StatsView {
    /// The player's rows. The "Skill DPS" stat is one row per Full DPS skill, in PoB's order.
    #[serde(default)]
    pub stats: Vec<StatRow>,
    /// The main skill's minion, when it has one.
    #[serde(default)]
    pub minion: Vec<StatRow>,
    /// The sidebar's main-skill dropdowns.
    #[serde(default)]
    pub main_skill: MainSkillSelector,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StatRow {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub text: String,
    #[serde(default)]
    pub color: Option<String>,
    /// Lines PoB draws under the row, colour codes kept: a Full DPS skill's part and source.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// The sidebar's main-skill selector (`RefreshSkillSelectControls` on the build's controls). A
/// control is `None` while PoB doesn't show it for the main skill.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MainSkillSelector {
    pub group: DropList,
    #[serde(default)]
    pub skill: Option<DropList>,
    #[serde(default)]
    pub part: Option<DropList>,
    /// The stage count's text
    #[serde(default)]
    pub stages: Option<String>,
    /// The mine count's text
    #[serde(default)]
    pub mines: Option<String>,
    #[serde(default)]
    pub minion: Option<DropList>,
    #[serde(default)]
    pub minion_skill: Option<DropList>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigView {
    #[serde(default)]
    pub sets: Vec<ConfigSet>,
    pub active_set: u32,
    /// The active set's custom modifier groups, in PoB's order.
    #[serde(default)]
    pub custom_mods: Vec<CustomModGroup>,
    #[serde(default)]
    pub sections: Vec<ConfigSection>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CustomModGroup {
    pub title: String,
    pub enabled: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigSet {
    pub id: u32,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigSection {
    pub name: String,
    pub column: u32,
    #[serde(default)]
    pub options: Vec<ConfigOption>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigOption {
    pub var: String,
    pub kind: ConfigKind,
    #[serde(default)]
    pub allow_zero: bool,
    pub label: String,
    pub shown: bool,
    /// PoB's "set but invalid" state: the option has a non-default value but its condition
    /// isn't met any more. PoB colours its label red; the tooltip then ends with a warning line.
    #[serde(default)]
    pub invalid: bool,
    /// The current value. Its type follows the option: a bool, a number or a string, and for
    /// lists the raw `val`, which can be a number; use `selected` to match a `Choice`.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub placeholder: Option<serde_json::Value>,
    /// For lists, `tostring(val)` of the current value, matching a `Choice::value`.
    #[serde(default)]
    pub selected: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub tooltip: Vec<TooltipLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigKind {
    Check,
    Count,
    Integer,
    Float,
    List,
    Text,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TooltipLine {
    Line { size: u32, text: String },
    Separator { separator: u32 },
}

/// Power of each unallocated node for the chosen stat, while or after PoB calculates it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodePower {
    pub stat: Option<String>,
    pub nodes: Vec<NodePowerEntry>,
    pub max_single: f64,
    pub max_offence: f64,
    pub max_defence: f64,
    pub finished: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodePowerEntry {
    pub id: u32,
    pub single: f64,
    #[serde(default)]
    pub path: Option<f64>,
    #[serde(default)]
    pub offence: Option<f64>,
    #[serde(default)]
    pub defence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PowerReport {
    pub stat: String,
    pub rows: Vec<PowerReportRow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PowerReportRow {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub power: f64,
    pub power_text: String,
    pub path_power: f64,
    pub path_power_text: String,
    pub allocated: bool,
    pub path_dist: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodeTooltip {
    pub id: u32,
    pub lines: Vec<TooltipLine>,
}

/// What PoB's group list shows when the mouse is over a socket group (`AddValueTooltip`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GroupTooltipView {
    pub group: usize,
    pub lines: Vec<TooltipLine>,
}

/// The passive tree's geometry for the active spec, in PoB's tree coordinates. It changes only
/// with the tree version or cluster jewel subgraphs, so it is sent apart from [`TreeState`].
#[derive(Debug, Clone)]
pub struct TreeLayout {
    /// `spec.treeVersion`
    pub version: String,
    /// Every node with a position in the active spec, ascendancies included
    pub nodes: Vec<TreeNode>,
    /// Connections, as indices into `nodes`
    pub edges: Vec<(usize, usize)>,
    /// Sprite sheets, indexed by [`Sprite::sheet`]. `None` if the image couldn't be loaded.
    pub sheets: Vec<Option<Arc<SheetImage>>>,
    /// The version plus a digest of the node ids; caches built from the layout are stale when
    /// it changes.
    pub key: String,
}

/// A decoded sprite sheet.
pub struct SheetImage {
    pub path: PathBuf,
    pub size: [usize; 2],
    /// RGBA, 8 bits per channel, not premultiplied
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for SheetImage {
    // the pixels would flood any log
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetImage")
            .field("path", &self.path)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// A rectangle on a sprite sheet.
#[derive(Debug, Clone, Copy)]
pub struct Sprite {
    pub sheet: usize,
    /// x, y, width, height in pixels
    pub rect: [f32; 4],
}

/// A node's artwork as [unallocated, allocated] pairs.
#[derive(Debug, Default, Clone)]
pub struct NodeArt {
    pub icon: [Option<Sprite>; 2],
    pub frame: [Option<Sprite>; 2],
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub id: u32,
    pub pos: [f32; 2],
    /// PoB's node type: "Normal", "Notable", "Keystone", "Mastery", "Socket", "ClassStart" or
    /// "AscendClassStart"
    pub kind: String,
    pub name: String,
    /// Stat descriptions, one per line
    pub stats: Vec<String>,
    /// (group, orbit). Nodes sharing both are connected by an arc around `group_center`.
    pub orbit: Option<(u32, u32)>,
    pub group_center: Option<[f32; 2]>,
    pub art: NodeArt,
    /// The ascendancy the node belongs to
    pub ascendancy: Option<String>,
    /// Whether PoB's right-click offers to replace this node's modifier: a tattoo, a small
    /// attribute, a "+30 to" attribute notable, or a keystone that isn't an expansion skill
    /// (R18, PassiveTreeView.lua:519-526)
    pub replaceable: bool,
    /// Whether the node is a mastery with effects to choose from (R19)
    pub mastery_effects: bool,
}

/// The build's state on the active tree.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TreeState {
    pub version: String,
    pub version_label: String,
    /// The newest tree version's id, when the spec's is older
    #[serde(default)]
    pub convertible_to: Option<String>,
    pub class_id: u32,
    pub ascendancy_id: u32,
    /// Sorted by name, like the class dropdown
    #[serde(default)]
    pub classes: Vec<ClassChoice>,
    pub points: Points,
    #[serde(default)]
    pub allocated: Vec<u32>,
    /// Allocated mastery to its chosen effect
    #[serde(default)]
    pub masteries: Vec<MasteryChoice>,
    /// Every mastery node that is allocated or reachable, with the effects PoB offers for it
    #[serde(default)]
    pub mastery_options: Vec<MasteryOptions>,
    /// Reachable unallocated nodes: the path of nodes that allocating one takes
    #[serde(default)]
    pub paths: Vec<NodePath>,
    /// Allocated nodes: the nodes that deallocating one drops (`node.depends`)
    #[serde(default)]
    pub depends: Vec<NodePath>,
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub search_matches: Vec<u32>,
    /// Empty when not comparing
    #[serde(default)]
    pub compare: Vec<CompareColour>,
    #[serde(default)]
    pub jewels: Vec<JewelSocket>,
    #[serde(default)]
    pub specs: Vec<SpecEntry>,
    /// 0-based
    pub active_spec: usize,
    /// 0-based
    #[serde(default)]
    pub compare_spec: Option<usize>,
    #[serde(default)]
    pub power_stats: Vec<String>,
    #[serde(default)]
    pub heatmap: Option<String>,
    #[serde(default)]
    pub max_depth: Option<u32>,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ClassChoice {
    pub id: u32,
    pub name: String,
    /// Choosing it would reset the tree unless the tree connects to the class
    pub needs_confirm: bool,
    #[serde(default)]
    pub ascendancies: Vec<AscendancyChoice>,
}

/// Id 0 is "None", as PoB lists it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AscendancyChoice {
    pub id: u32,
    /// The label the dropdown shows
    pub name: String,
    /// What `TreeNode::ascendancy` holds (PoB's `ascendancyName`); not always the label
    pub ascend_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Points {
    pub used: u32,
    pub available: u32,
    pub ascendancy_used: u32,
    pub ascendancy_available: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct MasteryChoice {
    pub node: u32,
    pub effect: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MasteryOptions {
    pub node: u32,
    #[serde(default)]
    pub options: Vec<MasteryOption>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MasteryOption {
    pub id: u32,
    /// The effect's stats joined with " / ", like PoB's mastery popup
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodePath {
    pub node: u32,
    #[serde(default)]
    pub path: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CompareColour {
    pub node: u32,
    pub rgb: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JewelSocket {
    pub node: u32,
    pub name: String,
    #[serde(default)]
    pub radius_nodes: Vec<u32>,
    /// [inner, outer] radius in tree units
    #[serde(default)]
    pub ring: Option<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SpecEntry {
    pub title: String,
    pub version: String,
    pub points: u32,
}

/// The build's notes, and the colour buttons PoB's notes tab offers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotesView {
    pub text: String,
    #[serde(default)]
    pub colours: Vec<NoteColour>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteColour {
    pub label: String,
    /// The caret code the button inserts, such as `^xAF6025`
    pub code: String,
}

/// The gem picker's list for a search, while or after PoB's DPS sort runs (R9-R14).
#[derive(Debug, Clone, PartialEq)]
pub struct GemSearchView {
    pub query: GemSearchQuery,
    pub rows: Vec<GemRow>,
    /// The DPS without the gem, which the rows' `dps` compare against
    pub base_dps: f64,
    /// The DPS sort hasn't finished
    pub sorting: bool,
    /// The last progress the sort reported
    pub percent: Option<u32>,
    /// The search's group is gone (a recalculation added or dropped an item group); this is
    /// the last view of the search.
    pub ended: bool,
}

/// An item database panel: PoB's controls and, once its list builder ends, the rows (R13-R16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDbView {
    pub db: ItemDb,
    /// PoB is still loading the databases
    pub loading: bool,
    /// The list builder hasn't ended; no rows are sent until it has
    pub sorting: bool,
    pub percent: Option<u32>,
    pub slot: DropList,
    pub kind: DropList,
    /// The uniques database only
    #[serde(default)]
    pub league: Option<DropList>,
    #[serde(default)]
    pub requirement: Option<DropList>,
    #[serde(default)]
    pub obtainable: Option<DropList>,
    #[serde(default)]
    pub sort: Option<DropList>,
    pub search: String,
    pub search_mode: DropList,
    #[serde(default)]
    pub rows: Vec<DbRow>,
    /// The list's own text ("No items found that match those filters.", "Loading...")
    pub note: String,
}

/// One database item the list shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbRow {
    pub name: String,
    /// `GetRowValue`: the name in its rarity colour
    pub label: String,
    pub base: String,
    /// The measured power of a stat sort; none when the item fits no slot
    #[serde(default)]
    pub sort_value: Option<f64>,
}

/// One gem the picker lists, in PoB's order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GemRow {
    pub gem_id: String,
    pub name: String,
    /// The PoB colour code PoB draws the name in
    pub colour: String,
    pub tags: String,
    pub support: bool,
    /// The gem can support the group's skills
    pub can_support: bool,
    /// The DPS with this gem, once PoB has calculated it
    #[serde(default)]
    pub dps: Option<f64>,
    /// PoB's colour for that DPS: better, worse or the same
    #[serde(default)]
    pub dps_colour: Option<String>,
    #[serde(default)]
    pub mark: Option<DpsMark>,
}

/// What PoB draws beside a calculated gem (R14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum DpsMark {
    /// A support that can support the group's skills
    Check,
    /// A skill with a global effect
    Plus,
}

/// The lines of a gem tooltip (R15).
#[derive(Debug, Clone, PartialEq)]
pub struct GemTooltipView {
    pub query: GemTooltipQuery,
    pub lines: Vec<TooltipLine>,
}

/// The Skills tab: PoB's skill sets, socket groups, gems and gem options.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillsView {
    #[serde(default)]
    pub sets: Vec<SkillSetEntry>,
    pub active_set: u32,
    #[serde(default)]
    pub groups: Vec<SocketGroupView>,
    #[serde(default)]
    pub slot_choices: Vec<SlotChoice>,
    pub options: SkillOptionsView,
    #[serde(default)]
    pub can_undo: bool,
    #[serde(default)]
    pub can_redo: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillSetEntry {
    pub id: u32,
    /// PoB shows "Default" for a set with no title
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SocketGroupView {
    pub label: String,
    /// The group list's row text, with PoB's colour codes
    pub display_label: String,
    #[serde(default)]
    pub slot: Option<String>,
    pub enabled: bool,
    pub slot_enabled: bool,
    pub full_dps: bool,
    /// The group count's text; only item-sourced groups have one
    #[serde(default)]
    pub count: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_note: Option<String>,
    pub is_main: bool,
    #[serde(default)]
    pub imbued: Option<ImbuedView>,
    #[serde(default)]
    pub sockets_line: Option<String>,
    pub can_optimise: bool,
    #[serde(default)]
    pub gems: Vec<GemView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImbuedView {
    #[serde(default)]
    pub name: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GemView {
    pub name_spec: String,
    #[serde(default)]
    pub gem_id: Option<String>,
    pub level: u32,
    pub quality: u32,
    pub enabled: bool,
    #[serde(default)]
    pub count: Option<u32>,
    /// A PoB colour code
    pub colour: String,
    pub support: bool,
    pub matches_socket: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub toggle1: Option<GemToggleView>,
    #[serde(default)]
    pub toggle2: Option<GemToggleView>,
}

/// A Vaal gem's "Enable <skill>" check box.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GemToggleView {
    pub label: String,
    pub on: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SlotChoice {
    pub label: String,
    #[serde(default)]
    pub slot: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillOptionsView {
    pub sort_by_dps: bool,
    pub sort_field: DropList,
    pub default_level: DropList,
    #[serde(default)]
    pub default_level_help: Vec<String>,
    pub default_quality: String,
    pub support_types: DropList,
    pub show_legacy: bool,
}

/// The Calcs tab: PoB's sections as `CalcSectionControl:DrawContent` shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CalcsView {
    #[serde(default)]
    pub sections: Vec<CalcsSection>,
    #[serde(default)]
    pub selector: CalcsSelector,
    /// The search box's text
    #[serde(default)]
    pub search: String,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// The Items tab: the build's items, the equipment slots and the item sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemsView {
    /// In PoB's order (`itemOrderList`)
    #[serde(default)]
    pub items: Vec<ItemEntry>,
    /// The item list's loadout filter
    #[serde(default)]
    pub list_filter: DropList,
    /// The ids the list shows under its filter, in order
    #[serde(default)]
    pub listed: Vec<u32>,
    /// The list can be reordered (no filter is on)
    #[serde(default)]
    pub can_reorder: bool,
    /// In PoB's order (`orderedSlots`)
    #[serde(default)]
    pub slots: Vec<SlotView>,
    #[serde(default)]
    pub sets: Vec<ItemSetEntry>,
    pub active_set: u32,
    /// The active set shows its swap weapons
    #[serde(default)]
    pub second_weapon_set: bool,
    #[serde(default)]
    pub show_stat_differences: bool,
    #[serde(default)]
    pub can_undo: bool,
    #[serde(default)]
    pub can_redo: bool,
}

/// One item of the build.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemEntry {
    pub id: u32,
    pub name: String,
    pub base: String,
    pub rarity: String,
    /// PoB's colour code for the rarity
    pub colour: String,
    /// The list row's text, with the "(Unused)" or "(Used in ...)" marker and PoB's colour codes
    pub display_label: String,
    /// The slot PoB would equip it in first
    #[serde(default)]
    pub primary_slot: Option<String>,
    /// The first slot it is equipped in (in this set, or in another one)
    #[serde(default)]
    pub equipped: Option<EquippedIn>,
    /// The title of another set that has it as an abyss jewel
    #[serde(default)]
    pub abyss_set: Option<String>,
    /// The title of another tree that has it socketed
    #[serde(default)]
    pub tree: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EquippedIn {
    pub slot: String,
    pub label: String,
    /// The title of the set, when it isn't the active one
    #[serde(default)]
    pub set: Option<String>,
}

/// One equipment slot, with PoB's own dropdown for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SlotView {
    /// PoB's slot name, e.g. "Ring 2" or "Jewel 26725"
    pub name: String,
    pub label: String,
    pub kind: SlotKind,
    /// The slot an abyssal socket belongs to
    #[serde(default)]
    pub parent: Option<String>,
    /// The tree node of a jewel socket
    #[serde(default)]
    pub node: Option<u32>,
    /// PoB shows the slot
    pub shown: bool,
    /// PoB lets the dropdown be used (it has an item to choose)
    pub enabled: bool,
    /// The equipped item's id, 0 for none
    pub selected: u32,
    /// The ids the dropdown offers after "None"
    #[serde(default)]
    pub choices: Vec<u32>,
    /// The flask's activation box, for a flask slot
    #[serde(default)]
    pub flask_active: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotKind {
    #[default]
    Gear,
    Swap,
    Abyssal,
    Flask,
    Graft,
    Jewel,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemSetEntry {
    pub id: u32,
    /// "Default" for a set with no title
    pub title: String,
}

/// One of the selector's dropdowns. Labels keep PoB's colour codes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DropList {
    #[serde(default)]
    pub labels: Vec<String>,
    /// 0-based
    pub selected: usize,
    pub enabled: bool,
}

/// The Calcs tab's skill and mode selector. A control is `None` while PoB doesn't show it for
/// the main skill.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CalcsSelector {
    pub group: DropList,
    #[serde(default)]
    pub skill: Option<DropList>,
    #[serde(default)]
    pub part: Option<DropList>,
    /// The stage count's text
    #[serde(default)]
    pub stages: Option<String>,
    /// The mine count's text
    #[serde(default)]
    pub mines: Option<String>,
    #[serde(default)]
    pub show_minion: Option<bool>,
    #[serde(default)]
    pub minion: Option<DropList>,
    #[serde(default)]
    pub minion_skill: Option<DropList>,
    pub mode: DropList,
    #[serde(default)]
    pub mode_tooltip: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalcsSection {
    pub id: String,
    pub group: u32,
    pub width_cols: u32,
    /// A PoB colour code, `^xRRGGBB`
    pub colour: String,
    pub enabled: bool,
    #[serde(default)]
    pub subs: Vec<CalcsSub>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalcsSub {
    pub label: String,
    #[serde(default)]
    pub extra: Option<String>,
    pub collapsed: bool,
    pub search_match: bool,
    #[serde(default)]
    pub rows: Vec<CalcsRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalcsRow {
    /// Position in PoB's `subSec.data`, 0-based
    pub index: usize,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub colour: Option<String>,
    pub text_size: u32,
    pub search_match: bool,
    #[serde(default)]
    pub cells: Vec<CalcsCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalcsCell {
    /// Position in PoB's `rowData`, 0-based
    pub index: usize,
    /// As `formatCalcStr` gives it, colour codes included
    pub text: String,
    /// The cell has a breakdown to show
    pub breakdown: bool,
}

/// One cell of the Calcs tab, by 0-based positions in PoB's own (unfiltered) data.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct CellRef {
    pub section: String,
    pub sub: usize,
    pub row: usize,
    pub cell: usize,
}

/// What PoB's Copy button writes for a socket group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SocketGroupText {
    pub group: usize,
    pub text: String,
}

/// What PoB's breakdown control shows for a [`CellRef`]; no sections when the cell has none
/// (or is gone, or its flags no longer hold).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakdownView {
    pub cell: CellRef,
    #[serde(default)]
    pub sections: Vec<BreakdownSection>,
    /// Why the query failed, when it did; the sections are then empty
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BreakdownSection {
    Text {
        /// Separators formatted, colour codes kept
        #[serde(default)]
        lines: Vec<String>,
        size: u32,
    },
    Table {
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        footer: Option<String>,
        /// Only the columns some row has a value for
        #[serde(default)]
        columns: Vec<BreakdownColumn>,
        #[serde(default)]
        rows: Vec<BreakdownRow>,
    },
    Radius {
        radius: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakdownColumn {
    pub label: String,
    pub key: String,
    #[serde(default)]
    pub right: bool,
}

/// A row's cells by column key, as the breakdown draws them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BreakdownRow {
    #[serde(default)]
    pub cells: BTreeMap<String, String>,
    /// Shown on hovering the cell
    #[serde(default)]
    pub tooltips: BTreeMap<String, Vec<TooltipLine>>,
    /// The passive node the cell points at (shown on the tree)
    #[serde(default)]
    pub nodes: BTreeMap<String, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::LuaSerdeExt;

    fn lua_value<T: serde::de::DeserializeOwned>(code: &str) -> T {
        let lua = mlua::Lua::new();
        let value: mlua::Value = lua.load(code).eval().unwrap();
        lua.from_value(value).unwrap()
    }

    #[test]
    fn config_view_from_lua_table() {
        let view: ConfigView = lua_value(
            r#"return {
              sets = { { id = 1, title = "Default" } },
              active_set = 1,
              custom_mods = { { title = "Default", enabled = true, text = "+1 to all" } },
              sections = { {
                name = "General", column = 1,
                options = {
                  { var = "usePowerCharges", kind = "check", allow_zero = false,
                    label = "Use power charges?", shown = true, value = true,
                    choices = {}, tooltip = {} },
                  { var = "enemyIsBoss", kind = "list", allow_zero = false,
                    label = "Enemy is a Boss?", shown = true, value = "None",
                    placeholder = 5,
                    choices = { { label = "No", value = "None" }, { label = "Yes", value = "Boss" } },
                    tooltip = { { size = 14, text = "Hello" }, { separator = 8 } } },
                } } },
            }"#,
        );
        let expected = ConfigView {
            sets: vec![ConfigSet {
                id: 1,
                title: "Default".into(),
            }],
            active_set: 1,
            custom_mods: vec![CustomModGroup {
                title: "Default".into(),
                enabled: true,
                text: "+1 to all".into(),
            }],
            sections: vec![ConfigSection {
                name: "General".into(),
                column: 1,
                options: vec![
                    ConfigOption {
                        var: "usePowerCharges".into(),
                        kind: ConfigKind::Check,
                        allow_zero: false,
                        label: "Use power charges?".into(),
                        shown: true,
                        invalid: false,
                        selected: None,
                        value: Some(serde_json::json!(true)),
                        placeholder: None,
                        choices: vec![],
                        tooltip: vec![],
                    },
                    ConfigOption {
                        var: "enemyIsBoss".into(),
                        kind: ConfigKind::List,
                        allow_zero: false,
                        label: "Enemy is a Boss?".into(),
                        shown: true,
                        invalid: false,
                        selected: None,
                        value: Some(serde_json::json!("None")),
                        placeholder: Some(serde_json::json!(5)),
                        choices: vec![
                            Choice {
                                label: "No".into(),
                                value: "None".into(),
                            },
                            Choice {
                                label: "Yes".into(),
                                value: "Boss".into(),
                            },
                        ],
                        tooltip: vec![
                            TooltipLine::Line {
                                size: 14,
                                text: "Hello".into(),
                            },
                            TooltipLine::Separator { separator: 8 },
                        ],
                    },
                ],
            }],
        };
        assert_eq!(view, expected);
    }

    #[test]
    fn tree_state_from_lua_table() {
        let full: TreeState = lua_value(
            r#"return {
              version = "3_28", version_label = "3.28", convertible_to = "3_29",
              class_id = 3, ascendancy_id = 1,
              classes = { { id = 3, name = "Witch", needs_confirm = true,
                            ascendancies = { { id = 0, name = "None", ascend_id = "None" }, { id = 1, name = "Necromancer", ascend_id = "Necromancer" } } } },
              points = { used = 10, available = 122, ascendancy_used = 2, ascendancy_available = 8 },
              allocated = { 5, 6 },
              masteries = { { node = 7, effect = 70 } },
              mastery_options = { { node = 7, options = { { id = 70, text = "a / b" } } } },
              paths = { { node = 8, path = { 8, 5 } } },
              depends = { { node = 6, path = { 6 } } },
              search = "life", search_matches = { 5 },
              compare = { { node = 5, rgb = { 1, 0.5, 0 } } },
              jewels = { { node = 9, name = "Jewel", radius_nodes = { 5 }, ring = { 0, 960 } } },
              specs = { { title = "Default", version = "3_28", points = 10 } },
              active_spec = 0, compare_spec = 1,
              power_stats = { "Life" }, heatmap = "Life", max_depth = 4,
              can_undo = true, can_redo = false,
            }"#,
        );
        assert_eq!(full.convertible_to.as_deref(), Some("3_29"));
        assert_eq!(full.classes[0].ascendancies[1].name, "Necromancer");
        assert!(full.classes[0].needs_confirm);
        assert_eq!(full.points.available, 122);
        assert_eq!(
            full.masteries,
            [MasteryChoice {
                node: 7,
                effect: 70
            }]
        );
        assert_eq!(full.mastery_options[0].options[0].text, "a / b");
        assert_eq!(full.paths[0].path, [8, 5]);
        assert_eq!(full.depends[0].node, 6);
        assert_eq!(full.compare[0].rgb, [1.0, 0.5, 0.0]);
        assert_eq!(full.jewels[0].ring, Some([0.0, 960.0]));
        assert_eq!(full.specs[0].points, 10);
        assert_eq!((full.compare_spec, full.max_depth), (Some(1), Some(4)));
        assert_eq!(full.heatmap.as_deref(), Some("Life"));

        let bare: TreeState = lua_value(
            r#"return {
              version = "3_28", version_label = "3.28", class_id = 0, ascendancy_id = 0,
              points = { used = 0, available = 122, ascendancy_used = 0, ascendancy_available = 8 },
              active_spec = 0, can_undo = false, can_redo = false,
            }"#,
        );
        assert!(bare.classes.is_empty() && bare.allocated.is_empty());
        assert!(bare.masteries.is_empty() && bare.mastery_options.is_empty());
        assert!(bare.paths.is_empty() && bare.depends.is_empty());
        assert!(bare.search_matches.is_empty() && bare.compare.is_empty());
        assert!(bare.jewels.is_empty() && bare.specs.is_empty());
        assert!(bare.power_stats.is_empty());
        assert_eq!(bare.convertible_to, None);
        assert_eq!(
            (bare.compare_spec, bare.heatmap, bare.max_depth),
            (None, None, None)
        );
    }

    #[test]
    fn optional_fields_may_be_missing() {
        let view: BuildView = lua_value(
            r#"return { name = "n", class = "Witch", level = 90,
                        unsaved = false, can_undo = false, can_redo = true }"#,
        );
        assert_eq!(view.path, None);
        assert_eq!(view.ascendancy, None);
        assert_eq!(view.level, 90);
        assert!(view.can_redo);
    }
}

/// PoB's tooltip for an item (`AddItemTooltip` on a `Tooltip`), as data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTooltip {
    /// The rarity, as PoB's `tooltipHeader`
    #[serde(default)]
    pub header: Option<String>,
    /// PoB's `influenceHeader1` and `influenceHeader2`, where set
    #[serde(default)]
    pub influences: Vec<String>,
    /// PoB's `foilType`
    #[serde(default)]
    pub foil: Option<String>,
    /// The border colour code
    #[serde(default)]
    pub colour: Option<String>,
    #[serde(default)]
    pub lines: Vec<ItemTooltipLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemTooltipLine {
    Line {
        size: u32,
        text: String,
        #[serde(default)]
        font: Option<String>,
        #[serde(default)]
        center: bool,
        #[serde(default)]
        background: Option<String>,
        /// The mod line a click on this line toggles (the display item only)
        #[serde(default)]
        mod_line: Option<ModLineRef>,
    },
    Separator {
        separator: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModLineList {
    Enchant,
    Scourge,
    Implicit,
    Explicit,
    Crucible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModLineRef {
    pub list: ModLineList,
    /// 0-based
    pub index: usize,
}

/// The answer to an `ItemTooltip` query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTooltipView {
    pub query: ItemTooltipQuery,
    pub tooltip: ItemTooltip,
}

/// The answer to a `ControlTooltip` query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlTooltipView {
    pub query: ControlTooltipQuery,
    pub tooltip: ItemTooltip,
}

/// The open popup, whatever it is: PoB's controls as data (R5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopupView {
    pub kind: PopupKind,
    pub title: String,
    /// Ordered by position (top to bottom, then left to right)
    pub controls: Vec<PopupControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PopupControlKind {
    Label,
    Button,
    Dropdown,
    Slider,
    Check,
    Edit,
    List,
}

/// One control of a popup. The fields of the other kinds hold their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopupControl {
    /// PoB's control key; a list's children are `parent.child`
    pub name: String,
    pub kind: PopupControlKind,
    pub label: String,
    /// `GetPos()` relative to the popup
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub shown: bool,
    pub enabled: bool,
    /// Dropdown
    #[serde(default)]
    pub choices: Vec<String>,
    pub selected: usize,
    /// Slider, 0..=1
    pub value: f64,
    pub divisions: Option<u32>,
    /// Check, Edit
    pub checked: bool,
    pub text: String,
    /// PoB's static `tooltipText`
    pub tooltip_text: Option<String>,
    /// The control builds a tooltip on hover (ask with `ControlTooltip`)
    pub hover_tooltip: bool,
    /// List
    #[serde(default)]
    pub rows: Vec<PopupRow>,
    pub row_selected: Option<String>,
    /// A list's `defaultText`
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopupRow {
    pub key: String,
    pub label: String,
}

/// The answer to a `CopyItem` query: the text PoB copies, with its `\r\n` line ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemText {
    pub id: u32,
    pub text: String,
}

/// The item on the display panel (PoB's `displayItem`) with the controls PoB shows for it.
/// Each optional part is present only while PoB shows its control.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplayItemView {
    /// The item's id in the build; none for an item that isn't in the build yet
    pub id: Option<u32>,
    pub name: String,
    pub base: String,
    /// PoB's rarity name ("RARE")
    pub rarity: String,
    /// What PoB's "Edit..." popup opens with: the rarity dropdown (0 Normal ... 4 Relic) and
    /// the item text without its Rarity line
    pub edit_rarity: usize,
    pub edit_text: String,
    /// "Save" for an item in the build, "Add to build" for a new one
    pub save_label: String,
    pub version: Option<DropList>,
    #[serde(default)]
    pub variants: Vec<VariantView>,
    #[serde(default)]
    pub sockets: Vec<SocketView>,
    pub can_add_socket: bool,
    /// The two influence dropdowns, or none
    #[serde(default)]
    pub influences: Vec<DropList>,
    pub quality: Option<String>,
    pub catalyst: Option<DropList>,
    pub catalyst_quality: Option<String>,
    #[serde(default)]
    pub range_lines: Vec<RangeLineView>,
    /// The windows PoB's panel offers for this item (R3), in the panel's order
    #[serde(default)]
    pub popups: Vec<PopupKind>,
    /// The shown affix dropdowns of a crafted item (R9), in PoB's slot order
    #[serde(default)]
    pub affixes: Vec<AffixView>,
    /// "Modifier sorting", while PoB shows it
    pub crafting_sort: Option<DropList>,
    /// A crafted cluster jewel's controls (R11)
    pub cluster: Option<ClusterView>,
    /// The crafted, crucible and custom mods PoB lists with a Remove button (R12)
    #[serde(default)]
    pub custom_mods: Vec<CustomModView>,
    pub tooltip: ItemTooltip,
}

/// One line of the display item's custom-mod list
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomModView {
    /// "Crafted", "Crucible" or "Custom"
    pub kind: String,
    /// `itemLib.formatModLine`'s text
    pub line: String,
}

/// One of PoB's affix dropdowns with the slider under it
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AffixView {
    /// 0..=8: `displayItemAffix1..9`
    pub slot: usize,
    /// "Prefix:", "Suffix:" or "Explicit:"
    pub label: String,
    pub list: DropList,
    /// The slider's value (0..=1) while PoB shows it
    pub range: Option<f64>,
    /// The slider's division count: the tiers of the chosen modifier group
    pub tiers: Option<u32>,
}

/// A crafted cluster jewel's skill dropdown and "Added Passives" slider
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClusterView {
    pub skill: DropList,
    pub count: u32,
    pub min: u32,
    pub max: u32,
}

/// One of the six variant dropdowns
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VariantView {
    /// 0..=5
    pub control: usize,
    pub list: DropList,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SocketView {
    /// "R", "G", "B" or "W"
    pub colour: String,
    pub shown: bool,
    /// Linked to the next socket; none for the last one
    pub linked: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RangeLineView {
    pub label: String,
    pub range: Option<f64>,
    pub slider: bool,
    /// A Foulborn box next to the line
    pub mutable: bool,
    pub mutated: bool,
}

/// The network requests: the pending ones in the order they began, and the ones finished since
/// the last view, each sent once (R10).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestsView {
    #[serde(default)]
    pub pending: Vec<PendingRequest>,
    #[serde(default)]
    pub finished: Vec<FinishedRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingRequest {
    pub id: u64,
    /// What the request does, such as `Fetching characters` (R10)
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinishedRequest {
    pub id: u64,
    pub label: String,
    /// Why it failed: PoB's message, curl's, or `Cancelled` (R25)
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub cancelled: bool,
    /// A callback of the request initialised a new build (R14)
    #[serde(default)]
    pub new_build: bool,
}

/// The Import/Export tab's build sharing: the sites Share offers, the last link it made, and
/// PoB's import box (R16, R17).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShareView {
    /// The sites Share uploads to, in PoB's dropdown order
    #[serde(default)]
    pub sites: Vec<SiteRef>,
    /// The site chosen last in this build, if any
    #[serde(default)]
    pub last_site: Option<String>,
    /// The link the last Share made, until the code box changes
    #[serde(default)]
    pub link: Option<String>,
    /// A Share is uploading
    #[serde(default)]
    pub sharing: bool,
    pub import: ImportCodeView,
}

/// A build site by PoB's id (`"POBBin"`) and the name it shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteRef {
    pub id: String,
    pub label: String,
}

/// PoB's import box as it stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportCodeView {
    pub text: String,
    /// PoB's validator accepts the text
    pub valid: bool,
    /// PoB's verdict, such as `URL is valid (pobb.in)`, with colour codes stripped
    #[serde(default)]
    pub detail: String,
    /// The id of the site the text names
    #[serde(default)]
    pub site: Option<String>,
    /// A link is being fetched
    pub fetching: bool,
    /// The open build has a file, so it can be replaced
    pub can_import_to_this_build: bool,
}

/// The Import/Export tab's account import (R18, R19).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountView {
    #[serde(default)]
    pub oauth: OAuthView,
    #[serde(default)]
    pub public: PublicView,
}

/// The sign-in half of the account import; Task 7 fills it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OAuthView {
    #[serde(default)]
    pub signed_in: bool,
}

/// PoB's import by account name: the box, its history and the characters it listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicView {
    /// The account name box
    pub account: String,
    pub realm: Realm,
    /// The accounts PoB remembers, in its order
    #[serde(default)]
    pub history: Vec<String>,
    pub stage: PublicStage,
    /// The league drop-down's entries in PoB's order, ending in the label "All" (which is
    /// `league: None` in `ImportPublicCharacter`); empty before a list
    #[serde(default)]
    pub leagues: Vec<String>,
    /// Every character of the account, in PoB's order (lowercase name); empty before a list
    #[serde(default)]
    pub characters: Vec<CharacterEntry>,
    /// PoB's "Character import status" line, with colour codes stripped
    #[serde(default)]
    pub status: String,
}

impl Default for PublicView {
    fn default() -> Self {
        PublicView {
            account: String::new(),
            realm: Realm::Pc,
            history: Vec::new(),
            stage: PublicStage::EnterAccount,
            leagues: Vec::new(),
            characters: Vec::new(),
            status: String::new(),
        }
    }
}

/// PoB's `charImportMode` of the account name import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicStage {
    EnterAccount,
    Fetching,
    SelectCharacter,
    Importing,
}

/// A character of the listed account, as PoB's list shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterEntry {
    pub name: String,
    /// GGG's `class`: the ascendancy when there is one
    pub class: String,
    pub level: u32,
    pub league: String,
}
