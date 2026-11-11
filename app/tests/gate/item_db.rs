//! Gate tests for the uniques and rares databases: the engine's `SetItemDb` job against
//! unmodified upstream PoB's own `ItemDBControl`, drawn until its list builder ends.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::crafting::crafting_round_trip;
use super::items_harness::{
    assert_same, json, json_of, open_popups, run_items_recipe, settle, strip_nulls,
    upstream_with_items_lib,
};
use super::{Session, fixtures_dir, lock};
use auto_pob::engine::{
    Backend, BackgroundTask, Command, ConfigValue, ControlTooltipQuery, DbSnapshot, DropList,
    ItemDb, ItemDbQuery, ItemDbView,
};
use mlua::LuaSerdeExt;

/// A query by the labels PoB's dropdowns show
#[derive(Clone)]
struct Labels {
    db: ItemDb,
    slot: &'static str,
    kind: &'static str,
    league: &'static str,
    requirement: &'static str,
    obtainable: &'static str,
    sort: &'static str,
    search: &'static str,
    mode: &'static str,
}

impl Labels {
    fn uniques() -> Labels {
        Labels {
            db: ItemDb::Uniques,
            slot: "Any slot",
            kind: "Any type",
            league: "Any league",
            requirement: "Any requirements",
            obtainable: "Obtainable",
            sort: "Sort by Name",
            search: "",
            mode: "Anywhere",
        }
    }

    fn rares() -> Labels {
        Labels {
            db: ItemDb::Rares,
            ..Labels::uniques()
        }
    }

    /// Upstream's table for `dbDump`: PoB's own lists resolve the labels
    fn upstream(&self, lua: &mlua::Lua) -> mlua::Table {
        let t = lua.create_table().unwrap();
        t.set("slot", self.slot).unwrap();
        t.set("kind", self.kind).unwrap();
        t.set("search", self.search).unwrap();
        t.set("search_mode", self.mode).unwrap();
        if self.db == ItemDb::Uniques {
            t.set("league", self.league).unwrap();
            t.set("requirement", self.requirement).unwrap();
            t.set("obtainable", self.obtainable).unwrap();
            t.set("sort", self.sort).unwrap();
        }
        t
    }

    /// Our query, with the indices found in the labels of a view of the same database
    fn ours(&self, lists: &ItemDbView) -> ItemDbQuery {
        let at = |list: &DropList, label: &str| {
            list.labels
                .iter()
                .position(|l| l == label)
                .unwrap_or_else(|| panic!("no entry {label} in {:?}", list.labels))
        };
        let unique = self.db == ItemDb::Uniques;
        ItemDbQuery {
            db: self.db,
            slot: at(&lists.slot, self.slot),
            kind: at(&lists.kind, self.kind),
            league: if unique {
                at(lists.league.as_ref().unwrap(), self.league)
            } else {
                0
            },
            requirement: if unique {
                at(lists.requirement.as_ref().unwrap(), self.requirement)
            } else {
                0
            },
            obtainable: if unique {
                at(lists.obtainable.as_ref().unwrap(), self.obtainable)
            } else {
                0
            },
            sort: if unique {
                at(lists.sort.as_ref().unwrap(), self.sort)
            } else {
                0
            },
            search: self.search.into(),
            search_mode: at(&lists.search_mode, self.mode),
        }
    }
}

fn db_name(db: ItemDb) -> &'static str {
    match db {
        ItemDb::Uniques => "Uniques",
        ItemDb::Rares => "Rares",
    }
}

/// Opens the query and lets the job finish, as the engine loop would; then the panel
fn ours(s: &mut Session, query: &ItemDbQuery) -> ItemDbView {
    let affected = s.run(Command::SetItemDb(query.clone())).unwrap();
    assert!(affected.item_db);
    settle(s);
    panel(s)
}

fn panel(s: &mut Session) -> ItemDbView {
    match s.backend.item_db_view().unwrap() {
        DbSnapshot::View(view) => view,
        other => panic!("no panel: {other:?}"),
    }
}

