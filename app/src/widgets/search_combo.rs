//! A dropdown with a filter box (spec 5): the list opens with a search field on top, typing
//! narrows the entries, and a pick is reported as an index into the whole list, never as a
//! position in the filtered one.

use super::pob_text::pob_text;
use eframe::egui;

/// Draws the combo; returns the list index of an entry picked this frame (not the entry that
/// is already selected). A `selected` past the list shows no selection and doesn't panic.
pub fn search_combo(
    ui: &mut egui::Ui,
    id: egui::Id,
    list: &[String],
    selected: usize,
) -> Option<usize> {
    search_combo_ui(ui, id, list, selected, None, &mut |_, _| {}).1
}

/// As `search_combo`, with a fixed `width` and `entry`, which is told each drawn entry's list
/// index and response (tooltips, tests). Returns the combo's own response too.
pub fn search_combo_ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    list: &[String],
    selected: usize,
    width: Option<f32>,
    entry: &mut dyn FnMut(usize, &egui::Response),
) -> (egui::Response, Option<usize>) {
    let default = ui.visuals().text_color();
    let current = list.get(selected).map_or("", String::as_str);
    let filter_id = id.with("filter");
    let open_id = id.with("opened");
    let mut picked = None;
    let mut combo = egui::ComboBox::from_id_salt(id)
        .selected_text(pob_text(current, 14.0, default))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    if let Some(width) = width {
        combo = combo.width(width);
    }
    let shown = combo.show_ui(ui, |ui| {
        let mut filter = ui
            .data_mut(|d| d.get_temp::<String>(filter_id))
            .unwrap_or_default();
        let first = !ui.data_mut(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
        ui.data_mut(|d| d.insert_temp(open_id, true));
        let edit = ui.add(
            egui::TextEdit::singleline(&mut filter)
                .id(filter_id)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
        if first {
            edit.request_focus();
        }
        let needle = filter.to_lowercase();
        let mut first_match = None;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .auto_shrink([true, true])
            .show(ui, |ui| {
                for (i, label) in list.iter().enumerate() {
                    let text = pob_text(label, 14.0, default);
                    if !needle.is_empty() && !text.text.to_lowercase().contains(&needle) {
                        continue;
                    }
                    first_match.get_or_insert(i);
                    let item = ui.selectable_label(i == selected, text);
                    entry(i, &item);
                    if item.clicked() {
                        if i == selected {
                            ui.close();
                        } else {
                            picked = Some(i);
                        }
                    }
                }
            });
        if edit.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(i) = first_match
            && i != selected
        {
            picked = Some(i);
        }
        if picked.is_some() {
            filter.clear();
            ui.close();
        }
        ui.data_mut(|d| d.insert_temp(filter_id, filter));
    });
    if shown.inner.is_none() {
        // closed: the next opening starts with an empty filter and the focus in it
        ui.data_mut(|d| {
            d.insert_temp(open_id, false);
            d.insert_temp(filter_id, String::new());
        });
    }
    (shown.response, picked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
    use std::collections::HashMap;

    struct Harness {
        ctx: egui::Context,
        time: f64,
        list: Vec<String>,
        selected: usize,
        combo: Rect,
        entries: HashMap<usize, Rect>,
        picks: Vec<usize>,
    }

    impl Harness {
        fn new(list: &[&str], selected: usize) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                time: 1.0,
                list: list.iter().map(|s| (*s).to_owned()).collect(),
                selected,
                combo: Rect::NOTHING,
                entries: HashMap::new(),
                picks: Vec::new(),
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut seen = HashMap::new();
            let mut combo = Rect::NOTHING;
            let mut picks = Vec::new();
            let output = self.ctx.run_ui(input, |ui| {
                let (response, pick) = search_combo_ui(
                    ui,
                    egui::Id::unique("combo"),
                    &self.list,
                    self.selected,
                    None,
                    &mut |i, r| {
                        seen.insert(i, r.rect);
                    },
                );
                combo = response.rect;
                picks.extend(pick);
            });
            output.drop_without_applying_deltas();
            self.combo = combo;
            // an open combo draws its entries; a closed one keeps the last frame's
            if !seen.is_empty() || !self.entries.is_empty() {
                self.entries = seen;
            }
            self.picks.extend(picks);
        }

        fn click(&mut self, pos: Pos2) {
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![button(true)],
                vec![button(false)],
                vec![],
                vec![],
            ] {
                self.frame(events);
            }
        }

        fn open(&mut self) {
            self.click(self.combo.center());
            self.frame(vec![]);
            assert!(!self.entries.is_empty(), "the combo did not open");
        }
    }

    #[test]
    fn searchable_dropdown_filters_but_sends_list_index() {
        let mut h = Harness::new(&["Alpha", "Beta", "Gamma", "Delta"], 0);
        h.open();
        assert_eq!(h.entries.len(), 4, "{:?}", h.entries.keys());
        h.frame(vec![Event::Text("ga".into())]);
        h.frame(vec![]);
        let shown: Vec<usize> = h.entries.keys().copied().collect();
        assert_eq!(shown, [2], "only Gamma matches");
        h.click(h.entries[&2].center());
        assert_eq!(h.picks, [2], "the list index, not the filtered position 0");
    }

    #[test]
    fn dropdown_pick_without_a_filter_sends_the_list_index() {
        let mut h = Harness::new(&["Alpha", "Beta", "Gamma"], 0);
        h.open();
        h.click(h.entries[&1].center());
        assert_eq!(h.picks, [1]);
    }

    #[test]
    fn colour_codes_do_not_take_part_in_the_filter() {
        let mut h = Harness::new(&["^xFF0000Red", "^7Blue"], 0);
        h.open();
        h.frame(vec![Event::Text("x".into())]);
        h.frame(vec![]);
        assert!(h.entries.is_empty(), "the code letters are not text");
        h.frame(vec![Event::Key {
            key: Key::Backspace,
            physical_key: Some(Key::Backspace),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Event::Text("blu".into())]);
        h.frame(vec![]);
        assert_eq!(h.entries.keys().copied().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn a_selection_past_the_list_shows_nothing_and_does_not_panic() {
        let mut h = Harness::new(&["Alpha", "Beta"], 9);
        h.open();
        assert_eq!(h.entries.len(), 2);
        h.click(h.entries[&0].center());
        assert_eq!(h.picks, [0]);
    }
}
