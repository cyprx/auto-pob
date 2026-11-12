//! The Notes tab: PoB's notes editor, its colour buttons and a rendered preview.

use crate::engine::views::{NoteColour, NotesView};
use crate::engine::{Command, Snapshot};
use crate::widgets::pob_text::pob_text;
use eframe::egui;
use std::ops::Range;

/// PoB's description above the buttons (`NotesTab.lua`).
const DESCRIPTION: &str = "^7You can use Ctrl +/- (or Ctrl+Scroll) to zoom in and out and Ctrl+0 to reset.\nThis field also supports different colors.  Using the caret symbol (^) followed by a Hex code or a number (0-9) will set the color.\nBelow are some common color codes PoB uses:";

/// Edits are sent this long after the last keystroke.
const DEBOUNCE: f64 = 0.5;

/// The notes box's line height in PoB (`NotesTab.lua:34`), which its zoom starts from and
/// Ctrl+0 goes back to
pub const NOTES_LINE_HEIGHT: u32 = 16;

/// PoB's pixel sizes to egui points, as the tooltips scale them
const TEXT_SCALE: f32 = 0.9;

/// PoB's `EditControl:ZoomText` (EditControl.lua:184-205): Ctrl and `+` or `-` (or the wheel)
/// changes the line height by one, `0` resets it, always within 10..=100. Any other key leaves
/// it alone.
pub fn zoom_line_height(height: u32, key: char, default: u32) -> u32 {
    let next = match key {
        '+' => height + 1,
        '-' => height.saturating_sub(1),
        '0' => default,
        _ => height,
    };
    next.clamp(10, 100)
}

/// PoB's `NotesTab:SetColor` (ruling R7). `sel` is a byte range on char boundaries; an empty
/// one inserts `code` at the cursor. Returns the new text and the cursor's byte position.
pub fn apply_colour(text: &str, sel: Range<usize>, code: &str) -> (String, usize) {
    let end = sel.end.min(text.len());
    let start = sel.start.min(end);
    let (before, selected, after) = (&text[..start], &text[start..end], &text[end..]);
    let mut out = String::with_capacity(text.len() + code.len() + 8);
    out.push_str(before);
    out.push_str(code);
    if selected.is_empty() {
        let cursor = out.len();
        out.push_str(after);
        return (out, cursor);
    }
    // PoB drops the selection's own `^xRRGGBB` codes and ends on the last of them
    let mut last = "^7";
    let mut i = 0;
    while i < selected.len() {
        match hex_code_at(selected, i) {
            Some(len) => {
                last = &selected[i..i + len];
                i += len;
            }
            None => {
                let ch = selected[i..].chars().next().unwrap_or('\0');
                out.push(ch);
                i += ch.len_utf8().max(1);
            }
        }
    }
    out.push_str(last);
    let cursor = out.len();
    out.push_str(after);
    (out, cursor)
}

/// The length of the `^x` + six hex digits code at byte `i`, if there is one.
fn hex_code_at(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    (b[i] == b'^'
        && b.get(i + 1) == Some(&b'x')
        && b.get(i + 2..i + 8)
            .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)))
    .then_some(8)
}

fn byte_of_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(b, _)| b)
}

/// The Notes tab's state: the editor's text and the edit not yet sent.
pub struct NotesUi {
    text: String,
    colours: Vec<NoteColour>,
    /// When the latest unsent edit was made.
    dirty_since: Option<f64>,
    preview: bool,
    edit_id: egui::Id,
    /// The seq of the latest `SetNotes` sent. A Notes snapshot with a lower seq was taken
    /// before that command ran and would put older text back.
    sent_seq: u64,
    #[cfg(test)]
    button_rects: Vec<egui::Rect>,
    /// The editor's text size, PoB's line height: its zoom is the notes' alone and isn't
    /// saved (controller ruling on notes zoom)
    pub(crate) line_height: u32,
}

impl Default for NotesUi {
    fn default() -> Self {
        NotesUi {
            text: String::new(),
            colours: Vec::new(),
            dirty_since: None,
            preview: false,
            edit_id: egui::Id::unique("notes_edit"),
            sent_seq: 0,
            #[cfg(test)]
            button_rects: Vec::new(),
            line_height: NOTES_LINE_HEIGHT,
        }
    }
}

