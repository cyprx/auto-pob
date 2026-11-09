//! Writes the fixture builds and their golden data from unmodified upstream PoB.
//!
//! Usage: `cargo run -p auto-pob --example golden -- [pob_dir]` (default: `AUTO_POB_POB_DIR`).

use anyhow::{Context as _, Result, bail};
use auto_pob::engine::upstream::{Golden, golden_of, load_xml, run_recipe, save_xml, upstream_lua};
use auto_pob::game::Game;
use std::{
    fs,
    path::{Path, PathBuf},
};

const RECIPES: [&str; 4] = ["new", "attack", "dot", "minion"];

fn write_golden(path: &Path, golden: &Golden) -> Result<()> {
    let mut text = serde_json::to_string_pretty(golden)?;
    text.push('\n');
    fs::write(path, text).with_context(|| format!("Writing {}", path.display()))
}

fn main() -> Result<()> {
    let pob_dir: PathBuf = std::env::args_os()
        .nth(1)
        .or_else(|| std::env::var_os("AUTO_POB_POB_DIR"))
        .map(PathBuf::from)
        .context("pass the PoB folder or set AUTO_POB_POB_DIR")?;
    // PoB changes the working directory, so resolve our paths first
    let pob_dir = pob_dir.canonicalize()?;
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let version = Game::Poe1.pob_version();
    let lib = fs::read_to_string(fixtures.join("lib.lua"))?;

    for name in RECIPES {
        let recipe = fs::read_to_string(fixtures.join(format!("{name}.lua")))?;
        let lua = upstream_lua(&pob_dir)?;
        run_recipe(&lua, &lib, &recipe)?;
        fs::write(fixtures.join(format!("{name}.xml")), save_xml(&lua)?)?;
        write_golden(
            &fixtures.join(format!("{name}.golden.json")),
            &golden_of(&lua, version)?,
        )?;
        eprintln!("wrote {name}");
    }

    // builds kept only on this machine: no recipe, the XML is the source
    let local = fixtures.join("local");
    if local.is_dir() {
        let mut xmls: Vec<PathBuf> = fs::read_dir(&local)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "xml"))
            .collect();
        xmls.sort();
        for xml in xmls {
            let Some(name) = xml.file_stem().and_then(|s| s.to_str()) else {
                bail!("odd file name {}", xml.display());
            };
            let lua = upstream_lua(&pob_dir)?;
            load_xml(&lua, &fs::read_to_string(&xml)?, name)?;
            write_golden(
                &local.join(format!("{name}.golden.json")),
                &golden_of(&lua, version)?,
            )?;
            eprintln!("wrote local/{name}");
        }
    }
    Ok(())
}
