//! PoB's name boxes: the character filter and length limit of its `EditControl`, and the
//! "has a visible character" test its Save buttons make.
//!
//! A box made without a filter takes `^%w%p ` under LuaJIT (`main.unicode` is off), so it keeps
//! only ASCII letters, digits, punctuation and spaces (EditControl.lua:46). The boxes for build
//! and folder names pass `\/:%*%?"<>|%c` instead, which drops those characters and leaves every
//! other byte alone (BuildListControl.lua:90, Main.lua:1696, Build.lua:1399). The limit counts
//! bytes (EditControl.lua:148). PoB never trims: the Save buttons ask for `%S` and save the box as
//! it stands.

use crate::engine::bytes_text::{pob_char_len, pob_len};
use eframe::egui;

/// PoB's limit for set, build and folder names
pub const NAME_LIMIT: usize = 100;

/// Which characters a name box drops
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// `EditControl`'s own filter, when the box passes none
    Plain,
    /// The build and folder name boxes' filter
    Path,
}

impl Filter {
    /// Whether the box keeps `c`
    pub fn keeps(self, c: char) -> bool {
        match self {
            Filter::Plain => c.is_ascii_alphanumeric() || c.is_ascii_punctuation() || c == ' ',
            Filter::Path => !c.is_ascii_control() && !"\\/:*?\"<>|".contains(c),
        }
    }
}

/// What the box would hold after `text` is typed into it
pub fn filtered(text: &str, filter: Filter) -> String {
    text.chars().filter(|c| filter.keeps(*c)).collect()
}

/// Lua's `%s` in the C locale
fn is_lua_space(c: char) -> bool {
    c == ' ' || ('\t'..='\r').contains(&c)
}

/// Whether the box matches `%S`, as every Save button asks. `text` is what the box holds: typed
/// text has been through `edit`'s filter, but the box's first text (a name being renamed) has
/// not, and its non-ASCII bytes are not spaces to Lua either.
pub fn has_visible(text: &str) -> bool {
    text.chars().any(|c| !is_lua_space(c))
}

/// Whether this frame pastes text into the box
pub fn pasted(ui: &egui::Ui) -> bool {
    ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))))
}

/// PoB's `sanitiseText` (Common.lua:251-274), the item text box's paste filter: only for a
/// text with a byte from 128 up or a "<", it drops `<...>` tags, turns the dashes, the minus
/// sign, "ä" and "ö" into ASCII, and every other byte from 128 up into "?". It works on bytes, so
/// a lone Windows-1252 dash byte inside another character becomes "-" too, as in PoB.
pub fn sanitise_text(text: &str) -> String {
    let bytes = text.as_bytes();
    if !bytes.iter().any(|&b| b >= 128 || b == b'<') {
        return text.to_owned();
    }
    // %b<>: a balanced pair, nested ones inside it
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut depth = 0;
            let mut end = None;
            for (j, &b) in bytes.iter().enumerate().skip(i) {
                if b == b'<' {
                    depth += 1;
                } else if b == b'>' {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(j);
                        break;
                    }
                }
            }
            if let Some(end) = end {
                i = end + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let replace = |v: Vec<u8>, from: &[u8], to: &[u8]| -> Vec<u8> {
        let mut r = Vec::with_capacity(v.len());
        let mut i = 0;
        while i < v.len() {
            if v[i..].starts_with(from) {
                r.extend_from_slice(to);
                i += from.len();
            } else {
                r.push(v[i]);
                i += 1;
            }
        }
        r
    };
    for (from, to) in [
        (&b"\xE2\x80\x90"[..], &b"-"[..]),
        (b"\xE2\x80\x91", b"-"),
        (b"\xE2\x80\x92", b"-"),
        (b"\xE2\x80\x93", b"-"),
        (b"\xE2\x80\x94", b"-"),
        (b"\xE2\x80\x95", b"-"),
        (b"\xE2\x88\x92", b"-"),
        (b"\xC3\xA4", b"a"),
        (b"\xC3\xB6", b"o"),
        (b"\x96", b"-"),
        (b"\x97", b"-"),
        (b"\xE4", b"a"),
        (b"\xF6", b"o"),
    ] {
        out = replace(out, from, to);
    }
    out.iter()
        .map(|&b| if b >= 128 { '?' } else { char::from(b) })
        .collect()
}

