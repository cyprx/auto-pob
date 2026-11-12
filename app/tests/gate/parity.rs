//! The PoB parity inventory (SP7 R2): every PoB 2.67.2 control, popup title and keyboard
//! shortcut must have a row in `fixtures/parity.toml` saying whether we ported it, ruled it out
//! or still owe it. Included from `gate.rs`, whose helpers it reaches through `super::`.
//!
//! - **Controls** are the keys of `main.controls`, `build.controls`, each tab's `controls` and
//!   `main.modes.LIST.controls`, and one level of nested `controls`, read from upstream PoB with
//!   `attack.xml` loaded. Rows are `[controls."<tab>.<key>"]` or `<tab>.<key>.<child>`. Controls
//!   held in a tab's array part (item slots, Calcs sections, Config options) have no name, so
//!   they are keyed by class, `<tab>.#<class>`. The Skills tab's per-gem controls are keyed
//!   `gemSlot#…`, whatever the fixture's gem count. Controls PoB makes only later (the Compare
//!   Config view's `cfg_p_*`/`cfg_c_*`, made once a comparison loads) are not listed.
//! - **Popups** are the titles of every `:OpenPopup(` (third argument) and `:OpenConfirmPopup(`
//!   (first argument) call in PoB's `Classes` and `Modules`. A title written as
//!   `cond and "A" or "B"` yields both. Rows are `[popups."<title>"]`; a title computed any other
//!   way is keyed by its call site, `[popups."<file>:<line>"]`, as is every `:OpenMessagePopup(`
//!   call (their behaviour, behind generic titles like "Error", is per site).
//! - **Keys** are every `event.key == "<k>"` in the same folders, as `[keys."<area>.<k>"]` where
//!   the area is the tab the file belongs to (see `key_area`); every widget-level `key == "<k>"`
//!   in `Classes`, as `[keys."<file stem>.<k>"]`; and every wiki-key site, as
//!   `[keys."<file stem>.wiki"]`. Modifier-held mouse behaviour (`IsKeyDown` around a click) is
//!   not scanned; the Task 2 report lists it.
//!
//! Each row has exactly one of `ported`, `ruled_out` (naming an R# or D-# ruling) or `gap`
//! (naming the task that closes it), plus an optional `note`.

use super::{fixtures_dir, load_xml, lock, pob_dir, upstream_lua};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// `main.controls` (the bottom-left Options/About buttons), `build.controls` and each tab's
/// control host, by the `<tab>` name the table uses.
const HOSTS: [(&str, &str); 12] = [
    ("main", "main"),
    ("build", "build"),
    ("tree", "build.treeTab"),
    ("skills", "build.skillsTab"),
    ("items", "build.itemsTab"),
    ("calcs", "build.calcsTab"),
    ("config", "build.configTab"),
    ("notes", "build.notesTab"),
    ("import", "build.importTab"),
    ("compare", "build.compareTab"),
    ("party", "build.partyTab"),
    ("list", "main.modes.LIST"),
];

