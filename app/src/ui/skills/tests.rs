use super::groups::drop_index;
use super::*;
use crate::engine::views::{
    DropList, GemView, SkillOptionsView, SkillSetEntry, SlotChoice, SocketGroupText,
    SocketGroupView, TooltipLine,
};
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

fn gem(name: &str) -> GemView {
    GemView {
        name_spec: name.into(),
        gem_id: Some(format!("id_{name}")),
        level: 20,
        quality: 0,
        enabled: true,
        colour: "^x33FF77".into(),
        ..GemView::default()
    }
}

fn group(label: &str, gems: Vec<GemView>) -> SocketGroupView {
    SocketGroupView {
        label: label.into(),
        display_label: label.into(),
        enabled: true,
        slot_enabled: true,
        full_dps: true,
        gems,
        ..SocketGroupView::default()
    }
}

fn list(labels: &[&str]) -> DropList {
    DropList {
        labels: labels.iter().map(|s| s.to_string()).collect(),
        selected: 0,
        enabled: true,
    }
}

fn view(groups: Vec<SocketGroupView>) -> SkillsView {
    SkillsView {
        sets: vec![
            SkillSetEntry {
                id: 1,
                title: "Main".into(),
                untitled: false,
            },
            SkillSetEntry {
                id: 2,
                title: "Alt".into(),
                untitled: false,
            },
        ],
        active_set: 1,
        groups,
        slot_choices: vec![
            SlotChoice {
                label: "None".into(),
                slot: None,
            },
            SlotChoice {
                label: "Weapon 1".into(),
                slot: Some("Weapon 1".into()),
            },
        ],
        options: SkillOptionsView {
            sort_field: list(&["Full DPS", "Combined DPS"]),
            default_level: list(&["Normal max", "Level 1"]),
            default_quality: "0".into(),
            support_types: list(&["All"]),
            ..SkillOptionsView::default()
        },
        ..SkillsView::default()
    }
}

pub(crate) fn sample() -> SkillsView {
    view(vec![
        group("Boneshatter", vec![gem("Boneshatter"), gem("Melee")]),
        group("Empty", vec![]),
    ])
}

fn item_group() -> SocketGroupView {
    SocketGroupView {
        source: Some("Ring".into()),
        source_note: Some("From Cospri".into()),
        count: Some("1".into()),
        imbued: None,
        gems: vec![gem("Item gem")],
        ..group("Item", vec![])
    }
}

struct Harness {
    ui: SkillsUi,
    ctx: egui::Context,
    time: f64,
    copied: Vec<String>,
    /// The next frame's paste is a right-click's
    right_paste: bool,
}

fn debug(commands: &[Command]) -> Vec<String> {
    commands.iter().map(|c| format!("{c:?}")).collect()
}

impl Harness {
    fn new(view: SkillsView) -> Self {
        let mut ui = SkillsUi::default();
        ui.on_snapshot(&Snapshot::Skills(Arc::new(view)));
        let mut h = Self {
            ui,
            ctx: egui::Context::default(),
            time: 1.0,
            copied: Vec::new(),
            right_paste: false,
        };
        h.frame(vec![]);
        h
    }

    fn frame_out(&mut self, events: Vec<Event>) -> SkillsOutput {
        self.time += 0.2;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut out = SkillsOutput::default();
        let time = self.time;
        let right_paste = std::mem::take(&mut self.right_paste);
        let output = self.ctx.run_ui(input, |ui| {
            if right_paste {
                crate::ui::shortcuts::mark_right_click_paste(ui.ctx());
            }
            // egui may run a frame twice; each pass's output counts
            let pass = self.ui.show(ui, time, false);
            out.commands.extend(pass.commands);
            out.queries.extend(pass.queries);
        });
        self.copied.extend(
            output
                .platform_output
                .commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t.clone()),
                    _ => None,
                }),
        );
        output.drop_without_applying_deltas();
        out
    }

    fn frame(&mut self, events: Vec<Event>) -> SkillsOutput {
        self.frame_out(events)
    }

    fn click(&mut self, key: &str) -> SkillsOutput {
        let pos = self.ui.rect(key).center();
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut all = SkillsOutput::default();
        for events in [
            vec![Event::PointerMoved(pos)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            let o = self.frame_out(events);
            all.commands.extend(o.commands);
            all.queries.extend(o.queries);
        }
        all
    }

    fn press(&mut self, key: Key) -> SkillsOutput {
        self.frame_out(vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }])
    }

    fn frame_key(&mut self, key: Key, modifiers: Modifiers) -> SkillsOutput {
        self.frame_out(vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }])
    }

    fn type_text(&mut self, text: &str) -> SkillsOutput {
        self.frame_out(vec![Event::Text(text.into())])
    }
}

