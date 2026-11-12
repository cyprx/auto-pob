//! auto-pob's own settings file, `auto-pob.toml` (R12, R14, R15).
//!
//! PoB's settings stay in `Settings.xml`; this file holds what only auto-pob has: the user
//! folder, the release-notice switch, the trade realm and league, and the per-setting overrides
//! (a PoB setting auto-pob keeps its own value for). The engine never writes it, the UI does.
//!
//! ```toml
//! user_dir = "/home/me/pob"   # optional; AUTO_POB_USER_DIR wins over it
//! update_check = false        # AUTO_POB_UPDATE_CHECK=1 forces it on
//!
//! [trade]
//! realm = "PC"                # PC, PS4 or Xbox
//! league = "Standard"
//!
//! [overrides]                 # Settings.xml keys, as text
//! colorPositive = "^x33FF77"
//! ```
//!
//! Saving edits the file in place: comments, key order and keys we don't know stay. A file that
//! no longer parses is renamed to `auto-pob.toml.bad` (then `.bad-1`, ...) and said so, never
//! overwritten. Writes go to a temporary file in the same folder, then rename over the file.

use crate::game::Game;
use anyhow::{Context as _, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Value};

/// The trade realms the trade site knows (R13).
pub const REALMS: [&str; 3] = ["PC", "PS4", "Xbox"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    /// Where `Builds/` and `Settings.xml` live; None is the default folder
    pub user_dir: Option<PathBuf>,
    /// The opt-in release notice (R14)
    pub update_check: bool,
    /// "PC", "PS4" or "Xbox"
    pub trade_realm: String,
    pub trade_league: String,
    /// `Settings.xml` key -> the text auto-pob runs with instead (R12)
    pub overrides: BTreeMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            user_dir: None,
            update_check: false,
            trade_realm: "PC".into(),
            trade_league: "Standard".into(),
            overrides: BTreeMap::new(),
        }
    }
}

/// `AUTO_POB_CONFIG`, else `<config dir>/auto-pob/auto-pob.toml`.
pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("AUTO_POB_CONFIG").filter(|p| !p.is_empty()) {
        return std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(path));
    }
    directories::BaseDirs::new().map_or_else(
        || PathBuf::from("auto-pob.toml"),
        |dirs| dirs.config_dir().join("auto-pob").join("auto-pob.toml"),
    )
}

/// Reads the keys we know. Text that is not TOML is an error; a key with a bad value is left
/// out of the result with a warning, and every other key still loads.
fn parse(text: &str) -> Result<(DocumentMut, AppSettings, Vec<String>)> {
    let doc: DocumentMut = text.parse().context("not valid TOML")?;
    let mut s = AppSettings::default();
    let mut warnings = Vec::new();
    let mut warn = |w: String| {
        log::warn!("auto-pob.toml: {w}");
        warnings.push(w);
    };
    if let Some(item) = doc.get("user_dir") {
        match item.as_str() {
            Some(dir) => s.user_dir = (!dir.trim().is_empty()).then(|| PathBuf::from(dir)),
            None => warn("user_dir must be a string; ignored".into()),
        }
    }
    if let Some(item) = doc.get("update_check") {
        match item.as_bool() {
            Some(on) => s.update_check = on,
            None => warn("update_check must be true or false; ignored".into()),
        }
    }
    if let Some(item) = doc.get("trade") {
        match item.as_table_like() {
            None => warn("trade must be a table; ignored".into()),
            Some(trade) => {
                if let Some(realm) = trade.get("realm") {
                    match realm.as_str() {
                        Some(r) if REALMS.contains(&r) => s.trade_realm = r.to_owned(),
                        _ => {
                            warn("trade.realm must be \"PC\", \"PS4\" or \"Xbox\"; ignored".into())
                        }
                    }
                }
                if let Some(league) = trade.get("league") {
                    match league.as_str() {
                        Some(l) => s.trade_league = l.to_owned(),
                        None => warn("trade.league must be a string; ignored".into()),
                    }
                }
            }
        }
    }
    if let Some(item) = doc.get("overrides") {
        match item.as_table_like() {
            None => warn("overrides must be a table; ignored".into()),
            Some(table) => {
                for (key, item) in table.iter() {
                    let text = match item.as_value() {
                        Some(Value::String(s)) => s.value().clone(),
                        Some(Value::Boolean(b)) => b.value().to_string(),
                        Some(Value::Integer(i)) => i.value().to_string(),
                        Some(Value::Float(f)) => f.value().to_string(),
                        _ => {
                            warn(format!(
                                "overrides.{key} must be a string, number or true/false; ignored"
                            ));
                            continue;
                        }
                    };
                    s.overrides.insert(key.to_owned(), text);
                }
            }
        }
    }
    Ok((doc, s, warnings))
}

