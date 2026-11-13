use super::*;
use crate::engine::views::{
    BuildBrowseEntry, BuildBrowseView, BuildView, CompareConfigRow, CompareItemRow, CompareRow,
    CompareSkillRow, ConfigSet, PopupControlKind,
};
use crate::theme;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

fn control(name: &str, kind: PopupControlKind) -> PopupControl {
    PopupControl {
        name: name.into(),
        kind,
        label: String::new(),
        pos: [0.0, 0.0],
        size: [150.0, 20.0],
        shown: true,
        enabled: true,
        choices: vec![],
        selected: 0,
        value: 0.0,
        divisions: None,
        checked: false,
        text: String::new(),
        tooltip_text: None,
        hover_tooltip: false,
        rows: vec![],
        row_selected: None,
        row_highlight: None,
        note: None,
        reorder: false,
    }
}

fn dropdown(name: &str, choices: &[&str]) -> PopupControl {
    PopupControl {
        choices: choices.iter().map(|c| c.to_string()).collect(),
        ..control(name, PopupControlKind::Dropdown)
    }
}

/// One Items row; every test builds its rows here, as the struct may grow. `diff_label` is
/// whatever the engine sends: the UI draws it as it is.
pub(super) fn item_row(
    slot: &str,
    ours: Option<u32>,
    theirs: Option<&str>,
    diff_label: &str,
    can_copy: bool,
) -> CompareItemRow {
    CompareItemRow {
        slot: slot.into(),
        label: slot.into(),
        ours,
        theirs: theirs.map(str::to_owned),
        differ: true,
        can_copy,
        can_equip: can_copy,
        diff_label: diff_label.into(),
        ours_colour: "^7".into(),
        theirs_colour: "^7".into(),
        ..CompareItemRow::default()
    }
}

fn row(label: &str, diff: &str, better: Option<bool>) -> CompareRow {
    CompareRow {
        label: format!("^7{label}:"),
        ours: "^71,000".into(),
        theirs: "^7900".into(),
        diff: diff.into(),
        better,
    }
}

pub(crate) fn sample() -> CompareView {
    let mut hidden = dropdown("cmpSkillPart", &["Part 1"]);
    hidden.shown = false;
    let mut overlay = control(OVERLAY_CHECK, PopupControlKind::Check);
    overlay.label = "Overlay comparison".into();
    overlay.checked = true;
    CompareView {
        entries: vec!["dot".into(), "^7Imported build".into()],
        active: 0,
        headers: ["^x33FF77attack".into(), "^xFF9922dot".into()],
        controls: vec![
            dropdown("compareSpecSelect", &["Default", "Leveling"]),
            dropdown("compareSkillSetSelect", &["Default"]),
            dropdown("cmpSocketGroup", &["Fireball", "Arc"]),
            hidden,
            control("cmpStageCount", PopupControlKind::Edit),
            dropdown("primaryItemSetSelect", &["Default", "Swap"]),
            dropdown("compareItemSetSelect2", &["Theirs"]),
            overlay,
            PopupControl {
                label: "Expanded mode".into(),
                ..control(EXPANDED_CHECK, PopupControlKind::Check)
            },
            PopupControl {
                selected: 2,
                ..dropdown("leftVersionSelect", &["3.25", "3.26", "3.27"])
            },
            PopupControl {
                selected: 1,
                ..dropdown("rightVersionSelect", &["3.25", "3.26", "3.27"])
            },
            PopupControl {
                label: "Show All Configurations".into(),
                ..control("configToggleBtn", PopupControlKind::Button)
            },
        ],
        summary: vec![
            row("Life", "^x33FF77+100", Some(true)),
            row("Mana", "^xDD0022-50", Some(false)),
            row("Armour", "", None),
        ],
        items: vec![
            CompareItemRow {
                theirs_colour: "^xAF6025".into(),
                ..item_row(
                    "Body Armour",
                    Some(2),
                    Some("Tabula Rasa"),
                    "^xFF9922(different)",
                    true,
                )
            },
            CompareItemRow {
                can_equip: false,
                ..item_row(
                    "Ring 3",
                    None,
                    Some("Ming's Heart"),
                    "^xDD0022(missing)",
                    true,
                )
            },
            item_row("Ring 1", Some(1), None, "^x80A080(extra)", false),
            item_row("Jewel 61834", None, None, "^8(both empty)", false),
            CompareItemRow {
                diff_label: "^x33FF77(match)".into(),
                ours_unallocated: true,
                ours_name: Some("Cobalt Jewel".into()),
                ours_colour: "^x8888FF".into(),
                theirs_colour: "^xFFFF77".into(),
                ..item_row(
                    "Jewel 100",
                    Some(4),
                    Some("Prismatic Jewel"),
                    "^x33FF77(match)",
                    true,
                )
            },
        ],
        skills: vec![CompareSkillRow {
            label: "^7Fireball".into(),
            theirs_label: Some("^7Fireball Echo".into()),
            ours: vec!["^7Fireball 20/0".into()],
            theirs: vec!["^x33FF77+ Spell Echo".into()],
            differ: true,
        }],
        config: vec![
            CompareConfigRow {
                var: "usePowerCharges".into(),
                label: "Do you use Power Charges?".into(),
                ours: "Yes".into(),
                theirs: "No".into(),
                common: false,
            },
            CompareConfigRow {
                var: "enemyIsBoss".into(),
                label: "Is the enemy a Boss?".into(),
                ours: "None".into(),
                theirs: "Pinnacle".into(),
                common: false,
            },
            CompareConfigRow {
                var: "bandit".into(),
                label: "Bandit quest:".into(),
                ours: "Kill all".into(),
                theirs: "Kill all".into(),
                common: true,
            },
        ],
        switched: false,
    }
}

