//! The game switch. Only Path of Exile 1 is supported; game-specific values live here.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Game {
    Poe1,
}

impl Game {
    /// GitHub repository that hosts this game's Path of Building.
    pub fn repo(self) -> &'static str {
        match self {
            Game::Poe1 => "PathOfBuildingCommunity/PathOfBuilding",
        }
    }

    /// The pinned Path of Building release.
    pub fn pob_version(self) -> &'static str {
        match self {
            Game::Poe1 => "2.67.2",
        }
    }

    /// Where the pinned PoB files are installed.
    pub fn install_dir(self, data_dir: &Path) -> PathBuf {
        match self {
            Game::Poe1 => data_dir
                .join("auto-pob")
                .join("poe1")
                .join(format!("pob-{}", self.pob_version())),
        }
    }

    /// Default user folder (shared with the classic app's layout).
    pub fn default_user_dir(self, data_dir: &Path) -> PathBuf {
        match self {
            Game::Poe1 => data_dir.join("RustyPathOfBuilding1").join("userdata"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poe1_paths() {
        let d = Path::new("/d");
        assert_eq!(Game::Poe1.repo(), "PathOfBuildingCommunity/PathOfBuilding");
        assert_eq!(Game::Poe1.pob_version(), "2.67.2");
        assert_eq!(
            Game::Poe1.install_dir(d),
            Path::new("/d/auto-pob/poe1/pob-2.67.2")
        );
        assert_eq!(
            Game::Poe1.default_user_dir(d),
            Path::new("/d/RustyPathOfBuilding1/userdata")
        );
    }
}
