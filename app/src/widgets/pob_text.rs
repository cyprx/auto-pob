//! PoB's inline colour codes (`^0`-`^9`, `^xRRGGBB`) as an egui layout job.
//!
//! Matches the classic app's `PoBString`: only `^<digit>` and `^x`/`^X` followed by exactly
//! six hex digits are codes; anything else (`^xZZ1234`, a trailing `^`) stays literal text.
//! Unlike the classic parser, empty sections (e.g. between adjacent codes) are not emitted.

use eframe::egui::{Color32, FontId, text::LayoutJob, text::TextFormat};

/// The colour of `^0`-`^9` (PoB's `colorCodes`).
fn digit_color(d: u8) -> Color32 {
    match d {
        b'0' => Color32::from_rgb(0, 0, 0),
        b'1' => Color32::from_rgb(255, 0, 0),
        b'2' => Color32::from_rgb(0, 255, 0),
        b'3' => Color32::from_rgb(0, 0, 255),
        b'4' => Color32::from_rgb(255, 255, 0),
        b'5' => Color32::from_rgb(255, 0, 255),
        b'6' => Color32::from_rgb(0, 255, 255),
        b'7' => Color32::from_rgb(255, 255, 255),
        b'8' => Color32::from_rgb(178, 178, 178),
        _ => Color32::from_rgb(102, 102, 102),
    }
}

/// Parses a colour code at the start of `s` (which begins with `^`); returns it and its length.
fn parse_code(s: &[u8]) -> Option<(Color32, usize)> {
    match s {
        [b'^', d @ b'0'..=b'9', ..] => Some((digit_color(*d), 2)),
        [b'^', b'x' | b'X', hex @ ..]
            if hex.len() >= 6 && hex[..6].iter().all(u8::is_ascii_hexdigit) =>
        {
            let byte = |i: usize| {
                u8::from_str_radix(std::str::from_utf8(&hex[i..i + 2]).unwrap(), 16).unwrap()
            };
            Some((Color32::from_rgb(byte(0), byte(2), byte(4)), 8))
        }
        _ => None,
    }
}

/// Lays out `text`, switching colour at each PoB colour code. `default` applies until the
/// first code.
pub fn pob_text(text: &str, size: f32, default: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    append_pob_text(&mut job, text, &FontId::proportional(size), default);
    job
}

/// Appends `text` to `job` in `font`, switching colour at each PoB colour code; `default`
/// applies until the first code. Returns the colour in force at the end.
pub fn append_pob_text(
    job: &mut LayoutJob,
    text: &str,
    font: &FontId,
    default: Color32,
) -> Color32 {
    let bytes = text.as_bytes();
    let mut color = default;
    let mut start = 0;
    let mut i = 0;
    let flush = |job: &mut LayoutJob, start: usize, end: usize, color: Color32| {
        if end > start {
            job.append(
                &text[start..end],
                0.0,
                TextFormat::simple(font.clone(), color),
            );
        }
    };
    while i < bytes.len() {
        if bytes[i] == b'^'
            && let Some((new_color, len)) = parse_code(&bytes[i..])
        {
            flush(job, start, i, color);
            color = new_color;
            i += len;
            start = i;
        } else {
            i += 1;
        }
    }
    flush(job, start, bytes.len(), color);
    color
}

