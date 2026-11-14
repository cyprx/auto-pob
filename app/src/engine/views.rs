//! Typed snapshots of PoB state, deserialised from Lua tables.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use super::commands::{
    ControlTooltipQuery, GemSearchQuery, GemTooltipQuery, ItemDb, ItemTooltipQuery, PopupKind,
    Realm,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BuildView {
    pub name: String,
    /// What Save As's name box opens with (Build.lua:1400)
    #[serde(default)]
    pub save_as_name: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// The build name's tooltip: its folder and name, for a build in a folder of Builds
    /// (Build.lua:166-171)
    #[serde(default)]
    pub name_tooltip: Option<String>,
    pub class: String,
    #[serde(default)]
    pub ascendancy: Option<String>,
    pub level: u32,
    pub unsaved: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// The top bar's Auto/Manual button: the level follows the points spent (R21)
    #[serde(default)]
    pub level_auto: bool,
    /// The bloodline dropdown (`secondaryAscendDrop`) as PoB lists it: (id, label), "None"
    /// (id 0) first. PoB enables it only with more than one entry.
    #[serde(default)]
    pub secondary: Vec<(u32, String)>,
    /// The chosen bloodline's id; 0 is "None"
    #[serde(default)]
    pub secondary_selected: u32,
    /// The loadouts dropdown (`buildLoadouts`) as PoB holds it, colour codes kept. Entries
    /// starting with `^7^7` are PoB's headers and actions ("Loadouts:", "-----", "New
    /// Loadout", "Sync", "Help >>"), see [`is_loadout_marker`].
    #[serde(default)]
    pub loadouts: Vec<String>,
    /// The loadout PoB shows as selected (0-based into `loadouts`); `None` while it shows the
    /// "Loadouts:" header
    #[serde(default)]
    pub loadout_selected: Option<usize>,
    /// The point display's tooltip (`pointDisplay.req`): required level and estimated
    /// progress, colour codes kept
    #[serde(default)]
    pub progress: String,
    /// The level box's tooltip (`characterLevel.tooltipFunc`): the experience multiplier per
    /// monster level
    #[serde(default)]
    pub level_tooltip: Option<ItemTooltip>,
    /// How many builds PoB has initialised (`glue.build_generation`): a failed open or import
    /// that still changed it replaced the build with a new one, which PoB never shows (it goes
    /// to its build list)
    #[serde(default)]
    pub generation: u64,
}

/// Whether a `BuildView.loadouts` entry is one of PoB's headers or actions rather than a
/// loadout: PoB marks them with a doubled `^7` so no set title can match them (Build.lua:851).
pub fn is_loadout_marker(entry: &str) -> bool {
    entry.starts_with("^7^7")
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BuildListView {
    pub folder: String,
    /// Every folder reachable from the Builds folder (links are not followed), for the
    /// "Move to…" list and the Save As folder picker. Filled in by the engine, not by PoB.
    #[serde(default)]
    pub tree: Vec<FolderNode>,
    /// The Builds folder, which `tree` and the entries' `folder` are below
    #[serde(default)]
    pub root: PathBuf,
    #[serde(default)]
    pub entries: Vec<BuildListEntry>,
    /// PoB's `buildSortMode`, which orders `entries`
    #[serde(default)]
    pub sort: BuildSort,
    /// PoB's `main.filterBuildList`: the search text the list is filtered by
    #[serde(default)]
    pub filter: String,
    /// Names and folders are escaped: the Builds tree holds a name that isn't UTF-8, whose
    /// bytes show as U+F780+ (`bytes_text`). A Rename or Copy sends it back with the name.
    /// Filled in by the engine.
    #[serde(default)]
    pub escaped: bool,
    /// The engine has a build the UI should show: PoB reopened the build its last exit
    /// recorded, so it starts on that build, not on its list. Filled in by the engine.
    #[serde(default)]
    pub build_open: bool,
}

/// A folder of the Builds tree
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FolderNode {
    /// Below the Builds folder, with a closing slash ("a/b/")
    pub path: String,
    /// Seconds since the epoch (the Save As picker's "Last Edited" sort)
    #[serde(default)]
    pub modified: u64,
    /// Nothing in it: PoB deletes only such a folder from the picker
    #[serde(default)]
    pub empty: bool,
}

/// PoB's four build list sorts (`sortMode` ids NAME, CLASS, EDITED, LEVEL)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum BuildSort {
    #[default]
    #[serde(rename = "NAME")]
    Name,
    #[serde(rename = "CLASS")]
    Class,
    #[serde(rename = "EDITED")]
    Edited,
    #[serde(rename = "LEVEL")]
    Level,
}