struct Harness {
    ui: CompareUi,
    /// What `show` is told: the engine is behind
    busy: bool,
    ctx: egui::Context,
    time: f64,
    /// The galleys drawn in the latest frame
    texts: Vec<Arc<egui::Galley>>,
}

fn debug(commands: &[Command]) -> Vec<String> {
    commands.iter().map(|c| format!("{c:?}")).collect()
}

impl Harness {
    fn new(view: Option<CompareView>) -> Self {
        let mut ui = CompareUi::default();
        ui.on_snapshot(&Snapshot::Compare(view.map(Arc::new)));
        let mut h = Self {
            ui,
            busy: false,
            ctx: egui::Context::default(),
            time: 1.0,
            texts: vec![],
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) -> CompareOutput {
        self.time += 0.2;
        self.ui.forget_rects();
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut out = CompareOutput::default();
        let output = self.ctx.run_ui(input, |ui| {
            // egui may run a frame twice; each pass's output counts
            let pass = self.ui.show(ui, self.busy);
            out.commands.extend(pass.commands);
            out.queries.extend(pass.queries);
            out.tree |= pass.tree;
        });
        self.texts = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.clone()),
                _ => None,
            })
            .collect();
        output.drop_without_applying_deltas();
        out
    }

    fn click(&mut self, key: &str) -> CompareOutput {
        // a window takes its new size a frame late: let it settle before aiming
        let settle = self.frame(vec![]);
        let pos = self.ui.rect(key).center();
        let mut out = self.click_at(pos);
        out.commands.splice(0..0, settle.commands);
        out.queries.splice(0..0, settle.queries);
        out
    }

    fn click_at(&mut self, pos: Pos2) -> CompareOutput {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut all = CompareOutput::default();
        for events in [
            vec![Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            let o = self.frame(events);
            all.commands.extend(o.commands);
            all.queries.extend(o.queries);
            all.tree |= o.tree;
        }
        all
    }

    fn hover(&mut self, key: &str) -> CompareOutput {
        let pos = self.ui.rect(key).center();
        let mut all = CompareOutput::default();
        for _ in 0..3 {
            let o = self.frame(vec![Event::PointerMoved(pos)]);
            all.queries.extend(o.queries);
        }
        all
    }

    fn type_text(&mut self, text: &str) -> CompareOutput {
        self.frame(vec![Event::Text(text.into())])
    }

    fn key(&mut self, key: Key) -> CompareOutput {
        self.frame(vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }])
    }

    /// The colour the drawn text `text` was drawn in
    fn colour_of(&self, text: &str) -> Option<egui::Color32> {
        self.texts.iter().find_map(|g| {
            (g.job.text == text).then(|| g.job.sections.first().map(|s| s.format.color))?
        })
    }

    fn drew(&self, text: &str) -> bool {
        self.texts.iter().any(|g| g.job.text == text)
    }
}

#[test]
fn compare_tab_hidden_until_a_build_is_open() {
    // no comparison yet: only the load menu, no views, selectors, entries or Remove
    let mut h = Harness::new(None);
    assert!(h.ui.has_rect("load"));
    for key in ["entries", "remove", "sub:summary", "compareSpecSelect"] {
        assert!(!h.ui.has_rect(key), "{key} drawn without a comparison");
    }
    assert!(!h.ui.tree_shown());
    // one loaded: every part shows
    h.ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(sample()))));
    h.frame(vec![]);
    h.frame(vec![]);
    for key in [
        "load",
        "entries",
        "remove",
        "sub:summary",
        "sub:config",
        "compareSpecSelect",
    ] {
        assert!(h.ui.has_rect(key), "{key} missing");
    }
    // the entries combo lists the labels; picking another selects it, Remove removes the
    // active one
    let out = h.click("entries");
    assert!(out.commands.is_empty());
    let out = h.click("entry:1");
    assert_eq!(debug(&out.commands), ["SelectCompare { index: 1 }"]);
    let out = h.click("remove");
    assert_eq!(debug(&out.commands), ["RemoveCompare { index: 0 }"]);
}