#[test]
fn drop_index_matches_list_control() {
    assert_eq!(drop_index(0, 2, true), Some(2));
    assert_eq!(drop_index(1, 1, false), None);
    assert_eq!(drop_index(1, 1, true), None);
    assert_eq!(drop_index(1, 0, true), None, "right below the row above");
    assert_eq!(drop_index(3, 1, false), Some(1));
    assert_eq!(drop_index(0, 3, false), Some(2));
}

#[test]
fn delete_group_with_gems_asks_first() {
    let mut h = Harness::new(sample());
    h.click("row_0");
    let o = h.press(Key::Delete);
    h.frame(vec![]);
    assert!(o.commands.is_empty(), "the first press sends nothing");
    assert!(
        h.ui.rects.map.contains_key("confirm_delete"),
        "a confirm opened"
    );
    let o = h.click("confirm_delete");
    assert_eq!(debug(&o.commands), ["DeleteSocketGroup { group: 0 }"]);
}

#[test]
fn delete_group_without_gems_is_immediate() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    let o = h.press(Key::Delete);
    assert_eq!(debug(&o.commands), ["DeleteSocketGroup { group: 1 }"]);
}

#[test]
fn item_group_offers_only_pob_fields() {
    let mut v = sample();
    v.groups.push(item_group());
    let mut h = Harness::new(v);
    h.click("row_2");
    // only what the item group's frame draws
    h.ui.rects.map.clear();
    h.frame(vec![]);
    for absent in ["slot", "imbued", "optimise", "gem_0_name", "gem_0_delete"] {
        assert!(!h.ui.rects.map.contains_key(absent), "{absent} is drawn");
    }
    for present in ["g2_label", "group_enabled", "group_full_dps", "g2_count"] {
        assert!(h.ui.rects.map.contains_key(present), "{present} missing");
    }
    h.click("g2_label");
    h.type_text("X");
    let o = h.press(Key::Enter);
    assert_eq!(
        debug(&o.commands),
        ["SetGroupLabel { group: 2, text: \"ItemX\" }"]
    );
}

#[test]
fn copy_reply_reaches_clipboard_only_when_asked() {
    let mut h = Harness::new(sample());
    let reply = |group, text: &str| {
        Snapshot::SocketGroupText(Arc::new(SocketGroupText {
            group,
            text: text.into(),
        }))
    };
    h.ui.on_snapshot(&reply(0, "unasked"));
    h.frame(vec![]);
    assert!(h.copied.is_empty(), "nothing was asked for");
    h.click("row_0");
    let o = h.frame(vec![Event::Copy]);
    assert_eq!(debug(&o.queries), ["CopySocketGroup { group: 0 }"]);
    h.ui.on_snapshot(&reply(1, "other group"));
    h.frame(vec![]);
    assert!(h.copied.is_empty(), "a reply for another group is ignored");
    h.ui.on_snapshot(&reply(0, "Label: x"));
    h.frame(vec![]);
    assert_eq!(h.copied, ["Label: x"]);
    h.ui.on_snapshot(&reply(0, "again"));
    h.frame(vec![]);
    assert_eq!(h.copied.len(), 1, "one copy per ask");
}

#[test]
fn ctrl_v_pastes_a_group_even_while_a_box_has_the_keyboard() {
    let mut h = Harness::new(sample());
    let o = h.frame(vec![Event::Paste("Label: p".into())]);
    assert_eq!(
        debug(&o.commands),
        ["PasteSocketGroup { text: \"Label: p\" }"]
    );
    // the tab reads Ctrl+V before its controls (SkillsTab.lua:628): the box takes the text
    // and PoB's PasteSocketGroup runs too
    h.click("g0_label");
    let o = h.frame(vec![Event::Paste("Label: q".into())]);
    assert_eq!(
        debug(&o.commands),
        ["PasteSocketGroup { text: \"Label: q\" }"]
    );
    // a right-click's paste into the box is no Ctrl+V
    h.click("g0_label");
    h.right_paste = true;
    let o = h.frame(vec![Event::Paste("Label: r".into())]);
    assert!(o.commands.is_empty(), "{:?}", debug(&o.commands));
}

