//! PoB's DropDownControl as one shared widget (DropDownControl.lua:11-15, 422-516;
//! SearchHost.lua:108-115). Every PoB dropdown is a SearchHost: while its list is dropped,
//! typing filters it, Backspace takes the search's last letter, and Esc empties the search,
//! or closes the list when it is empty. The wheel and the arrows step it (`scroll_step`).
//!
//! `Dropdown` is a drop-in for `egui::ComboBox`: the list's entries are drawn with [`item`],
//! [`value`], [`button`] or [`label`], which hide what the search doesn't match and close the
//! list on a pick.

use eframe::egui::{self, Key, Modifiers, WidgetText};

/// The search of the list being drawn, while its entries are added
fn current_id() -> egui::Id {
    egui::Id::NULL.with("pob_dropdown_search")
}

/// Whether `text` matches PoB's search: every word, in any case, each after the one before
/// (SearchHost.lua:41-57 `matchWords`; no PoB dropdown sets `ignoreSearchOrder`)
pub fn search_matches(search: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    let mut from = 0;
    for word in search.split_whitespace() {
        let word = word.to_lowercase();
        match text[from..].find(&word) {
            Some(at) => from += at + word.len(),
            None => return false,
        }
    }
    true
}

/// Which choices of the list being drawn the search kept, in the order they were added: the
/// entries the wheel and the arrows step through, not the headers and actions
fn visible_id() -> egui::Id {
    egui::Id::NULL.with("pob_dropdown_visible")
}

/// How many entries of the list being drawn were added, choices or not
fn added_id() -> egui::Id {
    egui::Id::NULL.with("pob_dropdown_added")
}

/// Whether the search keeps `text`; a choice's is noted for the list's stepping
/// (DropDownControl.lua:57-107)
fn matches(ui: &egui::Ui, text: &str, choice: bool) -> bool {
    let search: String = ui
        .ctx()
        .data(|d| d.get_temp(current_id()))
        .unwrap_or_default();
    let keep = search_matches(&search, text);
    ui.ctx().data_mut(|d| {
        *d.get_temp_mut_or_default::<usize>(added_id()) += 1;
        if choice {
            d.get_temp_mut_or_default::<Vec<bool>>(visible_id())
                .push(keep);
        }
    });
    keep
}

/// An entry hidden by the search: nothing drawn, no room taken, nothing clicked
fn hidden(ui: &mut egui::Ui) -> egui::Response {
    let n = ui
        .ctx()
        .data(|d| d.get_temp::<usize>(added_id()).unwrap_or(0));
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::Vec2::ZERO);
    ui.interact(
        rect,
        ui.scope_id().with(("dropdown_hidden", n)),
        egui::Sense::hover(),
    )
}

fn picked(ui: &mut egui::Ui, response: egui::Response) -> egui::Response {
    if response.clicked() {
        ui.close();
    }
    response
}

/// A selectable entry
pub fn item(ui: &mut egui::Ui, selected: bool, text: impl Into<WidgetText>) -> egui::Response {
    let text = text.into();
    if !matches(ui, text.text(), true) {
        return hidden(ui);
    }
    let response = ui.selectable_label(selected, text);
    picked(ui, response)
}

/// A selectable entry that sets `current` to `selected_value`
pub fn value<V: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut V,
    selected_value: V,
    text: impl Into<WidgetText>,
) -> egui::Response {
    let text = text.into();
    if !matches(ui, text.text(), true) {
        return hidden(ui);
    }
    let response = ui.selectable_value(current, selected_value, text);
    picked(ui, response)
}

/// An action entry (PoB's loadout actions): searched, but not a choice the wheel steps to
pub fn button(ui: &mut egui::Ui, text: impl Into<WidgetText>) -> egui::Response {
    let text = text.into();
    if !matches(ui, text.text(), false) {
        return hidden(ui);
    }
    let response = ui.button(text);
    picked(ui, response)
}

/// A header among the entries: picking it does nothing, but closes the list as any row of
/// PoB's does (DropDownControl.lua:476-484). Not a choice the wheel steps to.
pub fn label(ui: &mut egui::Ui, text: impl Into<WidgetText>) -> egui::Response {
    let text = text.into();
    if !matches(ui, text.text(), false) {
        return hidden(ui);
    }
    let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
    picked(ui, response)
}

/// A drop-in for `egui::ComboBox` with PoB's search
pub struct Dropdown<'a> {
    salt: egui::IdSalt,
    combo: egui::ComboBox<'a>,
}

