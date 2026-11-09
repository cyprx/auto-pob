//! Commands the UI sends to the engine thread.

use std::path::PathBuf;

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

    Undo(Tab),
    Redo(Tab),
    Lab(LabCommand),
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
}

/// Lab commands. Answers arrive as `Event::Lab`.
#[derive(Debug, Clone)]
pub enum LabCommand {
    /// Send the passive tree and the build's allocated nodes.
    Tree,
    /// Rank unallocated passives by `metric` (a calc output key); streams progress.
    NodePower {
        metric: &'static str,
        max_distance: u32,
    },
}