/// The entries dropdown is one of PoB's searchable dropdowns (SearchHost)
#[test]
fn the_entries_dropdown_filters_as_typed() {
    let mut h = Harness::new(Some(sample()));
    h.click("entries");
    h.frame(vec![]);
    h.frame(vec![Event::Text("imp".into())]);
    h.frame(vec![]);
    assert_eq!(
        h.ui.rect("entry:0").height(),
        0.0,
        "dot wasn't filtered out"
    );
    let out = h.click("entry:1");
    assert_eq!(debug(&out.commands), ["SelectCompare { index: 1 }"]);
}

#[test]
fn load_menu_sends_each_source() {
    let mut h = Harness::new(None);
    let folder = |path: &str| crate::engine::views::FolderNode {
        path: path.into(),
        modified: 1,
        empty: false,
    };
    h.ui.on_snapshot(&Snapshot::BuildList(Arc::new(BuildListView {
        folder: String::new(),
        tree: vec![folder("sub/"), folder("sub/deep/"), folder("zz/")],
        ..BuildListView::default()
    })));
    let browse = |folder: &str, names: &[&str]| {
        Snapshot::BuildBrowse(Arc::new(BuildBrowseView {
            folder: folder.into(),
            entries: names
                .iter()
                .map(|n| BuildBrowseEntry {
                    name: n.to_string(),
                    path: format!("/b/{folder}{n}.xml").into(),
                })
                .collect(),
            error: None,
        }))
    };
    // a saved build: the engine lists the folder's builds, the build list its folders
    h.click("load");
    let out = h.click("load:saved");
    assert!(out.commands.is_empty());
    assert_eq!(debug(&out.queries), ["BrowseBuilds { folder: \"\" }"]);
    assert!(h.ui.has_dialog());
    h.ui.on_snapshot(&browse("", &["dot"]));
    h.frame(vec![]);
    assert!(h.ui.has_rect("folder:sub") && h.ui.has_rect("folder:zz"));
    assert!(
        !h.ui.has_rect("folder:deep") && !h.ui.has_rect("folder:sub/deep"),
        "a folder of another level"
    );
    // into a folder, and back up
    let out = h.click("folder:sub");
    assert_eq!(debug(&out.queries), ["BrowseBuilds { folder: \"sub/\" }"]);
    assert!(h.ui.has_rect("folder:deep") && !h.ui.has_rect("saved:0"));
    // an answer for another folder is not shown here
    h.ui.on_snapshot(&browse("", &["dot"]));
    h.frame(vec![]);
    assert!(!h.ui.has_rect("saved:0"));
    h.ui.on_snapshot(&browse("sub/", &["alpha", "beta"]));
    // the window takes its new size in the next frame
    h.frame(vec![]);
    h.frame(vec![]);
    let out = h.click("saved:1");
    assert_eq!(
        debug(&out.commands),
        ["LoadCompare { source: File(\"/b/sub/beta.xml\") }"]
    );
    assert!(!h.ui.has_dialog());
    h.click("load");
    h.click("load:saved");
    h.ui.on_snapshot(&browse("", &["dot"]));
    h.frame(vec![]);
    h.frame(vec![]);
    h.click("folder:sub");
    let out = h.click("crumb:0");
    assert_eq!(debug(&out.queries), ["BrowseBuilds { folder: \"\" }"]);
    h.frame(vec![]);
    let out = h.click("saved:0");
    assert_eq!(
        debug(&out.commands),
        ["LoadCompare { source: File(\"/b/dot.xml\") }"]
    );

    // a code
    h.click("load");
    h.click("load:code");
    h.click("dialog_text");
    h.type_text("eNrtWk1z");
    let out = h.click("dialog_load");
    assert_eq!(
        debug(&out.commands),
        ["LoadCompare { source: Code(\"eNrtWk1z\") }"]
    );

    // a link is PoB's Import tab mode 3, which PoB offers only for a build with a file
    h.click("load");
    h.click("load:link");
    assert!(!h.ui.has_dialog(), "a link without a file");
    h.ui.on_snapshot(&Snapshot::Build(Arc::new(BuildView {
        name: "b".into(),
        path: Some("/b/b.xml".into()),
        class: "Witch".into(),
        ascendancy: None,
        level: 1,
        unsaved: false,
        can_undo: false,
        can_redo: false,
        ..BuildView::default()
    })));
    h.frame(vec![]);
    h.click("load");
    h.click("load:link");
    assert!(h.ui.has_dialog());
    h.click("dialog_text");
    h.type_text("https://pobb.in/abc");
    let out = h.click("dialog_load");
    assert_eq!(
        debug(&out.commands),
        ["ImportFromUrl { url: \"https://pobb.in/abc\", mode: Comparison }"]
    );
}