/// PoB's `StripEscapes` (HeadlessWrapper.lua:75-77): drops every `^<digit>`, then every
/// `^x` and six hex digits (lowercase "x" only, unlike the drawing parser)
pub fn strip_escapes(text: &str) -> String {
    let pass = |text: &str, len: usize, is_code: &dyn Fn(&[u8]) -> bool| {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'^' && bytes.len() - i >= len && is_code(&bytes[i + 1..i + len]) {
                i += len;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        // only ASCII bytes go, so the rest stays UTF-8
        String::from_utf8(out).unwrap_or_default()
    };
    let digits = pass(text, 2, &|c| c[0].is_ascii_digit());
    pass(&digits, 8, &|c| {
        c[0] == b'x' && c[1..].iter().all(u8::is_ascii_hexdigit)
    })
}

/// The colour a PoB code such as `^xAABBCC` selects; `default` for anything else.
pub fn pob_colour(code: &str, default: Color32) -> Color32 {
    let job = pob_text(&format!("{code}x"), 14.0, default);
    job.sections.first().map_or(default, |s| s.format.color)
}

/// PoB's `fitItemName` (TradeHelpers.lua:436-452): `colour .. name` when it `fits`, else the
/// longest start of `name` that fits with "..." after it. The cut never splits a character or
/// a colour code inside `name`.
pub fn fit_with_dots(colour: &str, name: &str, fits: impl Fn(&str) -> bool) -> String {
    let full = format!("{colour}{name}");
    if fits(&full) {
        return full;
    }
    // where a cut may fall: a character boundary outside a colour code
    let bytes = name.as_bytes();
    let mut cuts = vec![0];
    let mut i = 0;
    while i < bytes.len() {
        let step = match parse_code(&bytes[i..]) {
            Some((_, len)) if bytes[i] == b'^' => len,
            _ => name[i..].chars().next().map_or(1, char::len_utf8),
        };
        i += step;
        cuts.push(i);
    }
    let cut = |n: usize| format!("{colour}{}...", &name[..cuts[n]]);
    // the longest start that fits, as PoB's binary search finds it
    let (mut lo, mut hi) = (0, cuts.len() - 1);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cut(lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_cuts_with_dots_and_never_inside_a_code() {
        let short = |n: usize| move |t: &str| t.chars().count() <= n;
        assert_eq!(fit_with_dots("^7", "Ring", short(6)), "^7Ring");
        assert_eq!(
            fit_with_dots("^xAF6025", "Kaom's Heart", short(15)),
            "^xAF6025Kaom..."
        );
        // a code is kept whole or left out
        let name = "Jewel^xFF9922  (tree missing allocated node)";
        assert_eq!(fit_with_dots("^7", name, short(12)), "^7Jewel...");
        assert_eq!(fit_with_dots("^7", name, short(20)), "^7Jewel^xFF9922  ...");
        // nothing fits: the dots alone, as PoB
        assert_eq!(fit_with_dots("^7", "Ring", short(1)), "^7...");
        // characters stay whole
        assert_eq!(fit_with_dots("", "Ünïque name", short(5)), "Ün...");
    }

    const DEFAULT: Color32 = Color32::from_rgb(1, 2, 3);

    fn sections(text: &str) -> Vec<(String, Color32)> {
        let job = pob_text(text, 14.0, DEFAULT);
        job.sections
            .iter()
            .map(|s| {
                let range = usize::from(s.byte_range.start)..usize::from(s.byte_range.end);
                (job.text[range].to_string(), s.format.color)
            })
            .collect()
    }

    #[test]
    fn strip_escapes_is_pobs() {
        assert_eq!(
            strip_escapes("^1+5 to ^xAABBCCStr^XAABBCC"),
            "+5 to Str^XAABBCC"
        );
        assert_eq!(strip_escapes("^^12"), "^2");
        assert_eq!(strip_escapes("caf\u{e9}^7!^"), "caf\u{e9}!^");
    }

    #[test]
    fn plain_text_is_one_section() {
        assert_eq!(sections("abc"), vec![("abc".into(), DEFAULT)]);
    }

    #[test]
    fn digit_codes_switch_colour() {
        assert_eq!(
            sections("^1red^7white"),
            vec![
                ("red".into(), Color32::from_rgb(255, 0, 0)),
                ("white".into(), Color32::from_rgb(255, 255, 255)),
            ]
        );
    }

    #[test]
    fn hex_codes_switch_colour() {
        assert_eq!(
            sections("^xE05030Life"),
            vec![("Life".into(), Color32::from_rgb(224, 80, 48))]
        );
    }

    #[test]
    fn adjacent_codes_last_wins() {
        assert_eq!(
            sections("^1^2x"),
            vec![("x".into(), Color32::from_rgb(0, 255, 0))]
        );
    }

    #[test]
    fn malformed_codes_are_literal() {
        assert_eq!(sections("^xZZ1234a"), vec![("^xZZ1234a".into(), DEFAULT)]);
        assert_eq!(sections("50^"), vec![("50^".into(), DEFAULT)]);
    }
}