#[test]
fn esc_closes_the_skill_sets_window() {
    // every PoB popup closes on Esc (PopupDialog.lua:72-77)
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    assert!(h.ui.has_dialog());
    h.press(Key::Escape);
    h.frame(vec![]);
    assert!(!h.ui.has_dialog(), "Esc left it open");
}

#[test]
fn last_set_cannot_be_deleted() {
    let mut v = sample();
    v.sets.truncate(1);
    let mut h = Harness::new(v);
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_0");
    let o = h.click("sets_delete");
    assert!(o.commands.is_empty());
    assert!(
        !h.ui.rects.map.contains_key("sets_confirm_delete"),
        "no confirm for the last set"
    );
}

#[test]
fn deleting_a_set_asks_first() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    let o = h.click("sets_delete");
    assert!(o.commands.is_empty());
    let o = h.click("sets_confirm_delete");
    assert_eq!(debug(&o.commands), ["DeleteSkillSet { id: 2 }"]);
}

#[test]
fn copying_a_set_prefills_its_title() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    h.click("sets_copy");
    // PoB's Save waits for an edit, even on the prefilled name
    let o = h.click("sets_save");
    assert!(o.commands.is_empty(), "{:?}", o.commands);
    h.click("sets_name");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.type_text("Alt");
    let o = h.click("sets_save");
    assert_eq!(
        debug(&o.commands),
        ["CopySkillSet { id: 2, title: \"Alt\" }"]
    );
}

#[test]
fn focused_field_survives_incoming_view() {
    let mut h = Harness::new(sample());
    h.click("g0_label");
    h.type_text("!");
    let mut v = sample();
    v.groups[0].label = "Changed".into();
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(v)));
    h.frame(vec![]);
    let o = h.press(Key::Enter);
    assert_eq!(
        debug(&o.commands),
        ["SetGroupLabel { group: 0, text: \"Boneshatter!\" }"]
    );
    // unfocused, the field follows the view again
    h.frame(vec![]);
    let mut v = sample();
    v.groups[0].label = "Later".into();
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(v)));
    h.frame(vec![]);
    assert_eq!(h.ui.boxes.text(&"g0_label".to_owned()).unwrap(), "Later");
}

#[test]
fn selection_is_clamped_and_follows_new_groups() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    assert_eq!(h.ui.selected, 1);
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(view(vec![group("A", vec![])]))));
    assert_eq!(h.ui.selected, 0, "clamped");
    let o = h.click("new_group");
    assert_eq!(debug(&o.commands), ["AddSocketGroup"]);
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(view(vec![
        group("A", vec![]),
        group("B", vec![]),
    ]))));
    assert_eq!(h.ui.selected, 1, "the new group is selected");
}

#[test]
fn delete_all_asks_first() {
    let mut h = Harness::new(sample());
    let o = h.click("delete_all");
    assert!(o.commands.is_empty());
    let o = h.click("confirm_delete_all");
    assert_eq!(debug(&o.commands), ["DeleteAllSocketGroups"]);
}

#[test]
fn hovering_a_row_asks_for_its_tooltip_after_the_delay() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("row_0").center();
    let o = h.frame(vec![Event::PointerMoved(pos)]);
    assert!(o.queries.is_empty(), "too early");
    let o = h.frame(vec![]);
    assert_eq!(debug(&o.queries), ["GroupTooltip { group: 0 }"]);
    let o = h.frame(vec![]);
    assert!(o.queries.is_empty(), "asked once");
    h.ui.on_snapshot(&Snapshot::GroupTooltip(Arc::new(
        crate::engine::views::GroupTooltipView {
            group: 0,
            lines: vec![TooltipLine::Line {
                size: 14,
                text: "tip".into(),
            }],
        },
    )));
    h.frame(vec![]);
}