/// The bytes an edit kept at each end: it replaced `before[head..before.len() - tail]` with
/// `raw[head..raw.len() - tail]`
fn edit_span(before: &str, raw: &str) -> (usize, usize) {
    let head: usize = before
        .chars()
        .zip(raw.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let tail: usize = before[head..]
        .chars()
        .rev()
        .zip(raw[head..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    (head, tail)
}

/// A box's selection as its last frame left it: read before the box runs, it is what an edit
/// this frame replaces (PoB's `sel` and `caret`, EditControl.lua:140-172)
pub type Selection = Option<egui::text::CCursorRange>;

/// The selection of the text box `id`, read before the box runs this frame
pub fn selection(ctx: &egui::Context, id: egui::Id) -> Selection {
    egui::TextEdit::load_state(ctx, id).and_then(|state| state.cursor.char_range())
}

/// The selection of the box `ui` adds next under an auto id, read before it runs; `own` keeps
/// it only if the box did take that id
pub fn next_selection(ui: &egui::Ui) -> (egui::Id, Selection) {
    let id = ui.next_auto_id();
    (id, selection(ui.ctx(), id))
}

/// The selection `next_selection` read, if the box drawn is the one it read it for
pub fn own(next: (egui::Id, Selection), id: egui::Id) -> Selection {
    if next.0 == id { next.1 } else { None }
}

/// `edit_span` from the box's own selection: the edit replaced the selected text (or went in
/// at the caret) when `raw` keeps what lies outside it; anything else, such as a Delete or an
/// undo, falls back to the text diff
fn edit_span_at(before: &str, raw: &str, sel: Selection) -> (usize, usize) {
    let byte = |chars: usize| before.char_indices().nth(chars).map(|(i, _)| i);
    if let Some(range) = sel {
        let [lo, hi] = range.sorted_cursors();
        let count = before.chars().count();
        let (lo, hi) = (lo.index.0, hi.index.0);
        if hi <= count {
            let start = byte(lo).unwrap_or(before.len());
            let end = byte(hi).unwrap_or(before.len());
            let tail = before.len() - end;
            if raw.len() > start + tail
                && raw.starts_with(&before[..start])
                && raw.ends_with(&before[end..])
                && raw.is_char_boundary(raw.len() - tail)
            {
                return (start, tail);
            }
        }
    }
    edit_span(before, raw)
}

/// Where PoB's caret sits, in characters, when egui left it at `caret` in `raw` and `spliced`
/// made the box hold `result`. The edit ends at egui's caret, so the characters the filter
/// dropped or added all lie before it: PoB's caret is right after what went in, or where it
/// was when the box refused the edit (EditControl.lua:140-172).
pub fn shifted_caret(caret: usize, raw: &str, result: &str) -> usize {
    let (raw, result) = (raw.chars().count(), result.chars().count());
    (caret + result).saturating_sub(raw).min(result)
}

/// The selection PoB keeps, in characters, when the box refused an edit that replaced one:
/// `ReplaceSel` changes nothing when the filter drops a character or the limit is passed
/// (EditControl.lua:140-152). None for any other edit.
pub fn kept_selection(before: &str, raw: &str, result: &str) -> Option<(usize, usize)> {
    if result != before || raw == before {
        return None;
    }
    let (head, tail) = edit_span(before, raw);
    if head + tail >= before.len() {
        return None;
    }
    let start = before[..head].chars().count();
    let end = before[..before.len() - tail].chars().count();
    Some((start, end))
}

/// Puts the caret of the text box `id` where PoB's would be after `spliced` turned egui's
/// edit of `before`, `raw`, into `result`. egui leaves it after the raw edit, which a filter
/// that drops or widens characters, or a refused edit, moved. A refused edit leaves the box as
/// it was: `sel`, the selection read before the box ran, comes back as it was, direction and
/// all (EditControl.lua:140-152).
pub fn place_caret(
    ctx: &egui::Context,
    id: egui::Id,
    before: &str,
    raw: &str,
    result: &str,
    sel: Selection,
) {
    if raw == result {
        return;
    }
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id)
        && let Some(range) = state.cursor.char_range()
    {
        let refused = result == before;
        let range = match (refused, sel, kept_selection(before, raw, result)) {
            (true, Some(sel), _) => sel,
            (_, _, Some((start, end))) => egui::text::CCursorRange::two(
                egui::text::CCursor::new(start),
                egui::text::CCursor::new(end),
            ),
            _ => egui::text::CCursorRange::one(egui::text::CCursor::new(shifted_caret(
                range.primary.index.0,
                raw,
                result,
            ))),
        };
        state.cursor.set_char_range(Some(range));
        state.store(ctx, id);
    }
}

/// What PoB's box holds after the edit that turned `before` into `raw`. PoB filters and limits
/// only the text an edit inserts, so the text already in the box, such as a name being renamed,
/// stays as it is (EditControl.lua:140-172).
///
/// Typed text goes in one key at a time: over a selection the first key is `ReplaceSel`, which
/// refuses a character the filter drops (the selection stays), and every key after it is
/// `Insert`, which drops such a character; a key that would pass the limit is refused. A paste
/// is one insertion: `Insert` drops the rejected characters and refuses nothing else, `ReplaceSel`
/// refuses the whole text if it has one, and either refuses the whole text past the limit.
pub fn spliced(before: &str, raw: &str, filter: Filter, limit: usize, pasted: bool) -> String {
    spliced_by(before, raw, |c| filter.keeps(c), limit, pasted)
}

/// `spliced` with any filter: `keeps` says which characters the box takes
pub fn spliced_by(
    before: &str,
    raw: &str,
    keeps: impl Fn(char) -> bool,
    limit: usize,
    pasted: bool,
) -> String {
    spliced_with(before, raw, keeps, limit, pasted, None, None)
}

/// `spliced_by` knowing the box's selection before the edit (`selection`), so a paste over it
/// is PoB's `ReplaceSel` and one at the caret its `Insert`, whatever the texts look like
pub fn spliced_at(
    before: &str,
    raw: &str,
    sel: Selection,
    keeps: impl Fn(char) -> bool,
    limit: usize,
    pasted: bool,
) -> String {
    spliced_with(before, raw, keeps, limit, pasted, None, sel)
}

/// `spliced_by` for a box with a `pasteFilter`, which a paste goes through before anything
/// else (EditControl.lua:530), and the box's selection before the edit, if known
pub fn spliced_with(
    before: &str,
    raw: &str,
    keeps: impl Fn(char) -> bool,
    limit: usize,
    pasted: bool,
    paste_filter: Option<fn(&str) -> String>,
    sel: Selection,
) -> String {
    let (head, tail) = edit_span_at(before, raw, sel);
    let (front, rest) = raw.split_at(head);
    let (inserted, back) = rest.split_at(rest.len() - tail);
    if inserted.is_empty() {
        return raw.to_owned();
    }
    let mut replacing = head + tail < before.len();
    let mut out = String::new();
    if pasted {
        let filtered = paste_filter.map(|f| f(inserted));
        let inserted = filtered.as_deref().unwrap_or(inserted);
        // Ctrl+V turns every byte from 128 up into "?" before the filter sees it
        // (EditControl.lua:533); an escape is one of PoB's bytes
        let swapped: String = inserted
            .chars()
            .flat_map(|c| {
                let n = if c.is_ascii() { 1 } else { pob_char_len(c) };
                std::iter::repeat_n(if c.is_ascii() { c } else { '?' }, n)
            })
            // ReplaceSel and Insert drop "\r" before the filter (EditControl.lua:141, 163)
            .filter(|c| *c != '\r')
            .collect();
        let inserted = swapped.as_str();
        let kept: String = inserted.chars().filter(|c| keeps(*c)).collect();
        if kept.is_empty() || (replacing && kept != inserted) {
            return before.to_owned();
        }
        out = kept;
    } else {
        // the limit counts PoB's bytes: an escape is one (`bytes_text`). The text around the
        // insert is counted once, and what is kept as it grows, so a long insert stays linear.
        let around = if limit == usize::MAX {
            0
        } else {
            pob_len(front) + pob_len(back)
        };
        let mut held = 0;
        for c in inserted.chars() {
            if !keeps(c) {
                continue;
            }
            if limit != usize::MAX && around + held + pob_char_len(c) > limit {
                continue;
            }
            out.push(c);
            held += pob_char_len(c);
            replacing = false;
        }
        if replacing {
            return before.to_owned();
        }
    }
    let result = format!("{front}{out}{back}");
    if pasted && pob_len(&result) > limit {
        return before.to_owned();
    }
    result
}

/// A single-line name box that holds what PoB's would. True in the second place when the text
/// changed (PoB calls the box's change function only then).
pub fn edit(
    ui: &mut egui::Ui,
    text: &mut String,
    filter: Filter,
    limit: usize,
    build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> (egui::Response, bool) {
    let before = text.clone();
    let pasted = pasted(ui);
    // the box takes the next auto id unless `build` gives it one
    let next = next_selection(ui);
    let response = ui.add(build(egui::TextEdit::singleline(text)));
    let sel = own(next, response.id);
    crate::widgets::url_click::open_on_ctrl_click(ui, &response, text);
    crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &response, text);
    let mut changed = response.changed();
    if changed {
        let same = *text == before;
        let raw = std::mem::take(text);
        *text = spliced_at(&before, &raw, sel, |c| filter.keeps(c), limit, pasted);
        place_caret(ui.ctx(), response.id, &before, &raw, text, sel);
        // typing over the text with itself is an edit; a character the box drops is none
        changed = same || *text != before;
    }
    (response, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PoB's caret goes right after what an edit put in, or stays where it was when the box
    /// refused the edit (EditControl.lua:140-172)
    #[test]
    fn shifted_caret_follows_the_filtered_insertion() {
        // a "\r" dropped from a paste in the middle: egui's caret is after "x\r\ny"
        let raw = "Ax\r\nyB";
        let result = spliced_by("AB", raw, |_| true, usize::MAX, true);
        assert_eq!(result, "Ax\nyB");
        assert_eq!(shifted_caret(5, raw, &result), 4);
        // a paste that starts like the text after it: "b\r" pasted after "a" of "ab"
        let raw = "ab\rb";
        let result = spliced_by("ab", raw, |_| true, usize::MAX, true);
        assert_eq!(result, "abb");
        assert_eq!(shifted_caret(3, raw, &result), 2);
        // a non-ASCII character pasted as one "?" per byte moves it the other way
        let raw = "a\u{e9}b";
        let result = spliced_by("ab", raw, |_| true, usize::MAX, true);
        assert_eq!(result, "a??b");
        assert_eq!(shifted_caret(2, raw, &result), 3);
        // a refused key leaves the caret where it was
        let raw = "a/b";
        let result = spliced("ab", raw, Filter::Path, NAME_LIMIT, false);
        assert_eq!(result, "ab");
        assert_eq!(shifted_caret(2, raw, &result), 1);
    }

    /// PoB's ReplaceSel refuses a character its filter drops and keeps the selection
    /// (EditControl.lua:140-152)
    #[test]
    fn a_refused_edit_over_a_selection_keeps_it() {
        // "bc" of "abcd" selected, "/" typed into a build name box
        let raw = "a/d";
        let result = spliced("abcd", raw, Filter::Path, NAME_LIMIT, false);
        assert_eq!(result, "abcd");
        assert_eq!(kept_selection("abcd", raw, &result), Some((1, 3)));
        // with no selection, or a taken edit, nothing is kept
        assert_eq!(kept_selection("ab", "a/b", "ab"), None);
        assert_eq!(kept_selection("abcd", "axd", "axd"), None);
    }

    /// A build name box holding "abcd" with `sel` selected gets `events`, then "x": what it
    /// holds after each, and the selection the refused edit left
    fn name_box_run(
        sel: (usize, usize),
        events: Vec<egui::Event>,
    ) -> (String, String, Option<egui::text::CCursorRange>) {
        let ctx = egui::Context::default();
        let id = std::cell::Cell::new(egui::Id::NULL);
        let mut text = String::from("abcd");
        let run = |events: Vec<egui::Event>, text: &mut String| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 100.0),
                )),
                events,
                ..Default::default()
            };
            let out = ctx.run_ui(input, |ui| {
                id.set(edit(ui, text, Filter::Path, NAME_LIMIT, |e| e).0.id);
            });
            out.drop_without_applying_deltas();
        };
        run(vec![], &mut text);
        ctx.memory_mut(|m| m.request_focus(id.get()));
        run(vec![], &mut text);
        let mut state = egui::TextEdit::load_state(&ctx, id.get()).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(sel.0),
                egui::text::CCursor::new(sel.1),
            )));
        state.store(&ctx, id.get());
        run(events, &mut text);
        let after = text.clone();
        let kept = selection(&ctx, id.get());
        run(vec![egui::Event::Text("x".into())], &mut text);
        (after, text, kept)
    }

    /// PoB's ReplaceSel refuses a key or a paste its filter drops a character of, and keeps
    /// the selection, so the next key replaces it (EditControl.lua:140-152). The selection is
    /// the box's own, not guessed from the texts: a paste that starts or ends like the
    /// selected text is still ReplaceSel
    #[test]
    fn a_refused_edit_keeps_the_selection_in_the_box() {
        let text = |t: &str| vec![egui::Event::Text(t.into())];
        let paste = |t: &str| vec![egui::Event::Paste(t.into())];
        for (what, events) in [
            ("key", text("/")),
            ("paste b/", paste("b/")),
            ("paste bc/", paste("bc/")),
            ("paste bc/x", paste("bc/x")),
        ] {
            let (after, next, _) = name_box_run((1, 3), events);
            assert_eq!(after, "abcd", "{what}");
            assert_eq!(next, "axd", "{what}");
        }
        // a selection made right to left comes back the same way round
        let (_, _, kept) = name_box_run((3, 1), paste("bc/"));
        let kept = kept.expect("no selection");
        assert_eq!((kept.primary.index.0, kept.secondary.index.0), (1, 3));
    }

    #[test]
    fn sanitise_text_is_pobs() {
        // nothing to do without a high byte or a "<"
        assert_eq!(sanitise_text("Iron Ring\r\n+5"), "Iron Ring\r\n+5");
        assert_eq!(sanitise_text("a<b>c<d"), "ac<d");
        assert_eq!(sanitise_text("x<a<b>c>y"), "xy");
        assert_eq!(
            sanitise_text("10\u{2013}20 \u{2212}5 M\u{e4}r\u{f6}"),
            "10-20 -5 Maro"
        );
        // a character it doesn't know is "?" a byte; a lone 0x96 inside one becomes "-"
        assert_eq!(sanitise_text("\u{672c}"), "???");
        assert_eq!(sanitise_text("\u{416}"), "?-");
        assert_eq!(sanitise_text("\u{65e5}"), "?-?");
    }

    #[test]
    fn plain_keeps_ascii_text_only() {
        assert_eq!(filtered("Ab 1-_~\t日本é\n", Filter::Plain), "Ab 1-_~");
        assert!(has_visible("a"));
        assert!(!has_visible(&filtered("日本 é ", Filter::Plain)));
        assert!(!has_visible("  "));
    }

    #[test]
    fn path_drops_only_the_illegal_characters() {
        assert_eq!(
            filtered("a/b:c*d?e\"f<g>h|i\\j\tk日", Filter::Path),
            "abcdefghijk日"
        );
        // a non-ASCII character is visible to `%S`, and a Unicode space is not a Lua space
        assert!(has_visible("\u{3000}"));
        assert!(has_visible("\u{a0}"));
        assert!(!has_visible(" \t\u{b}"));
        assert!(!has_visible(&filtered("/:", Filter::Path)));
    }

    #[test]
    fn an_edit_filters_only_what_it_inserts() {
        let plain =
            |before: &str, raw: &str, pasted| spliced(before, raw, Filter::Plain, 100, pasted);
        for pasted in [false, true] {
            // a title the file holds is kept as it is; the inserted text is the only part checked
            assert_eq!(
                plain("\u{dc}n\u{ef}", "\u{dc}n\u{ef}x", pasted),
                "\u{dc}n\u{ef}x"
            );
            assert_eq!(
                plain("\u{dc}n\u{ef}", "\u{dc}n\u{ef}\u{7}", pasted),
                "\u{dc}n\u{ef}"
            );
            assert_eq!(plain("ab", "a\u{7}b", pasted), "ab");
            assert_eq!(plain("ab", "a\u{7}cb", pasted), "acb");
            // deleting never trips the limit
            let long = "x".repeat(120);
            assert_eq!(
                spliced(&long, &long[1..], Filter::Plain, 100, pasted).len(),
                119
            );
        }
    }

    #[test]
    fn typed_text_goes_in_key_by_key() {
        // over a selection the first accepted key replaces it, a rejected one leaves it
        assert_eq!(spliced("abc", "x5", Filter::Plain, 100, false), "x5");
        assert_eq!(spliced("abc", "\u{e9}5", Filter::Plain, 100, false), "5");
        assert_eq!(
            spliced("abc", "\u{e9}\u{e8}", Filter::Plain, 100, false),
            "abc"
        );
        // keys past the limit are refused one by one
        let full = "x".repeat(98);
        assert_eq!(
            spliced(&full, &format!("{full}yyyy"), Filter::Plain, 100, false).len(),
            100
        );
        assert_eq!(
            spliced("", &"x".repeat(130), Filter::Plain, 100, false).len(),
            100
        );
    }

    #[test]
    fn a_paste_is_refused_whole() {
        let full = "x".repeat(98);
        assert_eq!(
            spliced(&full, &format!("{full}yyyy"), Filter::Plain, 100, true),
            full
        );
        assert_eq!(spliced("", &"x".repeat(130), Filter::Plain, 100, true), "");
        // over a selection, one rejected character refuses the whole text; an insertion drops it
        assert_eq!(spliced("abc", "x\u{7}", Filter::Plain, 100, true), "abc");
        assert_eq!(spliced("abc", "xy", Filter::Plain, 100, true), "xy");
        assert_eq!(spliced("", "x\u{7}y", Filter::Plain, 100, true), "xy");
    }

    #[test]
    fn the_limit_counts_bytes() {
        let full = "\u{65e5}".repeat(33);
        assert_eq!(full.len(), 99);
        let more = format!("{full}\u{65e5}");
        // typed, a Path box keeps non-ASCII text up to 100 bytes
        assert_eq!(spliced("", &full, Filter::Path, NAME_LIMIT, false), full);
        assert_eq!(spliced(&full, &more, Filter::Path, NAME_LIMIT, false), full);
    }

    /// A box holding one of PoB's bytes as an escape (`bytes_text`) counts it as one byte, as
    /// PoB's limit does, and a paste of one turns it into one "?" (EditControl.lua:533)
    #[test]
    fn an_escaped_byte_counts_as_one() {
        let e = "\u{F7E9}";
        // 97 ASCII bytes and one escaped byte: 98 of 100
        let held = format!("{}{e}", "a".repeat(97));
        let typed = format!("{held}xy");
        assert_eq!(spliced(&held, &typed, Filter::Plain, 100, false), typed);
        let past = format!("{typed}z");
        assert_eq!(spliced(&typed, &past, Filter::Plain, 100, false), typed);
        // a paste: one "?" for the byte, and the limit counts PoB's bytes too
        assert_eq!(
            spliced("", &format!("x{e}"), Filter::Plain, 100, true),
            "x?"
        );
        assert_eq!(spliced(&held, &typed, Filter::Plain, 100, true), typed);
    }

    /// A long insert that isn't a paste (egui's own undo bringing back a cleared text, an IME
    /// commit) is counted once, not once per character
    #[test]
    fn a_long_typed_insert_is_linear() {
        let long = "a".repeat(40_000);
        let start = std::time::Instant::now();
        assert_eq!(spliced_by("", &long, |_| true, usize::MAX, false), long);
        let limited = format!("{}{}", "\u{F7E9}".repeat(10), long);
        assert_eq!(spliced_by("", &limited, |_| true, 50_000, false), limited);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_paste_turns_non_ascii_bytes_into_question_marks() {
        // Plain keeps "?": three bytes of a character make three marks
        assert_eq!(spliced("", "Caf\u{e9}", Filter::Plain, 100, true), "Caf??");
        // Path drops "?": the mark vanishes at the caret and refuses a paste over a selection
        assert_eq!(spliced("", "x\u{65e5}y", Filter::Path, 100, true), "xy");
        assert_eq!(spliced("abc", "x\u{65e5}y", Filter::Path, 100, true), "abc");
        let full = "\u{65e5}".repeat(33);
        assert_eq!(spliced("", &full, Filter::Path, NAME_LIMIT, true), "");
    }
}
