# auto-pob

An [egui](https://github.com/emilk/egui) front-end for Path of Building (PoE1). It runs PoB's
own Lua calculation engine headlessly on a background thread and draws the UI itself, so the
numbers are PoB's numbers.

## What it does

- **Build list**: browse the `Builds/` folder (folders first), open a build, import a PoB
  share code, start a new build.
- **Build screen**: tabs for the open build, with save, save as, undo/redo and an
  unsaved-changes prompt.
- **Stats sidebar**: PoB's sidebar stats, recalculated after every change. At the top, pick
  the main skill (socket group), the active skill, its part, the minion and minion skill, and
  type the stage and mine counts (sent on Enter or when you leave the box, Esc puts the old
  value back), as in PoB; controls PoB hides for the skill aren't shown. Each Full DPS skill
  gets its own line with PoB's notes underneath.
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
- **Skills tab**: PoB's socket groups and gems.
  - Keep several skill sets, switch between them, and create, rename, copy or delete them.
  - Add, copy, paste and delete socket groups, move them in the list, and right-click one to
    make it the main skill. Edit a group's label, slot, enabled and Full DPS flags and count.
    Groups that come from an item are shown read-only.
  - Edit each gem's name, level, quality, enabled state and count. Clicking a gem name opens
    a picker with PoB's search, filters and gem tooltips; sort by DPS to see each gem's
    effect on your damage, calculated in the background without blocking the UI. Fields
    send when you press Enter or leave them, and Esc reverts.
  - Gem options (sort, default level and quality, support types, legacy gems) are saved with
    the build, and hovering a group shows PoB's tooltip.
- **Items tab**: PoB's items, equipment slots and item sets.
  - The slots are in PoB's order: weapons, armour, rings, amulet and belt, flasks, abyssal
    sockets (shown while the item has them) and the jewel sockets of your tree. Pick an item
    in a slot's dropdown, or drag a row from the item list onto the slot. Flasks have their
    active box. Hover a slot or an entry for PoB's item tooltip, with the comparison with
    what you wear.
  - The item list shows every item of the build and where it is used. Sort it, drag rows to
    reorder them, filter it by item set, and delete items (Delete key or the "..." menu; it
    asks first when the item is in use). Ctrl+C on a row copies the item's text. Double-click
    a row to edit the item.
  - Item sets: switch the active set above the slots, switch between the two weapon sets
    (I / II), and create, rename, copy, reorder or delete sets in "Manage...".
  - Ctrl+V pastes an item from the clipboard into the display panel. "Create custom..." and
    "Edit..." open a text window for the item's rarity and text. The panel shows PoB's
    tooltip and PoB's own controls for the item: version and variants, sockets and links,
    quality, influence, catalyst, range sliders, Foulborn, and clicking a mod line to turn it
    off. "Save" (or "Add to build") puts the item in the build; "Cancel" drops it.
    Ctrl+D turns the stat comparison on tooltips on and off.
  - Ctrl+Z and Ctrl+Y undo and redo item and slot changes on this tab. Changes to the item
    being displayed are not part of the build until you save it, so they aren't counted as
    unsaved changes and can't be undone.
  - Limits:
    - There is no "Paste item" button, because the app can only read the clipboard on
      Ctrl+V. Use Ctrl+V or "Create custom...".
    - Hovering an item that grants a skill doesn't show the skill's gem tooltip (PoB shows it
      only while Shift is held).
    - The third ring slot and the graft slots only appear when the build enables them (a
      build with the extra ring slot, or a 3.27 tree for grafts). They are not yet checked
      against PoB.
    - PoB's first Undo on this tab after you load a build also removes the jewels socketed in
      the tree; equip them again. This is how PoB itself behaves.
- **Crafting, databases and tattoos** (on the Items and Tree tabs): PoB's own windows and
  controls, so the items they make are PoB's items.
  - With no item displayed, "Craft item..." opens PoB's crafter: pick a base and rarity, and
    create it. On a crafted rare the panel shows the prefix and suffix dropdowns with their
    tier sliders, and PoB's modifier sorting; cluster jewels get the skill and node count.
  - The panel's other buttons open PoB's windows: enchant, anoint, corrupt (including the
    implicit and roll range pages), add modifier, add implicit and add crucible modifier.
    Their dropdowns, sliders, lists and tooltips are PoB's, including the stat sorts that
    rank enchants and anoints by their effect on your build. Custom modifiers show in the
    panel and can be removed there.
  - The unique and rare item databases sit beside the item list. Filter by slot, type,
    league, requirements and source, search by name or modifier, and sort by name or by a
    stat. A stat sort runs in the background and the UI stays usable while it counts down.
    Hover a row for its tooltip, double-click to look at the item, and Ctrl+click to add
    it to the build and equip it, as in PoB.
  - Tattoos: right-click an eligible allocated node on the Tree tab to replace it with a
    tattoo, and "Remove all tattoos" clears them. Right-clicking an unallocated mastery also
    opens node replacement, as it does in PoB; an allocated mastery still opens the effect
    picker.
  - Limits:
    - There is no Ctrl+Shift add into a second slot, and no dragging from a database. Equip
      into the second ring through its slot dropdown.
    - Stat sorts inside the crafting and Add modifier windows run synchronously, as in PoB.
      The bench and essence sorts in Add modifier pause the app for 0.6 to 0.9 s.
    - PoB 2.67.2 itself raises an error when you pick "Prefix" or "Suffix" as the Add
      modifier window's source. The app shows PoB's error and leaves the window as PoB leaves
      it, with that source selected and the list empty; pick another source to carry on.
    - "Remove all tattoos" can't be undone.
- **Calcs tab**: PoB's own calculation breakdown, in sections.
  - Pick the skill, part, minion skill and calculation mode at the top, as in PoB, and show
    the minion's numbers instead of the player's.
  - Sections are laid out in as many columns as fit. Collapse the ones you don't need; this is
    saved with the build. Search highlights matching rows.
  - Hover a number to see how it is made up, or click it to pin the breakdown. Tables sort by
    column, and a node in a breakdown links to that node on the Tree tab.
- **Notes tab**: free text saved with the build. Buttons insert PoB's colour codes (normal,
  magic, rare, unique, the elements, strength, dexterity, intelligence) at the cursor or
  around the selection. Preview shows the text with the colours applied. Edits are sent to the
  build half a second after you stop typing.
- **Shortcuts**: Ctrl+S saves. Ctrl+Z undoes and Ctrl+Y (or Ctrl+Shift+Z) redoes, on the
  Tree, Skills, Items, Config and Calcs tabs. Each tab has its own history. They wait while a dialog is open.

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
  `builds.lua`, `stats.lua`, `config.lua`, `tooltip.lua`, `tree.lua`, `power.lua`, `calcs.lua`, `skills.lua`, `gems.lua` and `notes.lua`. If PoB
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
