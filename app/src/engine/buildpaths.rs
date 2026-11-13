//! Where a build list command may reach (R16): inside the user's Builds folder, symlinks
//! resolved. The glue repeats the lexical part of these checks; symlinks need the file system.

use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Component, Path, PathBuf};

const OUTSIDE: &str = "that path is outside the Builds folder";

fn outside() -> anyhow::Error {
    anyhow!(OUTSIDE)
}

/// `root` as PoB holds it (`main.buildPath`), with a closing slash
fn root_dir(root: &str) -> String {
    if root.ends_with('/') {
        root.to_string()
    } else {
        format!("{root}/")
    }
}

/// A folder below the Builds folder as the views spell it (escaped, `bytes_text`), as the path
/// of its own bytes: a name that isn't UTF-8 is listed escaped and found as itself
fn os_folder(folder: &str) -> PathBuf {
    super::bytes_text::path_from_escaped(folder)
}

/// A path below the Builds folder as the views spell it
fn spelled(rel: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    super::bytes_text::escape_name(rel.as_os_str().as_bytes()).into_owned()
}

fn real_root(root: &str) -> Result<PathBuf> {
    fs::canonicalize(root).map_err(|_| anyhow!("no such build"))
}

/// Whether anything below `dir` is a link, or can't be read (a copy or a move would follow a
/// link, and a chain of links can lead out of `root` in two hops or loop)
fn tree_has_links(dir: &Path) -> bool {
    let Ok(read) = fs::read_dir(dir) else {
        return true;
    };
    read.into_iter().any(|entry| {
        let Ok(entry) = entry else {
            return true;
        };
        let path = entry.path();
        match fs::symlink_metadata(&path) {
            Err(_) => true,
            Ok(meta) if meta.file_type().is_symlink() => true,
            Ok(meta) => meta.is_dir() && tree_has_links(&path),
        }
    })
}

/// Whether any part of `rel` below `base` is a link
fn path_has_links(base: &Path, rel: &Path) -> bool {
    let mut walk = base.to_path_buf();
    rel.components().any(|part| {
        walk.push(part);
        fs::symlink_metadata(&walk).is_ok_and(|m| m.file_type().is_symlink())
    })
}

/// The Builds entry `path` names, as PoB spells it (`root` + the path below it), or the reason
/// it is refused. `path` must be a build file or a folder that exists below `root`; a link is
/// accepted only when its target is below `root` too. `walks` is true for Copy and Move, which
/// follow links: they refuse a link anywhere in the path or the tree.
///
/// The path is bytes, as PoB's are: PoB on Linux lists a file whose name isn't UTF-8, and
/// renames, copies, moves and deletes it as itself.
pub(crate) fn entry(root: &str, path: &Path, walks: bool) -> Result<PathBuf> {
    let root = root_dir(root);
    let root_real = real_root(&root)?;
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(outside());
    }
    let below = |base: &Path| {
        path.strip_prefix(base)
            .ok()
            .filter(|rel| rel.file_name().is_some())
    };
    let rel = below(Path::new(&root))
        .or_else(|| below(&root_real))
        .ok_or_else(outside)?;
    let name = rel.file_name().ok_or_else(outside)?;
    if walks && path_has_links(&root_real, rel) {
        return Err(outside());
    }
    let parent = root_real.join(rel.parent().unwrap_or(Path::new("")));
    let parent_real = fs::canonicalize(&parent).map_err(|_| anyhow!("no such build"))?;
    let parent_rel = parent_real
        .strip_prefix(&root_real)
        .map_err(|_| outside())?;
    let full = parent_real.join(name);
    let meta = fs::symlink_metadata(&full).map_err(|_| anyhow!("no such build"))?;
    if meta.file_type().is_symlink() {
        let target = fs::canonicalize(&full).map_err(|_| outside())?;
        if !target.starts_with(&root_real) {
            return Err(outside());
        }
    }
    if walks && full.is_dir() && tree_has_links(&full) {
        return Err(outside());
    }
    let mut spelled = std::ffi::OsString::from(&root);
    if !parent_rel.as_os_str().is_empty() {
        spelled.push(parent_rel.as_os_str());
        spelled.push("/");
    }
    spelled.push(name);
    Ok(PathBuf::from(spelled))
}

