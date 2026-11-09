//! Downloads the pinned PoB into a folder: `cargo run -p auto-pob --example fetch_pob -- <dest>`.
//! Used by CI to prepare `AUTO_POB_POB_DIR` for the gate tests.

use auto_pob::{game::Game, pob_files};

fn main() -> anyhow::Result<()> {
    let dest = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: fetch_pob <dest-dir>"))?;
    pob_files::download_into(Game::Poe1, std::path::Path::new(&dest))
}