impl NotesUi {
    /// Forgets the build's state, as for another build.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot, seq: u64) {
        if let Snapshot::Notes(view) = snapshot {
            self.on_view(view, seq);
        }
    }

    /// An edit is waiting for its debounce time, so the build has unsaved changes the engine
    /// doesn't know about yet.
    pub fn has_pending(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// Drops the pending edit (the user chose not to save).
    pub fn discard(&mut self) {
        self.dirty_since = None;
    }

    /// `SetNotes` went out as command `seq`.
    pub fn sent(&mut self, seq: u64) {
        self.sent_seq = seq;
    }

    fn on_view(&mut self, view: &NotesView, seq: u64) {
        self.colours.clone_from(&view.colours);
        // The user's newer typing wins, and so does text sent after the view was taken. Comparing
        // seqs, unlike comparing the text, needs no copy of the engine's filtering and cannot
        // stall if a SetNotes fails and is never echoed.
        if self.dirty_since.is_none() && seq >= self.sent_seq {
            self.text.clone_from(&view.text);
        }
    }

    /// Sends the pending edit now.
    pub fn flush(&mut self) -> Option<Command> {
        self.dirty_since.take()?;
        Some(Command::SetNotes {
            text: self.text.clone(),
        })
    }

    fn touch(&mut self, now: f64) {
        self.dirty_since = Some(now);
    }

    #[cfg(test)]
    pub fn type_for_test(&mut self, text: &str, now: f64) {
        self.text = text.to_owned();
        self.touch(now);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, now: f64) -> Option<Command> {
        let size = 13.0;
        let default = ui.visuals().text_color();
        ui.label(pob_text(DESCRIPTION, size, default));
        let mut clicked = None;
        ui.add_enabled_ui(!self.preview, |ui| {
            egui::Grid::new("notes_colours").show(ui, |ui| {
                for (i, colour) in self.colours.iter().enumerate() {
                    let label =
                        pob_text(&format!("{}{}", colour.code, colour.label), size, default);
                    let button = ui.add_sized([100.0, 18.0], egui::Button::new(label));
                    #[cfg(test)]
                    {
                        if i == 0 {
                            self.button_rects.clear();
                        }
                        self.button_rects.push(button.rect);
                    }
                    if button.clicked() {
                        clicked = Some(colour.code.clone());
                    }
                    if i % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
        });
        ui.checkbox(&mut self.preview, "Preview");
        if let Some(code) = clicked {
            self.insert_colour(ui.ctx(), &code, now);
        }
        if self.preview {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| ui.label(pob_text(&self.text, size + 1.0, default)));
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    let rows = (ui.available_height()
                        / ui.text_style_height(&egui::TextStyle::Body))
                    .max(4.0) as usize;
                    let edit = egui::TextEdit::multiline(&mut self.text)
                        .id(self.edit_id)
                        .font(egui::FontId::proportional(
                            self.line_height as f32 * TEXT_SCALE,
                        ))
                        .desired_width(f32::INFINITY)
                        .desired_rows(rows);
                    let response = ui.add(edit);
                    if response.changed() {
                        self.touch(now);
                    }
                    self.zoom_keys(ui, &response);
                });
        }
        match self.dirty_since {
            Some(since) if now - since >= DEBOUNCE => self.flush(),
            Some(since) => {
                ui.request_repaint_after(std::time::Duration::from_secs_f64(
                    (DEBOUNCE - (now - since)).max(0.0),
                ));
                None
            }
            None => None,
        }
    }

    /// Ctrl +, - and 0, and Ctrl+wheel over the editor, zoom the notes' text only, as PoB's
    /// notes box does; the keys are taken before egui's own zoom of the whole window sees them.
    fn zoom_keys(&mut self, ui: &egui::Ui, response: &egui::Response) {
        use egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};
        if !response.has_focus() && !response.hovered() {
            return;
        }
        let mut keys = Vec::new();
        if response.has_focus() {
            ui.input_mut(|i| {
                // each press counts, as each key event reaches PoB's box
                while i.consume_shortcut(&ZOOM_RESET) {
                    keys.push('0');
                }
                while i.consume_shortcut(&ZOOM_IN) || i.consume_shortcut(&ZOOM_IN_SECONDARY) {
                    keys.push('+');
                }
                while i.consume_shortcut(&ZOOM_OUT) {
                    keys.push('-');
                }
            });
        }
        if response.hovered() {
            let zoom = ui.input(|i| i.zoom_delta());
            if zoom > 1.0 {
                keys.push('+');
            } else if zoom < 1.0 {
                keys.push('-');
            }
        }
        for key in keys {
            self.line_height = zoom_line_height(self.line_height, key, NOTES_LINE_HEIGHT);
        }
    }

    /// PoB's `SetColor` on the editor's selection or cursor.
    fn insert_colour(&mut self, ctx: &egui::Context, code: &str, now: f64) {
        let mut state = egui::TextEdit::load_state(ctx, self.edit_id).unwrap_or_default();
        let (a, b) = match state.cursor.char_range() {
            Some(range) => {
                let [lo, hi] = range.sorted_cursors();
                (lo.index.0, hi.index.0)
            }
            None => {
                let end = self.text.chars().count();
                (end, end)
            }
        };
        let sel = byte_of_char(&self.text, a)..byte_of_char(&self.text, b);
        let (text, cursor) = apply_colour(&self.text, sel, code);
        let cursor_char = text[..cursor].chars().count();
        self.text = text;
        self.touch(now);
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(cursor_char),
            )));
        state.store(ctx, self.edit_id);
        ctx.memory_mut(|m| m.request_focus(self.edit_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_inserts_at_cursor() {
        assert_eq!(apply_colour("ab", 1..1, "^7"), ("a^7b".to_owned(), 3));
    }

    #[test]
    fn colour_wraps_selection_with_last_colour() {
        let (text, _) = apply_colour("x^xFF0000red^x00FF00g y", 1..21, "^xAF6025");
        assert_eq!(text, "x^xAF6025redg^x00FF00 y");
    }

    #[test]
    fn colour_wraps_selection_without_codes_ends_with_reset() {
        let (text, cursor) = apply_colour("abc", 0..3, "^1");
        assert_eq!(text, "^1abc^7");
        assert_eq!(cursor, text.len());
    }

    #[test]
    fn colour_handles_non_ascii() {
        let (text, cursor) = apply_colour("é€x", 2..5, "^1");
        assert_eq!(text, "é^1€^7x");
        assert_eq!(cursor, "é^1€^7".len());
    }

    fn run(ui: &mut NotesUi, ctx: &egui::Context, now: f64) -> Option<Command> {
        let mut out = None;
        ctx.run_ui(
            egui::RawInput {
                time: Some(now),
                ..Default::default()
            },
            |ui_| out = ui.show(ui_, now),
        )
        .drop_without_applying_deltas();
        out
    }

    #[test]
    fn notes_debounce_500ms() {
        let ctx = egui::Context::default();
        let mut ui = NotesUi::default();
        ui.type_for_test("hello", 1.0);
        assert!(run(&mut ui, &ctx, 1.2).is_none());
        assert!(run(&mut ui, &ctx, 1.49).is_none());
        let sent = run(&mut ui, &ctx, 1.5);
        assert!(matches!(sent, Some(Command::SetNotes { ref text }) if text == "hello"));
        assert!(run(&mut ui, &ctx, 2.0).is_none(), "sent once");
        assert!(ui.flush().is_none());
    }

    #[test]
    fn incoming_view_does_not_clobber_pending_edit() {
        let mut ui = NotesUi::default();
        let view = |text: &str| NotesView {
            text: text.to_owned(),
            colours: Vec::new(),
        };
        ui.on_view(&view("from engine"), 1);
        assert_eq!(ui.text, "from engine");
        ui.type_for_test("typed", 1.0);
        ui.on_view(&view("stale"), 1);
        assert_eq!(ui.text, "typed");
        assert!(ui.flush().is_some());
        ui.on_view(&view("fresh"), 1);
        assert_eq!(ui.text, "fresh");
    }

    #[test]
    fn stale_echo_does_not_overwrite_newer_sent_text() {
        let view = |text: &str| NotesView {
            text: text.to_owned(),
            colours: Vec::new(),
        };
        let mut ui = NotesUi::default();
        ui.type_for_test("a", 1.0);
        assert!(ui.flush().is_some());
        ui.sent(5);
        ui.type_for_test("ab", 2.0);
        assert!(ui.flush().is_some());
        ui.sent(6);
        ui.on_view(&view("a"), 5);
        assert_eq!(ui.text, "ab");
        ui.on_view(&view("ab"), 6);
        assert_eq!(ui.text, "ab");
        // a later view (open, import) still loads
        ui.on_view(&view("other"), 7);
        assert_eq!(ui.text, "other");
    }

    fn run_events(
        ui: &mut NotesUi,
        ctx: &egui::Context,
        now: f64,
        events: Vec<egui::Event>,
    ) -> Option<Command> {
        let mut out = None;
        ctx.run_ui(
            egui::RawInput {
                time: Some(now),
                events,
                ..Default::default()
            },
            |ui_| out = ui.show(ui_, now),
        )
        .drop_without_applying_deltas();
        out
    }

    #[test]
    fn typing_into_the_editor_sends_after_500ms() {
        let ctx = egui::Context::default();
        let mut ui = NotesUi::default();
        run_events(&mut ui, &ctx, 0.0, vec![]);
        ctx.memory_mut(|m| m.request_focus(ui.edit_id));
        run_events(&mut ui, &ctx, 0.1, vec![]);
        assert!(run_events(&mut ui, &ctx, 1.0, vec![egui::Event::Text("hi".into())]).is_none());
        assert_eq!(ui.text, "hi");
        assert!(run_events(&mut ui, &ctx, 1.3, vec![]).is_none());
        let sent = run_events(&mut ui, &ctx, 1.5, vec![]);
        assert!(matches!(sent, Some(Command::SetNotes { ref text }) if text == "hi"));
    }

    #[test]
    fn clicking_a_colour_button_wraps_the_selection() {
        let ctx = egui::Context::default();
        let mut ui = NotesUi::default();
        ui.on_view(
            &NotesView {
                text: "é a^x00FF00bc d".to_owned(),
                colours: vec![NoteColour {
                    label: "FIRE".into(),
                    code: "^xB97123".into(),
                }],
            },
            1,
        );
        run_events(&mut ui, &ctx, 0.0, vec![]);
        ctx.memory_mut(|m| m.request_focus(ui.edit_id));
        run_events(&mut ui, &ctx, 0.05, vec![]);
        // select "a^x00FF00bc" (chars 2..13)
        let mut state = egui::TextEdit::load_state(&ctx, ui.edit_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(2),
                egui::text::CCursor::new(13),
            )));
        state.store(&ctx, ui.edit_id);
        run_events(&mut ui, &ctx, 0.1, vec![]);
        let at = ui.button_rects[0].center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        run_events(&mut ui, &ctx, 0.2, vec![egui::Event::PointerMoved(at)]);
        run_events(&mut ui, &ctx, 0.3, vec![button(true)]);
        run_events(&mut ui, &ctx, 0.4, vec![button(false)]);
        assert_eq!(ui.text, "é ^xB97123abc^x00FF00 d");
        assert!(ui.has_pending());
    }

    // ---- Task 13: the notes' own text zoom (controller ruling on notes zoom) ----

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }
    }

    #[test]
    fn zoom_line_height_follows_pob() {
        assert_eq!(zoom_line_height(16, '+', 16), 17);
        assert_eq!(zoom_line_height(16, '-', 16), 15);
        assert_eq!(zoom_line_height(30, '0', 16), 16);
        assert_eq!(zoom_line_height(10, '-', 16), 10);
        assert_eq!(zoom_line_height(100, '+', 16), 100);
        assert_eq!(zoom_line_height(20, 'x', 16), 20);
    }

    #[test]
    fn ctrl_plus_zooms_the_notes_not_the_window() {
        let ctx = egui::Context::default();
        let mut ui = NotesUi::default();
        run_events(&mut ui, &ctx, 0.0, vec![]);
        ctx.memory_mut(|m| m.request_focus(ui.edit_id));
        run_events(&mut ui, &ctx, 0.1, vec![]);
        run_events(&mut ui, &ctx, 0.2, vec![key(egui::Key::Plus)]);
        run_events(&mut ui, &ctx, 0.25, vec![key(egui::Key::Plus)]);
        assert_eq!(ui.line_height, NOTES_LINE_HEIGHT + 2);
        assert!((ctx.zoom_factor() - 1.0).abs() < 1e-6, "the window zoomed");
        run_events(&mut ui, &ctx, 0.3, vec![key(egui::Key::Minus)]);
        assert_eq!(ui.line_height, NOTES_LINE_HEIGHT + 1);
        run_events(&mut ui, &ctx, 0.4, vec![key(egui::Key::Num0)]);
        assert_eq!(ui.line_height, NOTES_LINE_HEIGHT);
        assert!(ui.text.is_empty(), "the keys typed text: {:?}", ui.text);
        // without the editor's focus the keys are left to the window
        ctx.memory_mut(|m| m.surrender_focus(ui.edit_id));
        run_events(&mut ui, &ctx, 0.5, vec![key(egui::Key::Plus)]);
        assert_eq!(ui.line_height, NOTES_LINE_HEIGHT);
    }
}