/// Upstream's panel for the query: PoB's controls set, its list control drawn to the end
fn theirs(lua: &mlua::Lua, labels: &Labels) -> serde_json::Value {
    let dump: mlua::Function = lua.globals().get("dbDump").unwrap();
    let value: mlua::Value = dump
        .call((db_name(labels.db), labels.upstream(lua)))
        .unwrap();
    dump_json(lua, value)
}

fn dump_json(lua: &mlua::Lua, value: mlua::Value) -> serde_json::Value {
    let mut json: serde_json::Value = strip_nulls(lua.from_value(value).unwrap());
    // how many times the list control was drawn: the check that a stat sort ends in one call
    json.as_object_mut().unwrap().remove("draws");
    json
}

fn tooltip_theirs(lua: &mlua::Lua, db: ItemDb, name: &str) -> serde_json::Value {
    let tooltip: mlua::Function = lua.globals().get("dbTooltip").unwrap();
    let value: mlua::Value = tooltip.call((db_name(db), name)).unwrap();
    strip_nulls(lua.from_value(value).unwrap())
}

/// The rows first, in the middle and last
fn sample(rows: usize) -> Vec<usize> {
    let mut picks = vec![0, rows / 2, rows - 1];
    picks.dedup();
    picks
}

/// Our panel equals upstream's, and so do the tooltips of three rows
fn same_panel(
    s: &mut Session,
    lua: &mlua::Lua,
    labels: &Labels,
    lists: &ItemDbView,
    what: &str,
) -> ItemDbView {
    let query = labels.ours(lists);
    let view = ours(s, &query);
    let want = theirs(lua, labels);
    assert_same(&json_of(&view), &want, what);
    assert!(!view.rows.is_empty(), "{what}: no rows");
    assert!(!view.loading && !view.sorting, "{what}");
    for i in sample(view.rows.len()) {
        let name = &view.rows[i].name;
        let tooltip = s
            .backend
            .control_tooltip_view(&ControlTooltipQuery::DbItem {
                db: labels.db,
                name: name.clone(),
            })
            .unwrap()
            .unwrap_or_else(|| panic!("{what}: no tooltip for {name}"));
        assert!(!tooltip.tooltip.lines.is_empty(), "{what}: {name}");
        assert_same(
            &json_of(&tooltip.tooltip),
            &tooltip_theirs(lua, labels.db, name),
            &format!("{what}: tooltip of {name}"),
        );
    }
    assert_eq!(open_popups(s), 0, "{what}: a popup was left open");
    view
}

/// Refuses with this message and leaves the panel and the items as they were
fn refused(s: &mut Session, command: Command, expected: &str) {
    let panel_before = json_of(&panel(s));
    let items_before = json(&s.backend.items_view().unwrap());
    let shown = format!("{command:?}");
    let err = s.run(command).expect_err(&shown);
    assert!(format!("{err:#}").contains(expected), "{shown}: {err:#}");
    assert_eq!(
        json_of(&panel(s)),
        panel_before,
        "{shown}: the panel changed"
    );
    assert_eq!(
        json(&s.backend.items_view().unwrap()),
        items_before,
        "{shown}: the items changed"
    );
    assert_eq!(open_popups(s), 0, "{shown}");
}

/// The database with every filter at its default: its panel holds the dropdowns' lists
fn lists(s: &mut Session, db: ItemDb) -> ItemDbView {
    ours(
        s,
        &ItemDbQuery {
            db,
            slot: 0,
            kind: 0,
            league: 0,
            requirement: 0,
            obtainable: 0,
            sort: 0,
            search: String::new(),
            search_mode: 0,
        },
    )
}

const LIFE_MOD: &str = "+500 to maximum Life";

