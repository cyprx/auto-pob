//! Gate tests for the name boxes: what PoB's own `EditControl`s keep and when their Save stays
//! disabled, for the set, tree and build names the engine types into them. Included from
//! `gate.rs`, whose helpers it reaches through `super::`.

use super::items_harness::open_popups;
use super::{Session, fixture_xml, lock};
use auto_pob::engine::{Backend, Command};

const JAPANESE: &str = "\u{65e5}\u{672c}";

/// The sets and trees PoB names through its popup: the box drops non-ASCII bytes, a name left
/// with nothing is refused with the popup closed, and what is left is saved as it stands
#[test]
#[ignore = "needs the pinned PoB install"]
fn set_and_tree_names_pass_through_pobs_edit_filter() {
    let _guard = lock();
    let mut s = Session::start("names-sets");
    s.open_fixture("attack", &fixture_xml("attack"));

    let skills = |s: &mut Session| -> Vec<String> {
        let sets = s.backend.skills_view().unwrap().sets;
        sets.into_iter().map(|e| e.title).collect()
    };
    let items = |s: &mut Session| -> Vec<String> {
        let sets = s.backend.items_view().unwrap().sets;
        sets.into_iter().map(|e| e.title).collect()
    };
    let trees = |s: &mut Session| -> Vec<String> {
        let sets = s.backend.tree_state_view().unwrap().specs;
        sets.into_iter().map(|e| e.title).collect()
    };
    let configs = |s: &mut Session| -> Vec<String> {
        let sets = s.backend.config_view().unwrap().sets;
        sets.into_iter().map(|e| e.title).collect()
    };
    let (skills0, items0, trees0, configs0) = (
        skills(&mut s),
        items(&mut s),
        trees(&mut s),
        configs(&mut s),
    );

    // nothing PoB's box keeps: no set, no tree, no popup left open
    for name in [JAPANESE, "  "] {
        s.run(Command::NewSkillSet { title: name.into() })
            .expect_err("a skill set with no name");
        s.run(Command::NewItemSet { title: name.into() })
            .expect_err("an item set with no name");
        s.run(Command::NewSpec { title: name.into() })
            .expect_err("a tree with no name");
        s.run(Command::NewConfigSet { title: name.into() })
            .expect_err("a config set with no name");
        assert_eq!(open_popups(&s), 0, "{name:?}");
    }
    // more than 100 bytes is more than the box takes
    let long = "x".repeat(101);
    s.run(Command::NewSkillSet {
        title: long.clone(),
    })
    .expect_err("a skill set name over the limit");
    s.run(Command::NewSpec { title: long })
        .expect_err("a tree name over the limit");
    assert_eq!(open_popups(&s), 0);
    assert_eq!(skills(&mut s), skills0);
    assert_eq!(items(&mut s), items0);
    assert_eq!(trees(&mut s), trees0);
    assert_eq!(configs(&mut s), configs0);

    // a name with something PoB keeps is saved without the rest, and is not trimmed
    let typed = format!("{JAPANESE} a ");
    s.run(Command::NewSkillSet {
        title: typed.clone(),
    })
    .unwrap();
    s.run(Command::NewItemSet {
        title: typed.clone(),
    })
    .unwrap();
    s.run(Command::NewSpec {
        title: typed.clone(),
    })
    .unwrap();
    s.run(Command::NewConfigSet { title: typed }).unwrap();
    assert_eq!(open_popups(&s), 0);
    assert_eq!(skills(&mut s).last().map(String::as_str), Some(" a "));
    assert_eq!(items(&mut s).last().map(String::as_str), Some(" a "));
    assert_eq!(trees(&mut s).last().map(String::as_str), Some(" a "));
    assert_eq!(configs(&mut s).last().map(String::as_str), Some(" a "));

    // a rename goes through the same box, and a copy too
    let last = trees(&mut s).len() - 1;
    s.run(Command::RenameSpec {
        index: last,
        title: format!("{JAPANESE}b"),
    })
    .unwrap();
    assert_eq!(trees(&mut s)[last], "b");
    s.run(Command::CopySpec {
        index: last,
        title: format!("c{JAPANESE}"),
    })
    .unwrap();
    assert_eq!(trees(&mut s).last().map(String::as_str), Some("c"));
    s.run(Command::ImportTreeLink {
        title: JAPANESE.into(),
        url: "https://www.pathofexile.com/passive-skill-tree/AAAABgAAAAAA".into(),
    })
    .expect_err("an import with no name");
    assert_eq!(open_popups(&s), 0);

    // a title the file holds keeps its non-ASCII text when a rename adds to it (Insert filters
    // only what it inserts, EditControl.lua:162)
    let id = s.backend.skills_view().unwrap().sets[0].id;
    s.backend
        .lua
        .load(format!(
            "pob.build.skillsTab.skillSets[{id}].title = '\\195\\156n\\195\\175'"
        ))
        .exec()
        .unwrap();
    s.run(Command::RenameSkillSet {
        id,
        title: "\u{dc}n\u{ef}x".into(),
    })
    .unwrap();
    assert_eq!(skills(&mut s)[0], "\u{dc}n\u{ef}x");
    s.run(Command::RenameSkillSet {
        id,
        title: "\u{dc}n\u{ef}x\u{e9}".into(),
    })
    .expect_err("a character the box drops is no edit");
    assert_eq!(skills(&mut s)[0], "\u{dc}n\u{ef}x");
    assert_eq!(open_popups(&s), 0);

    // a set made with a title shows it; the views say which sets have none
    assert!(
        s.backend
            .skills_view()
            .unwrap()
            .sets
            .iter()
            .all(|e| !e.untitled || e.title == "Default")
    );
}

/// A custom modifier group's title box is PoB's too (ConfigTab.lua:35), and Save As refuses a
/// name PoB's box would not hold
#[test]
#[ignore = "needs the pinned PoB install"]
fn custom_mod_titles_and_build_names_pass_through_pobs_edit_filter() {
    let _guard = lock();
    let mut s = Session::start("names-mods");
    s.open_fixture("attack", &fixture_xml("attack"));
    s.run(Command::AddCustomModGroup).unwrap();
    let group = s.config().custom_mods.len() - 1;
    s.run(Command::SetCustomModTitle {
        group,
        title: format!("{JAPANESE} t"),
    })
    .unwrap();
    assert_eq!(s.config().custom_mods[group].title, " t");
    // nothing left to type: the box keeps its text
    s.run(Command::SetCustomModTitle {
        group,
        title: JAPANESE.into(),
    })
    .unwrap();
    assert_eq!(s.config().custom_mods[group].title, " t");

    let before = std::fs::read_dir(s.builds_dir()).unwrap().count();
    for name in [
        " ", "a/b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\tb",
    ] {
        s.run(Command::SaveAs {
            folder: String::new(),
            name: name.into(),
            overwrite: false,
        })
        .expect_err(name);
    }
    assert_eq!(std::fs::read_dir(s.builds_dir()).unwrap().count(), before);
    // PoB's box keeps the rest, non-ASCII text included
    s.run(Command::SaveAs {
        folder: String::new(),
        name: format!("{JAPANESE} build "),
        overwrite: false,
    })
    .unwrap();
    assert!(
        s.builds_dir()
            .join(format!("{JAPANESE} build .xml"))
            .exists()
    );
}
