# auto-pob

An [egui](https://github.com/emilk/egui) front-end for Path of Building (PoE1). It runs PoB's
own Lua calculation engine headlessly on a background thread and draws the UI itself, so the
numbers are PoB's numbers.

## What it does

- **Build list**: PoB's own build list for the `Builds/` folder, with its folder tree.
  - Open a build, import a PoB share code or link, start a new build.
  - Rename, copy, move (to another folder, or Ctrl+X then Ctrl+V) and delete builds and
    folders, and create folders. These press PoB's own list controls, so names PoB refuses are
    refused here with PoB's text. Delete is permanent, as in PoB, and asks first with PoB's
    question. A build that is open, or a folder holding it, can't be renamed, moved or deleted.
  - Sort by name, class, level or last edited (PoB's four modes; the choice is saved in
    `Settings.xml`, shared with the classic app), and search with PoB's filter (a name, or
    `class:witch`). The search isn't saved.
  - Mouse4 and Mouse5 go back and forward between folders; Ctrl+N starts a new build.
- **Build screen**: tabs for the open build, with save, save as, undo/redo and an
  unsaved-changes prompt.
- **Top bar**: the build name, PoB's points display (hover for required level and progress),
  and on every tab:
  - the character level, typed or stepped, with PoB's Auto/Manual button (Auto follows the
    points spent) and its experience tooltip;
  - the bloodline dropdown (PoB's secondary ascendancy), enabled when the tree version offers
    bloodlines;
  - PoB's loadouts dropdown: switch the tree, item, skill and config sets together, make a
    New Loadout, Sync, or open the loadout help.
- **Stats sidebar**: PoB's sidebar stats, recalculated after every change. At the top, pick
  the main skill (socket group), the active skill, its part, the minion and minion skill, and
  type the stage and mine counts (sent on Enter or when you leave the box, Esc puts the old
  value back), as in PoB; controls PoB hides for the skill aren't shown. Each Full DPS skill
  gets its own line with PoB's notes underneath.
- **Config tab**: PoB's configuration options (checkboxes, numbers, lists, custom modifiers),
  with config sets, PoB's tooltips and validation. Changes recalculate the stats.
  - Config sets can be copied and reordered in the manager. Hovering an option shows PoB's
    "what if" tooltip with the change to your stats.
  - "Add Mod" on a custom modifier group opens PoB's Mod Browser and adds the picked modifier
    to the group.
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
  - The tree manager (Ctrl+M) reorders trees by dragging and copies them; Up and Down switch
    to the previous or next tree, P shows or hides the heat map and Ctrl+D turns the stat
    differences in node tooltips on and off. Shift traces a path through the nodes you hover,
    and a click allocates that path.
  - "Find Timeless Jewel" opens PoB's timeless jewel search (see **Timeless search** below).
- **Skills tab**: PoB's socket groups and gems.
  - Keep several skill sets, switch between them, create, rename, copy, delete or reorder
    them (drag in the manager).
  - Add, copy, paste and delete socket groups, move them in the list, and right-click one to
    make it the main skill. Edit a group's label, slot, enabled and Full DPS flags and count.
    Groups that come from an item are shown read-only.
  - Edit each gem's name, level, quality, enabled state and count. Clicking a gem name opens
    a picker with PoB's search, filters and gem tooltips; sort by DPS to see each gem's
    effect on your damage, calculated in the background without blocking the UI. Fields
    send when you press Enter or leave them, and Esc reverts.
  - Gem options (sort, default level and quality, support types, legacy gems) are saved with
    the build, and hovering a group shows PoB's tooltip.
  - Hovering a gem's quality, enable box or count shows PoB's "what if" tooltip: the change to
    your stats from 20 quality, or from turning the gem off or on. Ctrl+click on a group turns
    it on or off, Ctrl+right-click toggles its Full DPS.
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
  - The third ring slot and the graft slots appear when the build enables them (a build
    with the extra ring slot, or a 3.27 tree for grafts), as PoB shows them.
  - Shared items: PoB's shared item list and shared item sets, kept in `Settings.xml` across
    builds. Drag a build item or a database row onto the list to share it, drag a shared item
    onto a slot or the item list to copy it into the build, and drag a shared set onto the set
    list. Adding, deleting, renaming and reordering shared entries saves `Settings.xml` at
    once, as PoB does. Hold Shift over an item that grants a skill to see the skill's gem
    tooltip.
  - "Buy similar…" on the display panel, or in a slot's right-click menu, opens PoB's Buy
    Similar window (see **Trade links** below). E over a slot edits its item.
  - Limits:
    - There is no "Paste item" button, because the app can only read the clipboard on
      Ctrl+V. Use Ctrl+V or "Create custom...".
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
    it to the build and equip it, as in PoB. Ctrl+Shift+click equips it into the second slot
    it fits (the second ring, say), and a row can be dragged onto a slot or the item list.
  - Tattoos: right-click an eligible allocated node on the Tree tab to replace it with a
    tattoo, and "Remove all tattoos" clears them. Right-clicking an unallocated mastery also
    opens node replacement, as it does in PoB; an allocated mastery still opens the effect
    picker.
  - Limits:
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
  - Columns take PoB's widths, and a section can pop out as PoB's overlay. For spectre skills
    "Manage Spectres..." opens PoB's Spectre Library (as in the sidebar).
- **Notes tab**: free text saved with the build. Buttons insert PoB's colour codes (normal,
  magic, rare, unique, the elements, strength, dexterity, intelligence) at the cursor or
  around the selection. Preview shows the text with the colours applied. Edits are sent to the
  build half a second after you stop typing.
