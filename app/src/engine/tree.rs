//! Reads the passive tree's geometry out of PoB's Lua (see `glue/tree.lua`).

use super::views::{NodeArt, SheetImage, Sprite, TreeLayout, TreeNode};
use mlua::{Function, Lua, Table};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

/// The ids of the allocated nodes.
pub(super) fn allocated(lua: &Lua) -> mlua::Result<HashSet<u32>> {
    let glue: Table = lua.globals().get("glue")?;
    glue.get::<Function>("alloc_ids")?
        .call::<Table>(())?
        .sequence_values()
        .collect()
}

/// The key of the layout `layout` would build, without building it.
pub(super) fn layout_key(lua: &Lua) -> mlua::Result<String> {
    let glue: Table = lua.globals().get("glue")?;
    glue.get::<Function>("tree_layout_key")?.call(())
}

/// Reads the active spec's tree geometry from `glue.tree_layout()`.
pub(super) fn layout(lua: &Lua, pob_dir: &Path) -> mlua::Result<TreeLayout> {
    let glue: Table = lua.globals().get("glue")?;
    let tree: Table = glue.get::<Function>("tree_layout")?.call(())?;

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
                ascendancy: node.get("ascendancy")?,
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

    Ok(TreeLayout {
        version: tree.get("version")?,
        nodes,
        edges,
        sheets,
        key: tree.get("key")?,
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
