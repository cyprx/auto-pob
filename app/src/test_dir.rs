//! A folder under the system temp dir for one test, removed when the test ends (passed or
//! panicked), so test runs leave nothing behind.

use std::path::{Path, PathBuf};

pub struct TestDir(PathBuf);

impl TestDir {
    /// `<temp>/auto-pob-<name>-<pid>`, empty and made.
    pub fn new(name: &str) -> Self {
        let dir = Self::reserved(name);
        std::fs::create_dir_all(&dir.0).unwrap();
        dir
    }

    /// The same path, cleared but not made: for code that should make it itself.
    pub fn reserved(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("auto-pob-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // a test that made a folder read-only gets it back writable first
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut stack = vec![self.0.clone()];
            while let Some(dir) = stack.pop() {
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        if entry.file_type().is_ok_and(|t| t.is_dir()) {
                            stack.push(entry.path());
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_goes_when_the_test_ends() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TestDir::new("testdir");
        let path = dir.to_path_buf();
        std::fs::create_dir(dir.join("ro")).unwrap();
        std::fs::write(dir.join("ro").join("f"), "x").unwrap();
        std::fs::set_permissions(dir.join("ro"), std::fs::Permissions::from_mode(0o555)).unwrap();
        drop(dir);
        assert!(!path.exists());
        // a panicking test still clears its folder
        let path = std::panic::catch_unwind(|| {
            let dir = TestDir::reserved("testdir-panic");
            std::fs::create_dir_all(&*dir).unwrap();
            std::panic::panic_any(dir.to_path_buf());
        })
        .unwrap_err()
        .downcast::<PathBuf>()
        .unwrap();
        assert!(!path.exists());
    }
}