impl<'a> Dropdown<'a> {
    pub fn from_id_salt(salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        let salt = egui::IdSalt::new(salt);
        Self {
            salt,
            combo: egui::ComboBox::from_id_salt(salt)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        }
    }

    pub fn selected_text(mut self, text: impl egui::IntoAtoms<'a>) -> Self {
        self.combo = self.combo.selected_text(text);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.combo = self.combo.width(width);
        self
    }

    pub fn truncate(mut self) -> Self {
        self.combo = self.combo.truncate();
        self
    }

    /// Draws the button and, while dropped, the search box and `add_contents`
    pub fn show_ui<R>(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        // the ComboBox wraps the salt it is given once more
        let button = ui.make_persistent_id(egui::IdSalt::new(self.salt));
        let search_id = button.with("search");
        let open_id = button.with("opened");
        let open = egui::Popup::is_id_open(ui.ctx(), button.with("popup"));
        // Esc with a search empties it and keeps the list dropped (SearchHost.lua:109-111)
        let searching = ui
            .data(|d| d.get_temp::<String>(search_id))
            .is_some_and(|s| !s.is_empty());
        if open && searching && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            ui.data_mut(|d| d.insert_temp(search_id, String::new()));
            // egui dropped the box's focus on Esc; the list stays the selected control, so
            // typing goes on searching (DropDownControl.lua:422-436)
            ui.memory_mut(|m| m.request_focus(search_id));
        }
        let shown = self.combo.show_ui(ui, |ui| {
            let mut search: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
            let first = !ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
            ui.data_mut(|d| d.insert_temp(open_id, true));
            // typing goes to the search as soon as the list drops (DropDownControl.lua:422-427)
            let edit = ui.add(
                egui::TextEdit::singleline(&mut search)
                    .id(search_id)
                    .hint_text("Search")
                    .desired_width(f32::INFINITY),
            );
            if first {
                edit.request_focus();
            }
            crate::widgets::scroll_step::note_search(ui.ctx(), search_id);

            let active = !search.trim().is_empty();
            ui.data_mut(|d| {
                d.insert_temp(search_id, search.clone());
                d.insert_temp(current_id(), search);
                d.insert_temp(visible_id(), Vec::<bool>::new());
                d.insert_temp(added_id(), 0usize);
            });
            let inner = add_contents(ui);
            ui.data_mut(|d| {
                d.remove::<String>(current_id());
                let seen = d.get_temp::<Vec<bool>>(visible_id()).unwrap_or_default();
                d.remove::<Vec<bool>>(visible_id());
                d.remove::<usize>(added_id());
                // the matches the wheel and the arrows step through
                if active {
                    d.insert_temp(button.with("visible"), seen);
                } else {
                    d.remove::<Vec<bool>>(button.with("visible"));
                }
            });
            inner
        });
        if shown.inner.is_none() {
            // closed: the next drop starts with an empty search, focused
            ui.data_mut(|d| {
                d.insert_temp(open_id, false);
                d.insert_temp(search_id, String::new());
                d.remove::<Vec<bool>>(button.with("visible"));
            });
        }
        shown
    }
}

