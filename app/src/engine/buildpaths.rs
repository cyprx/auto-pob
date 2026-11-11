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

fn real_root(root: &str) -> Result<PathBuf> {
    fs::canonicalize(root).map_err(|_| anyhow!("no such build"))
}

/// Whether anything below `dir` is a link that leads out of `root` (a copy or a move would
/// follow it and take the files it leads to)
fn tree_escapes(dir: &Path, root: &Path) -> bool {
    let Ok(read) = fs::read_dir(dir) else {
        return false;
    };
    read.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            return false;
        };
        if meta.file_type().is_symlink() {
            return fs::canonicalize(&path).map_or(true, |real| !real.starts_with(root));
        }
        meta.is_dir() && tree_escapes(&path, root)
    })
}

/// The Builds entry `path` names, as PoB spells it (`root` + the path below it), or the reason
/// it is refused. `path` must be a build file or a folder that exists below `root`; a link is
/// accepted only when its target is below `root` too. `walks` is true for operations that
/// follow the links inside a folder (Copy and Move).
pub(crate) fn entry(root: &str, path: &Path, walks: bool) -> Result<String> {
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
    if walks && full.is_dir() && tree_escapes(&full, &root_real) {
        return Err(outside());
    }
    let parent_rel = parent_rel
        .to_str()
        .ok_or_else(|| anyhow!("no such build"))?;
    let name = name.to_str().ok_or_else(|| anyhow!("no such build"))?;
    let folder = if parent_rel.is_empty() {
        String::new()
    } else {
        format!("{parent_rel}/")
    };
    Ok(format!("{root}{folder}{name}"))
}

/// The Builds sub-path `folder` names, with a closing slash ("" for the root), or the reason it
/// is refused. A folder that doesn't exist is an error when `must_exist`, and passes (unless it
/// is spelled to escape) otherwise.
pub(crate) fn folder(root: &str, folder: &str, must_exist: bool) -> Result<String> {
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
    let joined = root_real.join(folder);
    let real = match fs::canonicalize(&joined) {
        Ok(real) => real,
        Err(_) if !must_exist => return Ok(lexical()),
        Err(_) => return Err(anyhow!("no such folder")),
    };
    let rel = real.strip_prefix(&root_real).map_err(|_| outside())?;
    if !real.is_dir() {
        return Err(anyhow!("no such folder"));
    }
    let rel = rel.to_str().ok_or_else(|| anyhow!("no such folder"))?;
    Ok(if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    })
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

    fn refusal(result: Result<String>) -> String {
        result.expect_err("should be refused").to_string()
    }

    #[test]
    fn entries_inside_pass_in_pob_spelling() {
        let tree = Tree::new("in");
        let root = tree.root();
        let a = tree.0.join("Builds/a.xml");
        assert_eq!(entry(&root, &a, false).unwrap(), format!("{root}a.xml"));
        let b = tree.0.join("Builds/sub/b.xml");
        assert_eq!(entry(&root, &b, true).unwrap(), format!("{root}sub/b.xml"));
        let sub = tree.0.join("Builds/sub");
        assert_eq!(entry(&root, &sub, true).unwrap(), format!("{root}sub"));
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
            format!("{root}sub/b.xml")
        );
        assert!(entry(&root, &tree.0.join("Builds/inner"), false).is_ok());
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
    fn folders() {
        let tree = Tree::new("folder");
        let root = tree.root();
        symlink(tree.0.join("outside"), tree.0.join("Builds/escape")).unwrap();
        assert_eq!(folder(&root, "", true).unwrap(), "");
        assert_eq!(folder(&root, "sub", true).unwrap(), "sub/");
        assert_eq!(folder(&root, "sub/", true).unwrap(), "sub/");
        for bad in [
            "..",
            "../outside",
            "sub/../..",
            "/tmp",
            "./sub",
            "escape",
            "escape/",
        ] {
            assert_eq!(refusal(folder(&root, bad, true)), OUTSIDE, "{bad}");
        }
        assert_eq!(refusal(folder(&root, "none", true)), "no such folder");
        assert_eq!(refusal(folder(&root, "a.xml", true)), "no such folder");
        // listing a folder that isn't there yet is allowed
        assert_eq!(folder(&root, "none", false).unwrap(), "none/");
        assert_eq!(refusal(folder(&root, "escape", false)), OUTSIDE);
    }
}