- **Network** (Import/Export tab and the build list): PoB's own import and sharing code, run in
  the background so the app stays usable while a request is out.
  - Import a build from a link: Maxroll, pob.codes, pobb.in, poe.ninja, poedb, Pastebin and Rentry. Paste the link into the import box; PoB says whether
    it recognises it. Import it as a new build, or into the open build when that build has a
    file. The build list takes a link too.
  - Share the open build to Maxroll, pob.codes, pobb.in, poe.ninja or poedb (the sites that
    accept uploads) and copy the link it gives you.
  - Resolve a PoEURL tree link when importing a tree, and shorten a tree link with PoEURL.
  - Import from your account name (public profiles): list the characters, pick one, and import
    its passive tree and items.
  - Sign in with Path of Exile (the same login as PoB) to list your characters and import them,
    with the same buttons and rate limiting as PoB. Sign-in tokens live in PoB's `Settings.xml`,
    so the classic app and auto-pob share them: signing out in either one signs out both.
  - A `pob://` link can be given on the command line. A pending request shows a line with a Cancel button; cancelling shows PoB's own failure text.
  - Browser: when the app needs your browser (the sign-in page), it opens it from the app. If
    that doesn't work, copy the link shown and open it yourself.
  - Nothing is sent unless you ask. Opening a build, switching tabs and typing in the import
    box send nothing; a request starts only when you press a button (or pass a link).
  - The release notice is opt-in: turn on "Look for new auto-pob releases" in Options (it is
    `update_check = true` in `auto-pob.toml`), or start the app with `AUTO_POB_UPDATE_CHECK=1`.
    It looks at this repository's latest release once and tells you if there is a newer one. It
    never updates anything. The variable only forces the notice on; the setting decides
    otherwise, and the default is off.
  - Limits:
    - A request that gets "unauthorised" refreshes your sign-in token at most twice. Past that
      it stops with an error and does not retry, where PoB itself would keep trying.
    - Cancelling while a token refresh is in progress can leave you signed out (the token is
      shared with the classic app); sign in again.
    - PoEURL may be offline. If it is, the PoEURL features show PoB's failure text.
    - The sign-in redirect listens on local ports 49082 to 49084, so it can't run while
      another PoB or auto-pob holds all of them.
    - Trade price queries and PoB's Archives are not included, and PoB's own self-update stays
      off. Trade links (Buy similar, timeless search) are built offline and opened in your
      browser.
- **Spectre Library**: for a Raise Spectre build, "Manage Spectres..." in the sidebar (or the
  Calcs selector) opens PoB's Spectre Library. Search, add, remove and reorder spectres;
  nothing changes until Save.
- **Compare tab**: PoB's comparison of the open build against one or more others.
  - Add a comparison from a saved build (the build picker), a share code, or a link (also
    "Import as comparison" on the Import tab). Pick between several with "Compare with:" and
    remove them with Remove. Comparisons are dropped when you open another build.
  - Summary: PoB's side-by-side stats with the differences coloured.
  - Tree: this build's tree coloured against the comparison's, PoB's tree and version
    selectors for each side, "Copy tree" and "Copy and use", and converting either tree to
    another version.
  - Items: the slots side by side with PoB's match/differ labels and tooltips for both items;
    Copy adds their item to your build, Equip also equips it, Buy opens Buy similar for it.
  - Skills and Config: PoB's socket group pairs, and the config options that differ, with
    "Copy Config from Compare Build". Item, set and skill selectors are PoB's.
  - Item and config copies can be undone on the Items and Config tabs.
  - Limits (R9): PoB's Calcs sub-view and power report on the Compare tab are not included;
    Calcs differences show through Summary.