/// `bytes` lowercased as a name: its UTF-8 parts by Unicode, any other byte as it is (two
/// different such bytes never compare equal)
fn lower_name(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.extend_from_slice(chunk.valid().to_lowercase().as_bytes());
        out.extend_from_slice(chunk.invalid());
    }
    out
}

/// The Builds sub-path `folder` names, with a closing slash ("" for the root), or the reason it
/// is refused. Both are spelled as the views spell folders: escaped (`bytes_text`). A folder
/// that doesn't exist is an error when `must_exist`, and passes (unless it is spelled to
/// escape) otherwise.
/// `strict` refuses a link anywhere in the path.
pub(crate) fn folder(root: &str, folder: &str, must_exist: bool, strict: bool) -> Result<String> {
    if folder.starts_with('/')
        || folder.starts_with('\\')
        || folder
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".")
    {
        return Err(outside());
    }
    let lexical = || match folder {
        "" => String::new(),
        f if f.ends_with('/') => f.to_string(),
        f => format!("{f}/"),
    };
    let root_real = match real_root(&root_dir(root)) {
        Ok(real) => real,
        // a Builds folder that isn't made yet lists as empty
        Err(_) if !must_exist => return Ok(lexical()),
        Err(e) => return Err(e),
    };
    let os = os_folder(folder);
    if strict && path_has_links(&root_real, &os) {
        return Err(outside());
    }
    let joined = root_real.join(&os);
    let real = match fs::canonicalize(&joined) {
        Ok(real) => real,
        Err(_) if !must_exist => return Ok(lexical()),
        Err(_) => return Err(anyhow!("no such folder")),
    };
    let rel = real.strip_prefix(&root_real).map_err(|_| outside())?;
    if !real.is_dir() {
        return Err(anyhow!("no such folder"));
    }
    let rel = spelled(rel);
    Ok(if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    })
}

/// Refuses Save As onto `name.xml` in `folder` (checked by [`folder`]) when that is a link,
/// a dangling one included: the save would write through it.
/// `name` is bytes, as PoB's are.
pub(crate) fn save_target(root: &str, folder: &str, name: &[u8]) -> Result<()> {
    use std::os::unix::ffi::OsStringExt;
    let file = std::ffi::OsString::from_vec([name, b".xml"].concat());
    let target = Path::new(&root_dir(root))
        .join(os_folder(folder))
        .join(file);
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(outside());
    }
    Ok(())
}

/// Refuses a new name that is taken in the entry's folder, exactly or as a case variant, a
/// link (also a dangling one) included: a rename or copy would replace or write through it.
/// `entry` is the approved source; `rename` lets the entry's own name through when only its
/// case changes. Names are bytes, as PoB's are.
pub(crate) fn name_free(entry: &Path, new_name: &[u8], rename: bool) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let taken = || anyhow!("a build with that name already exists");
    let (Some(dir), Some(own)) = (entry.parent(), entry.file_name().map(OsStrExt::as_bytes)) else {
        return Err(anyhow!("no such build"));
    };
    // a link to a folder is a folder, as PoB treats it (it renames it to the bare name)
    let is_folder = fs::metadata(entry).is_ok_and(|m| m.is_dir());
    let mut target = new_name.to_vec();
    if !is_folder {
        target.extend_from_slice(b".xml");
    }
    if rename && target == own {
        return Err(taken());
    }
    let lower = lower_name(&target);
    for found in fs::read_dir(dir).map_err(|_| anyhow!("no such build"))? {
        let found = found.map_err(|_| taken())?;
        let name = found.file_name();
        let name = name.as_bytes();
        // a case variant counts as taken: stricter than PoB on Linux, by ruling (N2)
        if lower_name(name) == lower && !(rename && name == own) {
            return Err(taken());
        }
    }
    Ok(())
}

/// The most folders and levels the tree lists (a Builds folder is never this deep)
const TREE_FOLDERS: usize = 2000;
const TREE_DEPTH: usize = 12;