impl BuildSort {
    /// PoB's id for the sort
    pub fn id(self) -> &'static str {
        match self {
            BuildSort::Name => "NAME",
            BuildSort::Class => "CLASS",
            BuildSort::Edited => "EDITED",
            BuildSort::Level => "LEVEL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildListEntry {
    pub name: String,
    pub is_folder: bool,
    pub path: PathBuf,
    /// The folder the entry is in, below the Builds folder ("" or "a/b/"). A search lists
    /// matches from every folder, so this is not always the folder shown.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub ascendancy: Option<String>,
    #[serde(default)]
    pub level: Option<u32>,
    /// PoB's confirm text for a delete of this entry, shown by the UI before it sends
    /// `DeleteBuild`. Empty when PoB deletes without asking (an empty folder).
    #[serde(default)]
    pub delete_prompt: String,
}

impl BuildListEntry {
    /// The question `delete_prompt` asks, as `DeleteBuild` takes it back
    pub fn delete_kind(&self) -> super::commands::DeleteKind {
        use super::commands::DeleteKind;
        match (self.is_folder, self.delete_prompt.is_empty()) {
            (false, _) => DeleteKind::Build,
            (true, true) => DeleteKind::EmptyFolder,
            (true, false) => DeleteKind::Folder,
        }
    }
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
    /// PoB's warning lines under the stats (`controls.warnings.lines`, NF-1), in PoB's order;
    /// empty while Options' "Show warnings" is off
    #[serde(default)]
    pub warnings: Vec<String>,
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
    /// PoB shows "Manage Spectres..." (`mainSkillMinionLibrary`) for the main skill (R18)
    #[serde(default)]
    pub spectre_library: bool,
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
    /// The colour code PoB's box shows each line of `text` in while it isn't selected: MAGIC
    /// for a line PoB reads as a modifier, UNSUPPORTED for one it can't (ConfigTab.lua:78-86)
    #[serde(default)]
    pub line_colours: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigSet {
    pub id: u32,
    pub title: String,
    /// The set has no title (a build never saved: ConfigTab.lua:133), so PoB's rename box
    /// opens empty
    #[serde(default)]
    pub untitled: bool,
}

impl ConfigSet {
    /// What PoB's rename box opens with: the title as the set has it, empty when it has none
    pub fn name_box_text(&self) -> String {
        if self.untitled {
            String::new()
        } else {
            self.title.clone()
        }
    }
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
    /// PoB's control takes input (`IsEnabled`): a Boss Skill Preset greys out "Enemy Damage
    /// Type" (ConfigOptions.lua:2286-2289)
    #[serde(default = "enabled_default")]
    pub enabled: bool,
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

fn enabled_default() -> bool {
    true
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
    /// The rows PoB's list shows under its filter and Show Masteries (NF-5): positions in
    /// `rows`, in the list's order
    #[serde(default)]
    pub listed: Vec<usize>,
    /// The filter dropdown, 0-based: Show Unallocated, ... & Clusters, Show Allocated
    #[serde(default)]
    pub filter: usize,
    /// The Show Masteries box
    #[serde(default)]
    pub masteries: bool,
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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NodeTooltip {
    pub id: u32,
    pub lines: Vec<TooltipLine>,
    /// The Shift-traced path the tooltip was built with (empty: none)
    #[serde(default)]
    pub trace: Vec<u32>,
    /// PoB's `tooltipHeader`: which header art the tooltip has (NOTABLE, KEYSTONE, ...)
    #[serde(default)]
    pub header: Option<String>,
    /// The oils of an anointable notable, drawn after its name (`SetRecipe`)
    #[serde(default)]
    pub recipe: Vec<String>,
    /// The tooltip of the skill the node grants, which PoB draws beside it (`skillTooltip`)
    #[serde(default)]
    pub skill: Option<SkillTip>,
}

/// A gem's tooltip beside a node's (PassiveTreeView.lua:1605-1625)
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SkillTip {
    #[serde(default)]
    pub header: Option<String>,
    pub lines: Vec<TooltipLine>,
}

/// What PoB's group list shows when the mouse is over a socket group (`AddValueTooltip`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct GroupTooltipView {
    pub group: usize,
    pub lines: Vec<TooltipLine>,
}

/// What hovering a control shows (R22): PoB's `tooltipFunc`, colour codes kept.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HoverTooltipView {
    pub target: crate::engine::HoverTarget,
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
    /// The connectors PoB draws (PassiveTree:BuildConnector): the tree's, then the cluster
    /// jewels'. Fewer than `edges`: none to a class start or a mastery, none between
    /// ascendancies.
    pub connectors: Vec<Connector>,
    /// Group background art: the groups', then the cluster jewels'
    pub groups: Vec<GroupArt>,
    /// The class background art, by class id (PassiveTreeView.lua:546-564)
    pub class_art: Vec<ClassArt>,
    /// The tree assets drawn by name (connectors, group and class backgrounds, socket overlays)
    /// and the view's own images (`ring`, `highlightRing`, ...)
    pub assets: HashMap<String, Sprite>,
    /// `tree.size`: PoB's zoom is relative to it
    pub size: f32,
    /// `data.jewelRadius`
    pub jewel_radii: Vec<JewelRadius>,
    /// Sprite sheets, indexed by [`Sprite::sheet`]. `None` if the image couldn't be loaded.
    pub sheets: Vec<Option<Arc<SheetImage>>>,
    /// The version plus a digest of the node ids; caches built from the layout are stale when
    /// it changes.
    pub key: String,
}

/// An image file the tree draws: a sprite sheet or one of PoB's whole-file assets. Its size is
/// read from the file's header when the layout is built; its pixels are decoded only when the UI
/// wants them, off the engine thread, and dropped once uploaded: the GPU copy is what gets drawn.
pub struct SheetImage {
    pub path: PathBuf,
    pub size: [usize; 2],
    pixels: std::sync::Mutex<Pixels>,
}

/// Where a file's pixels are
enum Pixels {
    /// Not decoded, or taken by the UI
    Unloaded,
    /// Being decoded
    Loading,
    /// RGBA, 8 bits per channel, not premultiplied
    Ready(Vec<u8>),
    /// The file couldn't be decoded
    Failed,
}

/// What [`SheetImage::take_rgba`] found
#[derive(Debug, PartialEq)]
pub enum SheetPixels {
    Ready(Vec<u8>),
    /// Not decoded yet: call [`SheetImage::decode`] (it may already be running)
    Pending,
    Failed,
}

impl SheetImage {
    /// An image with its pixels already decoded
    pub fn new(path: PathBuf, size: [usize; 2], rgba: Vec<u8>) -> SheetImage {
        SheetImage {
            path,
            size,
            pixels: std::sync::Mutex::new(Pixels::Ready(rgba)),
        }
    }

    /// An image of `size` whose pixels are decoded from `path` when asked for
    pub fn lazy(path: PathBuf, size: [usize; 2]) -> SheetImage {
        SheetImage {
            path,
            size,
            pixels: std::sync::Mutex::new(Pixels::Unloaded),
        }
    }

    fn pixels(&self) -> std::sync::MutexGuard<'_, Pixels> {
        self.pixels.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The decoded pixels, once: they are the caller's after this, and the image is undecoded
    /// again. `Pending` when they aren't decoded yet.
    pub fn take_rgba(&self) -> SheetPixels {
        let mut pixels = self.pixels();
        match std::mem::replace(&mut *pixels, Pixels::Unloaded) {
            Pixels::Ready(rgba) => SheetPixels::Ready(rgba),
            Pixels::Failed => {
                *pixels = Pixels::Failed;
                SheetPixels::Failed
            }
            Pixels::Loading => {
                *pixels = Pixels::Loading;
                SheetPixels::Pending
            }
            Pixels::Unloaded => SheetPixels::Pending,
        }
    }