- **Trade links** (Buy similar): PoB's Buy Similar window, from the display panel, a slot's
  menu or the Compare tab's Items view. Tick the mods and ranges to search, and "Open URL"
  opens the trade site search in your browser.
  - Limits (R10): the realm and league are auto-pob's (`[trade]` in `auto-pob.toml`, or
    Options) instead of PoB's league fetch, which needs a sign-in; type a fresh league name
    into Options. There is no search for an empty slot, as in PoB.
- **Timeless search**: the Tree tab's "Find Timeless Jewel" is PoB's window: pick the jewel,
  conqueror, socket and devotion, list the nodes you want with weights, and Search. The
  search runs in the background with a progress line and can be cancelled; the old results
  stay until a new search finishes. Hover a result for its nodes, double-click to add that
  jewel to the build, and "Open Trade URL" (a range with Shift+click) opens the trade search.
  - Limits (R17): PoB's search reads its lookup tables and caches them as `.bin` files next to
    its data, in the PoB folder (`Data/TimelessJewelData`), as PoB itself does: 3.4 to 3.6 MB
    for each of Lethal Pride, Brutal Restraint, Militant Faith and Elegant Hubris, 52 MB for
    Glorious Vanity, about 66 MB if all five are used; the Abyss jewels write none. Nodes are
    picked by PoB's dropdown and search list, not on the tree canvas.
