//! Runs Path of Building's calc engine headlessly on a worker thread.
//!
//! The Lua state lives on the worker thread. The UI sends [`Command`]s and receives
//! [`Event`]s over channels, so the UI never blocks on a calculation.

use anyhow::{Context as _, Result, anyhow, bail};
use flate2::{Compression, read::ZlibDecoder, read::ZlibEncoder};
use mlua::{Function, Lua, LuaString, Table, Value};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

/// Stats shown in the UI as (calc output key, label).
pub const STATS: &[(&str, &str)] = &[
    ("Life", "Life"),
    ("EnergyShield", "Energy Shield"),
    ("Mana", "Mana"),
    ("TotalEHP", "Effective HP"),
    ("Armour", "Armour"),
    ("Evasion", "Evasion"),
    ("FireResist", "Fire Res"),
    ("ColdResist", "Cold Res"),
    ("LightningResist", "Lightning Res"),
    ("ChaosResist", "Chaos Res"),
    ("CombinedDPS", "Combined DPS"),
];

/// Stats node power can be ranked by, as (calc output key, label).
pub const METRICS: &[(&str, &str)] = &[
    ("CombinedDPS", "Combined DPS"),
    ("Life", "Life"),
    ("EnergyShield", "Energy Shield"),
    ("TotalEHP", "Effective HP"),
    ("Armour", "Armour"),
    ("Evasion", "Evasion"),
];

pub enum ConfigKind {
    Check,
    /// Allowed values as (value, label)
    List(&'static [(&'static str, &'static str)]),
}

/// A Configuration tab option exposed in the UI.
pub struct ConfigOption {
    /// Variable name in PoB's `Modules/ConfigOptions.lua`
    pub var: &'static str,
    pub label: &'static str,
    pub kind: ConfigKind,
}

pub const CONFIG: &[ConfigOption] = &[
    ConfigOption {
        var: "usePowerCharges",
        label: "Power charges",
        kind: ConfigKind::Check,
    },
    ConfigOption {
        var: "useFrenzyCharges",
        label: "Frenzy charges",
        kind: ConfigKind::Check,
    },
    ConfigOption {
        var: "useEnduranceCharges",
        label: "Endurance charges",
        kind: ConfigKind::Check,
    },
    ConfigOption {
        var: "buffOnslaught",
        label: "Onslaught",
        kind: ConfigKind::Check,
    },
    ConfigOption {
        var: "enemyIsBoss",
        label: "Enemy is boss",
        kind: ConfigKind::List(&[
            ("None", "No"),
            ("Boss", "Standard boss"),
            ("Pinnacle", "Pinnacle boss"),
            ("Uber", "Uber pinnacle boss"),
        ]),
    },
];

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    /// Not set in the build; PoB uses the option's default
    Unset,
    Bool(bool),
    Text(String),
}