    /// Decodes the file, unless that is done or under way. Slow: call it off the UI's and the
    /// engine's threads.
    pub fn decode(&self) {
        if self.claim() {
            self.decode_claimed();
        }
    }

    /// Marks the pixels as being decoded, unless that is done or under way; true when the
    /// caller is to decode them (`decode_claimed`)
    pub fn claim(&self) -> bool {
        let mut pixels = self.pixels();
        if !matches!(*pixels, Pixels::Unloaded) {
            return false;
        }
        *pixels = Pixels::Loading;
        true
    }

    /// Decodes the pixels `claim` claimed
    pub fn decode_claimed(&self) {
        self.decode_with(|path| image::open(path).map(|i| i.into_rgba8()));
    }

    /// Decodes the claimed pixels with `decode`. A decoder that panics leaves the file failed,
    /// not loading for good.
    pub fn decode_with(
        &self,
        decode: impl FnOnce(&std::path::Path) -> image::ImageResult<image::RgbaImage>
        + std::panic::UnwindSafe,
    ) {
        let path = self.path.clone();
        let Ok(decoded) = std::panic::catch_unwind(move || decode(&path)) else {
            eprintln!("Unable to load {}: the decoder failed", self.path.display());
            self.fail();
            return;
        };
        *self.pixels() = match decoded {
            Ok(image) if [image.width() as usize, image.height() as usize] == self.size => {
                Pixels::Ready(image.into_raw())
            }
            Ok(_) => Pixels::Failed,
            Err(e) => {
                eprintln!("Unable to load {}: {e}", self.path.display());
                Pixels::Failed
            }
        };
    }

    /// Marks the file as one that can't be decoded
    pub fn fail(&self) {
        *self.pixels() = Pixels::Failed;
    }

    /// Whether the file couldn't be decoded
    pub fn is_failed(&self) -> bool {
        matches!(*self.pixels(), Pixels::Failed)
    }

    /// Whether the pixels are decoded and not taken
    pub fn has_rgba(&self) -> bool {
        matches!(*self.pixels(), Pixels::Ready(_))
    }

