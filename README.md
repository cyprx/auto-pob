# auto-pob

auto-pob is a native [egui](https://github.com/emilk/egui) front-end for
[Path of Building](https://github.com/PathOfBuildingCommunity/PathOfBuilding) (PoE1). It runs
PoB's own Lua calculation engine headlessly and draws the interface itself, so the numbers are
PoB's numbers. The feature list is in [app/README.md](app/README.md).

## Install from a release

Linux x86_64 only. Download `auto-pob-<version>-linux-x86_64.tar.gz` (and its `.sha256`) from the
[releases page](../../releases), check it, and run the installer:

```bash
sha256sum -c auto-pob-<version>-linux-x86_64.tar.gz.sha256
tar xzf auto-pob-<version>-linux-x86_64.tar.gz
cd auto-pob-<version>-linux-x86_64
./install.sh                    # installs into ~/.local
```

`install.sh [--prefix DIR] [--register-scheme] [--uninstall]` prints every path it writes and
touches nothing else. `--uninstall` removes exactly those files. It does not undo a `--register-scheme`
association (xdg-mime cannot unset one); it prints a note if `pob://` still points at auto-pob. Reset it
with `xdg-mime default <other>.desktop x-scheme-handler/pob` or by deleting the
`x-scheme-handler/pob` line from `~/.config/mimeapps.list`.

**`--register-scheme` is opt-in.** Without it nothing claims `pob://` links. With it the
installer adds `MimeType=x-scheme-handler/pob;` to the installed desktop file, runs
`xdg-mime default auto-pob.desktop x-scheme-handler/pob` and runs `update-desktop-database`, so
`pob://` links open in auto-pob. That replaces whichever program handles them now.

The binary links libcurl dynamically and needs glibc 2.35 or newer (the release is built on
Ubuntu 22.04).

## Build from source

```bash
cargo build --release          # needs LuaJIT and libcurl development files
cargo build --release --features vendored-lua   # builds LuaJIT from source instead
```

`cargo run` and `cargo build` build auto-pob (the workspace default member). `libcurl` is needed
by the bundled Lua modules.

## Running

```bash
auto-pob
```

On the first run it downloads the pinned Path of Building release (about 600 MB) into its data
folder. If it finds a classic Rusty Path of Building folder and no `auto-pob.toml`, it asks
whether to keep your builds and `Settings.xml` there or in another folder; otherwise it uses the
default. Its own settings live in
`auto-pob.toml` (PoB's stay in `Settings.xml`). Environment variables: `AUTO_POB_POB_DIR` (use
an existing PoB folder), `AUTO_POB_USER_DIR` (builds and settings folder), `AUTO_POB_CONFIG`
(settings file) and `AUTO_POB_UPDATE_CHECK=1` (release check). The full list and the
`auto-pob.toml` options are in [app/README.md](app/README.md).

## Privacy

Apart from the first-run PoB download, auto-pob sends no request unless you ask for one with a
button (an import, a sign-in, a link you pass). The release notice is off until you turn it on.
There is no telemetry.

## Tests

```bash
scripts/ci-local.sh            # everything CI does: ci.sh, the PoB gate, a test package
scripts/ci.sh                  # fmt, clippy, cargo test --workspace, installer test, shellcheck
AUTO_POB_POB_DIR=<pinned PoB> scripts/gate.sh -j2   # the comparison gate against real PoB
scripts/test-install.sh        # install.sh in a temp HOME with stubbed xdg-mime
```

`scripts/package.sh [--version V] [--out DIR]` builds the release tarball. `shellcheck` and
`actionlint` run when installed.

## Forks: enable GitHub Actions

Workflows do not run on a fork until you enable Actions in its Actions tab. A `vX.Y.Z` tag
(equal to `app/Cargo.toml`'s version) runs the gate, packages and publishes a release. A
`vX.Y.Z-test*` tag or a manual run packages and uploads a workflow artifact without releasing.

## Classic app

The original Rusty Path of Building runtime (the Lua UI through SimpleGraphic) stays in this
repository because auto-pob shares its `pob-runtime` crate. Build it with
`cargo run -p rusty-path-of-building`. It is no longer released from this fork; for packages use
the upstream project, [meehl/rusty-path-of-building](https://github.com/meehl/rusty-path-of-building)
(Flathub and Homebrew links below). Its history is in `CHANGELOG.md`.

Rusty Path of Building is a cross-platform runtime environment for [Path of Building](https://github.com/PathOfBuildingCommunity/PathOfBuilding) and [Path of Building 2](https://github.com/PathOfBuildingCommunity/PathOfBuilding-PoE2). It serves as a replacement for [SimpleGraphic](https://github.com/PathOfBuildingCommunity/PathOfBuilding-SimpleGraphic), the official runtime environment for PoB.

The main focus of this project is native Linux support for Path of Building. It also runs on Windows and macOS, though most of my testing and development is focused on Linux, with support for other platforms largely based on user reports. Contributions that improve support or fix bugs on other platforms are welcome.

### Usage

```bash
rusty-path-of-building [poe1|poe2]
```

### Installation

[![Packaging status](https://repology.org/badge/vertical-allrepos/rusty-path-of-building.svg)](https://repology.org/project/rusty-path-of-building/versions)

#### Flathub

[![Stable version badge](https://img.shields.io/flathub/v/community.pathofbuilding.PathOfBuilding)](https://flathub.org/en/apps/community.pathofbuilding.PathOfBuilding)

#### Homebrew

[Homebrew Tap](https://github.com/meehl/homebrew-rusty-path-of-building) (NOTE: This has been tested on macOS only)

#### Building from source

```bash
cargo build --release
```

##### Build Dependencies

- `LuaJIT` - Required by the `mlua` crate. Alternatively, enable the `vendored-lua` feature to build LuaJIT from source instead of using a system installation.
- `libcurl` - Required when the `bundled-lua-modules` feature (enabled by default) is used.

### Runtime Dependencies

Path of Building's Lua code requires the following C libraries:

- [Lua-cURLv3](https://github.com/Lua-cURL/Lua-cURLv3)
- [luautf8](https://github.com/starwing/luautf8)
- [luasocket](https://github.com/lunarmodules/luasocket)
- [lzip](https://github.com/PathOfBuildingCommunity/PathOfBuilding-SimpleGraphic/tree/master/libs/LZip)

By default, the `bundled-lua-modules` feature is enabled and registers Rust implementations of these libraries with the Lua runtime. The original C libraries don't need to be installed if this feature is enabled.

To use the original C libraries instead, disable the feature and make sure Lua can locate them. Please refer to the [Lua documentation](https://www.lua.org/manual/5.1/manual.html#pdf-package.cpath) for details on that.

### Known Issues

- If automatic updates fail, navigate to `~/.local/share/RustyPathOfBuilding{1,2}/` and delete the `rpob.version` file and the `Update` directory. This will force a complete re-download of PoB's latest assets and Lua code on the next startup.
- Automatic updates for weekly beta builds are supported, but compatibility is not checked. If you run into problems with a beta version, please don't open an issue for it.