/// Every control key upstream PoB has once `attack.xml` is loaded. With `dump`, each entry is
/// `key<TAB>class<TAB>label` instead.
fn upstream_controls(dump: bool) -> BTreeSet<String> {
    let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
    let xml = fs::read_to_string(fixtures_dir().join("attack.xml")).unwrap();
    load_xml(&lua, &xml, "attack").expect("loading attack.xml");
    // The build list makes its controls in `Init`, which lists `main.buildPath`: point that at
    // an empty folder of our own.
    let builds = std::env::temp_dir().join(format!("auto-pob-gate-parity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&builds);
    fs::create_dir_all(&builds).unwrap();
    lua.globals()
        .set("parityBuilds", format!("{}/", builds.display()))
        .unwrap();
    lua.load("main.buildPath = parityBuilds; main.modes.LIST:Init()")
        .set_name("=parity list")
        .exec()
        .expect("starting the build list");
    let hosts = HOSTS
        .iter()
        .map(|(tab, expr)| format!("{{ {tab:?}, {expr} }}"))
        .collect::<Vec<_>>()
        .join(", ");
    let keys: Vec<String> = lua
        .load(format!(
            r##"
            local out = {{}}
            -- With `dump`, each key also carries its class and label (for writing the table).
            local function describe(key, control)
                if not {dump} or type(control) ~= "table" then return key end
                local label = control.label
                if type(label) == "function" then
                    local ok, text = pcall(label, control)
                    label = ok and text or "<fn>"
                end
                return key .. "\t" .. tostring(control._className) .. "\t" .. tostring(label or "")
            end
            for _, pair in ipairs({{ {hosts} }}) do
                local tab, host = pair[1], pair[2]
                assert(type(host) == "table" and type(host.controls) == "table",
                    tab .. " has no controls table")
                for key, control in pairs(host.controls) do
                    -- Array-held controls (item slots, Calcs sections, Config options) have no
                    -- stable name, so they are keyed by class: `<tab>.#<class>`.
                    local name = type(key) == "string" and key
                        or "#" .. tostring(type(control) == "table" and control._className)
                    out[#out + 1] = describe(tab .. "." .. name, control)
                    if type(control) == "table" and type(control.controls) == "table" then
                        for child, inner in pairs(control.controls) do
                            if type(child) == "string" then
                                out[#out + 1] = describe(tab .. "." .. name .. "." .. child, inner)
                            end
                        end
                    end
                end
            end
            assert(#main.popups == 0, "a popup is open")
            return out
            "##
        ))
        .set_name("=parity controls")
        .eval()
        .expect("listing controls");
    let _ = fs::remove_dir_all(&builds);
    // the Skills tab has one `gemSlot<n>…` set per gem of the displayed group: key them as
    // `gemSlot#…` so the rows don't depend on the fixture's gem count
    keys.into_iter()
        .map(|k| match k.find("gemSlot") {
            Some(at) => {
                let (head, tail) = k.split_at(at + "gemSlot".len());
                format!(
                    "{head}#{}",
                    tail.trim_start_matches(|c: char| c.is_ascii_digit())
                )
            }
            None => k,
        })
        .collect()
}

/// PoB's `Classes` and `Modules` sources, by file stem.
fn pob_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for folder in ["Classes", "Modules"] {
        let dir = pob_dir().join(folder);
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "lua") {
                let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                out.push((stem, fs::read_to_string(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The top-level, comma-separated arguments of a call; `rest` starts just past its `(`.
fn call_args(rest: &str) -> Vec<String> {
    let mut args = vec![String::new()];
    let (mut depth, mut quote) = (0i32, None::<char>);
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            args.last_mut().unwrap().push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    args.last_mut().unwrap().push(n);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' if depth == 0 => break,
            ')' | '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(String::new());
                continue;
            }
            _ => {}
        }
        args.last_mut().unwrap().push(c);
    }
    args.into_iter().map(|a| a.trim().to_string()).collect()
}

/// The string literal `s` exactly is, if it is one.
fn literal(s: &str) -> Option<String> {
    let s = s.trim();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then(|| inner.to_string())
}

/// Titles a title argument stands for: a literal, or both sides of `cond and "A" or "B"`.
fn titles_of(arg: &str) -> Vec<String> {
    if let Some(t) = literal(arg) {
        return vec![t];
    }
    if let Some((head, b)) = arg.rsplit_once(" or ")
        && let Some((_, a)) = head.rsplit_once(" and ")
        && let (Some(a), Some(b)) = (literal(a), literal(b))
    {
        return vec![a, b];
    }
    Vec::new()
}

/// Every popup PoB opens: its literal titles, or `<file>:<line>` for a computed title.
fn upstream_popups(sources: &[(String, String)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (stem, text) in sources {
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") || trimmed.starts_with("function") {
                continue;
            }
            // a message popup's behaviour lives at its call site (generic titles like "Error"),
            // so each site is its own row
            for (needle, index) in [
                (":OpenPopup(", Some(2)),
                (":OpenConfirmPopup(", Some(0)),
                (":OpenMessagePopup(", None),
            ] {
                let mut rest = line;
                while let Some(at) = rest.find(needle) {
                    rest = &rest[at + needle.len()..];
                    let titles = index.and_then(|i| call_args(rest).get(i).map(|a| titles_of(a)));
                    match titles {
                        Some(titles) if !titles.is_empty() => out.extend(titles),
                        _ => {
                            out.insert(format!("{stem}:{}", n + 1));
                        }
                    }
                }
            }
        }
    }
    out
}

/// The `<area>` a file's key handling belongs to.
fn key_area(stem: &str) -> String {
    match stem {
        "Build" => "build",
        "BuildList" | "BuildListControl" => "list",
        "TreeTab" | "PassiveTreeView" => "tree",
        "SkillsTab" => "skills",
        "ItemsTab" => "items",
        "CalcsTab" => "calcs",
        "ConfigTab" => "config",
        "NotesTab" => "notes",
        "ImportTab" => "import",
        "CompareTab" => "compare",
        "PartyTab" => "party",
        "PopupDialog" => "popup",
        "Main" => "main",
        other => other,
    }
    .to_string()
}

/// Every key PoB handles:
/// - `event.key == "<k>"` (frame-level handlers) in `Classes` and `Modules`, as `<area>.<k>`;
/// - `key == "<k>"` not preceded by `.` or a word character (widget handlers such as
///   `OnKeyDown(key)`, `OnSelKeyDown`, `OnHoverKeyUp`) in `Classes`, as `<file stem>.<k>`;
/// - every `itemLib.wiki.matchesKey(` site (PoB's wiki key), as `<file stem>.wiki`.
fn upstream_keys(sources: &[(String, String)]) -> BTreeSet<String> {
    const FRAME: &str = "event.key == \"";
    const WIDGET: &str = "key == \"";
    const WIKI: &str = "itemLib.wiki.matchesKey(";
    let mut out = BTreeSet::new();
    for (stem, text) in sources {
        let mut rest = text.as_str();
        while let Some(at) = rest.find(FRAME) {
            rest = &rest[at + FRAME.len()..];
            let key = &rest[..rest.find('"').unwrap()];
            out.insert(format!("{}.{key}", key_area(stem)));
        }
        if !pob_dir()
            .join("Classes")
            .join(format!("{stem}.lua"))
            .exists()
        {
            continue;
        }
        for (at, _) in text.match_indices(WIDGET) {
            let before = text[..at].chars().next_back();
            if before.is_some_and(|c| c == '.' || c == '_' || c.is_alphanumeric()) {
                continue;
            }
            let rest = &text[at + WIDGET.len()..];
            out.insert(format!("{stem}.{}", &rest[..rest.find('"').unwrap()]));
        }
        if text.contains(WIKI) {
            out.insert(format!("{stem}.wiki"));
        }
    }
    out
}

/// One `parity.toml` row.
#[derive(Debug, Default)]
struct Row {
    fields: BTreeMap<String, String>,
}

/// Parses the table's restricted TOML: `[section."key"]` headers, `field = "string"` lines,
/// comments and blank lines. Anything else is an error, so a typo can't hide a row.
fn parse_table(text: &str) -> BTreeMap<String, BTreeMap<String, Row>> {
    let mut table: BTreeMap<String, BTreeMap<String, Row>> = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let at = || format!("parity.toml line {}: {raw}", n + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let header = header
                .strip_suffix(']')
                .unwrap_or_else(|| panic!("{}", at()));
            let (section, key) = header
                .split_once('.')
                .unwrap_or_else(|| panic!("{}: expected [section.\"key\"]", at()));
            let key = literal(key).unwrap_or_else(|| panic!("{}: quote the key", at()));
            let rows = table.entry(section.to_string()).or_default();
            assert!(!rows.contains_key(&key), "{}: duplicate row", at());
            rows.insert(key.clone(), Row::default());
            current = Some((section.to_string(), key));
            continue;
        }
        let (field, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("{}: expected field = \"value\"", at()));
        let value = literal(value).unwrap_or_else(|| panic!("{}: quote the value", at()));
        let (section, key) = current.as_ref().unwrap_or_else(|| panic!("{}", at()));
        let row = table.get_mut(section).unwrap().get_mut(key).unwrap();
        let old = row.fields.insert(field.trim().to_string(), value);
        assert!(old.is_none(), "{}: duplicate field", at());
    }
    table
}

/// Problems with one row's fields, if any.
fn row_problem(row: &Row) -> Option<String> {
    let status: Vec<_> = ["ported", "ruled_out", "gap"]
        .into_iter()
        .filter(|s| row.fields.contains_key(*s))
        .collect();
    if status.len() != 1 {
        return Some(format!(
            "needs exactly one of ported/ruled_out/gap, has {status:?}"
        ));
    }
    if let Some(extra) = row
        .fields
        .keys()
        .find(|f| !["ported", "ruled_out", "gap", "note"].contains(&f.as_str()))
    {
        return Some(format!("unknown field {extra}"));
    }
    let value = &row.fields[status[0]];
    let cites = |prefixes: &[&str]| {
        prefixes.iter().any(|p| {
            value.match_indices(p).any(|(i, _)| {
                value[i + p.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            })
        })
    };
    match status[0] {
        "ported" => {
            // the place starts with a repository path, which must exist
            let first = value.split_whitespace().next().unwrap_or("");
            let path = first.trim_end_matches([',', ';', ':', ')']);
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
            (!path.contains('/') || !root.join(path).exists()).then(|| {
                format!("ported must start with an existing repository path, not {first:?}")
            })
        }
        "ruled_out" if !cites(&["R", "D-"]) => Some("ruled_out needs an R# or D-# id".into()),
        "gap" if !cites(&["Task "]) => Some("gap needs \"Task N\"".into()),
        _ => None,
    }
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn parity_inventory_is_complete() {
    let _guard = lock();
    if std::env::var_os("PARITY_DUMP").is_some() {
        for line in upstream_controls(true) {
            eprintln!("parity-dump {line}");
        }
    }
    let sources = pob_sources();
    let upstream: BTreeMap<&str, BTreeSet<String>> = BTreeMap::from([
        ("controls", upstream_controls(false)),
        ("popups", upstream_popups(&sources)),
        ("keys", upstream_keys(&sources)),
    ]);
    for (section, keys) in &upstream {
        assert!(!keys.is_empty(), "found no upstream {section}");
    }

    let path = fixtures_dir().join("parity.toml");
    let table = parse_table(&fs::read_to_string(&path).unwrap_or_default());
    let mut problems = Vec::new();
    for section in table.keys() {
        if !upstream.contains_key(section.as_str()) {
            problems.push(format!("unknown section [{section}]"));
        }
    }
    let empty = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (section, keys) in &upstream {
        let rows = table.get(*section).unwrap_or(&empty);
        for key in keys.difference(&rows.keys().cloned().collect()) {
            problems.push(format!("missing: [{section}.{key:?}]"));
        }
        for (key, row) in rows {
            if !keys.contains(key) {
                problems.push(format!(
                    "stale (PoB has no such element): [{section}.{key:?}]"
                ));
            } else if let Some(p) = row_problem(row) {
                problems.push(format!("[{section}.{key:?}]: {p}"));
            }
            for status in ["ported", "ruled_out", "gap"] {
                if row.fields.contains_key(status) {
                    *counts.entry(status).or_default() += 1;
                }
            }
        }
    }
    eprintln!(
        "parity: {} rows; {counts:?}",
        table.values().map(|r| r.len()).sum::<usize>()
    );
    assert!(
        problems.is_empty(),
        "{} parity.toml problems ({}):\n{}",
        problems.len(),
        Path::new("app/tests/fixtures/parity.toml").display(),
        problems.join("\n")
    );
    // SP7's Definition of Done: every gap was closed or ruled out by Task 16
    let gaps: Vec<String> = table
        .iter()
        .flat_map(|(section, rows)| {
            rows.iter()
                .filter(|(_, row)| row.fields.contains_key("gap"))
                .map(move |(key, _)| format!("[{section}.{key:?}]"))
        })
        .collect();
    assert!(gaps.is_empty(), "parity gaps left: {gaps:?}");
}
