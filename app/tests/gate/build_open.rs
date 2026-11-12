//! Opening builds PoB can't open as they are, against upstream PoB opening the same files: a
//! build made for another game version (PoB's "Game Version" window, Build.lua:102-105 and
//! 1323-1350) and one with a passive tree version PoB doesn't know (TreeTab.lua:507-509). A
//! failed open, by file, code or link, leaves the engine usable (tests I-2).
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::longtail_harness::{LongtailSession, settings_folder};
use super::net_harness::{canned, canned_web, net_session};
use super::{Session, fixture_xml, golden, lock, pob_dir, share_code};
use auto_pob::engine::upstream::upstream_lua;
use auto_pob::engine::{Backend, Command, DbSnapshot, ImportMode, ItemDb, ItemDbQuery, PopupKind};
use mlua::LuaSerdeExt;
use pob_runtime::net::Canned;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// The state both sides are read through: `M` is PoB's `main`, `B` its build mode
const STATE: &str = r#"
    local out = { mode = M.mode, popups = #M.popups }
    if M.mode == "BUILD" and B.spec then
        local alloc = {}
        for id in pairs(B.spec.allocNodes) do alloc[#alloc + 1] = id end
        table.sort(alloc)
        out.alloc = alloc
        out.class = B.spec.curClassId
        out.level = B.characterLevel
        out.target = B.targetVersion
        out.name = B.buildName
        out.file = B.dbFileName or false
    end
    return out
"#;

/// The newest popup's title, its labels by control key, and its numbered lines
const POPUP: &str = r#"
    local top = M.popups[1]
    if not top then return nil end
    local labels, lines = {}, {}
    for key, control in pairs(top.controls) do
        if type(control.label) == "string" then labels[tostring(key)] = control.label end
    end
    for i, control in ipairs(top.controls) do lines[i] = control.label end
    return { title = top.title, labels = labels, lines = lines }
"#;

/// Upstream PoB opening a build as `main:SetMode` does: the file at `path`, or `xml` as an
/// import when there's no path. Returns the window the open showed (if any) and the state after
/// the open, then after pressing `press` on that window and running a frame.
fn upstream_open(
    path: Option<&Path>,
    name: &str,
    xml: &str,
    press: Option<&str>,
) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
    let lua = upstream_lua(&pob_dir()).expect("upstream PoB");
    let g = lua.globals();
    g.set("PATH", path.map(|p| p.display().to_string()))
        .unwrap();
    g.set("NAME", name).unwrap();
    g.set("XML", xml).unwrap();
    g.set("PRESS", press).unwrap();
    let value: mlua::Value = lua
        .load(format!(
            r#"
            M, B = launch.main, build
            local function state() {STATE} end
            local function popup() {POPUP} end
            assert(#M.popups == 0, "upstream started with a popup")
            if PATH then M:SetMode("BUILD", PATH, NAME) else M:SetMode("BUILD", false, NAME, XML) end
            -- PoB's frame draws a build that never made its tabs; whatever it raises, the
            -- window is what PoB shows
            pcall(runCallback, "OnFrame")
            local shown, opened = popup(), state()
            if PRESS and M.popups[1] then
                M.popups[1].controls[PRESS].onClick()
                runCallback("OnFrame")
            end
            return {{ shown, opened, state() }}
            "#
        ))
        .eval()
        .unwrap_or_else(|e| panic!("upstream open: {e}"));
    let out: (serde_json::Value, serde_json::Value, serde_json::Value) =
        lua.from_value(value).unwrap();
    out
}

fn our_state(lua: &mlua::Lua) -> serde_json::Value {
    let value: mlua::Value = lua
        .load(format!("local M, B = pob.main.main, pob.build\n{STATE}"))
        .eval()
        .unwrap();
    lua.from_value(value).unwrap()
}

/// attack.xml as a build saved for PoB's legacy game version (GameVersions.lua:3)
fn old_xml() -> String {
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let old = xml.replacen("targetVersion=\"3_0\"", "targetVersion=\"2_6\"", 1);
    assert_ne!(old, xml, "no targetVersion to change");
    old
}

/// attack.xml with a passive tree version PoB doesn't know
fn unknown_tree_xml() -> String {
    let xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let bad = xml.replacen("treeVersion=\"3_29\"", "treeVersion=\"9_99\"", 1);
    assert_ne!(bad, xml, "no treeVersion to change");
    bad
}

/// Our Game Version window equals upstream's: title, every label, the buttons
fn assert_window_matches(s: &mut Session, theirs: &serde_json::Value, what: &str) {
    let view = s
        .backend
        .popup_view()
        .unwrap()
        .unwrap_or_else(|| panic!("{what}: no window"));
    assert_eq!(view.kind, PopupKind::GameVersion, "{what}");
    assert_eq!(view.title, theirs["title"], "{what}");
    let ours: serde_json::Map<String, serde_json::Value> = view
        .controls
        .iter()
        .map(|c| (c.name.clone(), serde_json::Value::from(c.label.clone())))
        .collect();
    assert_eq!(serde_json::Value::Object(ours), theirs["labels"], "{what}");
    assert_eq!(view.enter.as_deref(), Some("convert"), "{what}");
}

/// Our state after an open that showed the window: a new build, as `blank` (the state of the
/// blank build the engine starts with), not the old one renamed
fn assert_new_build_behind(s: &mut Session, blank: &serde_json::Value, what: &str) {
    let build = s.backend.build_view().unwrap();
    assert_eq!(build.name, "Unnamed build", "{what}");
    assert_eq!(build.path, None, "{what}");
    assert_eq!(&our_state(&s.backend.lua), blank, "{what}");
}

/// What upstream's state after Convert shares with ours (its build list folder differs)
fn converted(state: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "alloc": state["alloc"], "class": state["class"], "level": state["level"],
        "target": state["target"], "name": state["name"], "popups": state["popups"],
        "mode": state["mode"],
    })
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn game_version_matches_pob() {
    let _guard = lock();
    let mut s = Session::start("game-version");
    let blank = our_state(&s.backend.lua);
    assert_eq!(blank["name"], "Unnamed build");
    s.open_fixture("attack", &fixture_xml("attack"));
    let old = s.builds_dir().join("old.xml");
    let xml = old_xml();
    fs::write(&old, &xml).unwrap();
    let (shown, opened, theirs) = upstream_open(Some(&old), "old", &xml, Some("convert"));
    assert_eq!(shown["title"], "Game Version");
    assert_eq!(
        opened["target"],
        serde_json::Value::Null,
        "upstream waits for the window"
    );

    // the open shows PoB's window, with a new build behind it; nothing reaches the file
    s.run(Command::OpenBuild { path: old.clone() })
        .expect("an old build opens to the window");
    assert_window_matches(&mut s, &shown, "file");
    assert_new_build_behind(&mut s, &blank, "file");
    let err = s.run(Command::Save).unwrap_err();
    assert_eq!(err.to_string(), "build has no file yet");
    assert_eq!(
        fs::read_to_string(&old).unwrap(),
        xml,
        "old.xml was written"
    );

    // Convert opens it converted, as upstream's does
    let affected = s
        .run(Command::PressPopupButton {
            name: "convert".into(),
        })
        .expect("convert");
    assert!(affected.build && affected.popup && affected.tree_layout);
    assert!(s.backend.popup_view().unwrap().is_none());
    assert_eq!(converted(&our_state(&s.backend.lua)), converted(&theirs));
    assert_eq!(theirs["target"], "3_0");
    assert_eq!(theirs["file"], old.display().to_string());
    let build = s.backend.build_view().unwrap();
    assert_eq!(build.name, "old");
    assert_eq!(build.path.as_deref(), Some(old.as_path()));
    assert_eq!(
        fs::read_to_string(&old).unwrap(),
        xml,
        "Convert wrote old.xml"
    );

    // Cancel, Escape and any other command leave the new build behind the window
    for end in [
        Command::PressPopupButton {
            name: "cancel".into(),
        },
        Command::ClosePopup,
        Command::NewBuild,
    ] {
        let what = format!("{end:?}");
        s.run(Command::OpenBuild { path: old.clone() }).unwrap();
        assert_window_matches(&mut s, &shown, &what);
        s.run(end).unwrap();
        assert!(s.backend.popup_view().unwrap().is_none(), "{what}");
        assert_new_build_behind(&mut s, &blank, &what);
    }
    // upstream's Cancel goes back to the build list (Build.lua:1344-1347)
    let (_, _, cancelled) = upstream_open(Some(&old), "old", &xml, Some("cancel"));
    assert_eq!(cancelled["mode"], "LIST");
    assert_eq!(cancelled["popups"], 0);

    // a code import of an old build: Convert has no text to reopen, so it makes an empty
    // build, as upstream's does (Build.lua:1342 passes no XML)
    let code = share_code(&s, &xml);
    s.run(Command::ImportShareCode { code }).unwrap();
    assert_window_matches(&mut s, &shown, "code");
    assert_new_build_behind(&mut s, &blank, "code");
    s.run(Command::PressPopupButton {
        name: "convert".into(),
    })
    .unwrap();
    let (_, _, theirs) = upstream_open(None, "Imported build", &xml, Some("convert"));
    assert_eq!(converted(&our_state(&s.backend.lua)), converted(&theirs));
    assert_eq!(theirs["name"], "Imported build");
    assert_eq!(
        theirs["alloc"], blank["alloc"],
        "upstream's is an empty build"
    );

    // the Import tab's own press (ImportTab.lua:665) and a link's callback run Init outside
    // an open: the same window, over the same new build
    let code = share_code(&s, &xml);
    s.run(Command::ImportFromUrl {
        url: code.clone(),
        mode: ImportMode::NewBuild,
    })
    .unwrap();
    assert_window_matches(&mut s, &shown, "import tab code");
    assert_new_build_behind(&mut s, &blank, "import tab code");
    s.run(Command::ClosePopup).unwrap();
    let web = canned_web();
    web.route("GET", "https://pobb.in/pob/gate", Canned::ok(code));
    let mut n = net_session("game-version-link", canned(&web), None);
    n.backend
        .apply(
            Command::ImportFromUrl {
                url: "https://pobb.in/gate".into(),
                mode: ImportMode::NewBuild,
            },
            &|_| {},
        )
        .expect("the request starts");
    let steps = n.settle_net(Duration::from_secs(15));
    assert!(steps.iter().all(Result::is_ok), "a step failed");
    let entry = n.requests().finished.remove(0);
    assert_eq!(entry.error, None, "link");
    let view = n.backend.popup_view().unwrap().expect("link: the window");
    assert_eq!(view.kind, PopupKind::GameVersion);
    assert_eq!(view.title, shown["title"]);
    assert_eq!(n.backend.build_view().unwrap().name, "Unnamed build");
    let popups: usize = n
        .backend
        .lua
        .load("return #pob.main.main.popups")
        .eval()
        .unwrap();
    assert_eq!(popups, 0, "link");

    // the engine goes on: the good build opens as before
    s.open_fixture("attack", &fixture_xml("attack"));
    super::assert_view_matches("attack", "stat", &s.stats().stats, &golden("attack").stats);
}

/// The window on the very first open, with no build behind it yet: PoB starts on its list
#[test]
#[ignore = "needs the pinned PoB install"]
fn game_version_on_the_first_open() {
    let _guard = lock();
    let mut s = LongtailSession::start("game-version-first", &|_| {
        "<PathOfBuilding>\n\t<Mode mode=\"LIST\"><Arg boolean=\"false\"/><Arg string=\"\"/></Mode>\n</PathOfBuilding>\n".into()
    });
    assert!(
        !s.backend.build_loaded().unwrap(),
        "PoB started on its list"
    );
    let old = settings_folder(&s.user).join("Builds/old.xml");
    let xml = old_xml();
    fs::write(&old, &xml).unwrap();
    let (shown, _, theirs) = upstream_open(Some(&old), "old", &xml, Some("convert"));
    s.run(Command::OpenBuild { path: old.clone() })
        .expect("an old build opens to the window");
    let view = s.popup().expect("the window");
    assert_eq!(view.kind, PopupKind::GameVersion);
    assert_eq!(view.title, shown["title"]);
    assert_eq!(s.backend.build_view().unwrap().name, "Unnamed build");
    assert_eq!(s.open_popups(), 0);
    s.run(Command::PressPopupButton {
        name: "convert".into(),
    })
    .expect("convert");
    assert_eq!(converted(&our_state(&s.backend.lua)), converted(&theirs));
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn unknown_tree_version_matches_pob() {
    let _guard = lock();
    let mut s = Session::start("unknown-tree");
    s.open_fixture("attack", &fixture_xml("attack"));
    let bad = s.builds_dir().join("bad.xml");
    let xml = unknown_tree_xml();
    fs::write(&bad, &xml).unwrap();
    let (shown, opened, _) = upstream_open(Some(&bad), "bad", &xml, None);
    assert_eq!(shown["title"], "Unknown Passive Tree Version");
    assert_eq!(opened["mode"], "LIST", "upstream gives up on the build");
    let lines: Vec<&str> = shown["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    let want = format!("Unknown Passive Tree Version: {}", lines.join(" "));
    assert!(
        want.contains("unrecognised version of the passive skill tree"),
        "{want}"
    );

    // by file, by code and by link: PoB's message, no popup left, and the engine goes on
    let web = canned_web();
    let mut n = net_session("unknown-tree-link", canned(&web), None);
    let code = share_code(&s, &xml);
    web.route("GET", "https://pobb.in/pob/gate", Canned::ok(code.clone()));
    let attempts: Vec<(&str, Command)> = vec![
        ("file", Command::OpenBuild { path: bad.clone() }),
        ("code", Command::ImportShareCode { code: code.clone() }),
        // a code in the Import tab's box: PoB's own press runs Init (ImportTab.lua:665)
        (
            "import tab code",
            Command::ImportFromUrl {
                url: code.clone(),
                mode: ImportMode::NewBuild,
            },
        ),
    ];
    for (what, open) in attempts {
        // the database panel of the build being left (R21)
        s.run(Command::SetItemDb(ItemDbQuery {
            db: ItemDb::Uniques,
            slot: 0,
            kind: 0,
            league: 0,
            requirement: 0,
            obtainable: 0,
            sort: 0,
            search: String::new(),
            search_mode: 0,
        }))
        .expect(what);
        assert!(matches!(
            s.backend.item_db_view().unwrap(),
            DbSnapshot::View(_)
        ));
        let generation = s.backend.build_view().unwrap().generation;
        let err = s.run(open).expect_err(what);
        // the failed open still left the build: a new one stands in, and its view says so,
        // so the UI shows the build list as PoB does
        assert_ne!(
            s.backend.build_view().unwrap().generation,
            generation,
            "{what}: the stand-in build looks like the old one"
        );
        assert!(
            matches!(s.backend.item_db_view().unwrap(), DbSnapshot::Ended),
            "{what}: the database outlived its build"
        );
        assert_eq!(err.to_string(), want, "{what}");
        assert_eq!(our_state(&s.backend.lua)["popups"], 0, "{what}");
        let prompt: bool = s
            .backend
            .lua
            .load("return pob.main.promptMsg ~= nil")
            .eval()
            .unwrap();
        assert!(!prompt, "{what}");
        assert_eq!(
            s.backend.build_view().unwrap().name,
            "Unnamed build",
            "{what}"
        );
        // the next frame keeps it: PoB's switch to its build list was called off
        s.backend.recalculate().unwrap();
        assert!(s.backend.build_loaded().unwrap(), "{what}");
        s.run(Command::NewBuild)
            .expect("a new build after the failure");
        s.open_fixture("attack", &fixture_xml("attack"));
        super::assert_view_matches("attack", what, &s.stats().stats, &golden("attack").stats);
    }

    // a link: the request fails with PoB's message, with a new build in place
    n.backend
        .apply(
            Command::ImportFromUrl {
                url: "https://pobb.in/gate".into(),
                mode: ImportMode::NewBuild,
            },
            &|_| {},
        )
        .expect("the request starts");
    let steps = n.settle_net(Duration::from_secs(15));
    assert!(steps.iter().all(Result::is_ok), "a step failed");
    let mut view = n.requests();
    assert_eq!(view.finished.len(), 1, "{view:?}");
    let entry = view.finished.remove(0);
    assert_eq!(entry.error.as_deref(), Some(want.as_str()), "link");
    let popups: usize = n
        .backend
        .lua
        .load("return #pob.main.main.popups")
        .eval()
        .unwrap();
    assert_eq!(popups, 0, "link");
    assert_eq!(n.backend.build_view().unwrap().name, "Unnamed build");
    n.backend.recalculate().unwrap();
    assert!(
        n.backend.build_loaded().unwrap(),
        "link: PoB went to its list"
    );
    n.backend
        .apply(Command::NewBuild, &|_| {})
        .expect("link: new build");
    let attack = n.user.join("Path of Building/Builds/attack.xml");
    fs::copy(fixture_xml("attack"), &attack).unwrap();
    n.backend
        .apply(Command::OpenBuild { path: attack }, &|_| {})
        .expect("link: the good build opens");
    n.backend.recalculate().unwrap();
    super::assert_view_matches(
        "attack",
        "link",
        &n.backend.stats_view().unwrap().stats,
        &golden("attack").stats,
    );
}

/// A build file that reads as nothing: PoB's list scan opens its cloud error for it and leaves
/// it out (BuildListHelpers.lua:22-24, 70-71; upstream's headless file search is a stub, so
/// only ours is run), and its open fails. Neither leaves a popup, and the engine goes on.
#[test]
#[ignore = "needs the pinned PoB install"]
fn an_empty_build_file_leaves_no_popup() {
    let _guard = lock();
    let mut s = Session::start("empty-build");
    let empty = s.builds_dir().join("empty.xml");
    fs::create_dir_all(empty.parent().unwrap()).unwrap();
    fs::write(&empty, "").unwrap();
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    // PoB leaves the file out of its list (BuildListHelpers.lua:70-71)
    let list = s.backend.build_list_view().unwrap();
    assert!(!list.entries.iter().any(|e| e.name == "empty"), "{list:?}");
    assert_eq!(
        our_state(&s.backend.lua)["popups"],
        0,
        "the list left PoB's popup"
    );
    s.run(Command::OpenBuild { path: empty })
        .expect_err("an empty file");
    assert_eq!(
        our_state(&s.backend.lua)["popups"],
        0,
        "the open left a popup"
    );
    s.open_fixture("attack", &fixture_xml("attack"));
    super::assert_view_matches("attack", "stat", &s.stats().stats, &golden("attack").stats);
}

/// The exit save records the open build as PoB's last mode, which PoB opens at its next start
/// (Main.lua:510-523), before any command: auto-pob must start on it, and on a build that
/// still needs its Game Version window
#[test]
#[ignore = "needs the pinned PoB install"]
fn starting_on_the_build_the_exit_save_recorded() {
    use auto_pob::engine::pob::PobBackend;
    use auto_pob::game::Game;
    let _guard = lock();
    let mut s = LongtailSession::start("restart", &|_| {
        "<PathOfBuilding>\n\t<Mode mode=\"LIST\"><Arg boolean=\"false\"/><Arg string=\"\"/></Mode>\n</PathOfBuilding>\n".into()
    });
    let builds = settings_folder(&s.user).join("Builds");
    let attack = builds.join("attack.xml");
    fs::copy(fixture_xml("attack"), &attack).unwrap();
    let old = builds.join("old.xml");
    fs::write(&old, old_xml()).unwrap();
    let restart = |s: &LongtailSession| {
        PobBackend::start_with(Game::Poe1, &s.pob, &s.user, canned(&canned_web()))
            .expect("starting on the recorded build")
    };

    s.run(Command::OpenBuild {
        path: attack.clone(),
    })
    .unwrap();
    s.backend.shutdown();
    let mut next = restart(&s);
    assert!(next.build_loaded().unwrap());
    assert_eq!(next.build_view().unwrap().name, "attack");
    let popups: usize = next
        .lua
        .load("return #pob.main.main.popups")
        .eval()
        .unwrap();
    assert_eq!(popups, 0);
    drop(next);

    // an old build converted but not saved: the file still needs the window
    s.run(Command::OpenBuild { path: old.clone() }).unwrap();
    s.run(Command::PressPopupButton {
        name: "convert".into(),
    })
    .unwrap();
    s.backend.shutdown();
    let mut next = restart(&s);
    let popups: usize = next
        .lua
        .load("return #pob.main.main.popups")
        .eval()
        .unwrap();
    assert_eq!(popups, 0, "a startup popup was left in PoB's stack");
    assert!(next.build_loaded().unwrap());
    let view = next.popup_view().unwrap().expect("the Game Version window");
    assert_eq!(view.kind, PopupKind::GameVersion);
    assert_eq!(next.build_view().unwrap().name, "Unnamed build");
    drop(next);

    // a recorded build PoB can't open: it starts on its list, and the next open works
    let bad = builds.join("bad.xml");
    fs::write(&bad, unknown_tree_xml()).unwrap();
    fs::write(
        s.settings_path(),
        format!(
            "<PathOfBuilding>\n\t<Mode mode=\"BUILD\"><Arg string=\"{}\"/><Arg string=\"bad\"/></Mode>\n</PathOfBuilding>\n",
            bad.display()
        ),
    )
    .unwrap();
    let mut next = restart(&s);
    let popups: usize = next
        .lua
        .load("return #pob.main.main.popups")
        .eval()
        .unwrap();
    assert_eq!(popups, 0, "the failed start's message stayed");
    assert!(!next.build_loaded().unwrap(), "PoB went to its list");
    next.apply(Command::NewBuild, &|_| {})
        .expect("a new build after the failed start");
    next.apply(Command::OpenBuild { path: attack }, &|_| {})
        .expect("the good build after the failed start");
    assert_eq!(next.build_view().unwrap().name, "attack");
}
