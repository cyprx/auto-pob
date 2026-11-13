//! The views show each byte PoB keeps that isn't UTF-8 as a private-use character
//! (`engine::bytes_text`, U+F780-U+F7FF), which no font has. Each default font gets a twin
//! whose only character map sends those characters to the glyph of the byte's Windows-1252
//! reading (0xE9 is "é", 0x92 is "’"; the five bytes Windows-1252 leaves undefined get U+FFFD's
//! glyph, or "?"). The twin follows its font in every family, so the text keeps its own
//! characters (one glyph each: the caret, the selection and the limits are untouched) and
//! reads as the Windows-1252 editor that wrote the bytes meant.

use crate::engine::bytes_text::{escapes, windows_1252};

/// `data` (a TrueType or OpenType font) with its `cmap` replaced by one mapping the escape
/// characters to the glyphs of their Windows-1252 readings; `None` for a font that can't be
/// read or has none of those glyphs
pub fn byte_twin(data: &[u8]) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let fallback = face
        .glyph_index('\u{FFFD}')
        .or_else(|| face.glyph_index('?'))?;
    let groups: Vec<(u32, u16)> = escapes()
        .map(|(c, b)| {
            let glyph = face.glyph_index(windows_1252(b)).unwrap_or(fallback);
            (c as u32, glyph.0)
        })
        .collect();
    replace_cmap(data, &cmap_format12(&groups))
}

/// A `cmap` table with one Windows Unicode (3, 10) format 12 subtable, one group per character
fn cmap_format12(groups: &[(u32, u16)]) -> Vec<u8> {
    let mut sub = Vec::new();
    let length = 16 + 12 * groups.len() as u32;
    sub.extend(12u16.to_be_bytes());
    sub.extend(0u16.to_be_bytes());
    sub.extend(length.to_be_bytes());
    sub.extend(0u32.to_be_bytes());
    sub.extend((groups.len() as u32).to_be_bytes());
    for (code, glyph) in groups {
        sub.extend(code.to_be_bytes());
        sub.extend(code.to_be_bytes());
        sub.extend(u32::from(*glyph).to_be_bytes());
    }
    let mut table = Vec::new();
    table.extend(0u16.to_be_bytes()); // version
    table.extend(1u16.to_be_bytes()); // one subtable
    table.extend(3u16.to_be_bytes()); // Windows
    table.extend(10u16.to_be_bytes()); // Unicode full repertoire
    table.extend(12u32.to_be_bytes()); // its offset
    table.extend(sub);
    table
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The font file `data` rebuilt with `cmap` in place of its own character map
fn replace_cmap(data: &[u8], cmap: &[u8]) -> Option<Vec<u8>> {
    let count = usize::from(be_u16(data, 4)?);
    let mut tables: Vec<([u8; 4], &[u8])> = Vec::with_capacity(count);
    for i in 0..count {
        let record = 12 + 16 * i;
        let tag: [u8; 4] = data.get(record..record + 4)?.try_into().ok()?;
        let offset = be_u32(data, record + 8)? as usize;
        let length = be_u32(data, record + 12)? as usize;
        let body = if &tag == b"cmap" {
            cmap
        } else {
            data.get(offset..offset + length)?
        };
        tables.push((tag, body));
    }
    tables.sort_by_key(|(tag, _)| *tag);
    let mut out = Vec::with_capacity(data.len() + cmap.len());
    out.extend_from_slice(data.get(0..12)?);
    let mut offset = 12 + 16 * count;
    let mut bodies = Vec::new();
    for (tag, body) in &tables {
        out.extend_from_slice(tag);
        out.extend(checksum(body).to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((body.len() as u32).to_be_bytes());
        let padded = body.len().div_ceil(4) * 4;
        bodies.push((*body, padded - body.len()));
        offset += padded;
    }
    for (body, pad) in bodies {
        out.extend_from_slice(body);
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    Some(out)
}

fn checksum(body: &[u8]) -> u32 {
    body.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn the_twin_maps_each_escape_to_its_windows_1252_glyph() {
        let defaults = egui::FontDefinitions::default();
        for name in ["Ubuntu-Light", "Hack"] {
            let data: &[u8] = (*defaults.font_data[name].font).as_ref();
            let twin = byte_twin(data).expect("a twin");
            let face = ttf_parser::Face::parse(data, 0).unwrap();
            let twin_face = ttf_parser::Face::parse(&twin, 0).unwrap();
            assert_eq!(
                twin_face.glyph_index('\u{F7E9}'),
                face.glyph_index('\u{E9}'),
                "{name}: é"
            );
            assert_eq!(
                twin_face.glyph_index('\u{F792}'),
                face.glyph_index('\u{2019}'),
                "{name}: ’"
            );
            // the twin has nothing else
            assert_eq!(twin_face.glyph_index('a'), None);
        }
    }
}