/// A folder the engine can't list (a Builds folder that went) is said in the picker, which
/// stops waiting; going elsewhere waits again for that folder's answer
#[test]
fn a_failed_browse_says_so_instead_of_spinning() {
    let mut h = Harness::new(None);
    h.click("load");
    let out = h.click("load:saved");
    assert_eq!(debug(&out.queries), ["BrowseBuilds { folder: \"\" }"]);
    h.frame(vec![]);
    assert!(!h.ui.has_rect("browse_error"));
    let failed = |folder: &str| {
        Snapshot::BuildBrowse(Arc::new(BuildBrowseView {
            folder: folder.into(),
            entries: vec![],
            error: Some("no such folder".into()),
        }))
    };
    // another folder's failure is not this one's
    h.ui.on_snapshot(&failed("sub/"));
    h.frame(vec![]);
    assert!(!h.ui.has_rect("browse_error"));
    h.ui.on_snapshot(&failed(""));
    h.frame(vec![]);
    assert!(h.ui.has_rect("browse_error"));
    assert!(h.drew("Couldn't list this folder: no such folder"));
    assert!(!h.drew("No builds here."));
    // an answer replaces the message
    h.ui.on_snapshot(&Snapshot::BuildBrowse(Arc::new(BuildBrowseView::default())));
    h.frame(vec![]);
    assert!(!h.ui.has_rect("browse_error"));
    assert!(h.drew("No builds here."));
}

/// A folder of thousands of builds lays out only the rows in view
#[test]
fn the_saved_build_picker_lays_out_only_the_rows_in_view() {
    let mut h = Harness::new(None);
    h.click("load");
    h.click("load:saved");
    let entries = (0..5000)
        .map(|i| BuildBrowseEntry {
            name: format!("build {i:04}"),
            path: format!("/b/build {i:04}.xml").into(),
        })
        .collect();
    h.ui.on_snapshot(&Snapshot::BuildBrowse(Arc::new(BuildBrowseView {
        folder: String::new(),
        entries,
        error: None,
    })));
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.ui.has_rect("saved:0"));
    assert!(!h.ui.has_rect("saved:4999"));
    let laid_out = h
        .texts
        .iter()
        .filter(|g| g.text().starts_with("build "))
        .count();
    assert!(
        (1..100).contains(&laid_out),
        "{laid_out} rows drawn of 5000"
    );
    let out = h.click("saved:3");
    assert_eq!(
        debug(&out.commands),
        ["LoadCompare { source: File(\"/b/build 0003.xml\") }"]
    );
}