pub enum Command {
    NewBuild,
    /// A PoB share code or build XML
    Load(String),
    SetLevel(u32),
    SetConfig(&'static str, ConfigValue),
    NodePower {
        metric: &'static str,
        max_distance: u32,
    },
}

pub enum Event {
    Status(String),
    /// Sent at startup and when a build is loaded, as builds may use different tree versions
    Tree(Tree),
    Build(BuildSummary),
    Progress {
        done: usize,
        total: usize,
    },
    NodePower(Vec<NodeScore>),
    Error(String),
}

pub struct BuildSummary {
    pub class: String,
    pub ascendancy: Option<String>,
    pub level: u32,
    /// Values for [`STATS`], in order
    pub stats: Vec<Option<f64>>,
    /// Values for [`CONFIG`], in order
    pub config: Vec<ConfigValue>,
    /// Ids of allocated passive nodes
    pub allocated: HashSet<u32>,
}

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
pub struct Tree {
    pub nodes: Vec<TreeNode>,
    /// Connections, as indices into `nodes`
    pub edges: Vec<(usize, usize)>,
    /// Sprite sheets, indexed by [`Sprite::sheet`]. `None` if the image couldn't be loaded.
    pub sheets: Vec<Option<Arc<SheetImage>>>,
}

/// A decoded sprite sheet.
pub struct SheetImage {
    pub path: PathBuf,
    pub size: [usize; 2],
    /// RGBA, 8 bits per channel, not premultiplied
    pub rgba: Vec<u8>,
}

/// A rectangle on a sprite sheet.
#[derive(Clone, Copy)]
pub struct Sprite {
    pub sheet: usize,
    /// x, y, width, height in pixels
    pub rect: [f32; 4],
}

/// A node's artwork as [unallocated, allocated] pairs.
#[derive(Default)]
pub struct NodeArt {
    pub icon: [Option<Sprite>; 2],
    pub frame: [Option<Sprite>; 2],
}

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

/// Handle to the engine's worker thread.
pub struct Engine {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
}

impl Engine {
    /// Starts PoB from `pob_dir` on a worker thread. `wake_ui` is called after every event.
    pub fn spawn(pob_dir: PathBuf, wake_ui: impl Fn() + Send + 'static) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();

        thread::spawn(move || {
            let send = |event| {
                let _ = event_tx.send(event);
                wake_ui();
            };
            run(&pob_dir, &command_rx, &send);
        });

        Self { commands, events }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

fn run(pob_dir: &Path, commands: &Receiver<Command>, send: &dyn Fn(Event)) {
    send(Event::Status("Starting Path of Building...".into()));
    let lua = match start(pob_dir) {
        Ok(lua) => lua,
        Err(e) => return send(Event::Error(format!("{e:#}"))),
    };
    send(tree(&lua).map_or_else(|e| Event::Error(format!("{e:#}")), Event::Tree));
    send(summary(&lua).map_or_else(|e| Event::Error(format!("{e:#}")), Event::Build));
    send(Event::Status("Ready".into()));

    for command in commands {
        let result = handle(&lua, command, send);
        if let Err(e) = result {
            send(Event::Error(format!("{e:#}")));
        }
    }
}

fn handle(lua: &Lua, command: Command, send: &dyn Fn(Event)) -> mlua::Result<()> {
    let lab: Table = lua.globals().get("lab")?;
    match command {
        Command::NewBuild => {
            lab.get::<Function>("new_build")?.call::<()>(())?;
            send(Event::Tree(tree(lua)?));
        }
        Command::Load(text) => {
            let text = text.trim();
            let function = if text.starts_with('<') {
                "load_xml"
            } else {
                "load_code"
            };
            lab.get::<Function>(function)?.call::<()>(text)?;
            send(Event::Tree(tree(lua)?));
        }
        Command::SetLevel(level) => lab.get::<Function>("set_level")?.call::<()>(level)?,
        Command::SetConfig(var, value) => {
            let value = match value {
                ConfigValue::Unset => Value::Nil,
                ConfigValue::Bool(b) => Value::Boolean(b),
                ConfigValue::Text(s) => Value::String(lua.create_string(&s)?),
            };
            lab.get::<Function>("set_config")?
                .call::<()>((var, value))?;
        }
        Command::NodePower {
            metric,
            max_distance,
        } => {
            // `send` isn't 'static, so the progress callback is created in a scope
            let scores = lua.scope(|scope| {
                let progress = scope.create_function(|_, (done, total): (usize, usize)| {
                    send(Event::Progress { done, total });
                    Ok(())
                })?;
                lab.get::<Function>("node_power")?
                    .call::<Table>((metric, max_distance, progress))
            })?;
            send(Event::NodePower(node_scores(&scores)?));
            return Ok(());
        }
    }
    send(Event::Build(summary(lua)?));
    Ok(())
}

/// Creates the Lua state and starts PoB's headless wrapper.
fn start(pob_dir: &Path) -> Result<Lua> {
    // mlua's error isn't Send + Sync, which anyhow requires; keep its message instead
    let lua_error = |e: mlua::Error| anyhow!("{e}");

    // PoB loads its files relative to the working directory
    std::env::set_current_dir(pob_dir)
        .with_context(|| format!("PoB not found in {}", pob_dir.display()))?;

    // `unsafe_new` gives PoB the full standard library, like the main runtime does
    let lua = unsafe { Lua::unsafe_new() };
    let utf8 = luautf8::create_module(&lua).map_err(lua_error)?;
    lua.register_module("lua-utf8", utf8).map_err(lua_error)?;
    lua.load(PRELUDE)
        .set_name("=prelude")
        .exec()
        .map_err(lua_error)?;
    lua.load(r#"dofile("HeadlessWrapper.lua")"#)
        .set_name("=wrapper")
        .exec()
        .map_err(lua_error)
        .context("Running PoB's HeadlessWrapper.lua")?;

    if lua
        .globals()
        .get::<Value>("build")
        .map_err(lua_error)?
        .is_nil()
    {
        bail!("Path of Building failed to start; see the console output");
    }

    // the wrapper stubs these out, but share codes need them
    let install = || -> mlua::Result<()> {
        lua.globals()
            .set("Inflate", lua.create_function(inflate)?)?;
        lua.globals()
            .set("Deflate", lua.create_function(deflate)?)?;
        lua.load(include_str!("glue.lua"))
            .set_name("@glue.lua")
            .exec()
    };
    install().map_err(lua_error)?;
    Ok(lua)
}

/// Runs before PoB's headless wrapper.
const PRELUDE: &str = r##"
package.path = "./lua/?.lua;./lua/?/init.lua;" .. package.path

-- command line arguments; PoB reads an optional build import URL from here
arg = {}

-- PoB's files start with a "#@" line that LuaJIT rejects; strip it when loading
local function load_stripped(name)
	local file, err = io.open(name, "rb")
	if not file then return nil, err end
	local source = file:read("*a"):gsub("^#@[^\n]*", "")
	file:close()
	return loadstring(source, "@" .. name)
end
loadfile = load_stripped
dofile = function(name) return assert(load_stripped(name))() end

-- the wrapper waits for input on startup errors; there is no console to answer it
io.read = function() return "" end
"##;

fn inflate(lua: &Lua, data: LuaString) -> mlua::Result<Value> {
    let mut out = Vec::new();
    match ZlibDecoder::new(&data.as_bytes()[..]).read_to_end(&mut out) {
        Ok(_) => Ok(Value::String(lua.create_string(&out)?)),
        Err(_) => Ok(Value::Nil),
    }
}

fn deflate(lua: &Lua, data: LuaString) -> mlua::Result<Value> {
    let mut out = Vec::new();
    ZlibEncoder::new(&data.as_bytes()[..], Compression::best())
        .read_to_end(&mut out)
        .map_err(mlua::Error::external)?;
    Ok(Value::String(lua.create_string(&out)?))
}

fn summary(lua: &Lua) -> mlua::Result<BuildSummary> {
    let lab: Table = lua.globals().get("lab")?;
    let stat_keys = lua.create_sequence_from(STATS.iter().map(|(key, _)| *key))?;
    let config_vars = lua.create_sequence_from(CONFIG.iter().map(|option| option.var))?;
    let summary: Table = lab
        .get::<Function>("summary")?
        .call((stat_keys, config_vars))?;

    let stats_table: Table = summary.get("stats")?;
    let stats = (1..=STATS.len())
        .map(|i| stats_table.get::<Option<f64>>(i))
        .collect::<mlua::Result<_>>()?;

    let config_table: Table = summary.get("config")?;
    let config = (1..=CONFIG.len())
        .map(|i| {
            Ok(match config_table.get::<Value>(i)? {
                Value::Boolean(b) => ConfigValue::Bool(b),
                Value::String(s) => ConfigValue::Text(s.to_string_lossy()),
                _ => ConfigValue::Unset,
            })
        })
        .collect::<mlua::Result<_>>()?;

    Ok(BuildSummary {
        class: summary.get("class")?,
        ascendancy: summary
            .get::<Option<String>>("ascendancy")?
            .filter(|a| a != "None"),
        level: summary.get("level")?,
        stats,
        config,
        allocated: summary
            .get::<Table>("allocated")?
            .sequence_values::<u32>()
            .collect::<mlua::Result<_>>()?,
    })
}

fn tree(lua: &Lua) -> mlua::Result<Tree> {
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

    // PoB loads its files relative to the working directory, which `start` set to its folder
    let pob_dir = std::env::current_dir().map_err(mlua::Error::external)?;
    let sheets = tree
        .get::<Table>("sheets")?
        .sequence_values::<String>()
        .map(|path| path.map(|path| load_sheet(&pob_dir.join(path))))
        .collect::<mlua::Result<_>>()?;

    Ok(Tree {
        nodes,
        edges,
        sheets,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Waits for the next event that isn't a status or progress update.
    fn next(engine: &Engine) -> Event {
        loop {
            match engine.events.recv_timeout(Duration::from_secs(120)) {
                Ok(Event::Status(_) | Event::Progress { .. }) => continue,
                Ok(event) => return event,
                Err(e) => panic!("no event from engine: {e}"),
            }
        }
    }

    fn expect_build(engine: &Engine) -> BuildSummary {
        loop {
            match next(engine) {
                // loading a build resends the tree first
                Event::Tree(_) => continue,
                Event::Build(build) => return build,
                Event::Error(e) => panic!("engine error: {e}"),
                _ => panic!("expected a build summary"),
            }
        }
    }

    fn stat(build: &BuildSummary, key: &str) -> Option<f64> {
        let i = STATS.iter().position(|(k, _)| *k == key).unwrap();
        build.stats[i]
    }

    /// Encodes like PoB's "Generate" share code: zlib, then URL-safe base64.
    fn share_code(xml: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut compressed = Vec::new();
        ZlibEncoder::new(xml.as_bytes(), Compression::best())
            .read_to_end(&mut compressed)
            .unwrap();
        let mut code = String::new();
        for chunk in compressed.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..=chunk.len() {
                code.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
            code.extend(std::iter::repeat_n('=', 3 - chunk.len()));
        }
        code
    }

    /// Drives the engine like the UI does. Needs a PoB1 install from rusty-path-of-building.
    #[test]
    #[ignore = "needs a local PoB install"]
    fn engine_end_to_end() {
        let pob_dir = directories::BaseDirs::new()
            .unwrap()
            .data_dir()
            .join("RustyPathOfBuilding1");
        let engine = Engine::spawn(pob_dir, || {});
        let Event::Tree(tree) = next(&engine) else {
            panic!("expected the tree first");
        };
        assert!(tree.nodes.len() > 2000, "main tree has ~2350 nodes");
        assert!(tree.edges.len() > 2000);
        assert!(!tree.sheets.is_empty() && tree.sheets.iter().all(Option::is_some));
        let with_icons = tree
            .nodes
            .iter()
            .filter(|n| n.art.icon[0].is_some())
            .count();
        assert!(
            with_icons * 10 > tree.nodes.len() * 9,
            "most nodes have an icon ({with_icons}/{})",
            tree.nodes.len()
        );
        let initial = expect_build(&engine);
        assert_eq!(initial.class, "Scion");
        assert!(!initial.allocated.is_empty(), "class start is allocated");

        let xml = r#"<PathOfBuilding><Build level="75" className="Witch" ascendClassName="None" mainSocketGroup="1" viewMode="TREE" targetVersion="3_0"/><Tree activeSpec="1"><Spec treeVersion="3_29" classId="3" ascendClassId="0" nodes=""/></Tree></PathOfBuilding>"#;
        engine.send(Command::Load(xml.into()));
        let from_xml = expect_build(&engine);
        assert_eq!((from_xml.class.as_str(), from_xml.level), ("Witch", 75));

        engine.send(Command::NewBuild);
        expect_build(&engine);
        engine.send(Command::Load(share_code(xml)));
        let from_code = expect_build(&engine);
        assert_eq!((from_code.class.as_str(), from_code.level), ("Witch", 75));
        assert_eq!(stat(&from_code, "Life"), stat(&from_xml, "Life"));

        engine.send(Command::SetLevel(90));
        let leveled = expect_build(&engine);
        assert_eq!(leveled.level, 90);
        assert!(stat(&leveled, "Life").unwrap() > stat(&from_code, "Life").unwrap());

        engine.send(Command::SetConfig(
            "enemyIsBoss",
            ConfigValue::Text("Pinnacle".into()),
        ));
        let configured = expect_build(&engine);
        let boss = CONFIG.iter().position(|o| o.var == "enemyIsBoss").unwrap();
        assert_eq!(
            configured.config[boss],
            ConfigValue::Text("Pinnacle".into())
        );

        engine.send(Command::NodePower {
            metric: "Life",
            max_distance: 6,
        });
        let Event::NodePower(scores) = next(&engine) else {
            panic!("expected node power results");
        };
        assert!(!scores.is_empty());
        assert!(scores.iter().all(|s| (1..=6).contains(&s.distance)));
        let best = scores
            .iter()
            .max_by(|a, b| a.per_point.total_cmp(&b.per_point))
            .unwrap();
        assert!(
            best.per_point > 0.0,
            "some node near the Witch start should add life"
        );
        eprintln!(
            "{} nodes scored; best for life: {} (+{:.1}/point over {} points)",
            scores.len(),
            best.name,
            best.per_point,
            best.distance
        );
    }
}
