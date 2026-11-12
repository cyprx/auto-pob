//! PoB's section layout for the Calcs tab: `CalcsTab:Draw` places the sections and
//! `CalcSectionControl:UpdateSize` gives their heights. Both are pure here.

use crate::engine::views::CalcsSection;

/// A section column's width (`CalcsTab.colWidth`)
pub const COL_WIDTH: f32 = 230.0;
/// The gap between sections, and between columns
pub const GAP: f32 = 8.0;
/// The room `CalcsTab:Draw` leaves above the first section for the search box (PoB's baseY).
pub const TOP: f32 = 30.0;

/// How many columns fit in `width` points.
pub fn max_cols(width: f32) -> usize {
    (width / (COL_WIDTH + GAP)).floor().max(0.0) as usize
}

/// A section's width in points.
pub fn section_width(width_cols: u32) -> f32 {
    let cols = width_cols.max(1) as f32;
    cols * COL_WIDTH + GAP * (cols - 1.0)
}

/// How far each sub-section advances `UpdateSize`'s running height, in order: 22 plus 18 a row
/// when it is open, 20 when it is collapsed. An open sub-section counts as collapsed while no
/// row has been seen yet in the section (`self.enabled` there is cumulative).
fn sub_advances(section: &CalcsSection) -> Vec<f32> {
    let mut any_rows = false;
    section
        .subs
        .iter()
        .map(|sub| {
            let rows = if sub.collapsed { 0 } else { sub.rows.len() };
            any_rows |= rows > 0;
            if any_rows && !sub.collapsed {
                22.0 + 18.0 * rows as f32
            } else {
                20.0
            }
        })
        .collect()
}

/// The vertical offset of each sub-section from the section's top, and the section's height.
pub fn sub_offsets(section: &CalcsSection) -> (Vec<f32>, f32) {
    let advances = sub_advances(section);
    let mut y = 0.0;
    let offsets = advances
        .iter()
        .map(|a| {
            let at = y;
            y += a;
            at
        })
        .collect();
    (offsets, section_height(section))
}

/// `CalcSectionControl:UpdateSize`'s height.
pub fn section_height(section: &CalcsSection) -> f32 {
    if !section.enabled || section.subs.first().is_none_or(|s| s.collapsed) {
        return 22.0;
    }
    2.0 + sub_advances(section).iter().sum::<f32>()
}

/// `CalcsTab:Draw`'s placement of the sections, as `(group, width in columns, height)` in
/// order. Gives each section its 0-based column and its top relative to the first section's.
/// `None` for a section with a group PoB doesn't know.
pub fn arrange(sections: &[(u32, u32, f32)], max_cols: usize) -> Vec<Option<(usize, f32)>> {
    arrange_in(sections, max_cols, 0.0)
}