/// A name wider than PoB's 310 px box is cut with "..." (`fitItemName`), the warning
/// included; a short one is drawn whole, and in expanded mode nothing is cut
#[test]
fn long_item_names_are_cut_with_dots_as_pob() {
    let long = "Beast Grip Exceptionally Long Unique Name Of The Ancient Vaal Empire";
    let mut view = sample();
    view.items = vec![
        CompareItemRow {
            ours_name: Some(long.into()),
            ..item_row(
                "Body Armour",
                Some(2),
                Some(long),
                "^xFF9922(different)",
                true,
            )
        },
        CompareItemRow {
            theirs_unallocated: true,
            ..item_row(
                "Jewel 100",
                None,
                Some("Cobalt Jewel"),
                "^xDD0022(missing)",
                true,
            )
        },
    ];
    let mut h = Harness::new(Some(view));
    h.click("sub:items");
    let cut: Vec<String> = h
        .texts
        .iter()
        .map(|g| g.job.text.clone())
        .filter(|t| t.ends_with("...") && long.starts_with(t.trim_end_matches("...")))
        .collect();
    assert_eq!(cut.len(), 2, "both sides cut: {cut:?}");
    assert!(cut[0].len() > 10 && cut[0].len() < long.len(), "{cut:?}");
    assert!(!h.drew(long), "the whole name was drawn");
    // the warning counts toward the width, as PoB fits `name .. warning`
    assert!(
        h.texts
            .iter()
            .any(|g| g.job.text.starts_with("Cobalt Jewel  (tree missing")
                && g.job.text.ends_with("...")),
        "{:?}",
        h.texts
            .iter()
            .map(|g| g.job.text.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn selector_row_sends_set_compare_control() {
    let mut h = Harness::new(Some(sample()));
    // PoB's captions, for the shown controls only
    assert!(h.drew("Tree set:") && h.drew("Skill set:") && h.drew("Skill:"));
    assert!(h.drew("Stages:"));
    assert!(!h.ui.has_rect("cmpSkillPart"), "a hidden control is drawn");
    assert!(!h.drew("Item set:"), "a missing control's caption is drawn");
    h.click("compareSpecSelect");
    let out = h.click("compareSpecSelect_1");
    assert_eq!(
        debug(&out.commands),
        [
            "SetCompareControl { name: \"compareSpecSelect\", value: Choice { index: 1, label: \
             \"Leveling\" } }"
        ]
    );
    // a count box sends its digits on Enter
    h.click("edit_cmpStageCount");
    h.type_text("5");
    assert!(h.ui.has_pending());
    let out = h.key(Key::Enter);
    assert_eq!(
        debug(&out.commands),
        ["SetCompareControl { name: \"cmpStageCount\", value: Text(\"5\") }"]
    );
    // one still being typed goes out when the app flushes it (leaving the tab)
    h.click("edit_cmpStageCount");
    h.type_text("7");
    assert_eq!(
        debug(&h.ui.flush().into_iter().collect::<Vec<_>>()),
        ["SetCompareControl { name: \"cmpStageCount\", value: Text(\"7\") }"]
    );
    // the Items view's own selectors show only there
    assert!(!h.ui.has_rect("primaryItemSetSelect"));
    h.click("sub:items");
    h.click("primaryItemSetSelect");
    let out = h.click("primaryItemSetSelect_1");
    assert_eq!(
        debug(&out.commands),
        [
            "SetCompareControl { name: \"primaryItemSetSelect\", value: Choice { index: 1, \
             label: \"Swap\" } }"
        ]
    );
}

#[test]
fn summary_colours_better_and_worse() {
    let h = Harness::new(Some(sample()));
    assert!(h.drew("Life:") && h.drew("1,000") && h.drew("900"));
    assert!(h.drew("attack") && h.drew("dot") && h.drew("Difference"));
    assert_eq!(h.colour_of("+100"), Some(theme::POSITIVE));
    assert_eq!(h.colour_of("-50"), Some(theme::NEGATIVE));
    let default = egui::Color32::from_rgb(1, 2, 3);
    let row = &sample().summary[2];
    assert_eq!(summary::diff_colour(row, default), default);
}

#[test]
fn overlay_check_is_disabled_and_the_tree_view_asks_for_the_canvas() {
    // R7 fallback: the overlay is the only tree view, so its box can't be unticked
    let mut h = Harness::new(Some(sample()));
    assert!(!h.ui.tree_shown());
    assert!(!h.frame(vec![]).tree);
    let out = h.click("sub:tree");
    assert!(out.tree, "the Tree view doesn't ask for our canvas");
    assert!(h.ui.tree_shown());
    let out = h.click("overlay");
    assert!(out.commands.is_empty(), "{:?}", out.commands);
    let out = h.click("copy_tree");
    assert_eq!(debug(&out.commands), ["CopyCompareTree { and_use: false }"]);
    let out = h.click("copy_tree_use");
    assert_eq!(debug(&out.commands), ["CopyCompareTree { and_use: true }"]);
    h.click("sub:summary");
    assert!(!h.ui.tree_shown());
}

#[test]
fn copy_item_sends_slot_and_use() {
    let mut h = Harness::new(Some(sample()));
    h.ui.on_snapshot(&Snapshot::Items(
        Arc::new(crate::ui::items::tests::sample()),
    ));
    h.click("sub:items");
    // PoB's difference labels, in the colours the engine's codes give them
    let colour = |code: &str| crate::widgets::pob_colour(code, egui::Color32::WHITE);
    for (label, code) in [
        ("(different)", "^xFF9922"),
        ("(missing)", "^xDD0022"),
        ("(extra)", "^x80A080"),
        ("(match)", "^x33FF77"),
    ] {
        assert_eq!(h.colour_of(label), Some(colour(code)), "{label}");
    }
    assert!(h.drew("(both empty)"));
    // names in their rarity's colour: ours from the Items view when the row has none
    assert!(h.drew("Kaom's Heart"));
    assert_eq!(h.colour_of("Tabula Rasa"), Some(colour("^xAF6025")));
    assert_eq!(h.colour_of("Ming's Heart"), Some(colour("^7")));
    // the name and its warning, cut to PoB's box: name colour first, the warning's last
    let warned = h
        .texts
        .iter()
        .find(|g| g.job.text.starts_with("Cobalt Jewel  (tree"))
        .expect("the warned jewel");
    let sections = &warned.job.sections;
    assert_eq!(sections.first().unwrap().format.color, colour("^x8888FF"));
    assert_eq!(sections.last().unwrap().format.color, colour("^xFF9922"));
    assert_eq!(h.colour_of("Prismatic Jewel"), Some(colour("^xFFFF77")));
    assert_eq!(h.colour_of("Missing slot"), Some(colour("^xBBBBBB")));
    let out = h.click("copy:Body Armour");
    assert_eq!(
        debug(&out.commands),
        ["CopyCompareItem { slot: \"Body Armour\", and_use: false }"]
    );
    let out = h.click("equip:Body Armour");
    assert_eq!(
        debug(&out.commands),
        ["CopyCompareItem { slot: \"Body Armour\", and_use: true }"]
    );
    let out = h.click("copy:Ring 3");
    assert_eq!(
        debug(&out.commands),
        ["CopyCompareItem { slot: \"Ring 3\", and_use: false }"]
    );
    // PoB's "Missing slot" where this build lacks the slot: no Equip
    assert!(!h.ui.has_rect("equip:Ring 3") && h.ui.has_rect("missing:Ring 3"));
    assert!(!h.ui.has_rect("copy:Ring 1"), "nothing of theirs to copy");
    assert!(!h.ui.has_rect("missing:Ring 1"));
    // PoB's marker comes from the engine, not from `differ`; an unallocated socket is flagged
    // as PoB does
    assert!(h.drew("(match)"));
    // hovers ask for ours and theirs, and show the answer
    let out = h.hover("theirs:Body Armour");
    let theirs = ItemTooltipQuery {
        id: 0,
        slot: Some("Body Armour".into()),
        theirs: true,
        shift: false,
        shared: None,
    };
    assert_eq!(
        debug(&out.queries),
        debug(&[Command::ItemTooltip(theirs.clone())])
    );
    h.ui.on_snapshot(&Snapshot::ItemTooltip(Arc::new(ItemTooltipView {
        query: theirs.clone(),
        tooltip: crate::engine::views::ItemTooltip {
            lines: vec![crate::engine::views::ItemTooltipLine::Line {
                size: 14,
                text: "^7Six linked".into(),
                font: None,
                center: false,
                background: None,
                mod_line: None,
            }],
            ..Default::default()
        },
    })));
    h.hover("theirs:Body Armour");
    assert_eq!(
        h.ui.tip.tooltip.as_ref().map(|t| &t.query),
        Some(&theirs),
        "their tooltip isn't kept"
    );
    let out = h.hover("ours:Body Armour");
    assert_eq!(
        debug(&out.queries),
        debug(&[Command::ItemTooltip(ItemTooltipQuery {
            id: 2,
            slot: Some("Body Armour".into()),
            theirs: false,
            shift: false,
            shared: None,
        })])
    );
}

#[test]
fn compare_none_resets_the_tab() {
    let mut h = Harness::new(Some(sample()));
    h.click("sub:config");
    h.click("config_search");
    h.type_text("boss");
    h.click("load");
    h.click("load:code");
    assert!(h.ui.has_dialog());
    h.click("edit_cmpStageCount");
    h.type_text("3");
    assert!(h.ui.has_pending());
    // the last comparison was removed
    h.ui.on_snapshot(&Snapshot::Compare(None));
    assert_eq!(h.ui.sub_tab(), SubTab::Summary);
    assert!(!h.ui.tree_shown());
    assert!(!h.ui.has_pending(), "a count for a comparison that is gone");
    assert!(h.ui.config_filter.is_empty());
    h.frame(vec![]);
    assert!(!h.ui.has_rect("sub:summary") && !h.ui.has_rect("entries"));
    // the load window the user opened stays
    assert!(h.ui.has_dialog());
    // a new comparison starts on the Summary
    h.ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(sample()))));
    h.frame(vec![]);
    assert!(h.drew("Life:"));
}

