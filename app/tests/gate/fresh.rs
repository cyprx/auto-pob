//! Gate test for a first start on nothing: no `Settings.xml`, no `Builds`, no `auto-pob.toml`,
//! not even the user folder. The rows, what the UI does with them, the files PoB makes at start
//! and the `Settings.xml` the first quit writes are compared with upstream PoB started the
//! same way. Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::lock;
use super::longtail_harness::pob_farm;
use super::net_harness::{canned, canned_web};
use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::upstream_lua;
use auto_pob::engine::{Backend, SettingsView};
use auto_pob::game::Game;
use auto_pob::ui::options::{UiEffects, ui_effects};
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every path below `dir`, relative to it, sorted
fn files(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(path.strip_prefix(base).unwrap().display().to_string());
            if path.is_dir() {
                walk(base, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The settings PoB keeps as booleans: a nil one is off, as our rows and PoB's Options show it
const BOOLS: [&str; 13] = [
    "edgeSearchHighlight",
    "showFlavourText",
    "showAnimations",
    "showAllItemAffixes",
    "disableScrollControlInteraction",
    "showThousandsSeparators",
    "useCompactValues",
    "showTitlebarName",
    "showWarnings",
    "slotOnlyTooltips",
    "migrateEldritchImplicits",
    "notSupportedModTooltips",
    "invertSliderScrollDirection",
];

/// Upstream PoB's value of each row's setting, as text. `connectionProtocol` and `proxyURL`
/// live on `launch` (Main.lua:740-742); a nil number is the default `SaveSettings` writes for it
/// (Main.lua:755-757), and `connectionProtocol`'s is the dropdown's first entry, 0.
const UPSTREAM_ROWS: &str = r#"
local main = launch.main
local NUMBERS = { connectionProtocol = 0, defaultGemQuality = 0, defaultCharLevel = 1,
	defaultItemAffixQuality = 0.5, dpiScaleOverridePercent = 0 }
local LAUNCH = { connectionProtocol = true, proxyURL = true }
local bools = {}
for _, key in ipairs(BOOLS) do bools[key] = true end
local out = {}
for _, key in ipairs(KEYS) do
	local value
	if LAUNCH[key] then value = launch[key] else value = main[key] end
	if value == nil then value = NUMBERS[key] end
	if bools[key] then
		out[key] = value and "true" or "false"
	else
		out[key] = value == nil and "" or tostring(value)
	end
end
return out
"#;

/// A `Settings.xml` as upstream's own reader parses it: `<Mode>` and its arguments, the
/// accounts, the shared items and `<Misc>`'s attributes. Attribute order comes from `pairs`, so
/// only the parsed form can be compared.
const READ_SETTINGS: &str = r#"
local xml = common.xml.LoadXMLFile(SETTINGS_FILE)
local out = { order = {}, mode = {}, args = {}, accounts = {}, shared = 0, misc = {} }
for _, node in ipairs(xml[1]) do
	if type(node) == "table" then
		table.insert(out.order, node.elem)
		if node.elem == "Mode" then
			out.mode = node.attrib
			for _, arg in ipairs(node) do
				local kind, value = next(arg.attrib)
				table.insert(out.args, kind .. "=" .. value)
			end
		elseif node.elem == "Accounts" then
			out.accounts = node.attrib
			for _, account in ipairs(node) do
				out.accounts["account " .. account.attrib.accountName] = account.attrib.sessionID or ""
			end
		elseif node.elem == "SharedItems" then
			out.shared = #node
		elseif node.elem == "Misc" then
			out.misc = node.attrib
		end
	end
end
return out
"#;

#[derive(Debug, PartialEq, serde::Deserialize)]
struct SettingsFile {
    order: Vec<String>,
    mode: BTreeMap<String, String>,
    args: Vec<String>,
    accounts: BTreeMap<String, String>,
    shared: usize,
    misc: BTreeMap<String, String>,
}

fn read_settings(lua: &mlua::Lua, path: &Path) -> SettingsFile {
    lua.globals()
        .set("SETTINGS_FILE", path.to_str().unwrap())
        .unwrap();
    let value: mlua::Value = lua
        .load(READ_SETTINGS)
        .eval()
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    lua.from_value(value).unwrap()
}

/// A user folder below an empty temp folder; the user folder itself isn't there
fn missing_user(root: &Path, side: &str) -> PathBuf {
    let parent = root.join(side);
    std::fs::create_dir_all(&parent).unwrap();
    parent.join("user")
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn a_fresh_folder_matches_pob() {
    let _guard = lock();
    let root = std::env::temp_dir().join(format!("auto-pob-fresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ours = missing_user(&root, "ours");
    let theirs = missing_user(&root, "theirs");
    let pob = pob_farm("fresh");

    // ---- ours: start on nothing ----
    let mut backend = PobBackend::start_with(Game::Poe1, &pob, &ours, canned(&canned_web()))
        .expect("starting PoB on an empty folder");
    let view: SettingsView = backend.settings_view().unwrap();
    let our_start = files(&ours);
    // PoB opens a new build on a fresh start (Main.lua:83-84), so the UI shows it
    assert!(backend.build_loaded().unwrap(), "PoB opens Unnamed build");

    // ---- upstream: the same start, through its own ChangeUserPath ----
    // HeadlessWrapper's MakeDir does nothing; SimpleGraphic's makes the folder
    let lua = upstream_lua(&pob).expect("upstream PoB");
    lua.globals()
        .set(
            "MakeDir",
            lua.create_function(|_, path: String| Ok(std::fs::create_dir_all(path).is_ok()))
                .unwrap(),
        )
        .unwrap();
    // HeadlessWrapper's GetDPIScaleOverridePercent answers 1; SimpleGraphic's renderer holds 0
    // until a Set (r_main.h `dpiScaleOverridePercent = 0`), as the classic runtime does
    // (src/pob/api/window.rs), so a real fresh PoB starts with 0 ("Use system default")
    let stub: i64 = lua
        .load("return launch.main.dpiScaleOverridePercent")
        .eval()
        .unwrap();
    assert_eq!(stub, 1, "HeadlessWrapper's stub changed: revisit this test");
    lua.globals()
        .set(
            "USER_PATH",
            format!("{}/Path of Building/", theirs.display()),
        )
        .unwrap();
    lua.load(
        r#"
        local main = launch.main
        launch.devMode = false
        main.dpiScaleOverridePercent = 0
        main:ChangeUserPath(USER_PATH)
        "#,
    )
    .exec()
    .unwrap();
    assert_eq!(
        our_start,
        files(&theirs),
        "the files a fresh start makes, against upstream's"
    );
    assert_eq!(our_start, ["Path of Building", "Path of Building/Builds"]);

    // ---- every row holds upstream's fresh value ----
    let keys: Vec<String> = view.rows.iter().map(|r| r.key.clone()).collect();
    lua.globals().set("KEYS", keys.clone()).unwrap();
    lua.globals()
        .set("BOOLS", BOOLS.map(String::from).to_vec())
        .unwrap();
    let want: BTreeMap<String, String> = lua
        .from_value(lua.load(UPSTREAM_ROWS).eval().unwrap())
        .unwrap();
    let mut got: BTreeMap<String, String> = view
        .rows
        .iter()
        .map(|r| (r.key.clone(), r.value.clone()))
        .collect();
    let (our_path, their_path) = (ours.to_str().unwrap(), theirs.to_str().unwrap());
    // the build path is PoB's default, below the fresh folder; upstream's is below its own
    assert_eq!(
        got["buildPath"],
        format!("{our_path}/Path of Building/Builds/")
    );
    let build_path = got["buildPath"].replacen(our_path, their_path, 1);
    got.insert("buildPath".into(), build_path);
    assert_eq!(got, want, "the rows on a fresh folder, against upstream");
    for row in &view.rows {
        assert!(
            !row.overridden,
            "{}: no auto-pob.toml, no override",
            row.key
        );
        assert_eq!(row.shared, row.value, "{}", row.key);
    }
    assert!(view.saved_overrides.is_empty());

    // ---- what the UI does with them: no zoom, PoB's own title switch ----
    assert_eq!(
        ui_effects(&view.rows),
        UiEffects {
            zoom: 1.0,
            show_title: true,
        }
    );

    // ---- the first quit writes what upstream's Shutdown writes ----
    backend.shutdown();
    lua.load("launch.main:Shutdown()").exec().unwrap();
    let prompt: Option<String> = lua.load("return launch.promptMsg").eval().unwrap();
    assert_eq!(prompt, None, "upstream's Shutdown saved without an error");
    assert_eq!(
        files(&ours),
        files(&theirs),
        "the files after the first quit"
    );
    let ours_file = read_settings(&lua, &ours.join("Path of Building/Settings.xml"));
    let theirs_file = read_settings(&lua, &theirs.join("Path of Building/Settings.xml"));
    assert_eq!(ours_file, theirs_file, "the first quit's Settings.xml");
    // the open Unnamed build, as PoB records it
    assert_eq!(ours_file.mode["mode"], "BUILD");
    assert_eq!(ours_file.args, ["boolean=false", "string=Unnamed build"]);
    assert_eq!(ours_file.misc["dpiScaleOverridePercent"], "0");

    drop(backend);
    drop(lua);
    let _ = std::fs::remove_dir_all(&pob);
    let _ = std::fs::remove_dir_all(&root);
}

/// Every function SimpleGraphic gives PoB (`ui_api.cpp`'s registrations, besides the image and
/// search handles' methods), so no PoB path can call a missing one
const SIMPLEGRAPHIC: [&str; 60] = [
    "RenderInit",
    "GetScreenSize",
    "GetScreenScale",
    "SetClearColor",
    "SetDrawLayer",
    "GetDrawLayer",
    "SetViewport",
    "SetDrawColor",
    "GetDrawColor",
    "SetDPIScaleOverridePercent",
    "GetDPIScaleOverridePercent",
    "DrawImage",
    "DrawImageQuad",
    "DrawString",
    "DrawStringWidth",
    "DrawStringCursorIndex",
    "StripEscapes",
    "GetAsyncCount",
    "GetCloudProvider",
    "SetWindowTitle",
    "GetCursorPos",
    "SetCursorPos",
    "ShowCursor",
    "IsKeyDown",
    "Copy",
    "Paste",
    "Deflate",
    "Inflate",
    "GetTime",
    "GetScriptPath",
    "GetRuntimePath",
    "GetUserPath",
    "MakeDir",
    "RemoveDir",
    "SetWorkDir",
    "GetWorkDir",
    "LaunchSubScript",
    "AbortSubScript",
    "IsSubScriptRunning",
    "LoadModule",
    "PLoadModule",
    "PCall",
    "ConPrintf",
    "ConPrintTable",
    "ConExecute",
    "ConClear",
    "print",
    "SpawnProcess",
    "OpenURL",
    "SetProfiling",
    "TakeScreenshot",
    "Restart",
    "Exit",
    "SetForeground",
    "SetCallback",
    "GetCallback",
    "SetMainObject",
    "NewImageHandle",
    "NewFileSearch",
    "GetVirtualScreenSize",
];

#[test]
#[ignore = "needs the pinned PoB install"]
fn the_host_functions_pob_calls_are_there() {
    let _guard = lock();
    let root = std::env::temp_dir().join(format!("auto-pob-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let user = missing_user(&root, "ours");
    let pob = pob_farm("host");
    let backend = PobBackend::start_with(Game::Poe1, &pob, &user, canned(&canned_web()))
        .expect("starting PoB");
    let missing: Vec<&str> = SIMPLEGRAPHIC
        .into_iter()
        .filter(|name| {
            backend
                .lua
                .globals()
                .get::<mlua::Value>(*name)
                .map_or(true, |v| !v.is_function())
        })
        .collect();
    assert!(missing.is_empty(), "missing host functions: {missing:?}");
    // the values PoB reads at start, as SimpleGraphic answers on a 1920x1080 display at 100%
    // with no override
    let (scale, dpi): (f64, i64) = backend
        .lua
        .load("return GetScreenScale(), GetDPIScaleOverridePercent()")
        .eval()
        .unwrap();
    assert_eq!((scale, dpi), (1.0, 0));
    // a tooltip line with a background (a gem mod line's) saves and restores the draw colour
    // (Tooltip.lua:605-609)
    backend
        .lua
        .load(
            r#"
            local tip = new("Tooltip")
            pob.main.main.showFlavourText = true
            tip:AddLine(16, "a line", nil, nil, "Assets/ItemsHeaderUniqueLeft.png")
            tip:Draw(0, 0, 100, 100, { x = 0, y = 0, width = 1920, height = 1080 })
            "#,
        )
        .exec()
        .expect("drawing a tooltip with a line background");
    drop(backend);
    let _ = std::fs::remove_dir_all(&pob);
    let _ = std::fs::remove_dir_all(&root);
}

/// New Loadout with a name of non-ASCII characters only, on the build a fresh start opens:
/// PoB's name box runs under LuaJIT without `main.unicode`, so its filter (EditControl.lua:46)
/// drops every byte, Save stays disabled (Build.lua:319-344) and nothing is made. Ours refuses.
#[test]
#[ignore = "needs the pinned PoB install"]
fn a_non_ascii_loadout_name_makes_nothing_as_in_pob() {
    let _guard = lock();
    let root = std::env::temp_dir().join(format!("auto-pob-loadout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let user = missing_user(&root, "ours");
    let pob = pob_farm("loadout");
    const NAME: &str = "日本語";

    let lua = upstream_lua(&pob).expect("upstream PoB");
    lua.globals().set("NAME", NAME).unwrap();
    let (unicode, buf, enabled, specs): (bool, String, bool, i64) = lua
        .load(
            r#"
            local drop = build.controls.buildLoadouts
            for i, value in ipairs(drop.list) do
                if value == "^7^7New Loadout" then drop:SetSel(i) end
            end
            local popup = main.popups[1]
            local edit = popup.controls.edit
            edit:SelectAll()
            edit:Insert(NAME)
            local enabled = popup.controls.save.enabled and true or false
            main:ClosePopup()
            return main.unicode and true or false, edit.buf, enabled, #build.treeTab.specList
            "#,
        )
        .eval()
        .unwrap();
    assert!(!unicode, "upstream runs without main.unicode");
    // the selection stays: Insert dropped everything before replacing it
    assert_eq!(buf, "New Loadout");
    assert!(!enabled, "PoB's Save stays disabled");
    assert_eq!(specs, 1);

    let mut backend = PobBackend::start_with(Game::Poe1, &pob, &user, canned(&canned_web()))
        .expect("starting PoB");
    let unicode: bool = backend
        .lua
        .load("return pob.main.main.unicode and true or false")
        .eval()
        .unwrap();
    assert!(!unicode, "our PoB runs without main.unicode too");
    let err = backend
        .apply(
            auto_pob::engine::Command::NewLoadout { name: NAME.into() },
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "the loadout needs a name");
    let specs: i64 = backend
        .lua
        .load("return #pob.build.treeTab.specList")
        .eval()
        .unwrap();
    assert_eq!(specs, 1, "nothing was made");
    drop(backend);
    drop(lua);
    let _ = std::fs::remove_dir_all(&pob);
    let _ = std::fs::remove_dir_all(&root);
}
