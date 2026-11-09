//! Runs unmodified upstream Path of Building (through its `HeadlessWrapper.lua`) to produce
//! reference data for the gate tests.

use anyhow::{Context as _, Result, anyhow};
use mlua::{Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

/// Starts upstream PoB headlessly in `pob_dir`, which also becomes the working directory.
///
/// The returned state has the wrapper's globals (`build`, `newBuild`, `loadBuildFromXML`)
/// and the real `Inflate`/`Deflate` (the wrapper stubs them).
pub fn upstream_lua(pob_dir: &Path) -> Result<Lua> {
    let lua = super::pob::new_lua(pob_dir)?;
    lua.load("dofile('HeadlessWrapper.lua')")
        .set_name("=headless")
        .exec()
        .map_err(|e| anyhow!("{e}"))
        .context("Starting upstream HeadlessWrapper.lua")?;
    super::native::register_compression(&lua).map_err(|e| anyhow!("{e}"))?;
    Ok(lua)
}

/// What upstream PoB shows for a build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Golden {
    pub pob_version: String,
    /// Every numeric `displayStats` entry present in the main output, by stat key.
    pub stats: BTreeMap<String, f64>,
    /// Like `stats`, for `minionDisplayStats` against the main skill's minion; absent without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minion_stats: Option<BTreeMap<String, f64>>,
    /// Config variables whose controls are shown, sorted.
    pub visible_config: Vec<String>,
    /// `configTab.input`: booleans, numbers and strings.
    pub config: BTreeMap<String, serde_json::Value>,
}

const EXTRACT: &str = r#"
local stats = {}
for _, s in ipairs(build.displayStats) do
	if s.stat then
		local v = build.calcsTab.mainOutput[s.stat]
		if type(v) == "number" then stats[s.stat] = v end
	end
end
local minionStats
local minion = build.calcsTab.mainOutput.Minion
if minion then
	minionStats = {}
	for _, s in ipairs(build.minionDisplayStats) do
		if s.stat then
			local v = minion[s.stat]
			if type(v) == "number" then minionStats[s.stat] = v end
		end
	end
end
local visible = {}
for var, control in pairs(build.configTab.varControls) do
	if control:IsShown() then visible[#visible + 1] = var end
end
table.sort(visible)
return { stats = stats, minion_stats = minionStats, visible_config = visible, config = build.configTab.input }
"#;

/// Reads the golden data from the build currently loaded in `lua`.
pub fn golden_of(lua: &Lua, pob_version: &str) -> Result<Golden> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        stats: BTreeMap<String, f64>,
        minion_stats: Option<BTreeMap<String, f64>>,
        #[serde(default)]
        visible_config: Vec<String>,
        #[serde(default)]
        config: BTreeMap<String, serde_json::Value>,
    }
    let value: mlua::Value = lua
        .load(EXTRACT)
        .set_name("=extract")
        .eval()
        .map_err(|e| anyhow!("{e}"))?;
    let raw: Raw = lua.from_value(value).map_err(|e| anyhow!("{e}"))?;
    Ok(Golden {
        pob_version: pob_version.to_string(),
        stats: raw.stats,
        minion_stats: raw.minion_stats,
        visible_config: raw.visible_config,
        config: raw.config,
    })
}

/// Resets to a new build, then applies the recipe (it uses `buildFixture` from `lib.lua`).
pub fn run_recipe(lua: &Lua, lib: &str, recipe: &str) -> Result<()> {
    lua.load("newBuild()").exec().map_err(|e| anyhow!("{e}"))?;
    lua.load(lib)
        .set_name("=fixture lib")
        .exec()
        .map_err(|e| anyhow!("{e}"))?;
    lua.load(recipe)
        .set_name("=recipe")
        .exec()
        .map_err(|e| anyhow!("{e}"))
        .context("Running the fixture recipe")
}

/// Loads a saved build into the wrapper's `build`.
pub fn load_xml(lua: &Lua, xml: &str, name: &str) -> Result<()> {
    let f = lua
        .globals()
        .get::<mlua::Function>("loadBuildFromXML")
        .map_err(|e| anyhow!("{e}"))?;
    f.call::<()>((xml, name)).map_err(|e| anyhow!("{e}"))
}

/// The saved XML of the current build.
pub fn save_xml(lua: &Lua) -> Result<String> {
    lua.load("return build:SaveDB('fixture')")
        .eval::<String>()
        .map_err(|e| anyhow!("{e}"))
}