// How often the tree walk read a folder or looked at an entry's metadata, counted in tests
#[cfg(test)]
thread_local! {
    static WALK_COUNTS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

fn walk_read(dir: &Path) -> std::io::Result<fs::ReadDir> {
    #[cfg(test)]
    WALK_COUNTS.with(|c| c.set((c.get().0 + 1, c.get().1)));
    fs::read_dir(dir)
}

fn walk_stat(path: &Path) -> std::io::Result<fs::Metadata> {
    #[cfg(test)]
    WALK_COUNTS.with(|c| c.set((c.get().0, c.get().1 + 1)));
    fs::symlink_metadata(path)
}

/// Every folder below `root`, links not followed (a link could lead out, or loop), shallowest
/// first, each in name order. Each folder is read once, and only folders are looked at: a
/// file's type comes from the listing, so a Builds folder of many builds costs no stat per build.
pub(crate) fn folder_tree(root: &str) -> Vec<crate::engine::views::FolderNode> {
    use crate::engine::views::FolderNode;
    let mut out: Vec<FolderNode> = Vec::new();
    // a folder to read, its path below the root, and where it is in `out` (none for the root)
    let mut level = vec![(PathBuf::from(root_dir(root)), String::new(), None)];
    for depth in 0..=TREE_DEPTH {
        let mut next = Vec::new();
        for (dir, rel, at) in level {
            let Ok(read) = walk_read(&dir) else {
                continue;
            };
            let mut any = false;
            let mut folders = Vec::new();
            for entry in read {
                any = true;
                // below the deepest level only whether a folder is empty counts
                if depth == TREE_DEPTH {
                    break;
                }
                let Ok(entry) = entry else {
                    continue;
                };
                // the listing's own type: a link is never a folder here
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    folders.push(entry);
                }
            }
            if let Some(at) = at {
                let node: &mut FolderNode = &mut out[at];
                node.empty = !any;
            }
            folders.sort_by_key(|e| e.file_name());
            for entry in folders {
                if out.len() >= TREE_FOLDERS {
                    break;
                }
                // a name that isn't UTF-8 is a folder too, spelled escaped as the list is
                let name = spelled(Path::new(&entry.file_name()));
                // looked at again, not followed: it may have changed since the listing
                let Ok(meta) = walk_stat(&entry.path()) else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                let path = format!("{rel}{name}/");
                out.push(FolderNode {
                    path: path.clone(),
                    modified: meta
                        .modified()
                        .ok()
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs()),
                    empty: false,
                });
                next.push((entry.path(), path, Some(out.len() - 1)));
            }
        }
        level = next;
    }
    out
}

/// The build files directly in `folder` below `root`, for the Compare tab's picker: regular
/// `.xml` files only (a link is never followed, nor a folder spelled through one), in name
/// order, each in PoB's spelling. Reads the disk and nothing else.
pub(crate) fn browse(root: &str, folder: &str) -> Result<crate::engine::views::BuildBrowseView> {
    use crate::engine::views::{BuildBrowseEntry, BuildBrowseView};
    // a Builds folder that went is a missing folder to the picker, not a missing build
    let real = real_root(&root_dir(root)).map_err(|_| anyhow!("no such folder"))?;
    let rel = self::folder(root, folder, true, true)?;
    let dir = real.join(os_folder(&rel));
    let mut entries = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|_| anyhow!("no such folder"))? {
        let Ok(entry) = entry else {
            continue;
        };
        // the name first: a folder of other files costs no stat per entry
        let file = entry.file_name();
        let Some(stem) =
            std::os::unix::ffi::OsStrExt::as_bytes(file.as_os_str()).strip_suffix(b".xml")
        else {
            continue;
        };
        // the entry's own type, a link's included: never followed
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() || stem.is_empty() {
            continue;
        }
        // a name that isn't UTF-8 is escaped, and its path is its own bytes
        entries.push(BuildBrowseEntry {
            name: super::bytes_text::escape_name(stem).into_owned(),
            path: Path::new(&root_dir(root)).join(os_folder(&rel)).join(&file),
        });
    }
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(BuildBrowseView {
        folder: rel,
        entries,
        error: None,
    })
}