#[test]
fn options_send_commands_and_show_the_view_index() {
    let mut h = Harness::new(sample());
    let o = h.click("sort_by_dps");
    assert_eq!(debug(&o.commands), ["SetSkillOption(SortByDps(true))"]);
    h.click("sort_field");
    h.frame(vec![]);
    let o = h.click("sort_field_item_1");
    assert_eq!(debug(&o.commands), ["SetSkillOption(SortField(1))"]);
}

#[test]
fn gem_fields_send_text_on_enter() {
    let mut h = Harness::new(sample());
    h.click("g0_s0_level");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.type_text("15");
    let o = h.press(Key::Enter);
    assert_eq!(
        debug(&o.commands),
        ["SetGemLevel { group: 0, slot: 0, text: \"15\" }"]
    );
}

#[test]
fn clicking_a_gem_name_asks_for_the_picker() {
    let mut h = Harness::new(sample());
    h.click("gem_2_name");
    let p = h.ui.picker.as_ref().expect("picker open");
    assert_eq!((p.group(), p.target()), (0, GemTarget::Slot(2)));
}

#[test]
fn an_edit_in_a_field_that_vanishes_is_flushed() {
    let mut h = Harness::new(sample());
    h.click("g0_label");
    h.type_text("!");
    let o = h.click("row_1");
    assert_eq!(
        debug(&o.commands),
        ["SetGroupLabel { group: 0, text: \"Boneshatter!\" }"]
    );
}

#[test]
fn escape_reverts_the_field_and_sends_nothing() {
    let mut h = Harness::new(sample());
    h.click("g0_label");
    h.type_text("!");
    let o = h.press(Key::Escape);
    assert!(o.commands.is_empty(), "{:?}", o.commands);
    let o = h.frame(vec![]);
    assert!(o.commands.is_empty(), "nothing sent after the revert");
    assert_eq!(
        h.ui.boxes.text(&"g0_label".to_owned()).unwrap(),
        "Boneshatter"
    );
    assert!(!h.ui.has_pending());
}

#[test]
fn a_field_that_vanishes_without_a_blur_is_flushed() {
    let mut h = Harness::new(sample());
    h.click("row_1");
    h.click("g1_label");
    h.type_text("!");
    // a snapshot removes the group, so the field isn't drawn and never sees a blur
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(view(vec![group("A", vec![])]))));
    let o = h.frame(vec![]);
    assert_eq!(
        debug(&o.commands),
        ["SetGroupLabel { group: 1, text: \"Empty!\" }"]
    );
    assert!(!h.ui.has_pending());
}

#[test]
fn an_open_picker_blocks_paste_and_closes_with_the_tab() {
    let mut h = Harness::new(sample());
    let o = h.click("gem_2_name");
    assert!(h.ui.has_dialog(), "the picker is a dialog");
    assert!(
        debug(&o.queries)
            .iter()
            .any(|q| q.starts_with("SearchGems")),
        "{:?}",
        o.queries
    );
    let o = h.frame(vec![Event::Paste("Boneshatter".into())]);
    assert!(
        !debug(&o.commands)
            .iter()
            .any(|c| c.starts_with("PasteSocketGroup")),
        "Paste fired under the picker"
    );
    assert!(matches!(
        h.ui.close_dialogs(),
        Some(Command::CloseGemSearch)
    ));
    assert!(!h.ui.has_dialog());
}

#[test]
fn picker_replies_reach_the_open_picker_and_closing_sends_close() {
    use crate::engine::GemSearchQuery;
    use crate::engine::views::GemSearchView;
    let mut h = Harness::new(sample());
    h.click("gem_2_name");
    h.ui.on_snapshot(&Snapshot::GemSearch(Arc::new(GemSearchView {
        query: GemSearchQuery {
            group: 0,
            target: GemTarget::Slot(2),
            text: String::new(),
            filter: crate::engine::GemFilter::All,
        },
        rows: vec![],
        base_dps: 0.0,
        sorting: false,
        percent: None,
        ended: true,
    })));
    let o = h.frame(vec![]);
    assert_eq!(debug(&o.commands), ["CloseGemSearch"]);
    assert!(!h.ui.has_dialog());
}

#[test]
fn the_picker_closes_when_the_groups_change_under_it() {
    let mut h = Harness::new(sample());
    h.click("gem_2_name");
    assert!(h.ui.has_dialog());
    // a group was added: index 0 may mean something else now
    let mut more = sample();
    more.groups.push(group("New", vec![]));
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(more)));
    let o = h.frame(vec![]);
    assert_eq!(debug(&o.commands), ["CloseGemSearch"]);
    assert!(!h.ui.has_dialog());
}

