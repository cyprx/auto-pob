//! The stats sidebar: PoB's main-skill selector, then the player's rows and the minion's,
//! coloured like PoB.

use crate::engine::views::{DropList, MainSkillSelector, StatRow, StatsView};
use crate::engine::{CalcsControl, CalcsCount, Command};
use crate::theme;
use crate::widgets::pob_text;
use eframe::egui;

/// Where a widget was drawn, so tests can click it. Does nothing outside tests.
#[cfg(test)]
fn note(ui: &egui::Ui, key: &str, rect: egui::Rect) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::NULL.with(("sidebar_rect", key.to_owned())), rect));
}

#[cfg(not(test))]
fn note(_ui: &egui::Ui, _key: &str, _rect: egui::Rect) {}

/// Draws the sidebar and returns what the user changed.
pub fn show(ui: &mut egui::Ui, stats: Option<&StatsView>, behind: bool) -> Vec<Command> {
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        theme::heading(ui, "Stats");
        if behind {
            ui.spinner();
        }
    });
    let Some(stats) = stats else {
        ui.label("Calculating…");
        return out;
    };
    egui::ScrollArea::vertical().show(ui, |ui| {
        selector(ui, &stats.main_skill, behind, &mut out);
        rows(ui, "player_stats", &stats.stats);
        if !stats.minion.is_empty() {
            ui.add_space(6.0);
            ui.strong("Minion");
            rows(ui, "minion_stats", &stats.minion);
        }
    });
    out
}

/// PoB's main-skill controls above the stats; the ones PoB hides aren't drawn.
fn selector(ui: &mut egui::Ui, sel: &MainSkillSelector, behind: bool, out: &mut Vec<Command>) {
    egui::Grid::new("main_skill_selector")
        .num_columns(2)
        .show(ui, |ui| {
            dropdown(
                ui,
                "group",
                "Main skill",
                CalcsControl::Group,
                Some(&sel.group),
                out,
            );
            dropdown(
                ui,
                "skill",
                "Active skill",
                CalcsControl::Skill,
                sel.skill.as_ref(),
                out,
            );
            dropdown(
                ui,
                "part",
                "Skill part",
                CalcsControl::Part,
                sel.part.as_ref(),
                out,
            );
            if let Some(text) = &sel.stages {
                count(
                    ui,
                    "stages",
                    "Stages",
                    CalcsCount::Stages,
                    text,
                    behind,
                    out,
                );
            }
            if let Some(text) = &sel.mines {
                count(
                    ui,
                    "mines",
                    "Active mines",
                    CalcsCount::Mines,
                    text,
                    behind,
                    out,
                );
            }
            dropdown(
                ui,
                "minion",
                "Minion",
                CalcsControl::Minion,
                sel.minion.as_ref(),
                out,
            );
            dropdown(
                ui,
                "minion_skill",
                "Minion skill",
                CalcsControl::MinionSkill,
                sel.minion_skill.as_ref(),
                out,
            );
        });
    ui.add_space(6.0);
}

fn dropdown(
    ui: &mut egui::Ui,
    key: &'static str,
    label: &str,
    control: CalcsControl,
    list: Option<&DropList>,
    out: &mut Vec<Command>,
) {
    let Some(list) = list else { return };
    ui.label(label);
    let default = ui.visuals().text_color();
    let current = list.labels.get(list.selected).map_or("", String::as_str);
    ui.add_enabled_ui(list.enabled, |ui| {
        let combo = egui::ComboBox::from_id_salt(("sidebar_select", key))
            .width(150.0)
            .selected_text(pob_text(current, 14.0, default))
            .show_ui(ui, |ui| {
                for (i, text) in list.labels.iter().enumerate() {
                    let item =
                        ui.selectable_label(i == list.selected, pob_text(text, 14.0, default));
                    note(ui, &format!("{key}_item_{i}"), item.rect);
                    if item.clicked() && i != list.selected {
                        out.push(Command::SetMainSkillSelect { control, index: i });
                    }
                }
            });
        note(ui, key, combo.response.rect);
    });
    ui.end_row();
}

/// The stage or mine count: sent on Enter or when the box loses focus; Esc puts back what the
/// build has.
fn count(
    ui: &mut egui::Ui,
    key: &'static str,
    label: &str,
    control: CalcsCount,
    text: &str,
    behind: bool,
    out: &mut Vec<Command>,
) {
    ui.label(label);
    let id = ui.make_persistent_id(("sidebar_count", key));
    let (mut buf, mut sent) = ui
        .data_mut(|d| d.get_temp::<(String, String)>(id))
        .unwrap_or_else(|| (text.to_owned(), text.to_owned()));
    let focused = ui.memory(|m| m.has_focus(id));
    if !focused && !behind {
        buf = text.to_owned();
        sent = text.to_owned();
    }
    let escape = focused && ui.input(|i| i.key_pressed(egui::Key::Escape));
    let response = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .id(id)
            .desired_width(50.0),
    );
    note(ui, key, response.rect);
    if escape {
        buf = sent.clone();
    }
    if response.lost_focus() && buf != sent {
        sent = buf.clone();
        out.push(Command::SetMainSkillCount {
            control,
            text: buf.clone(),
        });
    }
    ui.data_mut(|d| d.insert_temp(id, (buf, sent)));
    ui.end_row();
}

