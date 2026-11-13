//! Gate tests for the build list engine (R16): rename, copy, delete, new folder, move, sort and
//! search through PoB's own list mode, against upstream PoB's list driven by its own buttons
//! (`fixtures/longtail_lib.lua`), on temp Builds trees. Included from `gate.rs`, whose helpers
//! it reaches through `super::`.

use super::longtail_harness::{
    LongtailSession, misc_attributes, settings_folder, upstream_settings,
};
use super::{fixtures_dir, lock, pob_dir, upstream_lua};
use auto_pob::engine::{Backend, BuildListView, BuildSort, Command, DeleteKind};
use mlua::LuaSerdeExt;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The root's wording of an entry, from either side
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Row {
    name: String,
    is_folder: bool,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    ascendancy: Option<String>,
    #[serde(default)]
    level: Option<u32>,
    /// the path below the Builds folder
    rel: String,
}

fn rows(view: &BuildListView, builds: &Path) -> Vec<Row> {
    view.entries
        .iter()
        .map(|e| Row {
            name: e.name.clone(),
            is_folder: e.is_folder,
            class: e.class.clone(),
            ascendancy: e.ascendancy.clone(),
            level: e.level,
            rel: e
                .path
                .strip_prefix(builds)
                .unwrap_or_else(|_| panic!("{:?} is outside {builds:?}", e.path))
                .to_string_lossy()
                .into_owned(),
        })
        .collect()
}

fn names(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

/// `xml` with `from` replaced by `to` in its `<Build>` tag
fn patch(xml: &str, from: &str, to: &str) -> String {
    let start = xml.find("<Build ").expect("no Build tag");
    let end = start + xml[start..].find('>').unwrap();
    let tag = &xml[start..end];
    assert!(tag.contains(from), "{from} is not in {tag}");
    format!(
        "{}{}{}",
        &xml[..start],
        tag.replacen(from, to, 1),
        &xml[end..]
    )
}

/// The Builds tree both sides start from. Classes, ascendancies and levels differ, so that
/// each sort mode orders it differently:
/// `a` Marauder 90, `dot` Witch 75, `minion` Witch/Necromancer 60, `sub/b` Scion 1, `empty/`.
fn seed(builds: &Path) {
    let fixture =
        |name: &str| fs::read_to_string(fixtures_dir().join(format!("{name}.xml"))).unwrap();
    fs::create_dir_all(builds.join("sub")).unwrap();
    fs::create_dir_all(builds.join("empty")).unwrap();
    fs::write(builds.join("a.xml"), fixture("attack")).unwrap();
    fs::write(
        builds.join("dot.xml"),
        patch(&fixture("dot"), "level=\"90\"", "level=\"75\""),
    )
    .unwrap();
    let minion = patch(&fixture("minion"), "level=\"90\"", "level=\"60\"");
    fs::write(
        builds.join("minion.xml"),
        patch(
            &minion,
            "ascendClassName=\"None\"",
            "ascendClassName=\"Necromancer\"",
        ),
    )
    .unwrap();
    fs::write(builds.join("sub/b.xml"), fixture("new")).unwrap();
}

type Tree = BTreeMap<String, (Option<Vec<u8>>, SystemTime)>;

/// Every file (its bytes) and folder (`None`) below `root`, with modification times; links are
/// not followed.
fn tree(root: &Path) -> Tree {
    fn walk(dir: &Path, root: &Path, out: &mut Tree) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if meta.is_dir() {
                out.insert(rel, (None, meta.modified().unwrap()));
                walk(&path, root, out);
            } else if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(
                    rel,
                    (
                        Some(target.to_string_lossy().into_owned().into_bytes()),
                        meta.modified().unwrap(),
                    ),
                );
            } else {
                out.insert(
                    rel,
                    (Some(fs::read(&path).unwrap()), meta.modified().unwrap()),
                );
            }
        }
    }
    let mut out = Tree::new();
    walk(root, root, &mut out);
    out
}

/// A tree without the times
fn contents(tree: &Tree) -> BTreeMap<&str, &Option<Vec<u8>>> {
    tree.iter()
        .map(|(k, (bytes, _))| (k.as_str(), bytes))
        .collect()
}

