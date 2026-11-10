# auto-pob

An [egui](https://github.com/emilk/egui) front-end for Path of Building (PoE1). It runs PoB's
own Lua calculation engine headlessly on a background thread and draws the UI itself, so the
numbers are PoB's numbers.

## What it does

- **Build list**: browse the `Builds/` folder (folders first), open a build, import a PoB
  share code, start a new build.
- **Build screen**: tabs for the open build, with save, save as, undo/redo and an
  unsaved-changes prompt.
- **Stats sidebar**: PoB's sidebar stats, recalculated after every change.
- **Config tab**: PoB's configuration options (checkboxes, numbers, lists, custom modifiers),
  with config sets, PoB's tooltips and validation. Changes recalculate the stats.
- **Tree tab**: the passive tree, drawn with PoB's own art and rules. Drag to pan, scroll to
  zoom, hover for PoB's tooltip.
  - Click a node to allocate it (with the path to it) or to deallocate it. Click a mastery
    to pick its effect.
  - Change class and ascendancy.
  - Keep several trees, switch between them, rename, copy or delete them, and compare two.
  - Search nodes, and colour the tree by a heatmap of node power. A power report lists the
    best nodes to take.
  - Show jewel radii, convert a tree to another tree version, import a tree link and copy
    the link of the current tree.
- **Shortcuts**: Ctrl+S saves. Ctrl+Z undoes and Ctrl+Y (or Ctrl+Shift+Z) redoes, on the
  Tree and Config tabs. Each tab has its own history. They wait while a dialog is open.

## Running

```bash
cargo run -p auto-pob --release
```

On first run it downloads PoB 2.67.2 (about 600 MB) into
`<data dir>/auto-pob/poe1/pob-2.67.2`, where `<data dir>` is the platform data folder
(`~/.local/share` on Linux). A `.complete` marker is written last, so an interrupted
download is redone. This is the only network access.

| Variable | Effect |
|---|---|
| `AUTO_POB_POB_DIR` | Use this PoB folder instead of the pinned download (no download). |
| `AUTO_POB_USER_DIR` | Keep `Builds/` and `Settings.xml` here instead of `<data dir>/RustyPathOfBuilding1/userdata/`. |

auto-pob shares `Builds/` and `Settings.xml` with the classic app
(`rusty-path-of-building`), so builds saved by one open in the other.

## Architecture

- **Engine thread** (`src/engine/run.rs`): owns the Lua state and PoB. The UI never runs Lua.
  It sends `Command`s and receives `Event`s carrying immutable `Snapshot`s (build list,
  build, stats, config) that it draws. The engine asks for a repaint when a new one arrives,
  so frames keep drawing during a slow calculation.
- **Boundary rule**: only `src/engine/` touches PoB internals or Lua. `src/ui/` and
  `src/widgets/` use only `engine::views`, `engine::commands` and `Engine`/events.
- **Glue files** (`src/engine/glue/*.lua`) are the code that calls into PoB, one per area:
  `builds.lua`, `stats.lua`, `config.lua`, `tooltip.lua`, `tree.lua` and `power.lua`. If PoB
  renames or restructures something we call, this is where it breaks.
  `src/engine/bootstrap.lua` sets up the headless environment and `src/engine/pob.rs` is the
  backend that runs the glue.
- `src/game.rs` pins the PoB version and game-specific paths; `src/pob_files.rs` downloads
  and installs it.

## Tests

Fast tests need no PoB and run in CI:

```bash
cargo test --workspace
```

The **gate** compares our engine against unmodified upstream PoB (build load, stats, Config,
save round trip, share codes, and so on). It needs a PoB folder and is `#[ignore]`d by default:

```bash
AUTO_POB_POB_DIR=$HOME/.local/share/auto-pob/poe1/pob-2.67.2 \
  cargo test -p auto-pob --test gate -- --ignored --test-threads=1
```

Run it with `CI` unset: upstream's headless wrapper skips ModCache when `CI` is set, which can
change values. CI runs it in the `pob-gate` job, fetching PoB with
`cargo run -p auto-pob --example fetch_pob -- <dir>`.

Committed fixtures (`tests/fixtures/*.xml`) are small synthetic builds. To also test your own
builds, copy their XML into `tests/fixtures/local/` (git-ignored) and regenerate goldens;
the gate picks them up.

### Regenerating goldens

```bash
cargo run -p auto-pob --example golden -- <pob_dir>
```

This writes `<name>.golden.json` (expected stats and config from upstream PoB) for every
fixture. It also rewrites the fixture XML, whose element order is not stable, so revert
`*.xml` changes unless you meant to change the builds.

## Bumping PoB

1. Change the pinned version in `src/game.rs` (`Game::pob_version`). The CI cache key follows it.
2. Fetch the new release (`cargo run -p auto-pob --example fetch_pob -- <dir>`, or let the
   app download it) and run the golden example against it.
3. Review the golden diff: expected upstream changes versus breakage.
4. Fix the glue until the gate is green.
5. Record the bump in the project ledger.