#[test]
fn config_view_copies_filters_and_switches_our_set() {
    let mut h = Harness::new(Some(sample()));
    h.ui.on_snapshot(&Snapshot::Config(Arc::new(ConfigView {
        sets: vec![
            ConfigSet {
                id: 1,
                title: "Default".into(),
            },
            ConfigSet {
                id: 2,
                title: "Boss".into(),
            },
        ],
        active_set: 1,
        custom_mods: vec![],
        sections: vec![],
    })));
    h.click("sub:config");
    assert!(h.ui.has_rect("config_row:usePowerCharges"));
    assert!(h.ui.has_rect("config_row:enemyIsBoss"));
    let out = h.click("copy_config");
    assert_eq!(debug(&out.commands), ["CopyCompareConfig"]);
    h.click("config_search");
    h.type_text("BOSS");
    h.frame(vec![]);
    assert!(!h.ui.has_rect("config_row:usePowerCharges"));
    assert!(h.ui.has_rect("config_row:enemyIsBoss"));
    h.click("config_set");
    let out = h.click("config_set:2");
    assert_eq!(debug(&out.commands), ["SelectConfigSet { id: 2 }"]);
}

#[test]
fn skills_view_draws_both_sides() {
    let mut h = Harness::new(Some(sample()));
    h.click("sub:skills");
    assert!(h.drew("Fireball") && h.drew("Fireball 20/0") && h.drew("+ Spell Echo"));
    assert!(h.drew("Fireball Echo"), "their group header");
    assert_eq!(h.colour_of("+ Spell Echo"), Some(theme::POSITIVE));
}