fn rows(ui: &mut egui::Ui, id: &str, rows: &[StatRow]) {
    let default = ui.visuals().text_color();
    egui::Grid::new(id).striped(true).show(ui, |ui| {
        for row in rows {
            let label = format!("{}{}", row.color.as_deref().unwrap_or(""), row.label);
            ui.label(pob_text(&label, 14.0, default));
            ui.label(pob_text(&row.text, 14.0, default));
            ui.end_row();
            for note in &row.notes {
                ui.label(pob_text(note, 12.0, default));
                ui.label("");
                ui.end_row();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    fn list(labels: &[&str], enabled: bool) -> DropList {
        DropList {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            selected: 0,
            enabled,
        }
    }

    fn view(selector: MainSkillSelector) -> StatsView {
        StatsView {
            main_skill: selector,
            ..StatsView::default()
        }
    }

    struct Harness {
        ctx: egui::Context,
        stats: StatsView,
        time: f64,
    }

    impl Harness {
        fn new(stats: StatsView) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                stats,
                time: 1.0,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<String> {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = Vec::new();
            self.ctx
                .run_ui(input, |ui| out = show(ui, Some(&self.stats), false))
                .drop_without_applying_deltas();
            out.iter().map(|c| format!("{c:?}")).collect()
        }

        fn rect(&self, key: &str) -> Option<Rect> {
            self.ctx
                .data(|d| d.get_temp(egui::Id::NULL.with(("sidebar_rect", key.to_owned()))))
        }

        fn click(&mut self, key: &str) -> Vec<String> {
            let pos = self.rect(key).unwrap().center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut all = Vec::new();
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                all.extend(self.frame(events));
            }
            all
        }

        fn key(&mut self, key: Key) -> Vec<String> {
            let mut all = Vec::new();
            for pressed in [true, false] {
                all.extend(self.frame(vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]));
            }
            all
        }

        fn type_text(&mut self, text: &str) -> Vec<String> {
            self.frame(vec![Event::Text(text.to_owned())])
        }
    }

    fn base() -> MainSkillSelector {
        MainSkillSelector {
            group: list(&["Group A", "Group B"], true),
            ..MainSkillSelector::default()
        }
    }

    #[test]
    fn sidebar_selector_sends_main_skill_select() {
        let mut h = Harness::new(view(base()));
        let mut sent = h.click("group");
        for _ in 0..3 {
            sent.extend(h.frame(vec![]));
        }
        assert!(h.rect("group_item_1").is_some(), "the list opened");
        sent.extend(h.click("group_item_1"));
        assert_eq!(sent, ["SetMainSkillSelect { control: Group, index: 1 }"]);
    }

    #[test]
    fn hidden_sidebar_lists_are_not_drawn() {
        let h = Harness::new(view(base()));
        assert!(h.rect("group").is_some());
        for key in ["skill", "part", "stages", "mines", "minion", "minion_skill"] {
            assert!(h.rect(key).is_none(), "{key} is hidden");
        }
    }

    #[test]
    fn disabled_sidebar_list_sends_nothing() {
        let mut sel = base();
        sel.part = Some(list(&["One", "Two"], false));
        let mut h = Harness::new(view(sel));
        let mut sent = h.click("part");
        for _ in 0..3 {
            sent.extend(h.frame(vec![]));
        }
        assert!(
            h.rect("part_item_1").is_none(),
            "a greyed list doesn't open"
        );
        assert!(sent.is_empty());
    }

    fn counted(text: &str) -> Harness {
        let mut sel = base();
        sel.stages = Some(text.to_owned());
        Harness::new(view(sel))
    }

    #[test]
    fn count_is_sent_on_enter_and_on_blur_and_reverts_on_escape() {
        // Enter
        let mut h = counted("1");
        let mut sent = h.click("stages");
        sent.extend(h.type_text("2"));
        sent.extend(h.key(Key::Enter));
        assert_eq!(
            sent,
            ["SetMainSkillCount { control: Stages, text: \"12\" }"]
        );

        // blur: clicking elsewhere
        let mut h = counted("1");
        h.click("stages");
        h.type_text("3");
        let sent = h.click("group");
        assert_eq!(
            sent,
            ["SetMainSkillCount { control: Stages, text: \"13\" }"]
        );

        // Esc reverts and sends nothing
        let mut h = counted("1");
        h.click("stages");
        h.type_text("9");
        let mut sent = h.key(Key::Escape);
        sent.extend(h.frame(vec![]));
        assert!(sent.is_empty(), "{sent:?}");
        // and a later blur doesn't send the abandoned text either
        sent.extend(h.click("group"));
        assert!(sent.is_empty(), "{sent:?}");
    }

    #[test]
    fn unchanged_count_is_not_sent() {
        let mut h = counted("4");
        let mut sent = h.click("stages");
        sent.extend(h.key(Key::Enter));
        assert!(sent.is_empty());
    }

    #[test]
    fn full_dps_notes_are_drawn_without_panicking() {
        let mut stats = view(base());
        stats.stats.push(StatRow {
            key: "SkillDPS".into(),
            label: "2x Name (trigger)".into(),
            value: 1.0,
            text: "100".into(),
            color: Some("^xFF8800".into()),
            notes: vec!["^7part".into(), "^8source".into()],
        });
        Harness::new(stats);
    }
}