/// Refuses a move that PoB's `GetDestName` would turn into a replacement. It tries `name`,
/// `name[2]`, `name[3]` and so on while `io.open` succeeds (a readable file or a folder) and
/// takes the first it can't open; `os.rename` then replaces whatever is there, a link or an
/// unreadable file included.
pub(crate) fn move_target_free(entry: &Path, root: &str, dest_folder: &str) -> Result<()> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let Some(file) = entry.file_name().map(OsStrExt::as_bytes) else {
        return Err(anyhow!("no such build"));
    };
    let dir = Path::new(&root_dir(root)).join(os_folder(dest_folder));
    let base = file.strip_suffix(b".xml").unwrap_or(file);
    let mut candidate = file.to_vec();
    let mut i = 2;
    loop {
        let target = dir.join(std::ffi::OsString::from_vec(candidate));
        if fs::File::open(&target).is_err() {
            return match fs::symlink_metadata(&target) {
                Ok(_) => Err(anyhow!("a build with that name already exists")),
                Err(_) => Ok(()),
            };
        }
        candidate = [base, format!("[{i}].xml").as_bytes()].concat();
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Tree(PathBuf);
    impl Tree {
        fn new(tag: &str) -> Tree {
            let top =
                std::env::temp_dir().join(format!("auto-pob-bp-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&top);
            fs::create_dir_all(top.join("Builds/sub")).unwrap();
            fs::create_dir_all(top.join("outside")).unwrap();
            fs::write(top.join("Builds/a.xml"), "a").unwrap();
            fs::write(top.join("Builds/sub/b.xml"), "b").unwrap();
            fs::write(top.join("outside/s.xml"), "s").unwrap();
            Tree(top)
        }
        fn root(&self) -> String {
            format!("{}/", self.0.join("Builds").display())
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn refusal<T: std::fmt::Debug>(result: Result<T>) -> String {
        result.expect_err("should be refused").to_string()
    }

    #[test]
    fn entries_inside_pass_in_pob_spelling() {
        let tree = Tree::new("in");
        let root = tree.root();
        let a = tree.0.join("Builds/a.xml");
        let spelled = |rel: &str| PathBuf::from(format!("{root}{rel}"));
        assert_eq!(entry(&root, &a, false).unwrap(), spelled("a.xml"));
        let b = tree.0.join("Builds/sub/b.xml");
        assert_eq!(entry(&root, &b, true).unwrap(), spelled("sub/b.xml"));
        let sub = tree.0.join("Builds/sub");
        assert_eq!(entry(&root, &sub, true).unwrap(), spelled("sub"));
    }

    /// A file whose name isn't UTF-8 is an entry as itself, as PoB on Linux lists, renames,
    /// copies, moves and deletes it; its name is told apart from other such names
    #[test]
    fn a_name_that_isnt_utf8_is_an_entry_as_its_own_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let tree = Tree::new("latin");
        let root = tree.root();
        let os = |b: &[u8]| std::ffi::OsStr::from_bytes(b).to_owned();
        let path = tree.0.join("Builds").join(os(b"caf\xE9.xml"));
        fs::write(&path, "x").unwrap();
        let spelled = entry(&root, &path, true).unwrap();
        assert_eq!(
            spelled.as_os_str().as_bytes(),
            [root.as_bytes(), b"caf\xE9.xml"].concat()
        );
        // its own name, or the same in another case, is taken; another byte is not
        assert!(name_free(&spelled, b"caf\xE9", false).is_err());
        assert!(name_free(&spelled, b"CAF\xE9", false).is_err());
        assert!(name_free(&spelled, b"caf\xE8", false).is_ok());
        assert!(name_free(&spelled, b"caf\xE9 2", false).is_ok());
        fs::write(tree.0.join("Builds").join(os(b"caf\xE8.xml")), "y").unwrap();
        assert!(name_free(&spelled, b"caf\xE8", true).is_err());
        // a move to sub/ is free until sub/ holds a dangling link of its name
        assert!(move_target_free(&spelled, &root, "sub/").is_ok());
        symlink(
            tree.0.join("outside/gone.xml"),
            tree.0.join("Builds/sub").join(os(b"caf\xE9.xml")),
        )
        .unwrap();
        assert!(move_target_free(&spelled, &root, "sub/").is_err());
    }

    #[test]
    fn entries_outside_are_refused() {
        let tree = Tree::new("out");
        let root = tree.root();
        let outside_text = OUTSIDE;
        for path in [
            tree.0.join("outside/s.xml"),
            tree.0.join("Builds/../outside/s.xml"),
            tree.0.join("Builds/sub/../../outside/s.xml"),
            PathBuf::from("/tmp/x.xml"),
            PathBuf::from("a.xml"),
            tree.0.join("Builds"),
        ] {
            assert_eq!(
                refusal(entry(&root, &path, false)),
                outside_text,
                "{path:?}"
            );
        }
        assert_eq!(
            refusal(entry(&root, &tree.0.join("Builds/none.xml"), false)),
            "no such build"
        );
    }

    #[test]
    fn links_out_of_the_folder_are_refused() {
        let tree = Tree::new("link");
        let root = tree.root();
        symlink(tree.0.join("outside"), tree.0.join("Builds/escape")).unwrap();
        symlink(tree.0.join("outside/s.xml"), tree.0.join("Builds/s.xml")).unwrap();
        symlink(tree.0.join("Builds/sub"), tree.0.join("Builds/inner")).unwrap();
        for path in ["Builds/escape", "Builds/escape/s.xml", "Builds/s.xml"] {
            assert_eq!(
                refusal(entry(&root, &tree.0.join(path), false)),
                OUTSIDE,
                "{path}"
            );
        }
        // a link that stays inside is fine, and is spelled by where it leads
        assert_eq!(
            entry(&root, &tree.0.join("Builds/inner/b.xml"), false).unwrap(),
            PathBuf::from(format!("{root}sub/b.xml"))
        );
        assert!(entry(&root, &tree.0.join("Builds/inner"), false).is_ok());
    }

    #[test]
    fn copy_and_move_refuse_every_link_in_the_path_or_tree() {
        let tree = Tree::new("chain");
        let root = tree.root();
        // two hops: Builds/x/l -> Builds/y, Builds/y/out -> outside
        fs::create_dir_all(tree.0.join("Builds/x")).unwrap();
        fs::create_dir_all(tree.0.join("Builds/y")).unwrap();
        symlink(tree.0.join("Builds/y"), tree.0.join("Builds/x/l")).unwrap();
        symlink(tree.0.join("outside"), tree.0.join("Builds/y/out")).unwrap();
        let x = tree.0.join("Builds/x");
        assert_eq!(refusal(entry(&root, &x, true)), OUTSIDE);
        assert!(
            entry(&root, &x, false).is_ok(),
            "rename and delete don't follow"
        );
        // a link to a folder inside Builds is a link too
        symlink(tree.0.join("Builds/sub"), tree.0.join("Builds/inner")).unwrap();
        assert_eq!(
            refusal(entry(&root, &tree.0.join("Builds/inner/b.xml"), true)),
            OUTSIDE
        );
        assert_eq!(
            refusal(entry(&root, &tree.0.join("Builds/inner"), true)),
            OUTSIDE
        );
        assert_eq!(refusal(folder(&root, "inner", true, true)), OUTSIDE);
        assert!(folder(&root, "inner", true, false).is_ok());
        // a loop
        symlink(tree.0.join("Builds/loop"), tree.0.join("Builds/loop2")).unwrap();
        fs::create_dir_all(tree.0.join("Builds/loop")).unwrap();
        symlink(tree.0.join("Builds/loop"), tree.0.join("Builds/loop/self")).unwrap();
        assert_eq!(
            refusal(entry(&root, &tree.0.join("Builds/loop"), true)),
            OUTSIDE
        );
    }

    #[test]
    fn names_are_free_only_when_nothing_has_them_in_any_case() {
        let tree = Tree::new("names");
        let a = PathBuf::from(format!("{}a.xml", tree.root()));
        let err = |r: Result<()>| r.expect_err("taken").to_string();
        let taken = "a build with that name already exists";
        fs::write(tree.0.join("Builds/Other.xml"), "o").unwrap();
        symlink(
            tree.0.join("outside/gone.xml"),
            tree.0.join("Builds/dangling.xml"),
        )
        .unwrap();
        assert!(name_free(&a, b"fresh", false).is_ok());
        assert_eq!(err(name_free(&a, b"a", false)), taken);
        assert_eq!(err(name_free(&a, b"a", true)), taken);
        assert_eq!(err(name_free(&a, b"other", true)), taken, "a case variant");
        assert_eq!(err(name_free(&a, b"OTHER", false)), taken);
        assert_eq!(
            err(name_free(&a, b"dangling", false)),
            taken,
            "a dangling link"
        );
        // only the case of its own name changes
        assert!(name_free(&a, b"A", true).is_ok());
        // both a.xml and A.xml exist: a -> A would replace A.xml
        fs::write(tree.0.join("Builds/A.xml"), "A").unwrap();
        assert_eq!(err(name_free(&a, b"A", true)), taken);
        // a link to a folder is a folder: its new name is the bare name, not name.xml
        fs::write(tree.0.join("Builds/plain"), "p").unwrap();
        let link = tree.0.join("Builds/lnk");
        symlink(tree.0.join("Builds/sub"), &link).unwrap();
        assert_eq!(err(name_free(&link, b"plain", true)), taken);
    }

    #[test]
    fn the_folder_tree_lists_real_folders_only() {
        let tree = Tree::new("tree");
        fs::create_dir_all(tree.0.join("Builds/sub/deep")).unwrap();
        fs::create_dir_all(tree.0.join("Builds/.dot")).unwrap();
        symlink(tree.0.join("outside"), tree.0.join("Builds/escape")).unwrap();
        symlink(tree.0.join("Builds/sub"), tree.0.join("Builds/sub/loop")).unwrap();
        let folders = folder_tree(&tree.root());
        let listed: Vec<(&str, bool)> =
            folders.iter().map(|f| (f.path.as_str(), f.empty)).collect();
        // shallowest first; the links are not folders here; `sub` holds b.xml and `deep`
        assert_eq!(
            listed,
            [(".dot/", true), ("sub/", false), ("sub/deep/", true)]
        );
        assert!(folders.iter().all(|f| f.modified > 0));
    }

    #[test]
    fn the_folder_tree_looks_only_at_folders() {
        let tree = Tree::new("treecost");
        fs::create_dir_all(tree.0.join("Builds/sub/deep")).unwrap();
        for i in 0..50 {
            fs::write(tree.0.join(format!("Builds/b{i}.xml")), "b").unwrap();
            fs::write(tree.0.join(format!("Builds/sub/s{i}.xml")), "s").unwrap();
        }
        WALK_COUNTS.with(|c| c.set((0, 0)));
        let folders = folder_tree(&tree.root());
        assert_eq!(folders.len(), 2);
        // the root and each folder read once; a stat per folder, none per build
        assert_eq!(WALK_COUNTS.with(|c| c.get()), (3, 2));
    }

    #[test]
    fn the_folder_tree_stops_at_its_depth_and_still_knows_empty_folders() {
        let tree = Tree::new("treedepth");
        let mut deep = tree.0.join("Builds/sub");
        for i in 0..TREE_DEPTH + 2 {
            deep.push(format!("d{i}"));
        }
        fs::create_dir_all(&deep).unwrap();
        let folders = folder_tree(&tree.root());
        assert_eq!(folders.len(), TREE_DEPTH);
        let last = folders.last().unwrap();
        assert_eq!(last.path.matches('/').count(), TREE_DEPTH);
        assert!(!last.empty, "the deepest listed folder holds one more");
        assert!(folders.iter().all(|f| !f.empty));
    }

    #[test]
    fn a_move_never_replaces_what_the_probe_cannot_see() {
        use std::os::unix::fs::PermissionsExt;
        let tree = Tree::new("movetarget");
        let root = tree.root();
        let a = PathBuf::from(format!("{root}a.xml"));
        // sub/ holds b.xml (readable: PoB names the move b[2]) so move a.xml there
        assert!(move_target_free(&a, &root, "sub/").is_ok());
        let b = PathBuf::from(format!("{root}b.xml"));
        assert!(
            move_target_free(&b, &root, "sub/").is_ok(),
            "a readable file is PoB's [2]"
        );
        symlink(
            tree.0.join("outside/gone.xml"),
            tree.0.join("Builds/sub/a.xml"),
        )
        .unwrap();
        assert!(
            move_target_free(&a, &root, "sub/").is_err(),
            "a dangling link"
        );
        fs::remove_file(tree.0.join("Builds/sub/a.xml")).unwrap();
        let hidden = tree.0.join("Builds/sub/a.xml");
        fs::write(&hidden, "h").unwrap();
        fs::set_permissions(&hidden, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::File::open(&hidden).is_ok();
        assert!(
            readable || move_target_free(&a, &root, "sub/").is_err(),
            "an unreadable file"
        );
        fs::set_permissions(&hidden, fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_file(&hidden).unwrap();
        fs::create_dir_all(&hidden).unwrap();
        assert!(
            move_target_free(&a, &root, "sub/").is_ok(),
            "a folder: PoB's probe sees it"
        );
        fs::remove_dir_all(&hidden).unwrap();
        // PoB's name[2], name[3]: the first it can't open is replaced, so it must be free
        fs::write(&hidden, "readable").unwrap();
        assert!(move_target_free(&a, &root, "sub/").is_ok(), "a[2] is free");
        let second = tree.0.join("Builds/sub/a[2].xml");
        fs::write(&second, "h2").unwrap();
        assert!(
            move_target_free(&a, &root, "sub/").is_ok(),
            "a[2] readable, a[3] free"
        );
        fs::set_permissions(&second, fs::Permissions::from_mode(0o000)).unwrap();
        let can_read = fs::File::open(&second).is_ok();
        assert!(
            can_read || move_target_free(&a, &root, "sub/").is_err(),
            "unreadable a[2]"
        );
        fs::set_permissions(&second, fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_file(&second).unwrap();
        symlink(tree.0.join("outside/gone.xml"), &second).unwrap();
        assert!(
            move_target_free(&a, &root, "sub/").is_err(),
            "a dangling link at a[2]"
        );
        assert!(tree.0.join("outside/gone.xml").symlink_metadata().is_err());
    }

    #[test]
    fn a_folder_holding_a_link_out_is_refused_where_it_would_be_followed() {
        let tree = Tree::new("walk");
        let root = tree.root();
        symlink(tree.0.join("outside"), tree.0.join("Builds/sub/escape")).unwrap();
        let sub = tree.0.join("Builds/sub");
        assert_eq!(refusal(entry(&root, &sub, true)), OUTSIDE);
        // rename and delete don't follow it
        assert!(entry(&root, &sub, false).is_ok());
    }

    #[test]
    fn browsing_lists_build_files_only_and_never_leaves_the_folder() {
        let tree = Tree::new("browse");
        let root = tree.root();
        fs::write(tree.0.join("Builds/B second.xml"), "b").unwrap();
        fs::write(tree.0.join("Builds/notes.txt"), "n").unwrap();
        fs::create_dir_all(tree.0.join("Builds/dir.xml")).unwrap();
        symlink(
            tree.0.join("outside/s.xml"),
            tree.0.join("Builds/linked.xml"),
        )
        .unwrap();
        symlink(tree.0.join("outside"), tree.0.join("Builds/escape")).unwrap();
        let top = browse(&root, "").unwrap();
        assert_eq!(top.folder, "");
        let names: Vec<_> = top.entries.iter().map(|e| e.name.as_str()).collect();
        // no folder, link or other file; name order ignoring case
        assert_eq!(names, ["a", "B second"]);
        assert_eq!(top.entries[0].path, PathBuf::from(format!("{root}a.xml")));
        let sub = browse(&root, "sub").unwrap();
        assert_eq!(sub.folder, "sub/");
        assert_eq!(
            sub.entries[0].path,
            PathBuf::from(format!("{root}sub/b.xml"))
        );
        for bad in ["..", "../outside", "/tmp", "escape", "escape/"] {
            let e = browse(&root, bad).expect_err(bad).to_string();
            assert_eq!(e, OUTSIDE, "{bad}");
        }
        assert_eq!(
            browse(&root, "none").expect_err("missing").to_string(),
            "no such folder"
        );
        // a Builds folder that went is a missing folder too, not a missing build
        let gone = format!("{}/gone/", tree.0.display());
        assert_eq!(
            browse(&gone, "").expect_err("no root").to_string(),
            "no such folder"
        );
        // nothing was written or moved
        assert!(tree.0.join("Builds/a.xml").exists() && tree.0.join("outside/s.xml").exists());
    }

    #[test]
    fn folders() {
        let tree = Tree::new("folder");
        let root = tree.root();
        symlink(tree.0.join("outside"), tree.0.join("Builds/escape")).unwrap();
        assert_eq!(folder(&root, "", true, false).unwrap(), "");
        assert_eq!(folder(&root, "sub", true, false).unwrap(), "sub/");
        assert_eq!(folder(&root, "sub/", true, false).unwrap(), "sub/");
        for bad in [
            "..",
            "../outside",
            "sub/../..",
            "/tmp",
            "./sub",
            "escape",
            "escape/",
        ] {
            assert_eq!(refusal(folder(&root, bad, true, false)), OUTSIDE, "{bad}");
        }
        assert_eq!(
            refusal(folder(&root, "none", true, false)),
            "no such folder"
        );
        assert_eq!(
            refusal(folder(&root, "a.xml", true, false)),
            "no such folder"
        );
        // listing a folder that isn't there yet is allowed
        assert_eq!(folder(&root, "none", false, false).unwrap(), "none/");
        assert_eq!(refusal(folder(&root, "escape", false, false)), OUTSIDE);
    }
}