    /// Whether nothing has started decoding the file
    pub fn is_unloaded(&self) -> bool {
        matches!(*self.pixels(), Pixels::Unloaded)
    }
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    pub sheet: usize,
    /// x, y, width, height in pixels
    pub rect: [f32; 4],
}

/// What PoB draws for a node in each state (PassiveTreeView.lua:805-904). `base` and `overlay`
/// are [unallocated, path, allocated]; `effect` is [unallocated, allocated] (a mastery's
/// effect when allocated, a tattoo's always); `hover` is a hovered mastery's base. Which entry
/// is drawn depends on the node's kind, as in PoB.
#[derive(Debug, Default, Clone)]
pub struct NodeArt {
    pub base: [Option<Sprite>; 3],
    pub overlay: [Option<Sprite>; 3],
    pub effect: [Option<Sprite>; 2],
    pub hover: Option<Sprite>,
}

/// A connector PoB draws, between two nodes
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Connector {
    /// PoB's `nodeId1`, `nodeId2`
    pub ids: [u32; 2],
    /// The art's name without the state: `LineConnector` or `Orbit<n>`
    pub kind: String,
    #[serde(default)]
    pub ascendancy: Option<String>,
    /// Where the nodes are
    pub ends: [[f32; 2]; 2],
    /// An orbit's arc: the group's centre
    #[serde(default)]
    pub centre: Option<[f32; 2]>,
    /// An orbit's arc: the quad's other three corners from `centre`, per unit of art size
    /// (the art's width times 2.66, PassiveTree:BuildArc)
    #[serde(default)]
    pub quad: Option<[[f32; 2]; 3]>,
    /// An orbit's arc: how much of the art's 90 degrees it shows
    #[serde(default)]
    pub p: f32,
    /// Both nodes are conquered by an abyss jewel: the abyss art, where there is one
    #[serde(default)]
    pub abyss: bool,
}

/// A group's background art: at the group's centre, a half one mirrored below it
#[derive(Debug, Clone)]
pub struct GroupArt {
    pub pos: [f32; 2],
    pub art: Sprite,
    pub half: bool,
    /// An ascendancy's emblem: faded unless it is the build's or its bloodline
    pub ascendancy: Option<String>,
    pub flavour: Option<Flavour>,
}

/// An ascendancy's flavour text, drawn when zoomed in far enough
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Flavour {
    pub text: String,
    /// RRGGBB
    pub colour: String,
    pub x: f32,
    pub y: f32,
}

/// A group's background art as the glue lists it
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LuaGroupArt {
    pub x: f32,
    pub y: f32,
    pub art: LuaSprite,
    #[serde(default)]
    pub half: bool,
    #[serde(default)]
    pub ascendancy: Option<String>,
    #[serde(default)]
    pub flavour: Option<Flavour>,
}

impl LuaGroupArt {
    pub fn resolve(&self, sheets: &[Option<Arc<SheetImage>>]) -> Option<GroupArt> {
        Some(GroupArt {
            pos: [self.x, self.y],
            art: self.art.resolve(sheets)?,
            half: self.half,
            ascendancy: self.ascendancy.clone(),
            flavour: self.flavour.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClassArt {
    pub class: u32,
    pub art: Sprite,
    pub pos: [f32; 2],
}

/// One of `data.jewelRadius`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JewelRadius {
    pub inner: f32,
    pub outer: f32,
    pub colour: [u8; 3],
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
    /// Whether a hovered socket's radius colours it (PassiveTree.lua:645-648)
    pub in_radius: bool,
    /// A socket whose hover colours the nodes in its radius
    pub radius_socket: bool,
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
    /// The id (not the name) the chosen bloodline's nodes carry (`curSecondaryAscendClass.id`)
    #[serde(default)]
    pub bloodline: Option<String>,
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
    /// Shared: the engine keeps the lists while they don't change
    pub paths: std::sync::Arc<[NodePath]>,
    /// Allocated nodes: the nodes that deallocating one drops (`node.depends`)
    #[serde(default)]
    pub depends: std::sync::Arc<[NodePath]>,
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
    /// The nodes allocated in the compared spec that this spec doesn't have (a cluster jewel's
    /// subgraph): PoB draws them green and gives them a tooltip (PassiveTreeView.lua:1102-1153)
    #[serde(default)]
    pub compare_only: Vec<CompareOnlyNode>,
    /// The group art of the cluster jewels only the compared spec has (PassiveTreeView.lua:651-660)
    #[serde(default)]
    pub compare_only_groups: Vec<LuaGroupArt>,
    /// Their connectors between allocated nodes (PassiveTreeView.lua:747-770)
    #[serde(default)]
    pub compare_only_connectors: Vec<Connector>,
    /// What the art depends on beyond the allocation
    #[serde(default)]
    pub draw: DrawState,
    /// The sockets that draw radius rings, and their jewels' rings
    #[serde(default)]
    pub socket_rings: Vec<SocketRings>,
    #[serde(default)]
    pub power_stats: Vec<String>,
    #[serde(default)]
    pub heatmap: Option<String>,
    #[serde(default)]
    pub max_depth: Option<u32>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// PoB's `showHeatMap`: P can show the heat map before any stat was chosen, when
    /// `heatmap` is `None` and the map shows damage against defence
    #[serde(default)]
    pub heatmap_shown: bool,
    /// PoB's `showStatDifferences` of the tree (Ctrl+D)
    #[serde(default)]
    pub show_stat_differences: bool,
    /// PoB's Version dropdown: every tree version, `value` the id `ConvertSpec` takes (NF-7)
    #[serde(default)]
    pub versions: Vec<Choice>,
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

/// A node of the compared spec only, where PoB draws it
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CompareOnlyNode {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub kind: String,
    pub name: String,
    /// A mastery with effects to choose from: PoB shows a mastery's tooltip only then
    #[serde(default)]
    pub mastery_effects: bool,
    /// Its allocated icon and frame, from the layout's sheets
    #[serde(default)]
    pub art: Option<CompareOnlyArt>,
}

/// A compare-only node's allocated art
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CompareOnlyArt {
    #[serde(default)]
    pub base: Option<LuaSprite>,
    #[serde(default)]
    pub overlay: Option<LuaSprite>,
}

/// An image as the glue lists it: a 1-based sheet and a pixel rectangle, or no rectangle for
/// the whole file
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct LuaSprite {
    pub sheet: usize,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub w: Option<f32>,
    #[serde(default)]
    pub h: Option<f32>,
}

impl LuaSprite {
    /// The sprite, a whole file's rectangle its size; `None` for a file that couldn't be loaded
    pub fn resolve(&self, sheets: &[Option<Arc<SheetImage>>]) -> Option<Sprite> {
        let sheet = self.sheet.checked_sub(1)?;
        let image = sheets.get(sheet)?.as_ref()?;
        let rect = match (self.x, self.y, self.w, self.h) {
            (Some(x), Some(y), Some(w), Some(h)) => [x, y, w, h],
            _ => [0.0, 0.0, image.size[0] as f32, image.size[1] as f32],
        };
        Some(Sprite { sheet, rect })
    }
}

/// What the tree's art depends on beyond the allocation (PassiveTreeView.lua:805-845)
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DrawState {
    /// A spec is compared with (the Tree tab's, or the Compare tab's overlay)
    #[serde(default)]
    pub comparing: bool,
    /// Nodes the compared spec doesn't have
    #[serde(default)]
    pub compare_missing: Vec<u32>,
    /// Sockets holding a Thread of Hope-like jewel
    #[serde(default)]
    pub variable_sockets: Vec<u32>,
    /// Nodes the build grants (`grantedPassives`): drawn allocated
    #[serde(default)]
    pub granted: Vec<u32>,
    /// Nodes the compared spec allocates
    #[serde(default)]
    pub compare_allocated: Vec<u32>,
    /// Split Personality's path, drawn in its colour
    #[serde(default)]
    pub split_path: Vec<u32>,
    /// Nodes an Intuitive Leap-like jewel reaches
    #[serde(default)]
    pub leap: Vec<u32>,
    /// The overlay asset of each socket's jewel
    #[serde(default)]
    pub socket_overlays: Vec<SocketOverlay>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SocketOverlay {
    pub node: u32,
    pub overlay: String,
}

/// A socket that draws radius rings (PassiveTreeView.lua:1198-1248)
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SocketRings {
    pub node: u32,
    /// Its jewel is Thread of Hope-like: hovering it shows the annuli
    #[serde(default)]
    pub variable: bool,
    #[serde(default)]
    pub rings: Vec<RingImage>,
}

/// One image of a jewel's radius art (drawJewelRadius)
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RingImage {
    /// The view image's field name, a key of [`TreeLayout::assets`]
    pub image: String,
    /// Across, in tree units
    pub size: f32,
    /// `DrawImageRotated`'s angle: radians turned per 1/0.00003 ms
    pub spin: f32,
    pub x: f32,
    pub y: f32,
    pub tint: [f32; 4],
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
    /// "Default" for a tree with no title
    pub title: String,
    /// The tree has no title: PoB's rename box opens empty (PassiveSpecListControl.lua:49)
    #[serde(default)]
    pub untitled: bool,
    pub version: String,
    pub points: u32,
}

impl SpecEntry {
    /// What PoB's rename box opens with: the title as the file has it, empty when it has none
    pub fn name_box_text(&self) -> String {
        if self.untitled {
            String::new()
        } else {
            self.title.clone()
        }
    }
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
    /// The set has no title: PoB's rename box opens empty (SkillSetListControl.lua:56)
    #[serde(default)]
    pub untitled: bool,
}

impl SkillSetEntry {
    /// What PoB's rename box opens with: the title as the file has it, empty when it has none
    pub fn name_box_text(&self) -> String {
        if self.untitled {
            String::new()
        } else {
            self.title.clone()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SocketGroupView {
    pub label: String,
    /// PoB's `displayLabel`: the label, or the gems' names for a group with none
    #[serde(default)]
    pub name: String,
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
    /// PoB's "Passive tree:" dropdown on the Items tab (`specSelect`): `GetSpecList`'s labels,
    /// the active tree selected
    #[serde(default)]
    pub specs: Option<DropList>,
    /// PoB's shared items (`main.sharedItemList`, NF-2), in PoB's order
    #[serde(default)]
    pub shared_items: Vec<SharedItemEntry>,
    /// PoB's shared item sets (`main.sharedItemSetList`), in PoB's order
    #[serde(default)]
    pub shared_sets: Vec<SharedSetEntry>,
    /// PoB shows the shared list (and takes drags onto it) only while no item is displayed
    #[serde(default)]
    pub shared_shown: bool,
}

/// One row of PoB's shared item list
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedItemEntry {
    pub name: String,
    pub rarity: String,
    /// The row as PoB draws it (`GetRowValue`), colour codes included
    pub label: String,
}

/// One row of PoB's shared item set list
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedSetEntry {
    /// The row as PoB draws it: the title, or "Default"
    pub title: String,
    /// PoB's tooltip for the row: one "Slot: Item" line per slot
    #[serde(default)]
    pub lines: Vec<String>,
    /// The set's own title, which Rename starts from; none for an untitled set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_title: Option<String>,
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
    /// The set has no title: PoB's rename box opens empty (ItemSetListControl.lua:47)
    #[serde(default)]
    pub untitled: bool,
    /// The manager's row as PoB draws it, "  ^9(Current)" after the active set's title
    /// (ItemSetListControl.lua:79-83)
    #[serde(default)]
    pub row: String,
    /// PoB's tooltip for the set: one "Slot: Item" line per equipped slot (ItemsTab.lua:4138)
    #[serde(default)]
    pub lines: Vec<String>,
}

impl ItemSetEntry {
    /// What PoB's rename box opens with: the title as the file has it, empty when it has none
    pub fn name_box_text(&self) -> String {
        if self.untitled {
            String::new()
        } else {
            self.title.clone()
        }
    }
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
    /// PoB shows "Manage Spectres..." (`mainSkillMinionLibrary`) in the selector (R18)
    #[serde(default)]
    pub spectre_library: bool,
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
    /// The width PoB gives each cell column of a sub-section, by sub-section (R24):
    /// `colWidth` or the section's width less the label column (CalcSectionControl.lua:126)
    #[serde(default)]
    pub col_widths: Vec<f32>,
    /// PoB offers the section's "^" pop-out window: a section without controls (R24)
    #[serde(default)]
    pub pop_out: bool,
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

    fn row(key: &str, value: &str, effect: SettingEffect) -> SettingRow {
        SettingRow {
            key: key.into(),
            control: format!("{key}Control"),
            label: format!("The {key}"),
            shared: "shared".into(),
            value: value.into(),
            overridden: true,
            effect,
        }
    }

    #[test]
    fn compare_view_round_trips() {
        let row = |label: &str, diff: &str, better: Option<bool>| CompareRow {
            label: format!("^7{label}"),
            ours: "^7100".into(),
            theirs: "^xFFD62A120".into(),
            diff: diff.into(),
            better,
        };
        let view = CompareView {
            entries: vec!["dot".into(), "Imported build".into()],
            active: 1,
            headers: ["^x33FF77attack".into(), "^xFF9922Imported build".into()],
            controls: vec![PopupControl {
                name: "cmpMainSkill".into(),
                kind: PopupControlKind::Dropdown,
                label: String::new(),
                pos: [0.0, 0.0],
                size: [225.0, 20.0],
                shown: true,
                enabled: true,
                choices: vec!["Vaal Double Strike".into(), "Double Strike".into()],
                selected: 1,
                value: 0.0,
                divisions: None,
                checked: false,
                text: String::new(),
                ascii_only: false,
                digits: false,
                signed: false,
                numeric: false,
                limit: None,
                tooltip_text: None,
                hover_tooltip: false,
                rows: vec![],
                row_selected: None,
                row_highlight: None,
                note: None,
                reorder: false,
            }],
            summary: vec![
                row("Life", "^x33FF77+20 (+20.0%)", Some(true)),
                row("Mana", "^xDD0022-5", Some(false)),
                row("Armour", "", None),
            ],
            items: vec![
                CompareItemRow {
                    slot: "Body Armour".into(),
                    label: "Body Armour".into(),
                    ours: Some(3),
                    theirs: Some("Kaom's Heart".into()),
                    differ: true,
                    can_copy: true,
                    can_equip: true,
                    diff_label: "^xFFFF77(different)".into(),
                    ours_unallocated: false,
                    theirs_unallocated: true,
                    ours_name: Some("Doom Loop".into()),
                    ours_colour: "^xFFFF77".into(),
                    theirs_colour: "^xAF6025".into(),
                    ours_card: vec![
                        CardLine {
                            text: "^xAF6025Doom Loop".into(),
                            y: 0.0,
                        },
                        CardLine {
                            text: "^x33FF77+ +10 to Life".into(),
                            y: 18.0,
                        },
                    ],
                    theirs_card: vec![CardLine {
                        text: "^8(empty)".into(),
                        y: 0.0,
                    }],
                },
                CompareItemRow {
                    slot: "Ring 2".into(),
                    label: "Ring 2".into(),
                    ours: None,
                    theirs: None,
                    differ: false,
                    can_copy: false,
                    can_equip: false,
                    diff_label: "^8(both empty)".into(),
                    ours_unallocated: false,
                    theirs_unallocated: false,
                    ours_name: None,
                    ours_colour: "^7".into(),
                    theirs_colour: "^7".into(),
                    ours_card: vec![],
                    theirs_card: vec![],
                },
            ],
            skills: vec![CompareSkillRow {
                label: "^7Boneshatter".into(),
                theirs_label: None,
                ours: vec!["^x08F2FFBoneshatter^7 20/0".into()],
                theirs: vec![],
                differ: true,
            }],
            config: vec![CompareConfigRow {
                var: "usePowerCharges".into(),
                label: "Do you use Power Charges?".into(),
                ours: "^x33FF77Yes".into(),
                theirs: "^8(not set)".into(),
                common: true,
                ours_control: None,
                theirs_control: None,
            }],
            switched: true,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<CompareView>(&json).unwrap(), view);
        // the Lua table the glue builds: a row with no diff leaves `diff` and `better` out
        let from_lua: CompareView = lua_value(
            r#"return { entries = { "dot", "Imported build" }, active = 1,
              headers = { "^x33FF77attack", "^xFF9922Imported build" },
              controls = { { name = "cmpMainSkill", kind = "Dropdown", label = "",
                pos = { 0, 0 }, size = { 225, 20 }, shown = true, enabled = true,
                choices = { "Vaal Double Strike", "Double Strike" }, selected = 1, value = 0,
                checked = false, text = "", hover_tooltip = false, rows = {} } },
              summary = {
                { label = "^7Life", ours = "^7100", theirs = "^xFFD62A120",
                  diff = "^x33FF77+20 (+20.0%)", better = true },
                { label = "^7Mana", ours = "^7100", theirs = "^xFFD62A120",
                  diff = "^xDD0022-5", better = false },
                { label = "^7Armour", ours = "^7100", theirs = "^xFFD62A120" },
              },
              items = {
                { slot = "Body Armour", label = "Body Armour", ours = 3, theirs = "Kaom's Heart",
                  differ = true, can_copy = true, can_equip = true,
                  diff_label = "^xFFFF77(different)", ours_unallocated = false,
                  theirs_unallocated = true, ours_name = "Doom Loop", ours_colour = "^xFFFF77",
                  theirs_colour = "^xAF6025",
                  ours_card = { { text = "^xAF6025Doom Loop", y = 0 },
                    { text = "^x33FF77+ +10 to Life", y = 18 } },
                  theirs_card = { { text = "^8(empty)", y = 0 } } },
                { slot = "Ring 2", label = "Ring 2", differ = false, can_copy = false,
                  can_equip = false, diff_label = "^8(both empty)", ours_unallocated = false,
                  theirs_unallocated = false, ours_colour = "^7", theirs_colour = "^7" },
              },
              skills = { { label = "^7Boneshatter", ours = { "^x08F2FFBoneshatter^7 20/0" },
                theirs = {}, differ = true } },
              config = { { var = "usePowerCharges", label = "Do you use Power Charges?",
                ours = "^x33FF77Yes", theirs = "^8(not set)", common = true } },
              switched = true }"#,
        );
        assert_eq!(from_lua, view);
    }

    #[test]
    fn settings_view_round_trips() {
        let view = SettingsView {
            rows: vec![
                row("colorPositive", "^x33FF77", SettingEffect::PobText),
                row("showTitlebarName", "false", SettingEffect::Ui),
                row("showAnimations", "true", SettingEffect::NoEffect),
            ],
            saved_overrides: BTreeMap::from([("defaultCharLevel".to_string(), "90".to_string())]),
            options_lost: true,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<SettingsView>(&json).unwrap(), view);
        // an older writer without the overrides still reads
        let old: SettingsView = serde_json::from_str(r#"{"rows":[]}"#).unwrap();
        assert_eq!(old, SettingsView::default());
        // and the Lua table the glue builds reads the same, with an empty override table
        let from_lua: SettingsView = lua_value(
            r#"return { rows = { { key = "showTitlebarName", control = "showTitlebarNameControl",
              label = "The showTitlebarName", shared = "shared", value = "false",
              overridden = true, effect = "Ui" } }, saved_overrides = {} }"#,
        );
        assert_eq!(
            from_lua.rows,
            vec![row("showTitlebarName", "false", SettingEffect::Ui)]
        );
        assert!(from_lua.saved_overrides.is_empty());
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
                untitled: false,
            }],
            active_set: 1,
            custom_mods: vec![CustomModGroup {
                title: "Default".into(),
                enabled: true,
                text: "+1 to all".into(),
                line_colours: vec![],
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
                        enabled: true,
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
                        enabled: true,
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
    /// PoB's child tooltips beside it: the granted skills' gem tooltips, filled only while
    /// Shift is held (R34)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ItemTooltip>,
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

/// One row of the Compare tab's Summary, as PoB's `DrawStatList` draws it (R4). The texts keep
/// PoB's colour codes; `diff` is empty when the values are equal or the row has no number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareRow {
    pub label: String,
    pub ours: String,
    pub theirs: String,
    #[serde(default)]
    pub diff: String,
    /// The diff's colour: PoB's positive colour is better, its negative one worse
    #[serde(default)]
    pub better: Option<bool>,
}

/// The Compare tab with at least one comparison build (R4-R6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareView {
    /// The comparisons' labels, in PoB's order
    pub entries: Vec<String>,
    /// 0-based in `entries`
    pub active: usize,
    /// The Summary's column headers for this build and the active comparison, colour codes kept
    pub headers: [String; 2],
    /// R6's controls, in that list's order
    pub controls: Vec<PopupControl>,
    #[serde(default)]
    pub summary: Vec<CompareRow>,
    #[serde(default)]
    pub items: Vec<CompareItemRow>,
    #[serde(default)]
    pub skills: Vec<CompareSkillRow>,
    #[serde(default)]
    pub config: Vec<CompareConfigRow>,
    /// PoB switched to its Compare tab: an Import-tab comparison just landed (R5). True in the
    /// first view after it only.
    #[serde(default)]
    pub switched: bool,
}

/// One slot of the Compare tab's Items view, in PoB's order: the base slots (Ring 3 when either
/// build has it), the abyssal sockets, then the jewel sockets by node (R9)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompareItemRow {
    /// PoB's slot name (`Jewel <node>` for a tree socket), as `CopyCompareItem` takes it
    pub slot: String,
    pub label: String,
    /// This build's item in the slot
    #[serde(default)]
    pub ours: Option<u32>,
    /// The comparison's item in the slot: its name line
    #[serde(default)]
    pub theirs: Option<String>,
    /// The two items' texts differ
    pub differ: bool,
    /// PoB offers Copy for the comparison's item
    pub can_copy: bool,
    /// PoB offers Equip too: false where it draws "Missing slot" (Ring 3 on a build without
    /// the slot) or there is nothing to copy
    #[serde(default)]
    pub can_equip: bool,
    /// PoB's marker between the two items, colour code kept: "(match)" when the names are the
    /// same, "(different)", "(missing)", "(extra)" or "(both empty)" (`getSlotDiffLabel`)
    #[serde(default)]
    pub diff_label: String,
    /// This build's jewel sits in a socket its tree doesn't allocate: PoB's "(tree missing
    /// allocated node)" after the name
    #[serde(default)]
    pub ours_unallocated: bool,
    /// The same for the comparison's jewel
    #[serde(default)]
    pub theirs_unallocated: bool,
    /// This build's item name, as PoB draws it in the row
    #[serde(default)]
    pub ours_name: Option<String>,
    /// PoB's rarity colour code for each side's name (`getRarityColor`; "^7" for none)
    #[serde(default)]
    pub ours_colour: String,
    #[serde(default)]
    pub theirs_colour: String,
    /// PoB's expanded-mode card of each side (`DrawItemExpanded`), one entry per line drawn,
    /// colour codes kept: the name, the base's figures, the mods marked against the other
    /// side's ("+ " only on this side, "> " a higher value, "< " a lower one), Corrupted and
    /// the like; "(empty)" for no item. Empty while expanded mode is off.
    #[serde(default)]
    pub ours_card: Vec<CardLine>,
    #[serde(default)]
    pub theirs_card: Vec<CardLine>,
}

/// One line of an expanded-mode item card
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CardLine {
    pub text: String,
    /// Where PoB draws it, from the card's top: the name takes 18 px and a line 16, and PoB
    /// leaves 2 px after the base's figures and after each section of mods
    pub y: f32,
}

/// One pair of socket groups of the Compare tab's Skills view, matched as PoB's view matches
/// them; the lines are PoB's, colour codes kept ("+ " an extra gem, "- " a missing one)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompareSkillRow {
    /// This build's group header, or the comparison's when this build has no group in the pair
    pub label: String,
    /// The comparison's group header, when it has a group in the pair
    #[serde(default)]
    pub theirs_label: Option<String>,
    #[serde(default)]
    pub ours: Vec<String>,
    #[serde(default)]
    pub theirs: Vec<String>,
    pub differ: bool,
}

/// A config option the two builds set differently, in `ConfigOptions` order
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareConfigRow {
    pub var: String,
    pub label: String,
    pub ours: String,
    pub theirs: String,
    /// The builds set it alike: PoB lists it after its section's differing options when it
    /// is relevant to either build, or with "Show All Configurations" on
    #[serde(default)]
    pub common: bool,
    /// PoB's box for this build's value (`cfg_p_<var>`), set with `SetCompareControl`
    #[serde(default)]
    pub ours_control: Option<PopupControl>,
    /// PoB's box for the comparison's value (`cfg_c_<var>`)
    #[serde(default)]
    pub theirs_control: Option<PopupControl>,
}

/// The builds in one folder of the Builds folder (`BrowseBuilds`), by name
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildBrowseView {
    /// Below the Builds folder, with a closing slash ("" for the Builds folder itself)
    pub folder: String,
    /// The build files, `name` without ".xml"
    pub entries: Vec<BuildBrowseEntry>,
    /// Why the folder couldn't be listed (`entries` is then empty): the picker says so
    /// instead of waiting for an answer that won't come
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildBrowseEntry {
    pub name: String,
    pub path: PathBuf,
}

/// The open popup, whatever it is: PoB's controls as data (R5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopupView {
    pub kind: PopupKind,
    pub title: String,
    /// Ordered by position (top to bottom, then left to right)
    pub controls: Vec<PopupControl>,
    /// The button Enter presses when no control has the keyboard (the popup's
    /// `enterControl`, PopupDialog.lua:79-83)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<String>,
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
    /// Edit: the box has `EditControl`'s own filter, so it keeps only ASCII letters, digits,
    /// punctuation and spaces (EditControl.lua:46)
    #[serde(default)]
    pub ascii_only: bool,
    /// Edit: a number box's filter, `%D` (digits) or `^%-%d` (digits and "-")
    #[serde(default)]
    pub digits: bool,
    /// Edit: the filter is `^%-%d`
    #[serde(default)]
    pub signed: bool,
    /// Edit: PoB's `isNumeric`, which gives the box + and - and steps it on Up, Down and the
    /// wheel (EditControl.lua:64-75, 680-700); Buy Similar's boxes turn it off
    #[serde(default)]
    pub numeric: bool,
    /// Edit: the most bytes the box takes
    #[serde(default)]
    pub limit: Option<usize>,
    /// PoB's static `tooltipText`
    pub tooltip_text: Option<String>,
    /// The control builds a tooltip on hover (ask with `ControlTooltip`)
    pub hover_tooltip: bool,
    /// List
    #[serde(default)]
    pub rows: Vec<PopupRow>,
    pub row_selected: Option<String>,
    /// The other end of a Shift+click range from `row_selected` (the timeless results)
    #[serde(default)]
    pub row_highlight: Option<String>,
    /// A list's `defaultText`
    pub note: Option<String>,
    /// A list whose rows PoB lets the user drag into another order (`isMutable`); left out
    /// of the serialised view when false
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reorder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopupRow {
    pub key: String,
    pub label: String,
}

/// The answer to a `CopyItem` query: the text PoB copies, with its `\r\n` line ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemText {
    /// The item's id, or the shared item's index when `shared`
    pub id: u32,
    pub text: String,
    /// A shared item's text (`CopySharedItem`)
    #[serde(default)]
    pub shared: bool,
    /// A database row's text (`CopyDbItem`): the row's name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_item: Option<String>,
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
    /// PoB shows one slider and Foulborn box per range line (`showAllItemAffixes` on a
    /// unique) instead of the line dropdown
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stacked: bool,
    /// How many times PoB rebuilt the range line dropdown, which puts it back on the first
    /// line (UpdateDisplayItemRangeLines, ItemsTab.lua:2385-2396). A count of this engine's,
    /// not PoB state: left out of the views compared with PoB.
    #[serde(default, skip_serializing)]
    pub range_resets: u32,
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

/// Who acts on a setting's change (R13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingEffect {
    /// PoB formats its own text from it; nothing to do here
    PobText,
    /// The UI acts on it (zoom, window title, the tree canvas)
    Ui,
    /// PoB's option has no counterpart here
    NoEffect,
}

/// One of PoB's settings, as `Settings.xml` holds it: every value is a string (`true`/`false`,
/// numbers as `tostring` writes them, colours as `^x...`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingRow {
    /// The `Settings.xml` attribute
    pub key: String,
    /// The Options control that edits it; empty for none
    pub control: String,
    pub label: String,
    /// What `Settings.xml` holds (what an override hides)
    pub shared: String,
    /// What PoB runs with now
    pub value: String,
    pub overridden: bool,
    pub effect: SettingEffect,
}

/// PoB's settings and the app's overrides (R12). Needs no build.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsView {
    pub rows: Vec<SettingRow>,
    /// The overrides an Options Save changed since the last view, for the UI to write (R12)
    #[serde(default)]
    pub saved_overrides: BTreeMap<String, String>,
    /// A save PoB made by itself (a sign-in's token) cancelled the open Options window and its
    /// unsaved edits since the last view; said once
    #[serde(default)]
    pub options_lost: bool,
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

/// A trade search link PoB opened during the user's own press (R20). The UI opens it in the
/// browser and copies it; the engine never does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalUrl {
    pub url: String,
}

/// The Import/Export tab's build sharing: the sites Share offers, the last link it made, and
/// PoB's import box (R16, R17).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShareView {
    /// The sites Share uploads to, in PoB's dropdown order
    #[serde(default)]
    pub sites: Vec<SiteRef>,
    /// The site PoB's drop-down shows: the one chosen last, else PoB's default (Pastebin)
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

/// PoB's sign-in with Path of Exile, as its section shows it (R18). It never holds a token
/// (R21): only whether there is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthView {
    #[serde(default)]
    pub signed_in: bool,
    /// PoB's login timer runs (`oauthTimer`)
    #[serde(default)]
    pub signing_in: bool,
    /// The sign-in link PoB asked to open, while it waits for the redirect (R1)
    #[serde(default)]
    pub auth_url: Option<String>,
    /// When PoB gives up waiting for the redirect (epoch seconds)
    #[serde(default)]
    pub login_deadline: Option<i64>,
    /// The realm drop-down
    pub realm: Realm,
    /// The realm's characters have been fetched (PoB's Fetch button then says "Fetched")
    #[serde(default)]
    pub fetched: bool,
    /// The league drop-down in PoB's order, ending in "Any" (`league: None`)
    #[serde(default)]
    pub leagues: Vec<String>,
    /// The realm's characters, in PoB's order (lowercase name)
    #[serde(default)]
    pub characters: Vec<CharacterEntry>,
    /// PoB's "Character import status", with colour codes stripped
    #[serde(default)]
    pub status: String,
    /// The same with PoB's colour codes, as its label draws it
    #[serde(default)]
    pub status_coded: String,
    /// PoB's error line (`oauthErrCode`)
    #[serde(default)]
    pub error: Option<String>,
    /// When PoB's rate limiter lets the next fetch go (epoch seconds)
    #[serde(default)]
    pub retry_at: Option<i64>,
    /// A fetch or import runs (`oauthLoading`)
    #[serde(default)]
    pub busy: bool,
    /// The league drop-down's selection: PoB's last league, else its first; `None` for "Any"
    /// or no list (ImportTab.lua:121-131)
    #[serde(default)]
    pub league_selected: Option<String>,
    /// The character drop-down's selection: the last character imported, else the first
    /// (ImportTab.lua:1085-1093)
    #[serde(default)]
    pub character_selected: Option<String>,
}

impl Default for OAuthView {
    fn default() -> Self {
        OAuthView {
            signed_in: false,
            signing_in: false,
            auth_url: None,
            login_deadline: None,
            realm: Realm::Pc,
            fetched: false,
            leagues: Vec::new(),
            characters: Vec::new(),
            status: String::new(),
            status_coded: String::new(),
            error: None,
            retry_at: None,
            busy: false,
            league_selected: None,
            character_selected: None,
        }
    }
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
    /// The same with PoB's colour codes, as its label draws it
    #[serde(default)]
    pub status_coded: String,
    /// PoB's label under a name without a discriminator (`siteAccountNameMissingDiscriminator`)
    #[serde(default)]
    pub missing_discriminator: String,
    /// PoB's tip under the character import (`siteCharBanditNote`)
    #[serde(default)]
    pub bandit_note: String,
    /// The account PoB's history dropdown has selected, if any
    #[serde(default)]
    pub history_selected: Option<String>,
    /// The league drop-down's selection while a list shows: the last league used (or its
    /// Standard league), else the first; `None` for "All" (ImportTab.lua:991-1013)
    #[serde(default)]
    pub league_selected: Option<String>,
    /// The character drop-down's selection: the last character imported, else the first
    #[serde(default)]
    pub character_selected: Option<String>,
}

/// What PoB's account name box allows (ImportTab.lua:367-404): whether its "Missing
/// discriminator" label shows (a name not ending in `#1234` or `-1234`), and whether Start is
/// enabled (a visible character before that ending). `text` is the box as typed.
pub fn account_name_state(text: &str) -> (bool, bool) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let tagged = n >= 5
        && matches!(chars[n - 5], '#' | '-')
        && chars[n - 4..].iter().all(|c| c.is_ascii_digit());
    // Lua's %S: not one of the C locale's space characters
    let start =
        tagged && n >= 6 && !matches!(chars[n - 6], ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c');
    (!tagged && !text.is_empty(), start)
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
            status_coded: String::new(),
            missing_discriminator: String::new(),
            bandit_note: String::new(),
            history_selected: None,
            league_selected: None,
            character_selected: None,
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
    /// PoB's colour code for the class in the entry's detail (`BuildCharacterList`)
    #[serde(default)]
    pub colour: String,
}

/// PoB's help (`help.txt`) as its About window shows it (`main:OpenAboutPopup`,
/// Main.lua:1336-1440), answering `Help`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HelpView {
    /// In the file's order; the first is opened by default
    pub sections: Vec<HelpSection>,
    /// The window's labels above the list: PoB's version, "Based on Openarl's Path of
    /// Building" and the GitHub page, with PoB's colour codes
    #[serde(default)]
    pub header: Vec<String>,
    /// The window's "Version history" (`changelog.txt`), one section per version, newest
    /// first; its title is "Version <n> (<date>)". Empty when the file isn't installed.
    #[serde(default)]
    pub changelog: Vec<HelpSection>,
    /// PoB's files the window reads that aren't installed ("help.txt", "changelog.txt"); with
    /// help.txt missing there are no sections or labels either
    #[serde(default)]
    pub missing: Vec<String>,
}

/// One `---[Title]` section of `help.txt`, or one version of `changelog.txt`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HelpSection {
    pub title: String,
    /// The rows under the title, with PoB's colour codes. A row of two columns (a key and
    /// what it does) holds a tab between them.
    pub lines: Vec<String>,
}
