//! PoB's strings are bytes. A build saved by a Windows-1252 editor keeps bytes that aren't
//! UTF-8 in its notes, labels and items; PoB shows and saves them as they are.
//!
//! The views are Rust strings, so each such byte is shown as one private-use character,
//! U+F780 + (byte - 0x80), and turned back into the byte when the text comes back to PoB
//! (`encode`, in `Pob::call`). An edit to a field holding such bytes so keeps PoB's bytes on
//! save.
//!
//! The mapping is a bijection: a string that legitimately holds one of these characters
//! has the character's own UTF-8 bytes escaped too when it is decoded. Text coming back is
//! only turned into bytes once some text was decoded with an escape (`seen`), so a build with
//! no such bytes sends its private-use characters as they are.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

const FIRST: u32 = 0xF780;
const LAST: u32 = 0xF7FF;

static SEEN: AtomicBool = AtomicBool::new(false);

/// The character standing for the byte `b` (0x80..=0xFF)
fn escape(b: u8) -> char {
    char::from_u32(FIRST + u32::from(b) - 0x80).expect("a private-use character")
}

/// Whether `c` stands for one of PoB's bytes
pub fn is_escape(c: char) -> bool {
    (FIRST..=LAST).contains(&(c as u32))
}

/// The bytes `c` is in PoB's string: one for an escape
pub fn pob_char_len(c: char) -> usize {
    if is_escape(c) { 1 } else { c.len_utf8() }
}

/// The bytes `text` is in PoB's string, as its limits count them: an escape is one byte
pub fn pob_len(text: &str) -> usize {
    text.len() - 2 * text.chars().filter(|c| is_escape(*c)).count()
}

/// The UTF-8 of an escape character starts EF 9E or EF 9F
pub fn has_escape_bytes(bytes: &[u8]) -> bool {
    bytes
        .windows(2)
        .any(|w| w[0] == 0xEF && (w[1] == 0x9E || w[1] == 0x9F))
}

/// Whether `decode` changes these bytes
pub fn needs_decode(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err() || has_escape_bytes(bytes)
}

/// PoB's bytes as text: valid UTF-8 as it is, every other byte as its escape character
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if !needs_decode(bytes) {
        // checked above
        return Cow::Borrowed(std::str::from_utf8(bytes).unwrap_or_default());
    }
    let mut out = String::with_capacity(bytes.len() + 8);
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            if is_escape(c) {
                let mut buf = [0u8; 4];
                out.extend(c.encode_utf8(&mut buf).bytes().map(escape));
            } else {
                out.push(c);
            }
        }
        out.extend(chunk.invalid().iter().copied().map(escape));
    }
    SEEN.store(true, Ordering::Relaxed);
    Cow::Owned(out)
}

/// Text from the UI as PoB's bytes: each escape character back to its byte. Only once some
/// text was decoded with escapes; until then the text's own UTF-8.
pub fn encode(text: &str) -> Cow<'_, [u8]> {
    if !SEEN.load(Ordering::Relaxed) || !text.chars().any(is_escape) {
        return Cow::Borrowed(text.as_bytes());
    }
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        if is_escape(c) {
            out.push((c as u32 - FIRST + 0x80) as u8);
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    Cow::Owned(out)
}

/// `encode` applied to every string of `value`, nested tables included (a command's
/// arguments), in place; tables are walked to a small depth
pub fn encode_value(
    lua: &mlua::Lua,
    value: mlua::Value,
    depth: usize,
) -> mlua::Result<mlua::Value> {
    match value {
        mlua::Value::String(text) => {
            let fixed = match text.to_str() {
                Ok(s) => match encode(&s) {
                    Cow::Borrowed(_) => None,
                    Cow::Owned(bytes) => Some(bytes),
                },
                Err(_) => None,
            };
            match fixed {
                Some(bytes) => Ok(mlua::Value::String(lua.create_string(bytes)?)),
                None => Ok(mlua::Value::String(text)),
            }
        }
        mlua::Value::Table(table) if depth < 16 => {
            let pairs = table
                .pairs::<mlua::Value, mlua::Value>()
                .collect::<mlua::Result<Vec<_>>>()?;
            for (key, item) in pairs {
                if matches!(item, mlua::Value::String(_) | mlua::Value::Table(_)) {
                    let item = encode_value(lua, item, depth + 1)?;
                    table.raw_set(key, item)?;
                }
            }
            Ok(mlua::Value::Table(table))
        }
        other => Ok(other),
    }
}

/// Whether any text was decoded with escapes (tests)
pub fn seen() -> bool {
    SEEN.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_that_are_not_utf8_come_back_as_they_were() {
        let raw = b"Caf\xE9 \xFF\x80 ok \xC3";
        let text = decode(raw);
        assert_eq!(text.chars().filter(|c| is_escape(*c)).count(), 4);
        assert!(text.starts_with("Caf"));
        assert_eq!(&*encode(&text), raw);
        // an edit around the escapes keeps them
        let edited = text.replace("ok", "edited");
        assert_eq!(&*encode(&edited), b"Caf\xE9 \xFF\x80 edited \xC3");
    }

    #[test]
    fn valid_text_is_unchanged_and_an_escape_character_of_its_own_round_trips() {
        assert!(matches!(decode("café".as_bytes()), Cow::Borrowed("café")));
        // a string holding U+F7E9 itself, beside a bad byte: both survive the round trip
        let mut raw = "x\u{F7E9}y".as_bytes().to_vec();
        raw.push(0xE9);
        let text = decode(&raw);
        let want: String = ["x", "\u{F7EF}\u{F79F}\u{F7A9}", "y", "\u{F7E9}"].concat();
        assert_eq!(text, want);
        assert_eq!(&*encode(&text), &raw[..]);
    }
}