#[test]
#[ignore = "needs the pinned PoB install"]
fn item_db_matches_pob() {
    let _guard = lock();
    let started = std::time::Instant::now();
    let xml = fixtures_dir().join("attack.xml");
    let mut s = Session::start("item-db");
    s.open_fixture("attack", &xml);
    let lua = upstream_with_items_lib(&xml, "attack");
    // the dropdowns' lists, once PoB has loaded the databases
    let unique_lists = lists(&mut s, ItemDb::Uniques);
    let rare_lists = lists(&mut s, ItemDb::Rares);
    eprintln!("item_db: loaded {:?}", started.elapsed());

    let belt_by_life = Labels {
        slot: "Belt",
        sort: "Sort by Life",
        ..Labels::uniques()
    };
    let queries = [
        (
            "(a) uniques: ring, name sort",
            Labels {
                slot: "Ring",
                ..Labels::uniques()
            },
        ),
        (
            "(b) uniques: jewellery, life in modifiers, current level",
            Labels {
                kind: "Jewellery",
                search: "life",
                mode: "Modifiers",
                requirement: "Current level",
                league: "No league",
                obtainable: "Any source",
                ..Labels::uniques()
            },
        ),
        (
            "(c) rares: belt",
            Labels {
                slot: "Belt",
                ..Labels::rares()
            },
        ),
        ("(d) uniques: belt, sort by life", belt_by_life.clone()),
    ];
    let mut non_default = (false, false, false);
    let mut stat_values = Vec::new();
    for (what, labels) in &queries {
        let lists = if labels.db == ItemDb::Uniques {
            &unique_lists
        } else {
            &rare_lists
        };
        let query = labels.ours(lists);
        non_default.0 |= query.league > 0;
        non_default.1 |= query.requirement > 0;
        non_default.2 |= query.obtainable > 0;
        let view = same_panel(&mut s, &lua, labels, lists, what);
        if labels.sort == "Sort by Life" {
            stat_values = view.rows.iter().map(|r| r.sort_value).collect();
        } else {
            assert!(view.rows.iter().all(|r| r.sort_value.is_none()), "{what}");
        }
    }
    assert!(
        non_default.0 && non_default.1 && non_default.2,
        "{non_default:?}"
    );
    // the stat sort has values, and they differ from row to row
    let mut distinct: Vec<String> = stat_values.iter().map(|v| format!("{v:?}")).collect();
    distinct.sort();
    distinct.dedup();
    assert!(stat_values.iter().all(Option::is_some), "{stat_values:?}");
    assert!(distinct.len() >= 3, "{distinct:?}");
    eprintln!("item_db: queries {:?}", started.elapsed());

    // a recalculation restarts the sort, as PoB's Draw does (Review Focus 2)
    let before = panel(&mut s);
    s.run(Command::SetCustomModText {
        group: 0,
        text: LIFE_MOD.into(),
    })
    .unwrap();
    assert!(
        s.backend.next_background() == Some(BackgroundTask::ItemDb),
        "the recalculation didn't restart the sort"
    );
    settle(&mut s);
    let after = panel(&mut s);
    lua.load(format!(
        "build.configTab.customModsBlockControls[1].controls.textEdit:SetText({LIFE_MOD:?}, true)\n\
         build.buildFlag = true\n\
         runCallback('OnFrame')"
    ))
    .exec()
    .unwrap();
    let want = theirs(&lua, &belt_by_life);
    assert_same(&json_of(&after), &want, "after the recalculation");
    assert_ne!(
        before.rows.iter().map(|r| r.sort_value).collect::<Vec<_>>(),
        after.rows.iter().map(|r| r.sort_value).collect::<Vec<_>>(),
        "the values didn't move with the recalculation"
    );

    // refused: nothing changes
    let name = after.rows[0].name.clone();
    refused(
        &mut s,
        Command::ViewDbItem {
            db: ItemDb::Uniques,
            name: "No Such Item".into(),
        },
        "no such item in the database",
    );
    refused(
        &mut s,
        Command::AddDbItem {
            db: ItemDb::Rares,
            name: name.clone(),
        },
        "no such item in the database",
    );
    let mut bad = belt_by_life.ours(&unique_lists);
    bad.slot = unique_lists.slot.labels.len();
    refused(&mut s, Command::SetItemDb(bad), "no such choice");
    let mut rares_sort = Labels {
        slot: "Belt",
        ..Labels::rares()
    }
    .ours(&rare_lists);
    rares_sort.sort = 1;
    refused(&mut s, Command::SetItemDb(rares_sort), "no such filter");
    // a database that is still loading takes no clicks
    s.backend
        .lua
        .load("pob.main.main.uniqueDB.loading = true")
        .exec()
        .unwrap();
    for command in [
        Command::AddDbItem {
            db: ItemDb::Uniques,
            name: name.clone(),
        },
        Command::ViewDbItem {
            db: ItemDb::Uniques,
            name: name.clone(),
        },
    ] {
        let shown = format!("{command:?}");
        let err = s.run(command).expect_err(&shown);
        assert!(
            format!("{err:#}").contains("the item database is still loading"),
            "{shown}: {err:#}"
        );
    }
    s.backend
        .lua
        .load("pob.main.main.uniqueDB.loading = nil")
        .exec()
        .unwrap();

    // a click is answered during a stat sort, when the shown list is empty
    s.run(Command::SetItemDb(belt_by_life.ours(&unique_lists)))
        .unwrap();
    s.run(Command::SetConfig {
        var: "usePowerCharges".into(),
        value: ConfigValue::Bool(true),
    })
    .unwrap();
    let sorting = panel(&mut s);
    assert!(
        sorting.sorting && sorting.rows.is_empty(),
        "no sort is running"
    );
    let items_before = json(&s.backend.items_view().unwrap());
    let affected = s
        .run(Command::AddDbItem {
            db: ItemDb::Uniques,
            name: name.clone(),
        })
        .unwrap();
    assert!(affected.items && affected.build && affected.recalc);
    let items_after = json(&s.backend.items_view().unwrap());
    assert_ne!(items_after, items_before);
    assert!(
        items_after.to_string().contains(&name),
        "{name} was not added"
    );
    let affected = s
        .run(Command::ViewDbItem {
            db: ItemDb::Uniques,
            name: name.clone(),
        })
        .unwrap();
    assert!(affected.display_item);
    assert!(s.backend.display_item_view().unwrap().is_some());
    assert_eq!(open_popups(&s), 0);
    eprintln!("item_db_matches_pob: {:?}", started.elapsed());
}