fn set_mtime(path: &Path, secs: u64) {
    let file = fs::File::open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

/// Gives every file and folder below `root` a modification time from its path, so that two
/// trees with the same paths have the same times and the times are not in name order.
fn stamp(root: &Path) {
    let mut paths: Vec<String> = tree(root).into_keys().collect();
    paths.sort();
    // deepest first: a folder's own time is set after what is in it
    for (i, rel) in paths.iter().enumerate().rev() {
        let mix = rel
            .bytes()
            .fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
        set_mtime(
            &root.join(rel),
            1_700_000_000 + (mix % 997) * 10 + i as u64 * 1_000_000,
        );
    }
}

fn is_root() -> bool {
    let dir = std::env::temp_dir().join(format!("auto-pob-bl-root-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    set_mode(&dir, 0o555);
    let writable = fs::write(dir.join("x"), "x").is_ok();
    set_mode(&dir, 0o755);
    let _ = fs::remove_dir_all(&dir);
    writable
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Upstream PoB with the file natives, its build list started on its own temp Builds folder.
///
/// HeadlessWrapper stubs `NewFileSearch`, `MakeDir` and `RemoveDir`, so upstream runs on our
/// natives (native.rs). The comparison proves that PoB's list logic on our natives matches our
/// glue; it does not prove our natives match SimpleGraphic's. A native bug in ordering or in
/// `NewFileSearch`'s fields would show on both sides alike; native.rs's own unit tests cover the
/// natives.
struct Upstream {
    lua: mlua::Lua,
    dir: PathBuf,
}

impl Upstream {
    fn start(test: &str) -> Upstream {
        let dir =
            std::env::temp_dir().join(format!("auto-pob-bl-up-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Builds")).unwrap();
        let lua = upstream_lua(&pob_dir()).expect("upstream PoB");
        // the wrapper stubs the file functions: PoB's file system is ours (native.rs)
        auto_pob::engine::native::register(&lua, &pob_dir(), &dir).unwrap();
        auto_pob::engine::native::set_builds_root(&lua, Some(&dir.join("Builds")));
        lua.load(include_str!("../fixtures/longtail_lib.lua"))
            .set_name("=longtail_lib")
            .exec()
            .unwrap();
        lua.globals()
            .set("BUILDS", format!("{}/", dir.join("Builds").display()))
            .unwrap();
        lua.load("listStart(BUILDS)").exec().unwrap();
        Upstream { lua, dir }
    }

    fn builds(&self) -> PathBuf {
        self.dir.join("Builds")
    }

    fn path(&self, rel: &str) -> String {
        format!("{}/{rel}", self.builds().display())
    }

    fn call<R: mlua::FromLuaMulti>(&self, function: &str, args: impl mlua::IntoLuaMulti) -> R {
        let f: mlua::Function = self.lua.globals().get(function).unwrap();
        f.call(args)
            .unwrap_or_else(|e| panic!("upstream {function}: {e}"))
    }

    fn dump(&self, folder: &str) -> Vec<Row> {
        let value: mlua::Value = self.call("listDump", folder);
        self.lua.from_value(value).unwrap()
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        set_mode_tree(&self.dir);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Makes every folder below `dir` writable again, so a failed test still cleans up
fn set_mode_tree(dir: &Path) {
    if let Ok(read) = fs::read_dir(dir) {
        let _ = fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o755));
        for entry in read.filter_map(Result::ok) {
            if entry.path().is_dir() && !entry.path().is_symlink() {
                set_mode_tree(&entry.path());
            }
        }
    }
}

/// A PoB session and an upstream one over equal Builds trees, and the operations run on both
struct Pair {
    s: LongtailSession,
    up: Upstream,
    builds: PathBuf,
}

/// What PoB says when it leaves a Save or Create button disabled, as the engine words it
const NOT_OFFERED: &str = "PoB doesn't offer that here";
/// What the engine says before PoB's Save is asked, for a name that is taken
const TAKEN: &str = "a build with that name already exists";

impl Pair {
    fn path(&self, rel: &str) -> PathBuf {
        self.builds.join(rel)
    }

    /// The engine's list of `folder`, as PoB's list mode shows it, with no popup left open
    fn list(&mut self, folder: &str) -> BuildListView {
        self.s
            .run(Command::ListBuilds {
                folder: folder.into(),
            })
            .unwrap();
        assert_eq!(self.s.open_popups(), 0, "listing left a popup open");
        self.s.backend.build_list_view().unwrap()
    }

    /// Both trees hold the same files, and both lists show the same rows in every folder
    fn same(&mut self, what: &str) {
        let engine = tree(&self.builds);
        let upstream = tree(&self.up.builds());
        assert_eq!(
            contents(&engine),
            contents(&upstream),
            "{what}: the trees differ"
        );
        let mut folders = vec![String::new()];
        folders.extend(
            engine
                .iter()
                .filter(|(_, (bytes, _))| bytes.is_none())
                .map(|(rel, _)| format!("{rel}/")),
        );
        for folder in folders {
            let view = self.list(&folder);
            let ours = rows(&view, &self.builds);
            assert_eq!(
                ours,
                self.up.dump(&folder),
                "{what}: the list of '{folder}' differs"
            );
        }
        assert_eq!(self.s.open_popups(), 0, "{what}: a popup is open");
    }

    /// The upstream outcome: `None` for a success
    fn outcome(
        &self,
        upstream: Option<String>,
        engine: &anyhow::Result<auto_pob::engine::Affected>,
    ) {
        match (&upstream, engine) {
            (None, Ok(_)) => {}
            (Some(message), Err(e)) if message == "disabled" => {
                assert!(
                    e.to_string() == NOT_OFFERED || e.to_string() == TAKEN,
                    "{e}"
                )
            }
            (Some(_), Err(_)) => {}
            (up, engine) => panic!("upstream {up:?} but the engine {:?}", engine.as_ref().err()),
        }
    }

    fn rename(&mut self, rel: &str, name: &str) -> anyhow::Result<auto_pob::engine::Affected> {
        let result = self.s.run(Command::RenameBuild {
            path: self.path(rel),
            name: name.into(),
        });
        let up: Option<String> = self.up.call("listRename", (self.up.path(rel), name, false));
        self.outcome(up, &result);
        result
    }

    fn copy(&mut self, rel: &str, name: &str) -> anyhow::Result<auto_pob::engine::Affected> {
        let result = self.s.run(Command::CopyBuild {
            path: self.path(rel),
            name: name.into(),
        });
        let up: Option<String> = self.up.call("listRename", (self.up.path(rel), name, true));
        self.outcome(up, &result);
        result
    }

    fn mv(&mut self, rel: &str, folder: &str) -> anyhow::Result<auto_pob::engine::Affected> {
        let result = self.s.run(Command::MoveBuild {
            path: self.path(rel),
            folder: folder.into(),
        });
        let up: Option<String> = self.up.call("listMove", (self.up.path(rel), folder));
        self.outcome(up.clone(), &result);
        if let (Some(up), Err(e)) = (up, &result) {
            assert_eq!(e.to_string(), up, "PoB's own text");
        }
        result
    }

    fn copy_to(&mut self, rel: &str, folder: &str) -> anyhow::Result<auto_pob::engine::Affected> {
        let result = self.s.run(Command::CopyBuildTo {
            path: self.path(rel),
            folder: folder.into(),
        });
        let up: Option<String> = self.up.call("listCopyTo", (self.up.path(rel), folder));
        self.outcome(up.clone(), &result);
        if let (Some(up), Err(e)) = (up, &result) {
            assert_eq!(e.to_string(), up, "PoB's own text");
        }
        result
    }

    fn new_folder(
        &mut self,
        listed: &str,
        name: &str,
    ) -> anyhow::Result<auto_pob::engine::Affected> {
        self.s
            .run(Command::ListBuilds {
                folder: listed.into(),
            })
            .unwrap();
        let result = self.s.run(Command::NewFolder { name: name.into() });
        let up: Option<String> = self.up.call("listNewFolder", (listed, name));
        self.outcome(up.clone(), &result);
        if let (Some(up), Err(e)) = (up, &result) {
            assert_eq!(e.to_string(), up, "PoB's own text");
        }
        result
    }

    /// Deletes on both sides, after checking the engine's confirm text against PoB's
    fn delete(&mut self, rel: &str) -> String {
        let folder = rel
            .rsplit_once('/')
            .map_or(String::new(), |(f, _)| format!("{f}/"));
        let view = self.list(&folder);
        let entry = view
            .entries
            .iter()
            .find(|e| e.path == self.path(rel))
            .unwrap_or_else(|| panic!("{rel} is not listed"));
        let prompt = entry.delete_prompt.clone();
        let kind = entry.delete_kind();
        let (text, message): (String, Option<String>) =
            self.up.call("listDelete", (self.up.path(rel), true));
        assert_eq!(message, None);
        assert_eq!(prompt, text, "PoB's confirm text for {rel}");
        let result = self.s.run(Command::DeleteBuild {
            path: self.path(rel),
            kind,
        });
        self.outcome(None, &result);
        prompt
    }
}

fn session_settings(mode: &str) -> impl Fn(&Path) -> String + '_ {
    move |_| format!("<PathOfBuilding>\n{mode}\t<Misc/>\n</PathOfBuilding>\n")
}

const LIST_MODE: &str = "\t<Mode mode=\"LIST\"><Arg boolean=\"false\"/><Arg string=\"\"/></Mode>\n";

fn expected<S: AsRef<str>>(rows: &[Row], want: &[S]) {
    let want: Vec<&str> = want.iter().map(AsRef::as_ref).collect();
    assert_eq!(names(rows), want);
}

fn start_pair(test: &str, mode: &str) -> Pair {
    let s = LongtailSession::start(test, &session_settings(mode));
    let builds = settings_folder(&s.user).join("Builds");
    let up = Upstream::start(test);
    seed(&builds);
    seed(&up.builds());
    Pair { s, up, builds }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn build_list_ops() {
    let _guard = lock();
    let mut t = start_pair("build_list_ops", LIST_MODE);
    // a sibling of Builds that nothing here may touch
    let outside = settings_folder(&t.s.user).join("outside");
    fs::create_dir_all(outside.join("deep")).unwrap();
    fs::write(outside.join("sentinel.xml"), "sentinel").unwrap();
    fs::write(outside.join("deep/more.txt"), "more").unwrap();
    stamp(&outside);
    let outside_before = tree(&outside);

    // the start in LIST mode: PoB's list mode is there, no build is open
    t.same("start");
    let root = t.list("");
    expected(
        &rows(&root, &t.builds),
        &["empty", "sub", "a", "dot", "minion"],
    );

    // rename
    t.rename("a.xml", "a2").expect("rename");
    t.same("rename");
    assert!(t.path("a2.xml").is_file() && !t.path("a.xml").exists());

    // copy, byte for byte
    t.copy("a2.xml", "a3").expect("copy");
    t.same("copy");
    assert_eq!(
        fs::read(t.path("a3.xml")).unwrap(),
        fs::read(t.path("a2.xml")).unwrap()
    );

    // new folder, then move into it, then again with a name clash
    t.new_folder("", "n").expect("new folder");
    t.same("new folder");
    t.mv("a3.xml", "n/").expect("move");
    t.same("move");
    assert!(t.path("n/a3.xml").is_file() && !t.path("a3.xml").exists());
    t.copy("a2.xml", "a3").expect("copy again");
    t.mv("a3.xml", "n/").expect("move with a clash");
    t.same("move with a clash");
    assert!(t.path("n/a3[2].xml").is_file(), "PoB's name for a clash");
    assert!(t.path("n/a3.xml").is_file());

    // a folder into itself: PoB's text, nothing changes
    let err = t.mv("sub", "sub/").expect_err("into itself").to_string();
    assert_eq!(err, "A folder cannot be moved into itself.");
    t.same("a folder into itself");
    // a folder moved, and one copied (everything in it with it)
    t.mv("empty", "n/").expect("move a folder");
    t.same("move a folder");
    assert!(t.path("n/empty").is_dir() && !t.path("empty").exists());
    t.copy("sub", "subcopy").expect("copy a folder");
    t.same("copy a folder");
    assert!(t.path("subcopy/b.xml").is_file());

    // Ctrl+C then Ctrl+V in another folder: the original stays; a taken name gets PoB's
    // name[2]; a folder is copied with what is in it; never into itself
    t.copy_to("a2.xml", "n/").expect("copy into a folder");
    t.same("copy into a folder");
    assert!(t.path("n/a2.xml").is_file() && t.path("a2.xml").is_file());
    t.copy_to("a2.xml", "n/").expect("copy into a folder again");
    t.same("copy with a clash");
    assert!(t.path("n/a2[2].xml").is_file(), "PoB's name for a clash");
    let err = t
        .copy_to("sub", "sub/")
        .expect_err("a folder into itself")
        .to_string();
    assert_eq!(err, "A folder cannot be copied into itself.");
    t.same("a folder copied into itself");
    t.copy_to("n/empty", "sub/").expect("copy a folder");
    t.same("copy a folder into a folder");
    assert!(t.path("n/empty").is_dir() && t.path("sub/empty").is_dir());
    // the Save As picker's New Folder names its folder; the tree lists every folder, links not
    t.s.run(Command::NewFolderIn {
        folder: "n/".into(),
        name: "deep".into(),
    })
    .expect("new folder in another folder");
    assert!(t.path("n/deep").is_dir());
    let view = t.list("");
    let node = view
        .tree
        .iter()
        .find(|f| f.path == "n/deep/")
        .expect("the new folder is in the tree");
    assert!(node.empty && node.modified > 0);
    assert!(view.tree.iter().any(|f| f.path == "n/" && !f.empty));
    assert!(view.tree.iter().any(|f| f.path == "sub/empty/"));
    fs::remove_dir(t.path("n/deep")).unwrap();
    // in its own folder PoB asks for a name instead, which is Copy
    let err = t.s.run(Command::CopyBuildTo {
        path: t.path("a2.xml"),
        folder: String::new(),
    });
    assert_eq!(
        err.expect_err("same folder").to_string(),
        "that build is already in that folder"
    );
    t.same("a copy into the same folder");

    // a taken name and the same name leave PoB's Save disabled; the engine says why first
    let err = t
        .rename("dot.xml", "minion")
        .expect_err("taken")
        .to_string();
    assert_eq!(err, TAKEN);
    let err = t.copy("dot.xml", "dot").expect_err("same").to_string();
    assert_eq!(err, TAKEN);
    let err = t
        .new_folder("", "n")
        .expect_err("a folder that exists")
        .to_string();
    assert_eq!(err, "Couldn't create 'n': File exists");
    t.mv("sub", "n/").expect("a folder with a build in it");
    t.same("a folder moved with its build");
    assert!(t.path("n/sub/b.xml").is_file());
    t.mv("n/sub", "").expect("and back");
    t.same("refusals");

    // search: copies of the Witch builds, one of them in a folder
    t.copy("dot.xml", "zdot").unwrap();
    t.copy("minion.xml", "zminion").unwrap();
    t.copy("minion.xml", "witchling").unwrap();
    t.mv("witchling.xml", "sub/").unwrap();
    t.same("copies");
    let witch: Vec<Row> = {
        t.s.run(Command::SetBuildFilter {
            text: "class:witch".into(),
        })
        .unwrap();
        let view = t.list("");
        assert_eq!(view.filter, "class:witch");
        let _: String = t.up.call("listFilter", "class:witch");
        let ours = rows(&view, &t.builds);
        assert_eq!(ours, t.up.dump(""), "class:witch");
        ours
    };
    expected(&witch, &["dot", "minion", "witchling", "zdot", "zminion"]);
    assert_eq!(
        witch[2].rel, "sub/witchling.xml",
        "matches from every folder"
    );
    // an operation works on the entry whatever the search shows, and leaves the search as it was
    t.rename("sub/witchling.xml", "witch")
        .expect("rename under a search");
    assert_eq!(t.list("").filter, "class:witch");
    let view = t.list("");
    let ours = rows(&view, &t.builds);
    assert_eq!(ours, t.up.dump(""));
    expected(&ours, &["dot", "minion", "witch", "zdot", "zminion"]);
    // and one the search hides is reached by its path
    t.s.run(Command::RenameBuild {
        path: t.path("a2.xml"),
        name: "a2x".into(),
    })
    .expect("rename an entry the search hides");
    t.s.run(Command::RenameBuild {
        path: t.path("a2x.xml"),
        name: "a2".into(),
    })
    .unwrap();
    assert_eq!(t.list("").filter, "class:witch");
    // a name search; the search box takes PoB's filter (no parentheses)
    t.s.run(Command::SetBuildFilter { text: "dot".into() })
        .unwrap();
    let _: String = t.up.call("listFilter", "dot");
    let view = t.list("");
    let ours = rows(&view, &t.builds);
    assert_eq!(ours, t.up.dump(""));
    expected(&ours, &["dot", "zdot"]);
    t.s.run(Command::SetBuildFilter {
        text: String::new(),
    })
    .unwrap();
    let _: String = t.up.call("listFilter", "");
    t.same("search cleared");
    assert_eq!(t.list("").filter, "");

    // the open build is refused for every change; copying it is fine
    let open = t.path("n/a3.xml");
    t.s.run(Command::OpenBuild { path: open.clone() })
        .expect("open");
    // listing leaves the build loaded, and the guard with it
    t.list("");
    assert!(t.s.backend.build_loaded().unwrap());
    let open_err = |s: &mut LongtailSession, command: Command| {
        let what = format!("{command:?}");
        s.run(command).expect_err(&what).to_string()
    };
    for command in [
        Command::RenameBuild {
            path: open.clone(),
            name: "x".into(),
        },
        Command::MoveBuild {
            path: open.clone(),
            folder: String::new(),
        },
        Command::DeleteBuild {
            path: open.clone(),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: t.path("n"),
            kind: DeleteKind::Folder,
        },
        Command::RenameBuild {
            path: t.path("n"),
            name: "n9".into(),
        },
        Command::MoveBuild {
            path: t.path("n"),
            folder: "sub/".into(),
        },
    ] {
        let what = format!("{command:?}");
        assert_eq!(open_err(&mut t.s, command), "that build is open", "{what}");
    }
    assert!(open.is_file() && t.path("n").is_dir());
    t.s.run(Command::CopyBuild {
        path: open.clone(),
        name: "a3copy".into(),
    })
    .expect("copy of the open build");
    assert!(t.path("n/a3copy.xml").is_file());
    t.s.run(Command::DeleteBuild {
        path: t.path("n/a3copy.xml"),
        kind: DeleteKind::Build,
    })
    .expect("a build that isn't open");
    // a later Save still writes the original path
    let original = fs::read(&open).unwrap();
    set_mtime(&open, 1_000_000_000);
    t.s.run(Command::Save).expect("save");
    assert_eq!(
        t.s.backend.build_view().unwrap().path.as_deref(),
        Some(open.as_path())
    );
    let saved = fs::metadata(&open).unwrap().modified().unwrap();
    assert!(
        saved > UNIX_EPOCH + Duration::from_secs(1_000_000_001),
        "Save wrote the original path"
    );
    fs::write(&open, &original).unwrap();
    // with another build open the guard moves along
    t.s.run(Command::NewBuild).unwrap();
    t.rename("n/a3.xml", "a3r").expect("no longer open");
    t.rename("n/a3r.xml", "a3").expect("and back");
    t.same("open build");
    assert_eq!(t.s.open_popups(), 0);

    // sort: each mode against upstream's dropdown, in a folder with several of each
    t.s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    stamp(&t.builds);
    stamp(&t.up.builds());
    let owned = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let builds = t.builds.clone();
    let by = |sort: BuildSort| match sort {
        BuildSort::Name => owned(&[
            "n", "sub", "subcopy", "a2", "dot", "minion", "zdot", "zminion",
        ]),
        BuildSort::Class => owned(&[
            "n", "sub", "subcopy", "a2", "minion", "zminion", "dot", "zdot",
        ]),
        BuildSort::Level => owned(&[
            "n", "sub", "subcopy", "minion", "zminion", "dot", "zdot", "a2",
        ]),
        // folders and builds by modification time, newest first
        BuildSort::Edited => {
            let stamps = tree(&builds);
            let order = |folders: bool| {
                let mut found: Vec<(&String, SystemTime)> = stamps
                    .iter()
                    .filter(|(rel, (bytes, _))| !rel.contains('/') && bytes.is_none() == folders)
                    .map(|(rel, (_, time))| (rel, *time))
                    .collect();
                found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
                found
                    .into_iter()
                    .map(|(rel, _)| rel.trim_end_matches(".xml").to_string())
                    .collect::<Vec<_>>()
            };
            let mut all = order(true);
            all.extend(order(false));
            all
        }
    };
    let mut seen = Vec::new();
    for sort in [
        BuildSort::Class,
        BuildSort::Level,
        BuildSort::Edited,
        BuildSort::Name,
    ] {
        t.s.run(Command::SetBuildSort { sort }).unwrap();
        let chosen: String = t.up.call("listSort", sort.id());
        assert_eq!(chosen, sort.id());
        let view = t.list("");
        assert_eq!(view.sort, sort);
        let ours = rows(&view, &t.builds);
        assert_eq!(ours, t.up.dump(""), "{sort:?}");
        expected(&ours, &by(sort));
        seen.push(names(&ours).join(","));
        // the choice reaches Settings.xml, as PoB's own loader reads it
        let xml = String::from_utf8(t.s.settings_bytes()).unwrap();
        assert_eq!(
            misc_attributes(&xml)
                .get("buildSortMode")
                .map(String::as_str),
            Some(sort.id())
        );
        // (a whole upstream state each: once is enough)
        if sort == BuildSort::Level {
            assert_eq!(
                upstream_settings(&t.s.user, &["buildSortMode"])["buildSortMode"],
                "LEVEL"
            );
        }
        let n = t.list("n/");
        assert_eq!(rows(&n, &t.builds), t.up.dump("n/"), "{sort:?} in n/");
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4, "the four modes order the folder four ways");

    // delete: a build, the empty folder, a folder with things in it (recursive)
    let prompt = t.delete("zdot.xml");
    assert_eq!(
        prompt,
        "Are you sure you want to delete build:\nzdot\nThis cannot be undone."
    );
    assert!(!t.path("zdot.xml").exists());
    t.same("delete a build");
    let prompt = t.delete("n/empty");
    assert_eq!(prompt, "", "PoB deletes an empty folder without asking");
    assert!(!t.path("n/empty").exists());
    t.same("delete the empty folder");
    let prompt = t.delete("subcopy");
    assert_eq!(
        prompt,
        "The folder is not empty.\nAre you sure you want to delete folder:\nsubcopy\nThis cannot be undone."
    );
    assert!(
        !t.path("subcopy").exists(),
        "the folder went with what was in it"
    );
    t.same("delete a folder");
    let err =
        t.s.run(Command::DeleteBuild {
            path: t.path("zdot.xml"),
            kind: DeleteKind::Build,
        })
        .unwrap_err();
    assert_eq!(err.to_string(), "no such build");

    // a delete that fails: PoB ignores what os.remove answers, so the glue looks
    if is_root() {
        eprintln!("skipping the failed delete: root can write anywhere");
    } else {
        let before = tree(&t.builds);
        set_mode(&t.path("n"), 0o555);
        let result = t.s.run(Command::DeleteBuild {
            path: t.path("n/a3[2].xml"),
            kind: DeleteKind::Build,
        });
        let after_popups = t.s.open_popups();
        set_mode(&t.path("n"), 0o755);
        assert_eq!(
            result.expect_err("failed").to_string(),
            "couldn't delete that build"
        );
        assert!(t.path("n/a3[2].xml").is_file(), "the file stays");
        assert_eq!(after_popups, 0);
        assert_eq!(contents(&tree(&t.builds)), contents(&before));
    }

    // nothing reaches outside the Builds folder (Review Focus 1)
    let builds_before = tree(&t.builds);
    let a2 = t.path("a2.xml");
    let out = |s: &mut LongtailSession, command: Command| {
        let what = format!("{command:?}");
        let err = s.run(command).expect_err(&what).to_string();
        (what, err)
    };
    let sentinel = outside.join("sentinel.xml");
    let not_allowed = "that name isn't allowed";
    for name in ["..", ".", "a/b", "../x", "", "   ", "x?", "x\ny"] {
        let (what, err) = out(
            &mut t.s,
            Command::RenameBuild {
                path: a2.clone(),
                name: name.into(),
            },
        );
        assert_eq!(err, not_allowed, "{what}");
        let (what, err) = out(
            &mut t.s,
            Command::CopyBuild {
                path: a2.clone(),
                name: name.into(),
            },
        );
        assert_eq!(err, not_allowed, "{what}");
        t.s.run(Command::ListBuilds {
            folder: String::new(),
        })
        .unwrap();
        let (what, err) = out(&mut t.s, Command::NewFolder { name: name.into() });
        assert_eq!(err, not_allowed, "{what}");
    }
    let escapes = "that path is outside the Builds folder";
    let builds_str = t.builds.display().to_string();
    for command in [
        Command::DeleteBuild {
            path: "/tmp/x.xml".into(),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: sentinel.clone(),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: format!("{builds_str}/../outside/sentinel.xml").into(),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: "a2.xml".into(),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: outside.clone(),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: t.builds.clone(),
            kind: DeleteKind::Build,
        },
        Command::RenameBuild {
            path: sentinel.clone(),
            name: "x".into(),
        },
        Command::CopyBuild {
            path: sentinel.clone(),
            name: "x".into(),
        },
        Command::MoveBuild {
            path: sentinel.clone(),
            folder: String::new(),
        },
        Command::MoveBuild {
            path: a2.clone(),
            folder: "../outside".into(),
        },
        Command::MoveBuild {
            path: a2.clone(),
            folder: "n/../../outside".into(),
        },
        Command::MoveBuild {
            path: a2.clone(),
            folder: "/etc".into(),
        },
        Command::MoveBuild {
            path: a2.clone(),
            folder: "..".into(),
        },
    ] {
        let (what, err) = out(&mut t.s, command);
        assert_eq!(err, escapes, "{what}");
    }
    let (_, err) = out(
        &mut t.s,
        Command::MoveBuild {
            path: a2.clone(),
            folder: "none/".into(),
        },
    );
    assert_eq!(err, "no such folder");
    let (_, err) = out(
        &mut t.s,
        Command::ListBuilds {
            folder: "../outside".into(),
        },
    );
    assert_eq!(err, "that isn't a valid folder");
    // links that lead out of Builds
    use std::os::unix::fs::symlink;
    symlink(&outside, t.path("escape")).unwrap();
    symlink(&sentinel, t.path("esc.xml")).unwrap();
    fs::create_dir_all(t.path("trap")).unwrap();
    symlink(&outside, t.path("trap/link")).unwrap();
    for command in [
        Command::DeleteBuild {
            path: t.path("escape"),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: t.path("esc.xml"),
            kind: DeleteKind::Build,
        },
        Command::RenameBuild {
            path: t.path("escape"),
            name: "z".into(),
        },
        Command::CopyBuild {
            path: t.path("esc.xml"),
            name: "z".into(),
        },
        Command::MoveBuild {
            path: a2.clone(),
            folder: "escape/".into(),
        },
        Command::MoveBuild {
            path: t.path("escape"),
            folder: "n/".into(),
        },
        Command::MoveBuild {
            path: t.path("trap"),
            folder: "n/".into(),
        },
        Command::CopyBuild {
            path: t.path("trap"),
            name: "trap2".into(),
        },
        Command::ListBuilds {
            folder: "escape/".into(),
        },
    ] {
        let (what, err) = out(&mut t.s, command);
        assert_eq!(err, escapes, "{what}");
    }
    assert_eq!(t.s.open_popups(), 0);
    // a folder holding a link out is deleted without following it
    t.s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    t.s.run(Command::DeleteBuild {
        path: t.path("trap"),
        kind: DeleteKind::Folder,
    })
    .expect("delete the trap");
    assert!(!t.path("trap").exists());
    fs::remove_file(t.path("escape")).unwrap();
    fs::remove_file(t.path("esc.xml")).unwrap();
    assert_eq!(
        tree(&outside),
        outside_before,
        "the folder next to Builds is untouched"
    );
    assert_eq!(contents(&tree(&t.builds)), contents(&builds_before));
    assert_eq!(t.s.open_popups(), 0);
}

/// Listing, searching and refused commands write nothing; sorting writes `Settings.xml` and
/// nothing else, and never the value of an override (R29, Review Focus 3). PoB starts in BUILD
/// mode here, so the list mode is made on first use.
#[test]
#[ignore = "needs the pinned PoB install"]
fn build_list_no_silent_writes() {
    let _guard = lock();
    let mut s = LongtailSession::start("build_list_quiet", &|_| {
        "<PathOfBuilding>\n\t<Misc showThousandsSeparators=\"false\"/>\n</PathOfBuilding>\n".into()
    });
    let builds = settings_folder(&s.user).join("Builds");
    seed(&builds);
    stamp(&builds);
    s.run(Command::NewBuild).unwrap();
    let list = |s: &mut LongtailSession, folder: &str| {
        s.run(Command::ListBuilds {
            folder: folder.into(),
        })
        .unwrap();
        assert_eq!(s.open_popups(), 0);
        s.backend.build_list_view().unwrap()
    };
    let quiet = tree(&s.user);

    // browsing
    for folder in ["", "sub/", "empty/", "none/", ""] {
        list(&mut s, folder);
    }
    // the folder listed and PoB's build folder, as the view gives them (Task 16)
    let sub = list(&mut s, "sub/");
    assert_eq!(sub.folder, "sub/");
    assert_eq!(sub.root, builds);
    let view = list(&mut s, "");
    assert_eq!(
        names(&rows(&view, &builds)),
        ["empty", "sub", "a", "dot", "minion"]
    );
    // the question a delete asks is read, never answered: the empty folder is still there
    let prompts: Vec<&str> = view
        .entries
        .iter()
        .map(|e| e.delete_prompt.as_str())
        .collect();
    assert_eq!(prompts[0], "", "an empty folder is deleted without asking");
    assert!(
        prompts[1].starts_with("The folder is not empty.\n"),
        "{prompts:?}"
    );
    assert_eq!(
        prompts[2],
        "Are you sure you want to delete build:\na\nThis cannot be undone."
    );
    assert!(builds.join("empty").is_dir() && builds.join("a.xml").is_file());
    // searching
    for text in ["class:witch", "dot", "class:", "   ", "()", ""] {
        s.run(Command::SetBuildFilter { text: text.into() })
            .unwrap();
        list(&mut s, "");
    }
    assert_eq!(list(&mut s, "").filter, "");
    assert_eq!(
        tree(&s.user),
        quiet,
        "browsing and searching changed a file"
    );

    // commands PoB or the guards refuse
    let refused = [
        Command::RenameBuild {
            path: builds.join("a.xml"),
            name: "dot".into(),
        },
        Command::CopyBuild {
            path: builds.join("a.xml"),
            name: "a".into(),
        },
        Command::RenameBuild {
            path: builds.join("a.xml"),
            name: "..".into(),
        },
        Command::DeleteBuild {
            path: builds.join("gone.xml"),
            kind: DeleteKind::Build,
        },
        Command::DeleteBuild {
            path: "/tmp/x.xml".into(),
            kind: DeleteKind::Build,
        },
        Command::MoveBuild {
            path: builds.join("a.xml"),
            folder: "../x".into(),
        },
        Command::MoveBuild {
            path: builds.join("sub"),
            folder: "sub/".into(),
        },
        Command::NewFolder { name: "sub".into() },
    ];
    // PoB does nothing for a move into the folder the entry is in
    s.run(Command::MoveBuild {
        path: builds.join("a.xml"),
        folder: String::new(),
    })
    .expect("a move to the same folder is PoB's no-op");
    for command in refused {
        let what = format!("{command:?}");
        s.run(command).expect_err(&what);
        assert_eq!(s.open_popups(), 0, "{what}");
    }
    assert_eq!(tree(&s.user), quiet, "a refused command changed a file");

    // sorting: Settings.xml, with the shared value of an override, and nothing else
    let mut overrides = BTreeMap::new();
    overrides.insert("showThousandsSeparators".to_string(), "true".to_string());
    s.run(Command::SetOverrides { values: overrides }).unwrap();
    assert_eq!(tree(&s.user), quiet, "overrides are not written");
    s.run(Command::SetBuildSort {
        sort: BuildSort::Level,
    })
    .unwrap();
    let after = tree(&s.user);
    let changed: Vec<&String> = after
        .iter()
        .filter(|(k, v)| quiet.get(*k).map(|q| &q.0) != Some(&v.0))
        .map(|(k, _)| k)
        .collect();
    assert_eq!(
        changed,
        ["Path of Building/Settings.xml"],
        "only Settings.xml"
    );
    let misc = misc_attributes(&String::from_utf8(s.settings_bytes()).unwrap());
    assert_eq!(misc.get("buildSortMode").map(String::as_str), Some("LEVEL"));
    assert_eq!(
        misc.get("showThousandsSeparators").map(String::as_str),
        Some("false"),
        "the override stays out of the file"
    );
    assert!(s.eval::<bool>("return pob.main.main.showThousandsSeparators"));
    // the same choice again changes nothing
    s.run(Command::SetBuildSort {
        sort: BuildSort::Level,
    })
    .unwrap();
    assert_eq!(tree(&s.user), after);
    assert_eq!(list(&mut s, "").sort, BuildSort::Level);
    assert_eq!(s.open_popups(), 0);
}

/// Data-loss guards (the review's I1 to I4): a recursive delete is confined to Builds, a delete
/// is the one the user was asked about, a rename or copy never replaces or writes through
/// anything, and Move and Copy refuse any link.
#[test]
#[ignore = "needs the pinned PoB install"]
fn build_list_data_loss_guards() {
    use std::os::unix::fs::symlink;
    let _guard = lock();
    let mut s = LongtailSession::start("build_list_guards", &session_settings(LIST_MODE));
    let builds = settings_folder(&s.user).join("Builds");
    seed(&builds);
    let outside = settings_folder(&s.user).join("outside");
    fs::create_dir_all(outside.join("deep")).unwrap();
    fs::write(outside.join("sentinel.xml"), "sentinel").unwrap();
    fs::write(outside.join("deep/more.txt"), "more").unwrap();
    let outside_before = tree(&outside);
    let err = |s: &mut LongtailSession, command: Command| {
        let what = format!("{command:?}");
        s.run(command).expect_err(&what).to_string()
    };
    let delete = |path: PathBuf, kind| Command::DeleteBuild { path, kind };

    // I1: the native refuses a recursive delete anywhere but below Builds
    let refused = "refused: outside the builds folder";
    let try_remove = |s: &LongtailSession, path: &Path| -> (Option<bool>, Option<String>) {
        s.backend
            .lua
            .globals()
            .set("TARGET", path.display().to_string())
            .unwrap();
        s.eval("return RemoveDir(TARGET, true)")
    };
    assert_eq!(
        try_remove(&s, &builds.join("sub")).1.as_deref(),
        Some(refused),
        "nothing armed"
    );
    assert!(builds.join("sub/b.xml").is_file());
    // a command arms the root
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    s.run(Command::RenameBuild {
        path: builds.join("dot.xml"),
        name: "Dot".into(),
    })
    .expect("a case-only rename of its own name");
    s.run(Command::RenameBuild {
        path: builds.join("Dot.xml"),
        name: "dot".into(),
    })
    .unwrap();
    for path in [
        settings_folder(&s.user),
        outside.clone(),
        builds.clone(),
        builds.join("../outside"),
    ] {
        assert_eq!(
            try_remove(&s, &path).1.as_deref(),
            Some(refused),
            "{path:?}"
        );
    }
    symlink(&outside, builds.join("escape")).unwrap();
    assert_eq!(
        try_remove(&s, &builds.join("escape/deep")).1.as_deref(),
        Some(refused),
        "below a link"
    );
    // the link itself goes, what it leads to stays
    assert_eq!(try_remove(&s, &builds.join("escape")), (Some(true), None));
    assert!(fs::symlink_metadata(builds.join("escape")).is_err());
    assert_eq!(tree(&outside), outside_before);

    // I2: a delete is the one the user was asked about
    let sub = builds.join("sub");
    let empty = builds.join("empty");
    fs::write(empty.join("late.txt"), "arrived after the listing").unwrap();
    let changed = "the folder changed; look again";
    assert_eq!(
        err(&mut s, delete(empty.clone(), DeleteKind::EmptyFolder)),
        changed
    );
    assert!(empty.join("late.txt").is_file(), "the file survives");
    assert_eq!(
        err(&mut s, delete(sub.clone(), DeleteKind::EmptyFolder)),
        changed
    );
    assert_eq!(err(&mut s, delete(sub.clone(), DeleteKind::Build)), changed);
    assert!(sub.join("b.xml").is_file());
    fs::remove_file(empty.join("late.txt")).unwrap();
    assert_eq!(
        err(&mut s, delete(empty.clone(), DeleteKind::Folder)),
        changed
    );
    assert_eq!(
        err(
            &mut s,
            delete(builds.join("a.xml"), DeleteKind::EmptyFolder)
        ),
        changed
    );
    assert!(empty.is_dir() && builds.join("a.xml").is_file());
    assert_eq!(s.open_popups(), 0);

    // I3: a name that is taken, in any case or as a dangling link, is never written
    fs::write(builds.join("Zed.xml"), "Zed's bytes").unwrap();
    let taken = "a build with that name already exists";
    assert_eq!(
        err(
            &mut s,
            Command::RenameBuild {
                path: builds.join("dot.xml"),
                name: "zed".into()
            }
        ),
        taken
    );
    assert_eq!(
        err(
            &mut s,
            Command::CopyBuild {
                path: builds.join("dot.xml"),
                name: "ZED".into()
            }
        ),
        taken
    );
    assert_eq!(fs::read(builds.join("Zed.xml")).unwrap(), b"Zed's bytes");
    symlink(outside.join("made.xml"), builds.join("dang.xml")).unwrap();
    for command in [
        Command::CopyBuild {
            path: builds.join("dot.xml"),
            name: "dang".into(),
        },
        Command::RenameBuild {
            path: builds.join("dot.xml"),
            name: "dang".into(),
        },
    ] {
        assert_eq!(err(&mut s, command), taken);
    }
    symlink(outside.join("moved.xml"), sub.join("dot.xml")).unwrap();
    assert_eq!(
        err(
            &mut s,
            Command::MoveBuild {
                path: builds.join("dot.xml"),
                folder: "sub/".into()
            }
        ),
        taken
    );
    // and a copy into a folder, onto a link at the name PoB would try
    assert_eq!(
        err(
            &mut s,
            Command::CopyBuildTo {
                path: builds.join("dot.xml"),
                folder: "sub/".into()
            }
        ),
        taken
    );
    assert!(builds.join("dot.xml").is_file());
    fs::remove_file(builds.join("dang.xml")).unwrap();
    fs::remove_file(sub.join("dot.xml")).unwrap();
    assert_eq!(
        tree(&outside),
        outside_before,
        "nothing was written through a link"
    );

    // I4: a chain of links, two hops out, is refused by Move and Copy
    let escapes = "that path is outside the Builds folder";
    fs::create_dir_all(builds.join("x")).unwrap();
    fs::create_dir_all(builds.join("y")).unwrap();
    symlink(builds.join("y"), builds.join("x/l")).unwrap();
    symlink(&outside, builds.join("y/out")).unwrap();
    symlink(&sub, builds.join("inner")).unwrap();
    for command in [
        Command::MoveBuild {
            path: builds.join("x"),
            folder: "sub/".into(),
        },
        Command::CopyBuild {
            path: builds.join("x"),
            name: "x2".into(),
        },
        Command::MoveBuild {
            path: builds.join("a.xml"),
            folder: "x/l/".into(),
        },
        Command::MoveBuild {
            path: builds.join("inner/b.xml"),
            folder: String::new(),
        },
        Command::CopyBuild {
            path: builds.join("inner/b.xml"),
            name: "b2".into(),
        },
        Command::MoveBuild {
            path: builds.join("inner"),
            folder: "empty/".into(),
        },
        Command::CopyBuildTo {
            path: builds.join("x"),
            folder: "sub/".into(),
        },
        Command::CopyBuildTo {
            path: builds.join("a.xml"),
            folder: "x/l/".into(),
        },
        Command::CopyBuildTo {
            path: builds.join("inner/b.xml"),
            folder: String::new(),
        },
    ] {
        let what = format!("{command:?}");
        assert_eq!(err(&mut s, command), escapes, "{what}");
    }
    assert!(builds.join("x/l").is_symlink() && sub.join("b.xml").is_file());
    assert_eq!(
        tree(&outside),
        outside_before,
        "nothing was taken from or put in outside"
    );
    assert_eq!(s.open_popups(), 0);

    // N3: a link to a folder inside Builds is listed as a folder; it is renamed as one and can
    // be deleted, unlinking only the link
    symlink(&sub, builds.join("lnk")).unwrap();
    fs::write(builds.join("plain"), "plain file").unwrap();
    assert_eq!(
        err(
            &mut s,
            Command::RenameBuild {
                path: builds.join("lnk"),
                name: "plain".into()
            }
        ),
        taken
    );
    assert_eq!(fs::read(builds.join("plain")).unwrap(), b"plain file");
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .unwrap();
    let view = s.backend.build_list_view().unwrap();
    let entry = view
        .entries
        .iter()
        .find(|e| e.name == "lnk")
        .expect("listed");
    assert!(entry.is_folder && entry.delete_prompt.starts_with("The folder is not empty"));
    s.run(delete(builds.join("lnk"), entry.delete_kind()))
        .expect("delete the link");
    assert!(fs::symlink_metadata(builds.join("lnk")).is_err());
    assert!(sub.join("b.xml").is_file(), "its target stays");
    symlink(&empty, builds.join("lnk")).unwrap();
    s.run(delete(builds.join("lnk"), DeleteKind::EmptyFolder))
        .expect("an empty folder's link");
    assert!(fs::symlink_metadata(builds.join("lnk")).is_err() && empty.is_dir());
    fs::remove_file(builds.join("plain")).unwrap();

    // N4: a move onto a file nobody can read is refused (a readable one becomes name[2])
    if !is_root() {
        fs::write(sub.join("a.xml"), "hidden").unwrap();
        set_mode(&sub.join("a.xml"), 0o000);
        let result = s.run(Command::MoveBuild {
            path: builds.join("a.xml"),
            folder: "sub/".into(),
        });
        set_mode(&sub.join("a.xml"), 0o644);
        assert_eq!(result.expect_err("refused").to_string(), taken);
        assert_eq!(fs::read(sub.join("a.xml")).unwrap(), b"hidden");
        fs::remove_file(sub.join("a.xml")).unwrap();
    }

    // F1: PoB's name[2], name[3] probe: the first name it can't open is replaced by a move
    fs::write(sub.join("a.xml"), "readable").unwrap();
    fs::write(sub.join("a[2].xml"), "second").unwrap();
    symlink(outside.join("gone.xml"), sub.join("a[3].xml")).unwrap();
    assert_eq!(
        err(
            &mut s,
            Command::MoveBuild {
                path: builds.join("a.xml"),
                folder: "sub/".into()
            }
        ),
        taken,
        "a dangling link at a[3]"
    );
    fs::remove_file(sub.join("a[3].xml")).unwrap();
    if !is_root() {
        fs::write(sub.join("a[3].xml"), "hidden").unwrap();
        set_mode(&sub.join("a[3].xml"), 0o000);
        let result = s.run(Command::MoveBuild {
            path: builds.join("a.xml"),
            folder: "sub/".into(),
        });
        set_mode(&sub.join("a[3].xml"), 0o644);
        assert_eq!(result.expect_err("refused").to_string(), taken);
        assert_eq!(fs::read(sub.join("a[3].xml")).unwrap(), b"hidden");
        fs::remove_file(sub.join("a[3].xml")).unwrap();
    }
    assert!(builds.join("a.xml").is_file());
    for name in ["a.xml", "a[2].xml"] {
        fs::remove_file(sub.join(name)).unwrap();
    }
    assert_eq!(tree(&outside), outside_before);

    // N5: a.xml and A.xml both exist, a -> A
    fs::copy(builds.join("a.xml"), builds.join("A.xml")).unwrap();
    fs::write(builds.join("A.xml"), "the other one").unwrap();
    assert_eq!(
        err(
            &mut s,
            Command::RenameBuild {
                path: builds.join("a.xml"),
                name: "A".into()
            }
        ),
        taken
    );
    assert_eq!(fs::read(builds.join("A.xml")).unwrap(), b"the other one");
    fs::remove_file(builds.join("A.xml")).unwrap();

    // N1: a Builds folder that holds the user folder arms nothing and the list says so
    let user = s.user.clone();
    let real = s.eval::<String>("return pob.main.main.buildPath");
    s.backend
        .lua
        .globals()
        .set("NEWROOT", format!("{}/", user.display()))
        .unwrap();
    s.eval::<()>("pob.main.main.buildPath = NEWROOT");
    for command in [
        Command::ListBuilds {
            folder: String::new(),
        },
        delete(builds.join("sub"), DeleteKind::Folder),
    ] {
        assert_eq!(err(&mut s, command), "that Builds folder isn't allowed");
    }
    assert!(builds.join("sub/b.xml").is_file());
    // M1: a root that can't be resolved disarms the one armed before it
    s.backend
        .lua
        .globals()
        .set("NEWROOT", real.clone())
        .unwrap();
    s.eval::<()>("pob.main.main.buildPath = NEWROOT");
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .expect("the real root arms");
    s.backend
        .lua
        .globals()
        .set("NEWROOT", format!("{}/nowhere/", user.display()))
        .unwrap();
    s.eval::<()>("pob.main.main.buildPath = NEWROOT");
    assert_eq!(
        err(
            &mut s,
            Command::ListBuilds {
                folder: String::new()
            }
        ),
        "the Builds folder doesn't exist"
    );
    assert_eq!(
        try_remove(&s, &builds.join("sub")).1.as_deref(),
        Some(refused),
        "the old root is disarmed"
    );
    assert!(builds.join("sub/b.xml").is_file());
    s.backend.lua.globals().set("NEWROOT", real).unwrap();
    s.eval::<()>("pob.main.main.buildPath = NEWROOT");
    s.run(Command::ListBuilds {
        folder: String::new(),
    })
    .expect("the real root again");
    assert_eq!(s.open_popups(), 0);

    // Save As is as strict as the list's commands: no link on the way to its folder, a new
    // folder included, and none at the file an overwrite would write through
    s.run(Command::OpenBuild {
        path: builds.join("dot.xml"),
    })
    .unwrap();
    symlink(outside.join("written.xml"), builds.join("dang.xml")).unwrap();
    symlink(outside.join("sentinel.xml"), sub.join("over.xml")).unwrap();
    let before_files = tree(&builds);
    for (folder, name) in [
        ("x/l/", "through"),
        ("inner/", "through"),
        ("inner/made/", "through"),
        ("x/l/made/", "through"),
        ("", "dang"),
        ("sub/", "over"),
    ] {
        for overwrite in [false, true] {
            let command = Command::SaveAs {
                folder: folder.into(),
                name: name.into(),
                overwrite,
            };
            let what = format!("{command:?}");
            assert_eq!(err(&mut s, command), escapes, "{what}");
        }
    }
    assert_eq!(tree(&builds), before_files, "Save As wrote in Builds");
    assert_eq!(
        tree(&outside),
        outside_before,
        "Save As wrote through a link"
    );
    assert!(!sub.join("made").exists());
    // a real folder, new or not, still takes it
    s.run(Command::SaveAs {
        folder: "sub/made/".into(),
        name: "saved".into(),
        overwrite: false,
    })
    .expect("Save As into a new real folder");
    assert!(sub.join("made/saved.xml").is_file());
    assert_eq!(s.open_popups(), 0);
}