#[test]
fn switched_is_taken_once() {
    let mut view = sample();
    view.switched = true;
    let mut ui = CompareUi::default();
    ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(view))));
    assert!(ui.take_switched());
    assert!(!ui.take_switched());
    ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(sample()))));
    assert!(!ui.take_switched());
}

fn tooltip_reply(query: &ItemTooltipQuery, text: &str) -> Snapshot {
    Snapshot::ItemTooltip(Arc::new(ItemTooltipView {
        query: query.clone(),
        tooltip: crate::engine::views::ItemTooltip {
            lines: vec![crate::engine::views::ItemTooltipLine::Line {
                size: 14,
                text: format!("^7{text}"),
                font: None,
                center: false,
                background: None,
                mod_line: None,
            }],
            ..Default::default()
        },
    }))
}

#[test]
fn expanded_mode_shows_both_tooltips_inline_one_question_at_a_time() {
    let mut h = Harness::new(Some(sample()));
    h.click("sub:items");
    // PoB's box, sent as a check
    let out = h.click(EXPANDED_CHECK);
    assert_eq!(
        debug(&out.commands),
        ["SetCompareControl { name: \"itemsExpandedCheck\", value: Check(true) }"]
    );
    let mut view = sample();
    view.controls
        .iter_mut()
        .find(|c| c.name == EXPANDED_CHECK)
        .unwrap()
        .checked = true;
    h.ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(view.clone()))));
    let q = |id, slot: &str, theirs| ItemTooltipQuery {
        id,
        slot: Some(slot.into()),
        theirs,
        shift: false,
        shared: None,
    };
    // the rows' items in order, ours then theirs, each asked once the last is answered
    let expected = [
        q(2, "Body Armour", false),
        q(0, "Body Armour", true),
        q(0, "Ring 3", true),
        q(1, "Ring 1", false),
        q(4, "Jewel 100", false),
        q(0, "Jewel 100", true),
    ];
    for (i, query) in expected.iter().enumerate() {
        let out = h.frame(vec![]);
        assert_eq!(
            debug(&out.queries),
            debug(&[Command::ItemTooltip(query.clone())]),
            "question {i}"
        );
        // nothing more until it is answered
        assert!(h.frame(vec![]).queries.is_empty(), "question {i} twice");
        h.ui.on_snapshot(&tooltip_reply(query, &format!("card {i}")));
    }
    let out = h.frame(vec![]);
    assert!(out.queries.is_empty(), "{:?}", out.queries);
    h.frame(vec![]);
    assert!(
        h.drew("card 0") && h.drew("card 5"),
        "the cards aren't inline"
    );
    assert!(h.ui.expanded.has_card(&expected[3]));
    // the buttons stay
    let out = h.click("equip:Body Armour");
    assert_eq!(
        debug(&out.commands),
        ["CopyCompareItem { slot: \"Body Armour\", and_use: true }"]
    );
    // a new view asks again
    let mut changed = view;
    changed.items.pop();
    h.ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(changed))));
    assert!(!h.ui.expanded.has_card(&expected[0]));
    let out = h.frame(vec![]);
    assert_eq!(
        debug(&out.queries),
        debug(&[Command::ItemTooltip(expected[0].clone())])
    );
    // an item the engine never answers for is given up on
    h.time += 3.0;
    let out = h.frame(vec![]);
    assert_eq!(
        debug(&out.queries),
        debug(&[Command::ItemTooltip(expected[1].clone())])
    );
}