/// Commands are answered while the database sorts, as while the gem picker's DPS sort runs:
/// the engine takes one slice of PoB's list builder between commands (DoD 5).
#[test]
#[ignore = "needs the pinned PoB install"]
fn item_db_responsive() {
    use auto_pob::engine::{Engine, Event, Snapshot};
    use auto_pob::game::Game;
    use std::time::{Duration, Instant};
    let _guard = lock();
    super::pob_dir();
    let started = Instant::now();
    let user = std::env::temp_dir().join(format!("auto-pob-gate-item-db-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    std::fs::create_dir_all(&user).unwrap();
    let build = user.join("attack.xml");
    std::fs::copy(fixtures_dir().join("attack.xml"), &build).unwrap();
    let engine = Engine::spawn(Game::Poe1, std::env::temp_dir(), user.clone(), || {});
    engine.send(Command::OpenBuild { path: build });
    // the lists come with the first panel; Body Armour is slot 4, "Sort by Life" is found by label
    engine.send(Command::SetItemDb(ItemDbQuery {
        db: ItemDb::Uniques,
        slot: 4,
        kind: 0,
        league: 0,
        requirement: 0,
        obtainable: 0,
        sort: SORT_BY_LIFE,
        search: String::new(),
        search_mode: 0,
    }));
    let deadline = Instant::now() + Duration::from_secs(300);
    let next = || {
        let wait = deadline.saturating_duration_since(Instant::now());
        engine.events.recv_timeout(wait).expect("an event")
    };
    // the sort has started when the first slice reports progress
    let mut progress = 0;
    loop {
        match next() {
            Event::Progress {
                task: BackgroundTask::ItemDb,
                ..
            } => {
                progress += 1;
                break;
            }
            Event::Snapshot {
                snapshot: Snapshot::ItemDb(Some(view)),
                ..
            } => {
                assert_eq!(
                    view.sort.as_ref().unwrap().labels[SORT_BY_LIFE],
                    "Sort by Life"
                );
                assert!(view.rows.is_empty() || !view.sorting, "rows while sorting");
                assert!(
                    view.sorting || view.loading,
                    "the sort ended before the label"
                );
            }
            Event::Error { message, .. }
            | Event::StartupFailed { message }
            | Event::BackgroundFailed { message, .. } => panic!("{message}"),
            _ => {}
        }
    }
    let label_seq = engine.send(Command::SetGroupLabel {
        group: 0,
        text: "Mid sort".into(),
    });
    let mut labelled = false;
    loop {
        match next() {
            Event::Snapshot {
                seq,
                snapshot: Snapshot::Skills(view),
            } if seq >= label_seq => {
                assert_eq!(view.groups[0].label, "Mid sort");
                labelled = true;
            }
            Event::Snapshot {
                snapshot: Snapshot::ItemDb(Some(view)),
                ..
            } => {
                // rows arrive only with the end of the sort
                assert_eq!(view.sorting, view.rows.is_empty(), "{}", view.sorting);
                if !view.sorting {
                    assert!(labelled, "the sort ended before the label was answered");
                    assert!(view.rows.iter().all(|r| r.sort_value.is_some()));
                    break;
                }
            }
            Event::Progress {
                task: BackgroundTask::ItemDb,
                percent,
            } => {
                assert!(percent <= 100);
                progress += 1;
            }
            Event::Error { message, .. } | Event::BackgroundFailed { message, .. } => {
                panic!("{message}")
            }
            _ => {}
        }
    }
    assert!(progress >= 3, "{progress} progress events");
    engine.send(Command::Shutdown);
    while engine.events.recv_timeout(Duration::from_secs(30)).is_ok() {}
    assert!(!engine.is_alive());
    let _ = std::fs::remove_dir_all(&user);
    eprintln!(
        "item_db_responsive: {:?}, {progress} progress events",
        started.elapsed()
    );
}

/// "Sort by Life" in the uniques database's sort dropdown (checked against its labels)
const SORT_BY_LIFE: usize = 11;

/// Browse, view, save and add through the recorded recipe (DoD 3), then the round trip
#[test]
#[ignore = "needs the pinned PoB install"]
fn db_add_recipe_matches_pob() {
    let _guard = lock();
    let started = std::time::Instant::now();
    let (recipe, mut s) = run_items_recipe("db_add");
    // Kaom's Heart is shown, then saved and equipped; the Ctrl+click equips a ring and the
    // undo takes it away again
    let checks = super::items_harness::checkpoints(&recipe);
    assert_eq!(checks.len(), 4);
    let equipped = |i: usize, slot: &str| {
        checks[i].items["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == slot)
            .and_then(|s| s["selected"].as_u64())
            .unwrap_or(0)
    };
    assert!(checks[0].display.is_some() && checks[1].display.is_none());
    assert_ne!(equipped(1, "Body Armour"), 0, "the unique was not equipped");
    assert_eq!(equipped(1, "Ring 1"), 0);
    assert_ne!(equipped(2, "Ring 1"), 0, "the ring was not equipped");
    assert_eq!(equipped(3, "Ring 1"), 0, "the undo left the ring on");
    assert_eq!(checks[2].items["can_undo"], true);
    assert_eq!(open_popups(&s), 0);
    crafting_round_trip("db_add", &recipe, &mut s);
    eprintln!("db_add: {:?}", started.elapsed());
}