- **Options**: the build list's and the build screen's Options button opens PoB's own Options
  window (Save writes PoB's `Settings.xml`). Under it, auto-pob's own settings: the trade
  realm and league, the release notice, and per-setting overrides, kept in `auto-pob.toml`
  so the classic app keeps its own value (see below).
  - Limits (R13): PoB's beta channel, public builds and dev autosave options are left out.
    "Invert slider scroll direction", "Disable control scroll interaction" and "Show
    Animations" have no effect here, and say so.
- **Help**: F1 (or About) opens PoB's help, its sections and the changelog, at the section for
  the tab you're on. F1 over a node, item, database row or gem opens its wiki page instead.
  "Download PoB's help files" fetches PoB's `help.txt` and `changelog.txt` when the PoB
  folder lacks them; only that button uses the network.
- **Shortcuts**:
  - Ctrl+S saves. Ctrl+Z undoes and Ctrl+Y (or Ctrl+Shift+Z) redoes, on the Tree, Skills,
    Items, Config and Calcs tabs; each tab has its own history.
  - Ctrl+1 to Ctrl+6 switch tabs (Tree, Skills, Items, Calcs, Config, Notes), Ctrl+I opens
    Import/Export, Ctrl+W closes the build, Ctrl+F focuses the tab's search.
  - Tree: Ctrl+M, Up/Down, P, Ctrl+D, Ctrl+C on a hovered node (its text), PageUp and PageDown
    zoom. Items: E on a slot. F2 renames the selected set in the set managers; arrow keys,
    Home, End and Delete work in the lists, as in PoB.
  - Mouse4 goes back (to the list, or up a folder). F1 opens Help, or the wiki page of the
    hovered node, item, database row or gem.
  - While a dialog is open no shortcut fires, except F1 over a row of the gem list's window.
  - While a text box has the keyboard, the keys PoB reads before its controls still fire, as
    in PoB: Ctrl+S, Ctrl+1 to Ctrl+6, Ctrl+I, Ctrl+W, Mouse4 and Ctrl+F, and Ctrl+M and Up/Down
    on the Tree tab, E on the Items tab. The others wait: Ctrl+Z and Ctrl+Y (the box keeps its
    own undo), the tree view's P, Ctrl+D, Ctrl+C, PageUp and PageDown, the set lists' F2, the
    build list's Ctrl+N and F1's Help (a hovered row's, slot's or gem's wiki page still opens).
    A box of the top bar or the sidebar keeps every key but Ctrl+S, Ctrl+1 to Ctrl+6, Ctrl+I,
    Ctrl+W and Mouse4, and so does a dropped top-bar or sidebar list. Ctrl+W never edits the
    box it is pressed in.
  - Dropdowns and sliders step with the wheel and the arrow keys while pointed at, through a
    dropped list's search matches only. Under a dialog, the ones behind it don't step.
  - Esc closes a dialog or window, as PoB's popups do: the top one first, and only once no
    control takes the key. A text box with the keyboard takes the first Esc, which only
    leaves the box. A dropped list takes it to empty its search or close. A click outside a
    dialog never closes it.
- **Nothing written behind your back**: views, tooltips, Compare, trade links, the Spectre
  Library before Save and Help write nothing in your folders. Two things write into the PoB
  folder auto-pob downloaded, as PoB does: the timeless jewel search caches PoB's lookup tables
  there (`Data/TimelessJewelData/*.bin`), and Help's "Download PoB's help files" button, only
  when you click it, adds `help.txt` and `changelog.txt`. `Settings.xml` is written only by
  Options' Save, the build list's sort, the shared item edits, signing in or out, PoB refreshing
  an expired sign-in token, and quitting, where auto-pob saves what PoB saves at exit (the
  import history, the last account, the open build). A write you ask for is refused with a
  notice, and quitting leaves the file alone, when another program (classic PoB) changed
  `Settings.xml` since auto-pob last read or wrote it, or PoB couldn't read it at start.

## Running

```bash
cargo run -p auto-pob --release
```

On first run it downloads PoB 2.67.2 (about 600 MB) into
`<data dir>/auto-pob/poe1/pob-2.67.2`, where `<data dir>` is the platform data folder
(`~/.local/share` on Linux). A `.complete` marker is written last, so an interrupted
download is redone. Apart from that, the app only touches the network when you ask it to (see
the Network section).

| Variable | Effect |
|---|---|
| `AUTO_POB_POB_DIR` | Use this PoB folder instead of the pinned download (no download). |
| `AUTO_POB_CONFIG` | Read and write auto-pob's settings file here instead of `<config dir>/auto-pob/auto-pob.toml`. |
| `AUTO_POB_UPDATE_CHECK` | Set to `1` to look for a newer auto-pob release when the app starts, whatever `update_check` says. Off by default. |
| `AUTO_POB_USER_DIR` | Keep `Builds/` and `Settings.xml` here instead of `<data dir>/RustyPathOfBuilding1/userdata/`. Wins over `user_dir` in `auto-pob.toml`. |

### auto-pob.toml

auto-pob's own settings live in `auto-pob.toml` (PoB's stay in `Settings.xml`). Options writes it;
you can also edit it by hand, and comments and keys it doesn't know are kept.

```toml
user_dir = "/home/me/pob"   # the folder for Builds/ and Settings.xml; the default if absent
update_check = false        # the opt-in release notice

[trade]
realm = "PC"                # PC, PS4 or Xbox
league = "Standard"

[overrides]                 # Settings.xml keys auto-pob keeps its own value for
colorPositive = "^x33FF77"
```

Precedence for the user folder: `AUTO_POB_USER_DIR`, then `user_dir`, then the default. For the
release notice: `AUTO_POB_UPDATE_CHECK=1` forces it on, otherwise `update_check`.

On the first start with no `auto-pob.toml`, no `AUTO_POB_USER_DIR` and an installed classic app
with its user folder, auto-pob asks whether to use that folder or another one, and writes the
answer. Otherwise it uses the default and writes nothing until you save Options. The default is
the classic app's folder, so a later start doesn't ask about the folder auto-pob made there. A
file that cannot be parsed is renamed to `auto-pob.toml.bad` and the app says so.

auto-pob shares `Builds/` and `Settings.xml` with the classic app
(`rusty-path-of-building`), so builds saved by one open in the other.

## Architecture

- **Engine thread** (`src/engine/run.rs`): owns the Lua state and PoB. The UI never runs Lua.
  It sends `Command`s and receives `Event`s carrying immutable `Snapshot`s (build list,
  build, stats, config) that it draws. The engine asks for a repaint when a new one arrives,
  so frames keep drawing during a slow calculation.
- **Boundary rule**: only `src/engine/` touches PoB internals or Lua. `src/ui/` and
  `src/widgets/` use only `engine::views`, `engine::commands` and `Engine`/events.
- **Glue files** (`src/engine/glue/*.lua`) are the code that calls into PoB, one per area
  (builds, build list, stats, config, tree, power, calcs, skills, gems, items, item databases,
  crafting, notes, import and network, compare, timeless, settings, shared items, PoB's popup
  windows). If PoB renames or restructures something we call, this is where it breaks.
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
  cargo test -p auto-pob --features gate-probes --test gate -- --ignored --test-threads=1
```

`scripts/gate.sh [-j N] [-f FILTER]` runs each gate test in its own process, N at a time,
then the tests listed in `tests/gate-serial.txt` one by one, and writes the times to
`target/gate-times.tsv`. `scripts/ci-local.sh` runs the same checks as CI.

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