#[test]
fn gem_tooltip_is_asked_again_after_the_view_changes() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("gem_0_name").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    let o = h.frame(vec![]);
    assert_eq!(o.queries.len(), 1, "{:?}", o.queries);
    let query = match &o.queries[0] {
        Command::GemTooltip(q) => q.clone(),
        other => panic!("{other:?}"),
    };
    h.ui.on_snapshot(&Snapshot::GemTooltip(Arc::new(
        crate::engine::views::GemTooltipView {
            query,
            lines: vec![TooltipLine::Line {
                size: 14,
                text: "old".into(),
            }],
        },
    )));
    assert!(h.ui.gem_tooltip.is_some());
    let mut changed = sample();
    changed.groups[0].gems[0] = gem("Cleave");
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(changed)));
    assert!(h.ui.gem_tooltip.is_none(), "old lines kept");
    let mut asked = false;
    for _ in 0..4 {
        asked |= h
            .frame(vec![Event::PointerMoved(pos)])
            .queries
            .iter()
            .any(|q| matches!(q, Command::GemTooltip(_)));
    }
    assert!(asked, "no new GemTooltip query");
}

#[test]
fn gem_tooltip_is_forgotten_when_the_hover_ends() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("gem_0_name").center();
    h.frame(vec![Event::PointerMoved(pos)]);
    h.frame(vec![]);
    assert!(h.ui.gem_tooltip_asked.is_some());
    h.frame(vec![Event::PointerMoved(Pos2::new(1100.0, 650.0))]);
    assert!(h.ui.gem_tooltip_asked.is_none());
    assert!(h.ui.gem_tooltip.is_none());
}

#[test]
fn a_pending_default_quality_is_not_unsaved_but_still_flushes() {
    let mut h = Harness::new(sample());
    h.click("default_quality");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.type_text("20");
    h.frame(vec![]);
    assert!(
        !h.ui.has_pending(),
        "the option never marks the build unsaved"
    );
    assert_eq!(
        debug(&h.ui.flush().into_iter().collect::<Vec<_>>()),
        ["SetSkillOption(DefaultQuality(\"20\"))"]
    );
    // a gem field still counts (its two characters are full: type over them)
    h.click("g0_s0_level");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.type_text("1");
    h.frame(vec![]);
    assert!(h.ui.has_pending());
}

/// A click on `key` with Ctrl held, primary or secondary (PoB's NF-11 clicks)
fn ctrl_click(h: &mut Harness, key: &str, button: PointerButton) -> SkillsOutput {
    let pos = h.ui.rect(key).center();
    let mut all = SkillsOutput::default();
    for events in [
        vec![
            Event::ModifiersChanged(Modifiers::COMMAND),
            Event::PointerMoved(pos),
        ],
        vec![Event::PointerButton {
            pos,
            button,
            pressed: true,
            modifiers: Modifiers::COMMAND,
        }],
        vec![Event::PointerButton {
            pos,
            button,
            pressed: false,
            modifiers: Modifiers::COMMAND,
        }],
        vec![Event::ModifiersChanged(Modifiers::NONE)],
    ] {
        h.time += 0.2;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(h.time),
            events,
            ..Default::default()
        };
        let time = h.time;
        let ui = &mut h.ui;
        h.ctx
            .run_ui(input, |eui| {
                let pass = ui.show(eui, time, false);
                all.commands.extend(pass.commands);
            })
            .drop_without_applying_deltas();
    }
    all
}

// ---- Task 13: set reorder (R31) and "what if" tooltips (R22) ----

use crate::engine::HoverTarget;

fn drag(h: &mut Harness, from: Pos2, to: Pos2) -> Vec<Command> {
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mid = Pos2::new(from.x + 30.0, from.y + 6.0);
    let mut all = Vec::new();
    for events in [
        vec![Event::PointerMoved(from)],
        vec![button(from, true)],
        vec![Event::PointerMoved(mid)],
        vec![Event::PointerMoved(to)],
        vec![Event::PointerMoved(to)],
        vec![button(to, false)],
        vec![],
    ] {
        all.extend(h.frame(events).commands);
    }
    all
}

