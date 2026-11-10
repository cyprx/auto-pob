//! Commands the UI sends to the engine thread.

use std::path::PathBuf;

use super::views::CellRef;

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

    /// A query: the breakdown of a Calcs cell; the latest one in a batch is answered.
    Breakdown(CellRef),

    Undo(Tab),
    Redo(Tab),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Config,
    Tree,
    Calcs,
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
