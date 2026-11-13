//! PoB's name boxes: the character filter and length limit of its `EditControl`, and the
//! "has a visible character" test its Save buttons make.
//!
//! A box made without a filter takes `^%w%p ` under LuaJIT (`main.unicode` is off), so it keeps
//! only ASCII letters, digits, punctuation and spaces (EditControl.lua:46). The boxes for build
//! and folder names pass `\/:%*%?"<>|%c` instead, which drops those characters and leaves every
//! other byte alone (BuildListControl.lua:90, Main.lua:1696, Build.lua:1399). The limit counts
//! bytes (EditControl.lua:148). PoB never trims: the Save buttons ask for `%S` and save the box as
//! it stands.

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
    spliced_with(before, raw, keeps, limit, pasted, None)
}

/// `spliced_by` for a box with a `pasteFilter`, which a paste goes through before anything
/// else (EditControl.lua:530)
pub fn spliced_with(
    before: &str,
    raw: &str,
    keeps: impl Fn(char) -> bool,
    limit: usize,
    pasted: bool,
    paste_filter: Option<fn(&str) -> String>,
) -> String {
    // the edit replaced `before[head..before.len() - tail]` with `raw[head..raw.len() - tail]`
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
        // (EditControl.lua:533)
        let swapped: String = inserted
            .chars()
            .flat_map(|c| {
                let n = if c.is_ascii() { 1 } else { c.len_utf8() };
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
        for c in inserted.chars() {
            if !keeps(c) {
                continue;
            }
            let held = if replacing { 0 } else { out.len() };
            if front.len() + held + c.len_utf8() + back.len() > limit {
                continue;
            }
            out.push(c);
            replacing = false;
        }
        if replacing {
            return before.to_owned();
        }
    }
    let result = format!("{front}{out}{back}");
    if pasted && result.len() > limit {
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
    let response = ui.add(build(egui::TextEdit::singleline(text)));
    crate::widgets::url_click::open_on_ctrl_click(ui, &response, text);
    crate::widgets::edit_box::select_all_on_double_click(ui.ctx(), &response, text);
    let mut changed = response.changed();
    if changed {
        let same = *text == before;
        *text = spliced(&before, text, filter, limit, pasted);
        // typing over the text with itself is an edit; a character the box drops is none
        changed = same || *text != before;
    }
    (response, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

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
