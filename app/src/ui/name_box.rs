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

/// `name` with each character the path filter drops turned into "-", as Save As prefills
/// (`gsub("[\\/:%*%?\"<>|%c]", "-")`)
pub fn dashed(name: &str) -> String {
    name.chars()
        .map(|c| if Filter::Path.keeps(c) { c } else { '-' })
        .collect()
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

/// Applies the filter, then the byte limit, to `text` in place
pub fn clean(text: &mut String, filter: Filter, limit: usize) {
    *text = spliced("", text, filter, limit);
}

/// What PoB's box holds after the edit that turned `before` into `raw`. PoB filters and limits
/// only the text an edit inserts (`Insert`, EditControl.lua:162-172): the text already in the
/// box, such as a name being renamed, is kept as it is, non-ASCII bytes included.
pub fn spliced(before: &str, raw: &str, filter: Filter, limit: usize) -> String {
    // the edit replaced `raw[head..raw.len() - tail]`
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
    let mut out = String::from(front);
    // typed one by one, as PoB takes it: a character that no longer fits is refused
    let room = limit.saturating_sub(front.len() + back.len());
    let mut used = 0;
    for c in inserted.chars().filter(|c| filter.keeps(*c)) {
        if used + c.len_utf8() > room {
            break;
        }
        used += c.len_utf8();
        out.push(c);
    }
    out.push_str(back);
    out
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
    let response = ui.add(build(egui::TextEdit::singleline(text)));
    let mut changed = response.changed();
    if changed {
        let same = *text == before;
        *text = spliced(&before, text, filter, limit);
        // typing over the text with itself is an edit; a character the box drops is none
        changed = same || *text != before;
    }
    (response, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn save_as_prefill_dashes_the_forbidden_characters() {
        assert_eq!(dashed("a:b/c\u{7}\u{65e5}"), "a-b-c-\u{65e5}");
    }

    #[test]
    fn an_edit_filters_only_what_it_inserts() {
        // a title the file holds is kept as it is; the typed character is the only one checked
        assert_eq!(
            spliced("\u{dc}n\u{ef}", "\u{dc}n\u{ef}x", Filter::Plain, 100),
            "\u{dc}n\u{ef}x"
        );
        assert_eq!(
            spliced("\u{dc}n\u{ef}", "\u{dc}n\u{ef}\u{e9}", Filter::Plain, 100),
            "\u{dc}n\u{ef}"
        );
        assert_eq!(spliced("ab", "a\u{e9}b", Filter::Plain, 100), "ab");
        // deleting never trips the limit, an insertion past it is cut
        assert_eq!(
            spliced(&"x".repeat(120), &"x".repeat(119), Filter::Plain, 100).len(),
            119
        );
        assert_eq!(
            spliced(&"x".repeat(100), &"x".repeat(101), Filter::Plain, 100).len(),
            100
        );
    }

    #[test]
    fn the_limit_counts_bytes() {
        let mut text = "x".repeat(130);
        clean(&mut text, Filter::Path, NAME_LIMIT);
        assert_eq!(text.len(), 100);
        let mut text = "日".repeat(40);
        clean(&mut text, Filter::Path, NAME_LIMIT);
        assert_eq!(text.chars().count(), 33);
    }
}
