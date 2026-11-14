//! Reads the passive tree's geometry out of PoB's Lua (see `glue/tree.lua`).

use super::views::{
    ClassArt, Connector, JewelRadius, LuaGroupArt, LuaSprite, NodeArt, SheetImage, Sprite,
    TreeLayout, TreeNode,
};
use mlua::LuaSerdeExt;
use mlua::{Function, Lua, Table};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The key of the layout `layout` would build, without building it.
pub(super) fn layout_key(lua: &Lua) -> mlua::Result<String> {
    let glue: Table = lua.globals().get("glue")?;
    let digest: mlua::LuaString = glue.get::<Function>("tree_layout_digest")?.call(())?;
    Ok(key_of(&digest.as_bytes()))
}

/// The layout's key: its tree version, then a hash of everything it draws. The digest starts
/// with the version, on its own line. The hasher has fixed keys, so keys are stable.
fn key_of(digest: &[u8]) -> String {
    use std::hash::Hasher;
    let version_end = digest
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(digest.len());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(digest);
    format!(
        "{}:{:016x}",
        String::from_utf8_lossy(&digest[..version_end]),
        hasher.finish()
    )
}

/// Reads the active spec's tree geometry from `glue.tree_layout()`.
pub(super) fn layout(lua: &Lua, pob_dir: &Path) -> mlua::Result<TreeLayout> {
    let glue: Table = lua.globals().get("glue")?;
    let tree: Table = glue.get::<Function>("tree_layout")?.call(())?;

    // the files first: a whole file's sprite is as large as the file
    let sheets: Vec<Option<Arc<SheetImage>>> = tree
        .get::<Table>("sheets")?
        .sequence_values::<String>()
        .map(|path| path.map(|path| load_sheet(&pob_dir.join(path))))
        .collect::<mlua::Result<_>>()?;
    let sprite = |value: mlua::Value| -> mlua::Result<Option<Sprite>> {
        if value.is_nil() {
            return Ok(None);
        }
        Ok(lua.from_value::<LuaSprite>(value)?.resolve(&sheets))
    };

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
                art: node_art(&node.get::<Table>("art")?, &sprite)?,
                ascendancy: node.get("ascendancy")?,
                replaceable: node.get("replaceable")?,
                mastery_effects: node.get("mastery_effects")?,
                in_radius: node.get::<Option<bool>>("in_radius")?.unwrap_or(false),
                radius_socket: node.get::<Option<bool>>("radius_socket")?.unwrap_or(false),
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

    let connectors: Vec<Connector> = lua.from_value(tree.get("connectors")?)?;
    let groups = tree
        .get::<Table>("groups")?
        .sequence_values::<mlua::Value>()
        .map(|g| Ok(lua.from_value::<LuaGroupArt>(g?)?.resolve(&sheets)))
        .filter_map(Result::transpose)
        .collect::<mlua::Result<_>>()?;
    let class_art = tree
        .get::<Table>("class_art")?
        .sequence_values::<Table>()
        .map(|entry| {
            let entry = entry?;
            Ok(sprite(entry.get("art")?)?.map(|art| ClassArt {
                class: entry.get("class").unwrap_or(0),
                art,
                pos: [entry.get("x").unwrap_or(0.0), entry.get("y").unwrap_or(0.0)],
            }))
        })
        .filter_map(Result::transpose)
        .collect::<mlua::Result<_>>()?;
    let mut assets = HashMap::new();
    for entry in tree.get::<Table>("assets")?.sequence_values::<Table>() {
        let entry = entry?;
        if let Some(art) = sprite(entry.get("art")?)? {
            assets.insert(entry.get::<String>("name")?, art);
        }
    }
    let jewel_radii = tree
        .get::<Table>("jewel_radii")?
        .sequence_values::<Table>()
        .map(|r| {
            let r = r?;
            Ok(JewelRadius {
                inner: r.get("inner")?,
                outer: r.get("outer")?,
                colour: colour_code(&r.get::<String>("colour")?),
            })
        })
        .collect::<mlua::Result<_>>()?;

    Ok(TreeLayout {
        version: tree.get("version")?,
        nodes,
        edges,
        connectors,
        groups,
        class_art,
        assets,
        size: tree.get::<Option<f32>>("size")?.unwrap_or(1.0),
        jewel_radii,
        sheets,
        key: key_of(&tree.get::<mlua::LuaString>("digest")?.as_bytes()),
    })
}

/// A `^xRRGGBB` colour code's colour; white for anything else
fn colour_code(code: &str) -> [u8; 3] {
    let hex = code.strip_prefix("^x").unwrap_or(code);
    let channel = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .unwrap_or(255)
    };
    [channel(0), channel(2), channel(4)]
}

/// Decodes a sprite sheet, caching it since every loaded build resends the tree. The cache is
/// the engine thread's, and each UI gets an engine thread of its own (a restart spawns a new
/// one), so once that UI took a sheet's pixels to upload them it has the texture: the sheet is
/// handed out again without them rather than decoded again (about 30 ms for a 3.29 tree,
/// which a mastery pick would pay, since it resends the layout).
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
    let sheet = Arc::new(SheetImage::new(
        path.to_owned(),
        [image.width() as usize, image.height() as usize],
        image.into_raw(),
    ));
    CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(path.to_owned(), Arc::clone(&sheet))
    });
    Some(sheet)
}

fn node_art(
    art: &Table,
    sprite: &impl Fn(mlua::Value) -> mlua::Result<Option<Sprite>>,
) -> mlua::Result<NodeArt> {
    // a list of sprites, any of which may be missing
    let list = |key: &str, n: usize| -> mlua::Result<Vec<Option<Sprite>>> {
        let Some(list) = art.get::<Option<Table>>(key)? else {
            return Ok(vec![None; n]);
        };
        (1..=n).map(|i| sprite(list.get(i)?)).collect()
    };
    let three = |key| -> mlua::Result<[Option<Sprite>; 3]> {
        let v = list(key, 3)?;
        Ok([v[0], v[1], v[2]])
    };
    let effect = list("effect", 2)?;
    Ok(NodeArt {
        base: three("base")?,
        overlay: three("overlay")?,
        effect: [effect[0], effect[1]],
        hover: sprite(art.get("hover")?)?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn colour_codes_read_as_pob_writes_them() {
        assert_eq!(super::colour_code("^xBB6600"), [0xbb, 0x66, 0x00]);
        assert_eq!(super::colour_code("^x2222CC"), [0x22, 0x22, 0xcc]);
        assert_eq!(super::colour_code("^7"), [255, 255, 255]);
    }
}