#[test]
fn ctrl_clicks_on_a_group_toggle_it_and_its_full_dps() {
    let mut h = Harness::new(sample());
    let o = ctrl_click(&mut h, "row_1", PointerButton::Primary);
    // PoB's list selects the row and toggles it (ListControl.lua:338-370)
    assert_eq!(h.ui.selected, 1);
    let o2 = ctrl_click(&mut h, "row_0", PointerButton::Primary);
    assert_eq!(h.ui.selected, 0);
    assert_eq!(
        debug(&o2.commands),
        ["SetGroupEnabled { group: 0, on: false }"]
    );
    assert_eq!(debug(&o.commands).len(), 1);
    let o = ctrl_click(&mut h, "row_0", PointerButton::Secondary);
    assert_eq!(
        debug(&o.commands),
        ["SetGroupFullDps { group: 0, on: false }"]
    );
    // the right click's menu stays shut under Ctrl
    h.frame(vec![]);
    assert!(!egui::Popup::is_any_open(&h.ctx));
}

#[test]
fn f2_renames_the_selected_set() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    h.press(Key::F2);
    h.frame(vec![]);
    h.click("sets_name");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.type_text("Alt");
    let o = h.click("sets_save");
    assert_eq!(
        debug(&o.commands),
        ["RenameSkillSet { id: 2, title: \"Alt\" }"]
    );
}

#[test]
fn right_clicking_a_gem_name_opens_the_picker_too() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("gem_0_name").center();
    for events in [
        vec![Event::PointerMoved(pos)],
        vec![Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        vec![Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        vec![],
    ] {
        h.frame(events);
    }
    let p = h.ui.picker.as_ref().expect("picker open");
    assert_eq!((p.group(), p.target()), (0, GemTarget::Slot(0)));
}

#[test]
fn group_list_keys_move_the_selection_and_backspace_deletes() {
    let mut h = Harness::new(sample());
    h.click("row_0");
    h.press(Key::ArrowDown);
    h.frame(vec![]);
    // the second group has no gems: it goes at once
    let o = h.press(Key::Backspace);
    assert_eq!(debug(&o.commands), ["DeleteSocketGroup { group: 1 }"]);
}

#[test]
fn set_drag_sends_move_skill_set() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    let from = h.ui.rect("sets_row_0").center();
    let r = h.ui.rect("sets_row_1");
    let below = Pos2::new(r.center().x, r.bottom() - 2.0);
    assert_eq!(
        debug(&drag(&mut h, from, below)),
        ["MoveSkillSet { from: 0, to: 1 }"]
    );
    // a click on a row still only selects it
    let o = h.click("sets_row_1");
    assert!(o.commands.is_empty(), "{:?}", o.commands);
}

#[test]
fn gem_quality_hover_queries_once() {
    let mut h = Harness::new(sample());
    let pos = h.ui.rect("g0_s1_quality").center();
    let o = h.frame(vec![Event::PointerMoved(pos)]);
    assert!(o.queries.is_empty(), "too early: {:?}", o.queries);
    let o = h.frame(vec![]);
    let target = HoverTarget::GemQuality { group: 0, slot: 1 };
    assert_eq!(
        debug(&o.queries),
        debug(&[Command::HoverTooltip(target.clone())])
    );
    for _ in 0..3 {
        assert!(h.frame(vec![]).queries.is_empty(), "asked once");
    }
    // the answer is kept for the open question only
    let answer = |target: HoverTarget| {
        Snapshot::HoverTooltip(Arc::new(crate::engine::views::HoverTooltipView {
            target,
            lines: vec![TooltipLine::Line {
                size: 16,
                text: "^7Setting to 20 quality will give you:".into(),
            }],
        }))
    };
    h.ui.on_snapshot(&answer(HoverTarget::GemQuality { group: 0, slot: 0 }));
    assert!(h.frame(vec![]).queries.is_empty());
    h.ui.on_snapshot(&answer(target.clone()));
    assert!(h.frame(vec![]).queries.is_empty());
    // a new skills view asks again while still hovered
    let mut changed = sample();
    changed.groups[0].gems[1].quality = 20;
    h.ui.on_snapshot(&Snapshot::Skills(Arc::new(changed)));
    let o = h.frame(vec![Event::PointerMoved(pos)]);
    assert_eq!(debug(&o.queries), debug(&[Command::HoverTooltip(target)]));
    // leaving forgets it
    h.frame(vec![Event::PointerMoved(Pos2::new(1100.0, 650.0))]);
    assert!(h.ui.hover_tip.asked().is_none());
}