/// The search box's id of the dropdown made from `salt` in `ui`, for tests
#[cfg(test)]
pub fn search_id(ui: &egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    ui.make_persistent_id(egui::IdSalt::new(egui::IdSalt::new(salt)))
        .with("search")
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};
    use std::collections::HashMap;

    struct Harness {
        ctx: egui::Context,
        button: Rect,
        entries: HashMap<usize, Rect>,
        picks: Vec<usize>,
        /// What the wheel and the arrows stepped to
        steps: Vec<usize>,
        /// The entry the dropdown shows as selected
        selected: usize,
    }

    const LIST: [&str; 4] = ["Alpha", "Beta", "Gamma", "Delta"];

    impl Harness {
        fn new() -> Self {
            let mut h = Harness {
                ctx: egui::Context::default(),
                button: Rect::NOTHING,
                entries: HashMap::new(),
                picks: Vec::new(),
                steps: Vec::new(),
                selected: 0,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            let mut entries = HashMap::new();
            let mut picks = Vec::new();
            let mut button = Rect::NOTHING;
            let mut steps = Vec::new();
            let selected = self.selected;
            self.ctx
                .run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 600.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let r =
                            Dropdown::from_id_salt("d")
                                .selected_text("Alpha")
                                .show_ui(ui, |ui| {
                                    for (i, text) in LIST.iter().enumerate() {
                                        let r = item(ui, i == selected, *text);
                                        if r.rect.height() > 0.0 {
                                            entries.insert(i, r.rect);
                                        }
                                        if r.clicked() {
                                            picks.push(i);
                                        }
                                    }
                                });
                        button = r.response.rect;
                        steps.extend(crate::widgets::scroll_step::combo_step(
                            ui,
                            &r.response,
                            LIST.len(),
                            selected,
                        ));
                    },
                )
                .drop_without_applying_deltas();
            self.entries = entries;
            self.picks.extend(picks);
            self.steps.extend(steps);
            self.button = button;
        }

        fn click(&mut self, pos: Pos2) {
            let b = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            for events in [
                vec![Event::PointerMoved(pos)],
                vec![b(true)],
                vec![b(false)],
                vec![],
                vec![],
            ] {
                self.frame(events);
            }
        }

        fn key(&mut self, key: Key) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![]);
        }
    }

    #[test]
    fn typing_filters_backspace_edits_and_esc_clears_then_closes() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click(h.button.center());
        assert_eq!(h.entries.len(), 4, "the list didn't drop");
        // typing goes to the search (DropDownControl.lua:422-427)
        h.frame(vec![Event::Text("alx".into())]);
        h.frame(vec![]);
        assert!(h.entries.is_empty());
        // Backspace takes the last letter (SearchHost.lua:112-114)
        h.key(Key::Backspace);
        assert_eq!(h.entries.keys().copied().collect::<Vec<_>>(), [0]);
        // a pick closes the list
        h.key(Key::Escape);
        assert_eq!(
            h.entries.len(),
            4,
            "Esc with a search keeps the list dropped"
        );
        // with no search, Esc closes it
        h.key(Key::Escape);
        assert!(
            h.entries.is_empty(),
            "Esc with no search didn't close the list"
        );
        h.click(h.button.center());
        h.frame(vec![Event::Text("gam".into())]);
        h.frame(vec![]);
        let gamma = h.entries[&2].center();
        h.click(gamma);
        assert_eq!(h.picks, [2]);
        assert!(h.entries.is_empty(), "a pick closes the list");
    }

    fn text(h: &mut Harness, t: &str) {
        h.frame(vec![Event::Text(t.into())]);
        h.frame(vec![]);
    }

    #[test]
    fn after_esc_empties_the_search_typing_searches_again() {
        let mut h = Harness::new();
        h.click(h.button.center());
        text(&mut h, "gam");
        assert_eq!(h.entries.len(), 1);
        h.key(Key::Escape);
        assert_eq!(h.entries.len(), 4, "the list closed");
        // the list is still the selected control: typing goes on searching
        // (DropDownControl.lua:422-436)
        text(&mut h, "del");
        assert_eq!(h.entries.keys().copied().collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn search_words_match_in_order() {
        let mut h = Harness::new();
        h.click(h.button.center());
        // PoB's matchWords: each word after the one before (SearchHost.lua:41-57)
        text(&mut h, "ga ma");
        assert_eq!(h.entries.keys().copied().collect::<Vec<_>>(), [2]);
        let mut h = Harness::new();
        h.click(h.button.center());
        text(&mut h, "ma ga");
        assert!(h.entries.is_empty(), "words out of order matched");
        assert!(!search_matches("ta de", "Delta Data"));
        assert!(search_matches("de ta", "Delta"));
    }

    #[test]
    fn hidden_rows_leave_no_gap() {
        let mut h = Harness::new();
        h.click(h.button.center());
        let first = h.entries[&0].top();
        let spacing = h.entries[&1].top() - h.entries[&0].bottom();
        text(&mut h, "del");
        // the one match sits where the first row was
        assert_eq!(h.entries[&3].top(), first);
        // Beta and Delta, with Gamma hidden between them, are as close as any two rows
        text(&mut h, "\u{8}\u{8}\u{8}");
        let mut h = Harness::new();
        h.click(h.button.center());
        text(&mut h, "ta");
        assert_eq!(h.entries[&3].top() - h.entries[&1].bottom(), spacing);
    }

    #[test]
    fn stepping_goes_through_the_matches_only() {
        let mut h = Harness::new();
        h.click(h.button.center());
        // Beta and Delta match; Alpha, the selection, is hidden
        text(&mut h, "ta");
        h.key(Key::ArrowDown);
        h.selected = 1;
        h.frame(vec![]);
        h.key(Key::ArrowDown);
        h.selected = 3;
        h.frame(vec![]);
        h.key(Key::ArrowUp);
        // Alpha hidden: Down goes to the first match (DropDownControl.lua:57-107)
        assert_eq!(h.steps, [1, 3, 1]);
    }
}
