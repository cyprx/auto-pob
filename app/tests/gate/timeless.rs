//! The timeless jewel search (R17): PoB's "Find a Timeless Jewel" window as a build-owned
//! session, its Search press as a background job that yields from the LUT reads, against
//! upstream PoB pressing the same controls and searching synchronously.
//!
//! PoB caches the inflated LUTs as `.bin` files next to its data (DataJewelFileLoader.lua:83-91):
//! both sides here run on a folder of symlinks whose `Data/TimelessJewelData` is a real folder,
//! so those files land in it, never in the pinned folder. Serial (`gate-serial.txt`).

use super::net_harness::{canned, canned_web, net_user};
use super::{fixture_xml, lock, pob_dir};
use auto_pob::engine::pob::PobBackend;
use auto_pob::engine::upstream::{load_xml, upstream_lua};
use auto_pob::engine::{
    Affected, Backend, BackgroundTask, Command, ConfigValue, Event, PopupInput, PopupKind,
    PopupView,
};
use auto_pob::game::Game;
use mlua::LuaSerdeExt;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A folder of symlinks to the pinned PoB folder, except `Data/TimelessJewelData`, which is a
/// real folder of symlinks: PoB's LUT caches are written here
fn farm(test: &str) -> PathBuf {
    fn link_dir(from: &Path, to: &Path, deeper: &[&str]) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            if deeper.first().is_some_and(|d| name == *d) {
                link_dir(&entry.path(), &to.join(&name), &deeper[1..]);
            } else {
                std::os::unix::fs::symlink(entry.path(), to.join(&name)).unwrap();
            }
        }
    }
    let farm = std::env::temp_dir().join(format!("auto-pob-tl-pob-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&farm);
    link_dir(&pob_dir(), &farm, &["Data", "TimelessJewelData"]);
    farm
}

/// Upstream's side, written apart from the glue: PoB's own opener, the realm picked and the
/// refused league fetch's callback played (TreeTab.lua:2345-2372), every change a press of the
/// window's own controls, and the Search press run synchronously, as PoB runs it.
const UPSTREAM_LIB: &str = r#"
launches = 0
local stubLaunch = LaunchSubScript
LaunchSubScript = function(...)
    launches = launches + 1
    return stubLaunch(...)
end
local opened
OpenURL = function(url) opened = url end

local function popup()
    local p = main.popups[1]
    assert(p, "no window is open")
    return p
end

function openTimeless(realmIndex, leagues, leagueIndex)
    local depth = #main.popups
    build.treeTab:FindTimelessJewel()
    assert(#main.popups == depth + 1, "no window opened")
    local c = popup().controls
    c.realmSelection:SetSel(realmIndex)
    c.searchTradeLeagueSelect:SetList(leagues)
    c.searchTradeLeagueSelect:SetSel(leagueIndex)
end

function setControl(name, value)
    local c = popup().controls[name]
    assert(c and c:IsShown() and c:IsEnabled(), name .. " is not usable")
    if value.Choice then
        c:SetSel(value.Choice.index + 1)
    elseif value.Check ~= nil then
        c.state = value.Check
        c.changeFunc(value.Check)
    elseif value.Text then
        c:SetText(value.Text, true)
    else
        error("unknown input")
    end
end

function press(name)
    opened = nil
    local c = popup().controls[name]
    assert(c:IsShown() and c:IsEnabled(), name .. " is not usable")
    c.onClick()
    return opened
end

function closeTimeless()
    popup().controls.closeButton.onClick()
    assert(#main.popups == 0, "the window stayed open")
end
"#;

/// `timelessData.searchResults`, read the same way on both sides
const RESULTS: &str = r#"
local out = {}
for _, r in ipairs(BUILD.timelessData.searchResults) do
    table.insert(out, { seed = r.seed, total = r.total, label = r.label })
end
return out
"#;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct Found {
    seed: u32,
    total: f64,
    /// PoB's row: the seed and each desired node's weight, formatted
    label: String,
}

/// PoB sorts by total only, and ties come out in `pairs` order: compared by total, then seed
fn canonical(mut found: Vec<Found>) -> Vec<Found> {
    found.sort_by(|a, b| {
        b.total
            .partial_cmp(&a.total)
            .unwrap()
            .then(a.seed.cmp(&b.seed))
    });
    found
}

fn results_of(lua: &mlua::Lua, build: &str) -> Vec<Found> {
    let value: mlua::Value = lua
        .load(RESULTS.replace("BUILD", build))
        .eval()
        .expect("reading the results");
    canonical(lua.from_value(value).unwrap())
}

/// The league list the fetch would have set for the app's league (R10, R17)
fn leagues(league: &str) -> (Vec<String>, usize) {
    let fixed = ["Standard", "Hardcore", "Ruthless", "Hardcore Ruthless"];
    let mut list: Vec<String> = Vec::new();
    if !fixed.contains(&league) {
        list.push(league.to_string());
    }
    list.extend(fixed.iter().map(|s| s.to_string()));
    let index = list.iter().position(|l| l == league).unwrap() + 1;
    (list, index)
}

struct Ours {
    backend: PobBackend,
    user: PathBuf,
    /// The PoB folder of symlinks, with the LUT caches PoB wrote
    pob: PathBuf,
}

impl Drop for Ours {
    fn drop(&mut self) {
        // `remove_dir_all` unlinks the symlinks without following them
        let _ = fs::remove_dir_all(&self.pob);
        let _ = fs::remove_dir_all(&self.user);
    }
}

impl Ours {
    fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.backend
            .lua
            .load(code)
            .set_name("=timeless test")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    fn popups(&self) -> usize {
        self.eval("return #pob.main.main.popups")
    }

    fn run(&mut self, command: Command) -> anyhow::Result<Affected> {
        let what = format!("{command:?}");
        let out = self.backend.apply(command, &|_| {});
        if out.as_ref().is_ok_and(|a| a.recalc) {
            self.backend.recalculate().unwrap();
        }
        assert_eq!(self.popups(), 0, "{what} left a popup open");
        out
    }

    fn view(&mut self) -> PopupView {
        self.backend
            .popup_view()
            .unwrap()
            .expect("no timeless window is open")
    }

    fn set(&mut self, name: &str, value: PopupInput) {
        self.run(Command::SetPopupControl {
            name: name.into(),
            value,
        })
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    /// A choice by its label, as the window shows it
    fn choose(&mut self, name: &str, label: &str) -> PopupInput {
        let view = self.view();
        let control = view.controls.iter().find(|c| c.name == name).unwrap();
        let index = control
            .choices
            .iter()
            .position(|c| c == label)
            .unwrap_or_else(|| panic!("{name} has no {label}: {:?}", control.choices));
        let value = PopupInput::Choice {
            index,
            label: label.into(),
        };
        self.set(name, value.clone());
        value
    }

    /// Runs Timeless steps as the loop would until the job ends; returns the progress events
    fn finish(&mut self) -> Vec<u32> {
        let progress = RefCell::new(Vec::new());
        while self.backend.next_background() == Some(BackgroundTask::Timeless) {
            self.backend
                .background_step(BackgroundTask::Timeless, &|e| {
                    if let Event::Progress {
                        task: BackgroundTask::Timeless,
                        percent,
                    } = e
                    {
                        progress.borrow_mut().push(percent);
                    }
                })
                .expect("a timeless step");
            assert_eq!(self.popups(), 0, "a step left a popup open");
        }
        progress.into_inner()
    }

    /// One Timeless step; its progress, if it sent one
    fn step(&mut self) -> Option<u32> {
        let progress = RefCell::new(None);
        assert_eq!(
            self.backend.next_background(),
            Some(BackgroundTask::Timeless)
        );
        self.backend
            .background_step(BackgroundTask::Timeless, &|e| {
                if let Event::Progress { percent, .. } = e {
                    *progress.borrow_mut() = Some(percent);
                }
            })
            .expect("a timeless step");
        assert_eq!(self.popups(), 0, "a step left a popup open");
        progress.into_inner()
    }

    fn press(&mut self, name: &str) -> Affected {
        self.run(Command::PressPopupButton { name: name.into() })
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn results(&self) -> Vec<Found> {
        results_of(&self.backend.lua, "pob.build")
    }
}

/// Every file under `root`, with its bytes
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// One search: the jewel, its conqueror, the node id prefix, the desired nodes and the
/// devotion choices
type Case<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a [&'a str],
    &'a [(&'a str, &'a str)],
);

/// R17's search list: three desired nodes with weights
fn search_list(prefix: &str, ids: &[&str]) -> String {
    ids.iter()
        .enumerate()
        .map(|(i, id)| format!("{prefix}_{id}, {}, 0, 0", i + 2))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore]
fn timeless_search_matches_pob() {
    let _g = lock();
    let started = Instant::now();
    let web = canned_web();
    let pob = farm("search");
    let user = net_user("timeless", None);
    let backend = PobBackend::start_with(Game::Poe1, &pob, &user, canned(&web)).unwrap();
    let mut s = Ours {
        backend,
        user: user.clone(),
        pob: pob.clone(),
    };
    let builds = s.user.join("Path of Building/Builds");
    for name in ["attack", "dot"] {
        fs::copy(fixture_xml(name), builds.join(format!("{name}.xml"))).unwrap();
    }
    s.run(Command::OpenBuild {
        path: builds.join("attack.xml"),
    })
    .unwrap();
    let before = files(&s.user);
    let refused = s.backend.net_handle().refused();

    // ---- The realm mapping: PS4 is PoB's "Sony"; the league cache is filled, nothing fetched ----
    s.run(Command::SetTradeDefaults {
        realm: "PS4".into(),
        league: "Settlers".into(),
    })
    .unwrap();
    s.run(Command::OpenPopup(PopupKind::TimelessJewel)).unwrap();
    let view = s.view();
    assert_eq!(view.title, "Find a Timeless Jewel");
    assert_eq!(view.kind, PopupKind::TimelessJewel);
    let realm = view
        .controls
        .iter()
        .find(|c| c.name == "realmSelection")
        .unwrap();
    assert_eq!(realm.choices[realm.selected], "Sony");
    let league = view
        .controls
        .iter()
        .find(|c| c.name == "searchTradeLeagueSelect")
        .unwrap();
    assert_eq!(league.choices[league.selected], "Settlers");
    assert_eq!(
        s.backend.net_handle().refused(),
        refused,
        "a league fetch was attempted"
    );
    // a realm picked in the window keeps its leagues, there and back
    s.choose("realmSelection", "Xbox");
    s.choose("realmSelection", "Sony");
    assert_eq!(
        s.backend.net_handle().refused(),
        refused,
        "a realm pick fetched"
    );

    // ---- Upstream: the same build, the same window ----
    let up = upstream_lua(&pob).expect("starting upstream PoB");
    up.load("launch.devMode = false").exec().unwrap();
    load_xml(
        &up,
        &fs::read_to_string(fixture_xml("attack")).unwrap(),
        "attack",
    )
    .unwrap();
    up.load(UPSTREAM_LIB)
        .set_name("=timeless upstream")
        .exec()
        .unwrap();
    let call = |f: &str, args: mlua::MultiValue| -> mlua::Value {
        let func: mlua::Function = up.globals().get(f).unwrap();
        func.call(args).unwrap_or_else(|e| panic!("{f}: {e}"))
    };
    let (list, index) = leagues("Settlers");
    call(
        "openTimeless",
        mlua::MultiValue::from_iter([
            mlua::Value::Integer(2),
            up.to_value(&list).unwrap(),
            mlua::Value::Integer(index as i64),
        ]),
    );
    let up_set = |name: &str, value: &PopupInput| {
        call(
            "setControl",
            mlua::MultiValue::from_iter([up.to_value(name).unwrap(), up.to_value(value).unwrap()]),
        );
    };

    // ---- Lethal Pride (Kaom) and Militant Faith (Avarius, devotion 2 and 3) ----
    let socket_label = s
        .view()
        .controls
        .iter()
        .find(|c| c.name == "socketSelect")
        .unwrap()
        .choices[1]
        .clone();
    let cases: [Case; 2] = [
        (
            "Lethal Pride",
            "Kaom (Strength of Blood)",
            "karui_notable",
            &["add_armour", "add_burning_damage", "add_damage_from_crits"],
            &[],
        ),
        (
            "Militant Faith",
            "Avarius (Power of Purpose)",
            "templar_notable",
            &[
                "cold_conversion",
                "consecrated_ground_ailments",
                "minimum_endurance_charge",
            ],
            &[
                ("devotionSelect1", "Totem Damage"),
                ("devotionSelect2", "Brand Damage"),
            ],
        ),
    ];
    let mut kept = Vec::new();
    for (jewel, conqueror, prefix, nodes, devotion) in cases {
        let mut edits = vec![
            ("jewelSelect", s.choose("jewelSelect", jewel)),
            ("conquerorSelect", s.choose("conquerorSelect", conqueror)),
            ("socketSelect", s.choose("socketSelect", &socket_label)),
        ];
        for (name, label) in devotion {
            edits.push((name, s.choose(name, label)));
        }
        let text = PopupInput::Text(search_list(prefix, nodes));
        s.set("searchList", text.clone());
        edits.push(("searchList", text));
        for (name, value) in &edits {
            up_set(name, value);
        }

        // the press starts the job and returns at once; a second one is refused
        let pressed = s.press("searchButton");
        assert!(pressed.external_url.is_none());
        assert_eq!(
            s.backend.next_background(),
            Some(BackgroundTask::Timeless),
            "{jewel}: no job"
        );
        let again = s
            .run(Command::PressPopupButton {
                name: "searchButton".into(),
            })
            .unwrap_err()
            .to_string();
        assert_eq!(again, "a timeless search is already running");
        let progress = s.finish();
        assert_eq!(progress.last(), Some(&100), "{jewel}: {progress:?}");
        assert!(
            progress.windows(2).all(|w| w[0] <= w[1]),
            "{jewel}: progress fell: {progress:?}"
        );
        call(
            "press",
            mlua::MultiValue::from_iter([up.to_value("searchButton").unwrap()]),
        );
        let ours = s.results();
        let theirs = results_of(&up, "build");
        assert!(ours.len() > 100, "{jewel}: {} results", ours.len());
        assert!(ours.iter().any(|f| f.total > 0.0));
        assert_eq!(ours, theirs, "{jewel}: the results differ");
        // the window shows PoB's list and trade button
        let view = s.view();
        let list = view
            .controls
            .iter()
            .find(|c| c.name == "searchResults")
            .unwrap();
        assert_eq!(list.rows.len(), ours.len());
        assert!(
            view.controls
                .iter()
                .any(|c| c.name == "searchTradeButton" && c.enabled)
        );
        kept = ours;
    }

    // ---- "Search Maximum Amount" and "Open Trade URL": upstream's link ----
    s.set("searchMore", PopupInput::Check(true));
    up_set("searchMore", &PopupInput::Check(true));
    let url = s
        .press("searchTradeButton")
        .external_url
        .expect("no trade link")
        .url;
    let theirs: Option<String> = up
        .from_value(call(
            "press",
            mlua::MultiValue::from_iter([up.to_value("searchTradeButton").unwrap()]),
        ))
        .unwrap();
    assert_eq!(Some(url.clone()), theirs, "the trade links differ");
    assert!(
        url.starts_with("https://www.pathofexile.com/trade/search/sony/Settlers/?q="),
        "{url}"
    );

    // ---- Cancel keeps the previous results ----
    s.backend.timeless_budget_ms = 1;
    s.set(
        "searchList",
        PopupInput::Text(search_list("templar_notable", &["cold_conversion"])),
    );
    s.press("searchButton");
    s.step();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::Timeless));
    let ended = RefCell::new(Vec::new());
    let cancelled = s
        .backend
        .apply(Command::CancelTimelessSearch, &|e| {
            if let Event::Progress {
                task: BackgroundTask::Timeless,
                percent,
            } = e
            {
                ended.borrow_mut().push(percent);
            }
        })
        .unwrap();
    assert!(cancelled.popup);
    // the window's progress line is told the search is over
    assert_eq!(ended.into_inner(), [100]);
    assert_ne!(s.backend.next_background(), Some(BackgroundTask::Timeless));
    assert_eq!(s.results(), kept, "Cancel lost the previous results");

    // ---- Closing the window mid-search cancels it too ----
    s.press("searchButton");
    s.step();
    s.run(Command::ClosePopup).unwrap();
    assert_ne!(
        s.backend.next_background(),
        Some(BackgroundTask::Timeless),
        "the search outlived its window"
    );
    assert_eq!(s.results(), kept);
    s.run(Command::OpenPopup(PopupKind::TimelessJewel)).unwrap();

    // ---- A Lua error in a step: PoB's message in the window, the job over ----
    s.press("searchButton");
    s.step();
    s.eval::<()>(
        "local t = pob.build.timelessData
         saved = t.jewelType
         t.jewelType = setmetatable({}, { __index = function() error('boom', 0) end })",
    );
    let failed = s
        .backend
        .background_step(BackgroundTask::Timeless, &|_| {})
        .unwrap_err()
        .to_string();
    assert!(failed.contains("boom"), "{failed}");
    s.eval::<()>("pob.build.timelessData.jewelType = saved");
    assert_ne!(s.backend.next_background(), Some(BackgroundTask::Timeless));
    let msg = s
        .view()
        .controls
        .into_iter()
        .find(|c| c.name == "msg")
        .unwrap();
    assert!(msg.label.contains("boom"), "{}", msg.label);
    assert_eq!(s.popups(), 0);

    // ---- Responsive, with a socketed timeless jewel ----
    // a socket on the tree, holding a Lethal Pride: tree edits read its LUT outside the job
    let socket: u32 = s.eval(
        "local best, d = 0, 1e9
         for id in pairs(pob.build.itemsTab.sockets) do
            local n = pob.build.spec.nodes[id]
            if n and n.name ~= 'Charm Socket' and n.pathDist < d then best, d = id, n.pathDist end
         end
         return best",
    );
    s.run(Command::ClosePopup).unwrap();
    s.run(Command::AllocNode { id: socket }).unwrap();
    s.run(Command::PasteItem {
        text: "Rarity: UNIQUE\nLethal Pride\nTimeless Jewel\nRadius: Large\nImplicits: 0\nCommanded leadership over 12000 warriors under Kaom\nPassives in radius are Conquered by the Karui\nCorrupted".into(),
    })
    .unwrap();
    s.run(Command::SaveDisplayItem).unwrap();
    let jewel: u32 = s.eval(
        "local m = 0 for id in pairs(pob.build.itemsTab.items) do if id > m then m = id end end return m",
    );
    let slot = format!("Jewel {socket}");
    s.run(Command::EquipItem {
        slot: slot.clone(),
        id: Some(jewel),
    })
    .unwrap();
    s.eval::<()>(
        "lutCalls = 0
         local read = data.readLUT
         data.readLUT = function(...) lutCalls = lutCalls + 1 return read(...) end",
    );
    s.run(Command::OpenPopup(PopupKind::TimelessJewel)).unwrap();
    s.press("searchButton");
    s.step();
    assert_eq!(s.backend.next_background(), Some(BackgroundTask::Timeless));
    let calls_before: u32 = s.eval("return lutCalls");
    // a config change is answered between steps, with its stats
    let life_before = s.backend.stats_view().unwrap();
    s.run(Command::SetConfig {
        var: "usePowerCharges".into(),
        value: ConfigValue::Bool(true),
    })
    .unwrap();
    let life_after = s.backend.stats_view().unwrap();
    assert_ne!(
        format!("{life_before:?}"),
        format!("{life_after:?}"),
        "SetConfig changed no stat"
    );
    // the tree reads the jewel's LUT for itself, on the main thread, between steps
    s.run(Command::EquipItem {
        slot: slot.clone(),
        id: None,
    })
    .unwrap();
    s.run(Command::EquipItem {
        slot: slot.clone(),
        id: Some(jewel),
    })
    .unwrap();
    let calls_after: u32 = s.eval("return lutCalls");
    assert!(
        calls_after > calls_before,
        "the tree read no LUT between steps"
    );
    // a read from another coroutine passes straight through, never yields
    let node: u32 = s.eval(&format!(
        "for _, n in pairs(pob.build.spec.tree.nodes[{socket}].nodesInRadius[3]) do
            if n.isNotable and not n.isKeystone then return n.id end
         end"
    ));
    let (dead, read): (bool, bool) = s.eval(&format!(
        "local co = coroutine.create(function() return data.readLUT(12000, {node}, 2) end)
         local ok, got = coroutine.resume(co)
         return coroutine.status(co) == 'dead', ok and type(got) == 'table' and next(got) ~= nil"
    ));
    assert!(dead && read, "a read in another coroutine yielded");
    assert_eq!(
        s.backend.next_background(),
        Some(BackgroundTask::Timeless),
        "the search ended early"
    );
    let progress = s.finish();
    assert_eq!(progress.last(), Some(&100));

    // ---- Review Focus 4: a build switch mid-search ----
    s.press("searchButton");
    s.step().expect("a progress event");
    let switched = s
        .run(Command::OpenBuild {
            path: builds.join("dot.xml"),
        })
        .unwrap();
    assert!(switched.popup, "the window wasn't told");
    assert!(
        s.backend.popup_view().unwrap().is_none(),
        "the session survived"
    );
    assert_ne!(s.backend.next_background(), Some(BackgroundTask::Timeless));
    let dot_results: usize = s.eval("return #pob.build.timelessData.searchResults");
    assert_eq!(dot_results, 0, "dot got results");
    s.backend.timeless_budget_ms = auto_pob::engine::pob::TIMELESS_BUDGET_MS;
    s.run(Command::OpenPopup(PopupKind::TimelessJewel)).unwrap();
    s.choose("jewelSelect", "Lethal Pride");
    s.choose("conquerorSelect", "Kaom (Strength of Blood)");
    let dot_socket = s
        .view()
        .controls
        .iter()
        .find(|c| c.name == "socketSelect")
        .unwrap()
        .choices[1]
        .clone();
    s.choose("socketSelect", &dot_socket);
    s.set(
        "searchList",
        PopupInput::Text(search_list("karui_notable", &["add_armour"])),
    );
    s.press("searchButton");
    s.finish();
    assert!(!s.results().is_empty(), "a new search on dot found nothing");
    s.run(Command::ClosePopup).unwrap();

    // ---- No request, no file ----
    assert_eq!(
        s.backend.net_handle().refused(),
        refused,
        "a fetch was tried"
    );
    assert!(s.backend.net_handle().running_ids().is_empty());
    assert!(web.requests().is_empty());
    assert!(web.unrouted().is_empty());
    // upstream's window fetched its leagues (its stub launched nothing); ours never asked
    let launches: u32 = up.load("return launches").eval().unwrap();
    assert!(
        launches >= 1,
        "upstream never fetched: the comparison is vacuous"
    );
    assert_eq!(files(&s.user), before, "a file was written");
    assert_eq!(s.popups(), 0);
    let bins: Vec<String> = fs::read_dir(pob.join("Data/TimelessJewelData"))
        .unwrap()
        .filter_map(|e| {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".bin")
                .then(|| format!("{name} {}", e.metadata().unwrap().len()))
        })
        .collect();
    eprintln!("LUT caches written: {bins:?}");
    drop(s);
    let secs = started.elapsed().as_secs_f64();
    assert!(secs < 60.0, "took {secs:.1}s");
}