/// What reading the settings file found.
#[derive(Debug)]
pub enum Loaded {
    Absent,
    /// The settings, and one warning per key that was ignored
    Settings(AppSettings, Vec<String>),
    /// Not TOML at all; the file is untouched until the caller sets it aside
    Invalid(String),
    /// Could not be read (permissions, I/O): the file must not be renamed or written
    Unreadable(String),
}

pub fn load_checked(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Absent,
        Err(e) => return Loaded::Unreadable(format!("can't read {}: {e}", path.display())),
    };
    match parse(&text) {
        Ok((_, s, warnings)) => Loaded::Settings(s, warnings),
        Err(e) => Loaded::Invalid(format!("{}: {e:#}", path.display())),
    }
}

/// The settings in `path`; None when there is no file. A file that can't be read, or isn't
/// TOML, is an error that names the file; a bad value only drops its own key.
pub fn load(path: &Path) -> Result<Option<AppSettings>> {
    match load_checked(path) {
        Loaded::Absent => Ok(None),
        Loaded::Settings(s, _) => Ok(Some(s)),
        Loaded::Invalid(e) | Loaded::Unreadable(e) => bail!("{e}"),
    }
}

/// Moves a file we can't parse out of the way (`.bad`, then `.bad-1`, ...) so a save can't
/// lose it. Returns where it went.
pub fn back_up_corrupt(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("the settings path has no file name")?
        .to_string_lossy()
        .into_owned();
    let mut backup = path.with_file_name(format!("{name}.bad"));
    let mut n = 0;
    while backup.exists() {
        n += 1;
        backup = path.with_file_name(format!("{name}.bad-{n}"));
    }
    std::fs::rename(path, &backup)
        .with_context(|| format!("moving {} to {}", path.display(), backup.display()))?;
    Ok(backup)
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.value() == b.value(),
        (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
        (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
        (Value::Float(a), Value::Float(b)) => a.value() == b.value(),
        _ => false,
    }
}

/// `text` as a value of the same type as `old` when it reads as one (`90` over an integer),
/// else a string.
fn typed_like(old: Option<&Item>, text: &str) -> Value {
    match old.and_then(Item::as_value) {
        Some(Value::Integer(_)) => text.parse::<i64>().ok().map(Value::from),
        Some(Value::Float(_)) => text.parse::<f64>().ok().map(Value::from),
        Some(Value::Boolean(_)) => text.parse::<bool>().ok().map(Value::from),
        _ => None,
    }
    .unwrap_or_else(|| Value::from(text))
}

fn set_value(table: &mut dyn toml_edit::TableLike, key: &str, new: Value) {
    if let Some(Item::Value(old)) = table.get_mut(key) {
        if same(old, &new) {
            return;
        }
        // the new value takes the old one's spacing and trailing comment
        let decor = old.decor().clone();
        *old = new;
        *old.decor_mut() = decor;
    } else {
        table.insert(key, Item::Value(new));
    }
}

fn sub_table<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut dyn toml_edit::TableLike> {
    if !doc.contains_key(key) {
        doc.insert(key, Item::Table(toml_edit::Table::new()));
    }
    doc.get_mut(key)
        .and_then(Item::as_table_like_mut)
        .with_context(|| format!("{key} must be a table"))
}

fn apply(doc: &mut DocumentMut, s: &AppSettings) -> Result<()> {
    match &s.user_dir {
        Some(dir) => set_value(
            doc.as_table_mut(),
            "user_dir",
            Value::from(dir.to_string_lossy().into_owned()),
        ),
        None => {
            doc.remove("user_dir");
        }
    }
    set_value(
        doc.as_table_mut(),
        "update_check",
        Value::from(s.update_check),
    );
    let trade = sub_table(doc, "trade")?;
    set_value(trade, "realm", Value::from(s.trade_realm.as_str()));
    set_value(trade, "league", Value::from(s.trade_league.as_str()));
    if !s.overrides.is_empty() || doc.contains_key("overrides") {
        let table = sub_table(doc, "overrides")?;
        let stale: Vec<String> = table
            .iter()
            .map(|(k, _)| k.to_owned())
            .filter(|k| !s.overrides.contains_key(k))
            .collect();
        for key in stale {
            table.remove(&key);
        }
        for (key, text) in &s.overrides {
            let value = typed_like(table.get(key), text);
            set_value(table, key, value);
        }
    }
    Ok(())
}

const NEW_FILE_HEADER: &str = "# auto-pob settings. PoB's own options stay in Settings.xml.\n\
# AUTO_POB_USER_DIR overrides user_dir; AUTO_POB_UPDATE_CHECK=1 forces update_check on.\n";

/// Writes `s` to `path`: edits the file in place and keeps its comments and other keys, makes
/// the folder, and replaces the file by rename. Returns the backup's path when the file there
/// could not be parsed and was moved aside first.
pub fn save_noting(path: &Path, s: &AppSettings) -> Result<Option<PathBuf>> {
    let mut backup = None;
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let parsed = existing.map(|text| parse(&text).map(|(doc, ..)| doc));
    let fresh = !matches!(parsed, Some(Ok(_)));
    let mut doc = match parsed {
        Some(Ok(doc)) => doc,
        Some(Err(_)) => {
            backup = Some(back_up_corrupt(path)?);
            DocumentMut::new()
        }
        None => DocumentMut::new(),
    };
    apply(&mut doc, s)?;
    let mut text = doc.to_string();
    if fresh {
        text.insert_str(0, NEW_FILE_HEADER);
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let name = path
        .file_name()
        .context("the settings path has no file name")?;
    let temp = path.with_file_name(format!(
        "{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let write = || -> Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(e).with_context(|| format!("writing {}", path.display()));
    }
    Ok(backup)
}

/// `save_noting` for callers that don't tell the user about a backup.
pub fn save(path: &Path, s: &AppSettings) -> Result<()> {
    if let Some(backup) = save_noting(path, s)? {
        log::warn!("the old settings file was moved to {}", backup.display());
    }
    Ok(())
}

/// What the first-run screen has to settle (R15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRun {
    /// The classic app's folder exists: ask whose builds to use
    Ask { classic: PathBuf },
    /// Nothing to ask: today's default folder, and no file is written
    UseDefault,
}

/// What to do at start-up. None when a settings file or `AUTO_POB_USER_DIR` already settles the
/// folder.
pub fn first_run(
    settings: Option<&AppSettings>,
    data_dir: &Path,
    env_user_dir: bool,
) -> Option<FirstRun> {
    if settings.is_some() || env_user_dir {
        return None;
    }
    let classic = Game::Poe1.default_user_dir(data_dir);
    Some(if classic.is_dir() {
        FirstRun::Ask { classic }
    } else {
        FirstRun::UseDefault
    })
}

/// A typed folder with a leading `~` or `~/` replaced by `home`, made absolute.
pub fn expand_folder(text: &str, home: Option<&Path>) -> PathBuf {
    let text = text.trim();
    let path = match (text, home) {
        ("~", Some(home)) => home.to_path_buf(),
        (t, Some(home)) if t.starts_with("~/") => home.join(&t[2..]),
        _ => PathBuf::from(text),
    };
    std::path::absolute(&path).unwrap_or(path)
}

/// Makes sure `dir` can hold the user's files: makes it if it is missing, refuses a file, and
/// proves it is writable. Call it only once the user has confirmed the folder.
pub fn prepare_folder(dir: &Path) -> Result<()> {
    if dir.exists() && !dir.is_dir() {
        bail!("{} is a file, not a folder", dir.display());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("can't make {}", dir.display()))?;
    let probe = dir.join(format!(".auto-pob-write-test-{}", std::process::id()));
    std::fs::write(&probe, b"").with_context(|| format!("can't write in {}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// The user's answer to the first-run question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRunChoice {
    /// The classic app's folder
    Classic,
    /// Another folder
    Folder(PathBuf),
}

/// Settles the first run. Only an answer to `Ask` writes the file; `UseDefault` writes nothing
/// and gives the defaults.
pub fn settle_first_run(
    run: &FirstRun,
    choice: Option<FirstRunChoice>,
    path: &Path,
) -> Result<AppSettings> {
    let mut settings = AppSettings::default();
    let FirstRun::Ask { classic } = run else {
        return Ok(settings);
    };
    let Some(choice) = choice else {
        return Ok(settings);
    };
    settings.user_dir = Some(match choice {
        FirstRunChoice::Classic => classic.clone(),
        FirstRunChoice::Folder(dir) => dir,
    });
    save(path, &settings)?;
    Ok(settings)
}

/// The user folder: `AUTO_POB_USER_DIR`, else the settings file's, else the default.
pub fn resolve_user_dir(
    env: Option<PathBuf>,
    s: Option<&AppSettings>,
    default: PathBuf,
) -> PathBuf {
    env.or_else(|| s.and_then(|s| s.user_dir.clone()))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("auto-pob-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const COMMENTED: &str = "# my notes\nuser_dir = \"/old\" # where the builds are\nupdate_check = false\nfuture_key = 7\n\n# trade things\n[trade]\nrealm = \"PC\"\nleague = \"Standard\"\n\n[other]\nkeep = \"me\"\n\n[overrides]\n# a colour\ncolorPositive = \"^x00FF00\" # green\nshowAnimations = \"true\"\n";

    #[test]
    fn toml_round_trip_keeps_comments() {
        let dir = temp("roundtrip");
        let path = dir.join("auto-pob.toml");
        std::fs::write(&path, COMMENTED).unwrap();
        let mut s = load(&path).unwrap().unwrap();
        assert_eq!(s.user_dir, Some(PathBuf::from("/old")));
        assert_eq!(s.overrides["colorPositive"], "^x00FF00");
        s.user_dir = Some(PathBuf::from("/new"));
        s.update_check = true;
        s.trade_realm = "Xbox".into();
        s.trade_league = "Settlers".into();
        s.overrides.remove("showAnimations");
        s.overrides.insert("defaultCharLevel".into(), "90".into());
        s.overrides
            .insert("colorPositive".into(), "^xFF0000".into());
        save(&path, &s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for kept in [
            "# my notes",
            "# where the builds are",
            "future_key = 7",
            "# trade things",
            "[other]\nkeep = \"me\"",
            "# a colour",
            "# green",
        ] {
            assert!(text.contains(kept), "lost {kept:?} in\n{text}");
        }
        assert!(!text.contains("showAnimations"), "{text}");
        assert_eq!(load(&path).unwrap().unwrap(), s);
        // no temporary file is left behind
        assert!(
            std::fs::read_dir(&dir).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
        // saving the same settings again changes nothing
        save(&path, &s).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        // clearing the folder drops the key
        s.user_dir = None;
        save(&path, &s).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("user_dir"));
        assert_eq!(load(&path).unwrap().unwrap().user_dir, None);
    }

    #[test]
    fn save_makes_the_folder_and_a_commented_file() {
        let dir = temp("fresh");
        let path = dir.join("deep").join("er").join("auto-pob.toml");
        let s = AppSettings {
            update_check: true,
            ..AppSettings::default()
        };
        save(&path, &s).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("# auto-pob settings")
        );
        assert_eq!(load(&path).unwrap().unwrap(), s);
    }

    #[test]
    fn missing_file_is_none() {
        let dir = temp("missing");
        assert_eq!(load(&dir.join("auto-pob.toml")).unwrap(), None);
    }

    #[test]
    fn bad_toml_is_an_error_not_a_panic() {
        let dir = temp("bad");
        let path = dir.join("auto-pob.toml");
        for bad in ["update_check = [", "this is = = not toml", "[trade"] {
            std::fs::write(&path, bad).unwrap();
            let error = format!("{:#}", load(&path).unwrap_err());
            assert!(error.contains("auto-pob.toml"), "{error}");
            assert!(matches!(load_checked(&path), Loaded::Invalid(_)));
        }
        // numbers and booleans in [overrides] read as the text PoB would hold
        std::fs::write(&path, "[overrides]\nn = 125\nb = true\n").unwrap();
        let s = load(&path).unwrap().unwrap();
        assert_eq!(s.overrides["n"], "125");
        assert_eq!(s.overrides["b"], "true");
    }

    #[test]
    fn a_bad_value_drops_only_its_own_key() {
        let dir = temp("badvalue");
        let path = dir.join("auto-pob.toml");
        std::fs::write(
            &path,
            "update_check = \"yes\"\nuser_dir = 4\n[trade]\nrealm = \"Switch\"\nleague = \"Settlers\"\n[overrides]\nx = [1]\ngood = \"1\"\n",
        )
        .unwrap();
        let Loaded::Settings(s, warnings) = load_checked(&path) else {
            panic!("a bad value is not a bad file");
        };
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(!s.update_check);
        assert_eq!(s.user_dir, None);
        assert_eq!(s.trade_realm, "PC");
        assert_eq!(s.trade_league, "Settlers");
        assert_eq!(s.overrides.len(), 1);
        assert_eq!(s.overrides["good"], "1");
        // the file itself is untouched
        assert!(std::fs::read_to_string(&path).unwrap().contains("Switch"));
    }

    #[test]
    fn an_unreadable_file_is_reported_and_left_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp("unreadable");
        let path = dir.join("auto-pob.toml");
        std::fs::write(&path, "update_check = true\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_to_string(&path).is_err() {
            assert!(matches!(load_checked(&path), Loaded::Unreadable(_)));
            assert!(load(&path).is_err());
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn existing_value_types_are_kept() {
        let dir = temp("types");
        let path = dir.join("auto-pob.toml");
        std::fs::write(
            &path,
            "[overrides]\ndefaultCharLevel = 90 # level\nshowAnimations = true\nratio = 0.5\ntext = \"a\"\n",
        )
        .unwrap();
        let mut s = load(&path).unwrap().unwrap();
        assert_eq!(s.overrides["defaultCharLevel"], "90");
        save(&path, &s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("defaultCharLevel = 90 # level"), "{text}");
        assert!(text.contains("showAnimations = true"), "{text}");
        assert!(text.contains("ratio = 0.5"), "{text}");
        s.overrides.insert("defaultCharLevel".into(), "95".into());
        s.overrides.insert("showAnimations".into(), "false".into());
        s.overrides.insert("ratio".into(), "0.75".into());
        s.overrides.insert("text".into(), "b".into());
        s.overrides.insert("fresh".into(), "7".into());
        save(&path, &s).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("defaultCharLevel = 95 # level"), "{text}");
        assert!(text.contains("showAnimations = false"), "{text}");
        assert!(text.contains("ratio = 0.75"), "{text}");
        assert!(text.contains("text = \"b\""), "{text}");
        // a new key is text, as PoB holds it
        assert!(text.contains("fresh = \"7\""), "{text}");
        // text that does not read as the old type replaces it as text
        s.overrides.insert("defaultCharLevel".into(), "high".into());
        save(&path, &s).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("defaultCharLevel = \"high\"")
        );
    }

    #[test]
    fn saving_over_a_corrupt_file_keeps_it_as_a_backup() {
        let dir = temp("corrupt");
        let path = dir.join("auto-pob.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let backup = save_noting(&path, &AppSettings::default())
            .unwrap()
            .unwrap();
        assert_eq!(backup, dir.join("auto-pob.toml.bad"));
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "this is = = not toml"
        );
        assert!(load(&path).unwrap().is_some());
        // a second corrupt file does not overwrite the first backup
        std::fs::write(&path, "again = =").unwrap();
        let second = back_up_corrupt(&path).unwrap();
        assert_eq!(second, dir.join("auto-pob.toml.bad-1"));
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "this is = = not toml"
        );
    }

    #[test]
    fn user_dir_precedence_env_then_file_then_default() {
        let default = PathBuf::from("/default");
        let file = AppSettings {
            user_dir: Some("/file".into()),
            ..AppSettings::default()
        };
        let none = AppSettings::default();
        let env = Some(PathBuf::from("/env"));
        assert_eq!(
            resolve_user_dir(env.clone(), Some(&file), default.clone()),
            Path::new("/env")
        );
        assert_eq!(
            resolve_user_dir(None, Some(&file), default.clone()),
            Path::new("/file")
        );
        assert_eq!(
            resolve_user_dir(None, Some(&none), default.clone()),
            default
        );
        assert_eq!(resolve_user_dir(None, None, default.clone()), default);
        assert_eq!(resolve_user_dir(env, None, default), Path::new("/env"));
    }

    #[test]
    fn first_run_asks_only_when_classic_exists_and_no_file() {
        let data = temp("firstrun");
        // no classic folder: nothing to ask
        assert_eq!(first_run(None, &data, false), Some(FirstRun::UseDefault));
        let classic = Game::Poe1.default_user_dir(&data);
        std::fs::create_dir_all(&classic).unwrap();
        assert_eq!(
            first_run(None, &data, false),
            Some(FirstRun::Ask { classic })
        );
        // a file, or the env var, settles it
        assert_eq!(first_run(Some(&AppSettings::default()), &data, false), None);
        assert_eq!(first_run(None, &data, true), None);
    }

    #[test]
    fn folders_are_expanded_and_checked() {
        let dir = temp("folders");
        let home = dir.join("home");
        assert_eq!(expand_folder("~/pob", Some(&home)), home.join("pob"));
        assert_eq!(expand_folder(" ~ ", Some(&home)), home);
        // `~user` and a tilde inside a name are left alone
        assert!(expand_folder("~x/pob", Some(&home)).ends_with("~x/pob"));
        // a missing folder is made
        let fresh = dir.join("a").join("b");
        assert!(!fresh.exists());
        prepare_folder(&fresh).unwrap();
        assert!(fresh.is_dir());
        // a regular file is refused
        let file = dir.join("file");
        std::fs::write(&file, "x").unwrap();
        let e = format!("{:#}", prepare_folder(&file).unwrap_err());
        assert!(e.contains("is a file"), "{e}");
        let e = format!("{:#}", prepare_folder(&file.join("inside")).unwrap_err());
        assert!(e.contains("can't make"), "{e}");
        // so is a folder nobody can write in (skipped when running as root)
        use std::os::unix::fs::PermissionsExt as _;
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(locked.join("probe"), "").is_err() {
            let e = format!("{:#}", prepare_folder(&locked).unwrap_err());
            assert!(e.contains("can't write"), "{e}");
            let e = format!("{:#}", prepare_folder(&locked.join("sub")).unwrap_err());
            assert!(e.contains("can't make"), "{e}");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn first_run_choice_writes_the_file() {
        let dir = temp("choice");
        let path = dir.join("cfg").join("auto-pob.toml");
        let ask = FirstRun::Ask {
            classic: dir.join("classic"),
        };
        let s = settle_first_run(&ask, Some(FirstRunChoice::Classic), &path).unwrap();
        assert_eq!(s.user_dir, Some(dir.join("classic")));
        assert_eq!(load(&path).unwrap().unwrap(), s);
        let other =
            settle_first_run(&ask, Some(FirstRunChoice::Folder("/x/y".into())), &path).unwrap();
        assert_eq!(
            load(&path).unwrap().unwrap().user_dir,
            Some(PathBuf::from("/x/y"))
        );
        assert_eq!(other.user_dir, Some(PathBuf::from("/x/y")));
        // no answer yet: nothing written
        let none = dir.join("none.toml");
        settle_first_run(&ask, None, &none).unwrap();
        assert!(!none.exists());
        // the default needs no answer and writes nothing
        let default = dir.join("default.toml");
        let s = settle_first_run(
            &FirstRun::UseDefault,
            Some(FirstRunChoice::Classic),
            &default,
        )
        .unwrap();
        assert_eq!(s, AppSettings::default());
        assert!(!default.exists());
    }
}
