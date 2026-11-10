//! The lab: passive tree geometry and node power, run on the engine thread.

use super::commands::LabCommand;
use super::run::{Affected, Event};
use super::tree;
use super::views::{SheetImage, TreeNode};
use anyhow::{Result, anyhow};
use mlua::{Function, Lua, Table};
use std::{collections::HashSet, path::Path, sync::Arc};

#[derive(Debug, Clone)]
pub enum LabEvent {
    /// The main passive tree and which of its nodes the build allocates. Sent in answer to
    /// `LabCommand::Tree`.
    Tree(Tree),
    Progress {
        done: usize,
        total: usize,
    },
    NodePower(Vec<NodeScore>),
}

#[derive(Debug, Clone)]
pub struct NodeScore {
    pub id: u32,
    pub name: String,
    pub kind: String,
    /// Points needed to reach the node, including the node itself
    pub distance: u32,
    /// Change from allocating only this node
    pub gain: f64,
    /// Change from allocating this node and the path to it
    pub path_gain: f64,
    pub per_point: f64,
}

/// The main passive tree, in PoB's tree coordinates.
#[derive(Debug, Clone)]
pub struct Tree {
    pub nodes: Vec<TreeNode>,
    /// Connections, as indices into `nodes`
    pub edges: Vec<(usize, usize)>,
    /// Sprite sheets, indexed by `Sprite::sheet`. `None` if the image couldn't be loaded.
    pub sheets: Vec<Option<Arc<SheetImage>>>,
    /// Ids of the passive nodes the build allocates
    pub allocated: HashSet<u32>,
}

fn node_scores(table: &Table) -> mlua::Result<Vec<NodeScore>> {
    table
        .sequence_values::<Table>()
        .map(|row| {
            let row = row?;
            Ok(NodeScore {
                id: row.get("id")?,
                name: row.get("name")?,
                kind: row.get("kind")?,
                distance: row.get("distance")?,
                gain: row.get("gain")?,
                path_gain: row.get("path_gain")?,
                per_point: row.get("per_point")?,
            })
        })
        .collect()
}

/// Handles a lab command. `Tree` and `NodePower` answer with `Event::Lab` and change nothing in
/// the build, so no view is affected.
pub(super) fn apply(
    lua: &Lua,
    pob_dir: &Path,
    command: LabCommand,
    send: &dyn Fn(Event),
) -> Result<Affected> {
    let lab: Table = lua.globals().get("lab").map_err(|e| anyhow!("{e}"))?;
    match command {
        LabCommand::Tree => {
            let layout = tree::layout(lua, pob_dir).map_err(|e| anyhow!("{e}"))?;
            send(Event::Lab(LabEvent::Tree(Tree {
                nodes: layout.nodes,
                edges: layout.edges,
                sheets: layout.sheets,
                allocated: tree::allocated(lua).map_err(|e| anyhow!("{e}"))?,
            })));
        }
        LabCommand::NodePower {
            metric,
            max_distance,
        } => {
            // `send` isn't 'static, so the progress callback is created in a scope
            let scores = lua
                .scope(|scope| {
                    let progress = scope.create_function(|_, (done, total): (usize, usize)| {
                        send(Event::Lab(LabEvent::Progress { done, total }));
                        Ok(())
                    })?;
                    lab.get::<Function>("node_power")?.call::<Table>((
                        metric,
                        max_distance,
                        progress,
                    ))
                })
                .map_err(|e| anyhow!("{e}"))?;
            send(Event::Lab(LabEvent::NodePower(
                node_scores(&scores).map_err(|e| anyhow!("{e}"))?,
            )));
        }
    }
    Ok(Affected::default())
}
