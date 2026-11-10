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
            },
            SkillSetEntry {
                id: 2,
                title: "Alt".into(),
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
        let output = self.ctx.run_ui(input, |ui| {
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
fn paste_event_sends_paste_outside_text_fields() {
    let mut h = Harness::new(sample());
    let o = h.frame(vec![Event::Paste("Label: p".into())]);
    assert_eq!(
        debug(&o.commands),
        ["PasteSocketGroup { text: \"Label: p\" }"]
    );
    h.click("g0_label");
    let o = h.frame(vec![Event::Paste("Label: q".into())]);
    assert!(o.commands.is_empty(), "the text field takes the paste");
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
    assert_eq!(h.ui.edits["g0_label"], "Later");
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
    assert_eq!(h.ui.picker, Some((0, GemTarget::Slot(2))));
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
