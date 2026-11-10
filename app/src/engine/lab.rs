//! The lab: passive tree geometry and node power, run on the engine thread.

use super::commands::LabCommand;
use super::run::{Affected, Event};
use anyhow::{Result, anyhow};
use mlua::{Function, Lua, Table};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

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
    /// Sprite sheets, indexed by [`Sprite::sheet`]. `None` if the image couldn't be loaded.
    pub sheets: Vec<Option<Arc<SheetImage>>>,
    /// Ids of the passive nodes the build allocates
    pub allocated: HashSet<u32>,
}

/// A decoded sprite sheet.
pub struct SheetImage {
    pub path: PathBuf,
    pub size: [usize; 2],
    /// RGBA, 8 bits per channel, not premultiplied
    pub rgba: Vec<u8>,
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
    /// PoB's node type: "Normal", "Notable", "Keystone", "Mastery", "Socket" or "ClassStart"
    pub kind: String,
    pub name: String,
    /// Stat descriptions, one per line
    pub stats: Vec<String>,
    /// (group, orbit). Nodes sharing both are connected by an arc around `group_center`.
    pub orbit: Option<(u32, u32)>,
    pub group_center: Option<[f32; 2]>,
    pub art: NodeArt,
}

fn tree(lua: &Lua, pob_dir: &Path) -> mlua::Result<Tree> {
    let lab: Table = lua.globals().get("lab")?;
    let tree: Table = lab.get::<Function>("tree")?.call(())?;

    let nodes = tree
        .get::<Table>("nodes")?
        .sequence_values::<Table>()
        .map(|node| {
            let node = node?;
            let group = node.get::<Option<u32>>("group")?;
            let orbit = node.get::<Option<u32>>("orbit")?;
            let group_x = node.get::<Option<f32>>("group_x")?;
            let group_y = node.get::<Option<f32>>("group_y")?;
            Ok(TreeNode {
                id: node.get("id")?,
                pos: [node.get("x")?, node.get("y")?],
                kind: node.get("kind")?,
                name: node.get("name")?,
                stats: node
                    .get::<Table>("stats")?
                    .sequence_values()
                    .collect::<mlua::Result<_>>()?,
                orbit: group.zip(orbit),
                group_center: group_x.zip(group_y).map(|(x, y)| [x, y]),
                art: node_art(&node.get::<Table>("art")?)?,
            })
        })
        .collect::<mlua::Result<Vec<_>>>()?;

    let index: HashMap<u32, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let edges = tree
        .get::<Table>("edges")?
        .sequence_values::<Table>()
        .filter_map(|edge| {
            let edge = match edge {
                Ok(edge) => edge,
                Err(e) => return Some(Err(e)),
            };
            let ids = (edge.get::<u32>(1), edge.get::<u32>(2));
            match ids {
                (Ok(a), Ok(b)) => Some(Ok((*index.get(&a)?, *index.get(&b)?))),
                (Err(e), _) | (_, Err(e)) => Some(Err(e)),
            }
        })
        .collect::<mlua::Result<_>>()?;

    let sheets = tree
        .get::<Table>("sheets")?
        .sequence_values::<String>()
        .map(|path| path.map(|path| load_sheet(&pob_dir.join(path))))
        .collect::<mlua::Result<_>>()?;

    let allocated = tree
        .get::<Table>("allocated")?
        .sequence_values::<u32>()
        .collect::<mlua::Result<_>>()?;

    Ok(Tree {
        nodes,
        edges,
        sheets,
        allocated,
    })
}

/// Decodes a sprite sheet, caching it since every loaded build resends the tree.
fn load_sheet(path: &Path) -> Option<Arc<SheetImage>> {
    thread_local! {
        static CACHE: RefCell<HashMap<PathBuf, Arc<SheetImage>>> = RefCell::default();
    }
    if let Some(sheet) = CACHE.with(|cache| cache.borrow().get(path).cloned()) {
        return Some(sheet);
    }
    let image = match image::open(path) {
        Ok(image) => image.into_rgba8(),
        Err(e) => {
            eprintln!("Unable to load {}: {e}", path.display());
            return None;
        }
    };
    let sheet = Arc::new(SheetImage {
        path: path.to_owned(),
        size: [image.width() as usize, image.height() as usize],
        rgba: image.into_raw(),
    });
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(path.to_owned(), Arc::clone(&sheet))
    });
    Some(sheet)
}

fn node_art(art: &Table) -> mlua::Result<NodeArt> {
    // a { unallocated, allocated } pair of sprites, either of which may be missing
    let pair = |key: &str| -> mlua::Result<[Option<Sprite>; 2]> {
        let Some(pair) = art.get::<Option<Table>>(key)? else {
            return Ok([None, None]);
        };
        let sprite = |i: usize| -> mlua::Result<Option<Sprite>> {
            let Some(s) = pair.get::<Option<Table>>(i)? else {
                return Ok(None);
            };
            Ok(Some(Sprite {
                // Lua indices start at 1
                sheet: s.get::<usize>("sheet")? - 1,
                rect: [s.get("x")?, s.get("y")?, s.get("w")?, s.get("h")?],
            }))
        };
        Ok([sprite(1)?, sprite(2)?])
    };
    Ok(NodeArt {
        icon: pair("icon")?,
        frame: pair("frame")?,
    })
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

impl std::fmt::Debug for SheetImage {
    // the pixels would flood any log
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetImage")
            .field("path", &self.path)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
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
            let tree = tree(lua, pob_dir).map_err(|e| anyhow!("{e}"))?;
            send(Event::Lab(LabEvent::Tree(tree)));
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