/// [`arrange`] with the height the viewport has below the first section, which decides whether
/// the overflowing group 3 sections still fit under column 3.
pub fn arrange_in(
    sections: &[(u32, u32, f32)],
    max_cols: usize,
    view_height: f32,
) -> Vec<Option<(usize, f32)>> {
    let mut col_y: Vec<f32> = vec![0.0; 5.max(max_cols)];
    let mut max_y = 0.0f32;
    let mut out: Vec<Option<(usize, f32)>> = vec![None; sections.len()];
    for (i, &(group, width, height)) in sections.iter().enumerate() {
        let width = width.max(1) as usize;
        let col = match group {
            1 => {
                let mut col = 0;
                if width == 1 {
                    // the highest of the first three columns, the leftmost on a tie
                    for c in 1..3 {
                        if col_y[c] < col_y[col] {
                            col = c;
                        }
                    }
                } else {
                    let low = col_y[..3].iter().copied().fold(0.0, f32::max);
                    col_y[0] = low;
                }
                Some(col)
            }
            2 => (max_cols >= 4).then_some(3),
            3 => (max_cols >= 5).then_some(4),
            _ => None,
        };
        if let Some(col) = col {
            let y = col_y[col];
            out[i] = Some((col, y));
            let end = (col + width).min(col_y.len());
            col_y[col..end].fill(y + height + GAP);
            max_y = max_y.max(col_y[col]);
        }
    }
    if max_cols < 5 {
        // No room for a 5th column: group 3 goes under column 3, or into the highest of 1-3
        let low = col_y[0].max(col_y[1]).max(col_y[2]);
        col_y[..3].fill(low);
        for (i, &(group, _, height)) in sections.iter().enumerate() {
            // PoB's portrait layout (fewer than 4 columns) also sends group 2 here
            if !(group == 3 || (group == 2 && max_cols < 4)) {
                continue;
            }
            let mut col = 2;
            if col_y[col] + height + 4.0 >= view_height.max(max_y) {
                for c in (0..3).rev() {
                    if col_y[c] < col_y[col] {
                        col = c;
                    }
                }
            }
            let y = col_y[col];
            out[i] = Some((col, y));
            col_y[col] = y + height + GAP;
            max_y = max_y.max(col_y[col]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::views::{CalcsRow, CalcsSub};

    const SECTIONS: [(u32, u32, f32); 6] = [
        (1, 1, 100.0),
        (1, 1, 50.0),
        (1, 1, 70.0),
        (1, 1, 40.0),
        (2, 1, 300.0),
        (3, 1, 200.0),
    ];

    fn cols(out: &[Option<(usize, f32)>]) -> Vec<usize> {
        out.iter().map(|p| p.unwrap().0).collect()
    }

    fn ys(out: &[Option<(usize, f32)>]) -> Vec<f32> {
        out.iter().map(|p| p.unwrap().1).collect()
    }

    #[test]
    fn arrange_matches_pob_five_columns() {
        let out = arrange(&SECTIONS, 5);
        assert_eq!(cols(&out), [0, 1, 2, 1, 3, 4]);
        assert_eq!(ys(&out), [0.0, 0.0, 0.0, 58.0, 0.0, 0.0]);
    }

    #[test]
    fn arrange_overflows_group3_without_fifth_column() {
        let out = arrange(&SECTIONS, 4);
        assert_eq!(cols(&out), [0, 1, 2, 1, 3, 2]);
        // column 0 is the tallest of 0-2 at 100 + 8; the others are raised to it
        assert_eq!(ys(&out)[5], 108.0);
    }

    #[test]
    fn group3_takes_the_highest_column_when_column_3_has_no_room() {
        // no room under column 3 (not even for the first), so the highest of columns 1-3 is used
        let sections = [(1, 1, 100.0), (1, 1, 10.0), (3, 1, 20.0), (3, 1, 20.0)];
        let out = arrange(&sections, 3);
        assert_eq!(cols(&out), [0, 1, 2, 1]);
        assert_eq!(ys(&out)[2..], [108.0, 108.0]);
    }

    #[test]
    fn narrow_layout_keeps_defence_sections() {
        let out = arrange(&SECTIONS, 3);
        assert!(out.iter().all(Option::is_some), "{out:?}");
        assert!(cols(&out).iter().all(|&c| c < 3));
    }

    #[test]
    fn group3_stays_under_column_3_when_it_fits() {
        let sections = [(1, 1, 100.0), (1, 1, 10.0), (3, 1, 20.0), (3, 1, 20.0)];
        let out = arrange_in(&sections, 3, 1000.0);
        assert_eq!(cols(&out), [0, 1, 2, 2]);
        assert_eq!(ys(&out)[2..], [108.0, 136.0]);
    }

    #[test]
    fn three_wide_section_aligns_columns() {
        let sections = [(1, 1, 100.0), (1, 1, 50.0), (1, 3, 40.0), (1, 1, 10.0)];
        let out = arrange(&sections, 5);
        // the wide one starts under the lowest of the three, and all three then continue below it
        assert_eq!(out[2], Some((0, 108.0)));
        assert_eq!(out[3], Some((0, 156.0)));
    }

    fn sub(collapsed: bool, rows: usize) -> CalcsSub {
        CalcsSub {
            label: "s".into(),
            extra: None,
            collapsed,
            search_match: false,
            rows: (0..rows)
                .map(|index| CalcsRow {
                    index,
                    label: None,
                    colour: None,
                    text_size: 14,
                    search_match: false,
                    cells: vec![],
                })
                .collect(),
        }
    }

    fn section(enabled: bool, subs: Vec<CalcsSub>) -> CalcsSection {
        CalcsSection {
            id: "x".into(),
            group: 1,
            width_cols: 1,
            colour: "^xFFFFFF".into(),
            enabled,
            subs,
            col_widths: vec![],
            pop_out: false,
        }
    }

    #[test]
    fn section_height_matches_pob() {
        let open = section(true, vec![sub(false, 3), sub(false, 2)]);
        assert_eq!(section_height(&open), 2.0 + (54.0 + 22.0) + (36.0 + 22.0));
        let (offsets, height) = sub_offsets(&open);
        assert_eq!(offsets, [0.0, 76.0]);
        assert_eq!(height, 136.0);

        let first_collapsed = section(true, vec![sub(true, 0)]);
        assert_eq!(section_height(&first_collapsed), 22.0);
        assert_eq!(section_height(&section(false, vec![sub(false, 0)])), 22.0);

        // a later collapsed sub-section is 20
        let later = section(true, vec![sub(false, 1), sub(true, 0)]);
        assert_eq!(section_height(&later), 2.0 + 40.0 + 20.0);
    }
}