#[test]
fn version_pick_asks_as_pob_then_converts() {
    let mut h = Harness::new(Some(sample()));
    h.click("sub:tree");
    assert!(h.drew("Our tree version:") && h.drew("Their tree version:"));
    h.click("leftVersionSelect");
    let out = h.click("leftVersionSelect_0");
    assert!(out.commands.is_empty(), "converted before asking");
    assert!(h.ui.has_dialog());
    assert!(h.drew("Convert to Version 3.25"));
    let out = h.click("convert_copy");
    assert_eq!(
        debug(&out.commands),
        ["ConvertCompareTree { theirs: false, index: 0, label: \"3.25\", keep_copy: true }"]
    );
    assert!(!h.ui.has_dialog());
    // theirs, replaced
    h.click("rightVersionSelect");
    h.click("rightVersionSelect_2");
    let out = h.click("convert");
    assert_eq!(
        debug(&out.commands),
        ["ConvertCompareTree { theirs: true, index: 2, label: \"3.27\", keep_copy: false }"]
    );
    // Cancel converts nothing
    h.click("rightVersionSelect");
    h.click("rightVersionSelect_0");
    let out = h.click("convert_cancel");
    assert!(out.commands.is_empty());
    assert!(!h.ui.has_dialog());
}

#[test]
fn config_toggle_sends_the_other_state_and_common_rows_are_dimmed() {
    let mut h = Harness::new(Some(sample()));
    h.click("sub:config");
    assert!(h.drew("Show All Configurations"));
    let out = h.click("config_toggle");
    assert_eq!(
        debug(&out.commands),
        ["SetCompareControl { name: \"configToggleBtn\", value: Check(true) }"]
    );
    let default = h.colour_of("Is the enemy a Boss?").unwrap();
    let common = h.colour_of("Bandit quest:").unwrap();
    assert_ne!(default, common, "a common row isn't told apart");
    let mut view = sample();
    let toggle = view
        .controls
        .iter_mut()
        .find(|c| c.name == "configToggleBtn")
        .unwrap();
    toggle.checked = true;
    toggle.label = "Hide Ineligible Configurations".into();
    h.ui.on_snapshot(&Snapshot::Compare(Some(Arc::new(view))));
    h.frame(vec![]);
    let out = h.click("config_toggle");
    assert_eq!(
        debug(&out.commands),
        ["SetCompareControl { name: \"configToggleBtn\", value: Check(false) }"]
    );
}

#[test]
fn items_view_offers_our_tree_set() {
    let mut h = Harness::new(Some(sample()));
    let mut state = crate::engine::fake::empty_tree_state();
    state.specs = vec![
        crate::engine::views::SpecEntry {
            title: "Default".into(),
            version: "3_27".into(),
            points: 10,
        },
        crate::engine::views::SpecEntry {
            title: "Late".into(),
            version: "3_27".into(),
            points: 90,
        },
    ];
    h.ui.on_snapshot(&Snapshot::Tree(Arc::new(state)));
    h.click("sub:items");
    h.click("our_spec");
    let out = h.click("our_spec:1");
    assert_eq!(debug(&out.commands), ["SelectSpec { index: 1 }"]);
}

#[test]
fn buttons_wait_while_the_engine_is_behind() {
    let mut h = Harness::new(Some(sample()));
    h.busy = true;
    let mut sent = Vec::new();
    sent.extend(h.click("remove").commands);
    h.click("sub:items");
    for key in ["copy:Body Armour", "equip:Body Armour"] {
        sent.extend(h.click(key).commands);
    }
    h.click("sub:tree");
    for key in ["copy_tree", "copy_tree_use"] {
        sent.extend(h.click(key).commands);
    }
    h.click("sub:config");
    sent.extend(h.click("copy_config").commands);
    assert!(sent.is_empty(), "{sent:?}");
    // and work once it caught up
    h.busy = false;
    h.frame(vec![]);
    assert_eq!(
        debug(&h.click("copy_config").commands),
        ["CopyCompareConfig"]
    );
}

#[test]
fn a_load_window_leaves_its_box_on_esc_first_then_closes() {
    // PopupDialog.lua:68-77: the selected box takes the first Esc (EditControl.lua:505)
    let mut h = Harness::new(Some(sample()));
    h.click("load");
    h.click("load:code");
    assert!(h.ui.has_dialog());
    h.frame(vec![]);
    h.click("dialog_text");
    h.type_text("eNrtWk1z");
    h.key(Key::Escape);
    h.frame(vec![]);
    assert!(h.ui.has_dialog(), "the first Esc closed it");
    h.key(Key::Escape);
    h.frame(vec![]);
    assert!(!h.ui.has_dialog(), "the second Esc left it open");
}
