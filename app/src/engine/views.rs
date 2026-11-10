//! Typed snapshots of PoB state, deserialised from Lua tables.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StatsView {
    #[serde(default)]
    pub stats: Vec<StatRow>,
    /// The main skill's minion, when it has one.
    #[serde(default)]
    pub minion: Vec<StatRow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StatRow {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub text: String,
    #[serde(default)]
    pub color: Option<String>,
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

/// What PoB's breakdown control shows for a [`CellRef`]; no sections when the cell has none
/// (or is gone, or its flags no longer hold).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakdownView {
    pub cell: CellRef,
    #[serde(default)]
    pub sections: Vec<BreakdownSection>,
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
