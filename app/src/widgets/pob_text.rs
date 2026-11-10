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
    let bytes = text.as_bytes();
    let mut color = default;
    let mut start = 0;
    let mut i = 0;
    let flush = |job: &mut LayoutJob, start: usize, end: usize, color: Color32| {
        if end > start {
            job.append(
                &text[start..end],
                0.0,
                TextFormat::simple(FontId::proportional(size), color),
            );
        }
    };
    while i < bytes.len() {
        if bytes[i] == b'^'
            && let Some((new_color, len)) = parse_code(&bytes[i..])
        {
            flush(&mut job, start, i, color);
            color = new_color;
            i += len;
            start = i;
        } else {
            i += 1;
        }
    }
    flush(&mut job, start, bytes.len(), color);
    job
}

#[cfg(test)]
mod tests {
    use super::*;

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
