//! The long-tail harness: PoB sessions over a temp user folder and a private view of the
//! pinned PoB folder, and upstream reads of what a session wrote. Included from `gate.rs`.

use super::net_harness::{canned, canned_web};
use super::pob_dir;
use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::upstream_lua;
use auto_pob::engine::{Affected, Backend, Command, PopupControl, PopupView, SettingsView};
use auto_pob::game::Game;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A PoB backend over a temp user folder. PoB runs from a folder of symlinks to the pinned
/// one, except for `manifest.xml`, which is a copy: Options' Save rewrites that file, and the
/// pinned folder must never be touched, not even by a test that expects it not to be.
pub(crate) struct LongtailSession {
    pub backend: PobBackend,
    pub user: PathBuf,
    pub pob: PathBuf,
}

impl Drop for LongtailSession {
    fn drop(&mut self) {
        // `remove_dir_all` unlinks the symlinks without following them
        let _ = fs::remove_dir_all(&self.pob);
        let _ = fs::remove_dir_all(&self.user);
    }
}

/// The folder PoB's own user data lives in, under `user`
pub(crate) fn settings_folder(user: &Path) -> PathBuf {
    user.join("Path of Building")
}

/// A folder that looks like the pinned PoB folder but can be written to where it matters:
/// every entry links to the pinned one, `manifest.xml` is a copy.
fn pob_farm(test: &str) -> PathBuf {
    let farm = std::env::temp_dir().join(format!("auto-pob-lt-pob-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&farm);
    fs::create_dir_all(&farm).unwrap();
    let pinned = pob_dir();
    for entry in fs::read_dir(&pinned).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "manifest.xml" {
            fs::copy(entry.path(), farm.join(&name)).unwrap();
        } else {
            std::os::unix::fs::symlink(entry.path(), farm.join(&name)).unwrap();
        }
    }
    farm
}

impl LongtailSession {
    /// Starts PoB with the text `settings` makes as the user folder's `Settings.xml`; it is
    /// given the user folder, to make the folders the settings name.
    pub(crate) fn start(test: &str, settings: &dyn Fn(&Path) -> String) -> LongtailSession {
        let user = std::env::temp_dir().join(format!("auto-pob-lt-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&user);
        let folder = settings_folder(&user);
        fs::create_dir_all(folder.join("Builds")).unwrap();
        fs::write(folder.join("Settings.xml"), settings(&user)).unwrap();
        let pob = pob_farm(test);
        // the canned web has no routes: a request nobody meant to make fails
        let net = canned(&canned_web());
        let backend = PobBackend::start_with(Game::Poe1, &pob, &user, net).expect("starting PoB");
        LongtailSession { backend, user, pob }
    }

    /// Applies a command and recalculates when it asks for that, as the loop does.
    pub(crate) fn run(&mut self, command: Command) -> anyhow::Result<Affected> {
        let affected = self.backend.apply(command, &|_| {})?;
        if affected.recalc {
            self.backend.recalculate()?;
        }
        Ok(affected)
    }

    pub(crate) fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.backend
            .lua
            .load(code)
            .set_name("=longtail test")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    pub(crate) fn popup(&mut self) -> Option<PopupView> {
        self.backend.popup_view().expect("popup view")
    }

    pub(crate) fn settings(&mut self) -> SettingsView {
        self.backend.settings_view().expect("settings view")
    }

    pub(crate) fn settings_path(&self) -> PathBuf {
        settings_folder(&self.user).join("Settings.xml")
    }

    pub(crate) fn settings_bytes(&self) -> Vec<u8> {
        fs::read(self.settings_path()).unwrap_or_default()
    }

    /// `#main.popups`: what every command must leave at 0
    pub(crate) fn open_popups(&self) -> usize {
        self.eval("return #pob.main.main.popups")
    }

    pub(crate) fn manifest(&self) -> (Vec<u8>, std::time::SystemTime) {
        let path = self.pob.join("manifest.xml");
        (
            fs::read(&path).unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap(),
        )
    }
}

/// One control of a popup view
pub(crate) fn control<'a>(view: &'a PopupView, name: &str) -> &'a PopupControl {
    view.controls
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no control {name} in the popup"))
}

/// The attributes of the `<Misc>` element of a `Settings.xml`
pub(crate) fn misc_attributes(xml: &str) -> BTreeMap<String, String> {
    let start = xml.find("<Misc").expect("no Misc element");
    let end = start + xml[start..].find("/>").expect("Misc is not closed");
    let mut rest = &xml[start + "<Misc".len()..end];
    let mut out = BTreeMap::new();
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq].trim().to_string();
        let value_start = eq + 2;
        let value_len = rest[value_start..]
            .find('"')
            .expect("unterminated attribute");
        out.insert(
            name,
            rest[value_start..value_start + value_len]
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">"),
        );
        rest = &rest[value_start + value_len + 1..];
    }
    out
}

/// Upstream PoB's own `main:LoadSettings()` on `user`'s folder: the named fields, as JSON.
/// The state is a plain `HeadlessWrapper` one; nothing of ours is in it.
pub(crate) fn upstream_settings(user: &Path, fields: &[&str]) -> serde_json::Value {
    use mlua::LuaSerdeExt;
    let lua = upstream_lua(&pob_dir()).expect("upstream PoB");
    lua.globals()
        .set("USER_PATH", format!("{}/", settings_folder(user).display()))
        .unwrap();
    lua.globals()
        .set(
            "FIELDS",
            fields.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
        )
        .unwrap();
    let value: mlua::Value = lua
        .load(
            r#"
            local main = launch.main
            main.userPath = USER_PATH
            main:LoadSettings(true)
            local out = {}
            for _, field in ipairs(FIELDS) do
                local v = main[field]
                if v == nil then v = launch[field] end
                out[field] = v
            end
            return out
            "#,
        )
        .eval()
        .unwrap_or_else(|e| panic!("upstream LoadSettings: {e}"));
    lua.from_value(value).unwrap()
}