#[test]
fn gem_enabled_and_count_hovers_name_their_box() {
    let mut view = sample();
    view.groups[0].gems[0].count = Some(1);
    let mut h = Harness::new(view);
    for (key, target) in [
        (
            "gem_0_enabled",
            HoverTarget::GemEnabled { group: 0, slot: 0 },
        ),
        ("g0_s0_count", HoverTarget::GemCount { group: 0, slot: 0 }),
    ] {
        let pos = h.ui.rect(key).center();
        h.frame(vec![Event::PointerMoved(pos)]);
        let o = h.frame(vec![Event::PointerMoved(pos)]);
        assert_eq!(
            debug(&o.queries),
            debug(&[Command::HoverTooltip(target)]),
            "{key}"
        );
    }
}

/// A set's name box is PoB's (SkillSetListControl.lua:53-78): its filter drops every non-ASCII
/// byte, Save needs a visible character after an edit, and the name is saved as it stands
#[test]
fn a_skill_set_name_follows_pobs_box() {
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_new");
    for name in ["\u{65e5}\u{672c}", "\u{3a9}\u{3bc}"] {
        h.click("sets_name");
        h.frame_key(Key::A, Modifiers::COMMAND);
        h.type_text(name);
        let o = h.click("sets_save");
        assert!(o.commands.is_empty(), "{name}: {:?}", o.commands);
    }
    h.click("sets_name");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.type_text("\u{65e5}\u{672c} a ");
    h.frame(vec![]);
    assert_eq!(h.ui.sets.naming_text(), Some(" a "));
    let o = h.click("sets_save");
    assert_eq!(debug(&o.commands), ["NewSkillSet { title: \" a \" }"]);
    // 100 bytes at most
    h.frame(vec![]);
    h.click("sets_new");
    h.click("sets_name");
    // typed key by key it stops at the limit; pasted it is refused whole
    h.type_text(&"x".repeat(130));
    h.frame(vec![]);
    assert_eq!(h.ui.sets.naming_text().map(str::len), Some(100));
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.frame_key(Key::Backspace, Modifiers::NONE);
    h.frame(vec![Event::Paste("x".repeat(101))]);
    h.frame(vec![]);
    assert_eq!(h.ui.sets.naming_text(), Some(""), "refused whole");
}

/// PoB opens the rename box on the set's own title: a set with none opens it empty
#[test]
fn renaming_an_untitled_skill_set_starts_empty() {
    let mut view = sample();
    view.sets[0].untitled = true;
    let mut h = Harness::new(view);
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_0");
    h.click("sets_rename");
    assert_eq!(h.ui.sets.naming_text(), Some(""));
    // a titled one opens on its title
    let mut h = Harness::new(sample());
    h.click("manage");
    h.frame(vec![]);
    h.click("sets_row_1");
    h.click("sets_rename");
    assert_eq!(h.ui.sets.naming_text(), Some("Alt"));
}

/// PoB's socket group label box takes 50 bytes at most (SkillsTab.lua:161)
#[test]
fn a_group_label_takes_fifty_characters() {
    let mut h = Harness::new(sample());
    h.click("g0_label");
    h.frame_key(Key::A, Modifiers::COMMAND);
    h.frame_key(Key::Backspace, Modifiers::NONE);
    h.type_text(&"x".repeat(50));
    h.type_text("y");
    let o = h.press(Key::Enter);
    assert_eq!(
        debug(&o.commands),
        [format!(
            "SetGroupLabel {{ group: 0, text: \"{}\" }}",
            "x".repeat(50)
        )]
    );
}

/// The label box takes 50 bytes, not 50 characters (SkillsTab.lua:161, EditControl.lua:148)
#[test]
fn the_group_label_limit_counts_bytes() {
    let mut h = Harness::new(sample());
    h.click("g0_label");
    h.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    h.frame(vec![Event::Text("\u{65e5}".repeat(20))]);
    h.frame(vec![]);
    assert_eq!(
        h.ui.boxes.text(&"g0_label".to_owned()).unwrap(),
        "\u{65e5}".repeat(16)
    );
}
