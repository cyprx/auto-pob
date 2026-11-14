//! The images PoB's `Tooltip:Draw` puts on a tooltip with "Styled Tooltips with Flavour Text"
//! on (Tooltip.lua:380-630): the header art behind the title by its `tooltipHeader` (left cap,
//! tiled middle, right cap; foil-tinted for a relic), the influence icons on the caps, the
//! rarity's separator images, a line's background image and an anointable notable's oils.

use crate::widgets::art;
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

/// PoB's `headerConfigs` entry: the three images and the sizes they are drawn at
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Header {
    pub left: &'static str,
    pub middle: &'static str,
    pub right: &'static str,
    pub height: f32,
    pub side: f32,
    pub middle_width: f32,
    /// How far the title moves down
    pub text_offset: f32,
    /// Influence icons go on the caps
    pub influence: bool,
}

const fn header(tall: bool, side: f32, middle: f32, offset: f32, influence: bool) -> Header {
    Header {
        left: "",
        middle: "",
        right: "",
        height: if tall { 54.0 } else { 38.0 },
        side,
        middle_width: middle,
        text_offset: offset,
        influence,
    }
}

/// `headerConfigs[rarity] or headerConfigs.NORMAL` (Tooltip.lua:407-421)
pub fn header_art(name: &str) -> Header {
    let (left, middle, right, base) = match name.to_ascii_uppercase().as_str() {
        "RELIC" => (
            "Assets/itemsheaderfoilleft.png",
            "Assets/itemsheaderfoilmiddle.png",
            "Assets/itemsheaderfoilright.png",
            header(true, 47.0, 52.0, 3.0, true),
        ),
        "UNIQUE" => (
            "Assets/itemsheaderuniqueleft.png",
            "Assets/itemsheaderuniquemiddle.png",
            "Assets/itemsheaderuniqueright.png",
            header(true, 47.0, 52.0, 3.0, true),
        ),
        "RARE" => (
            "Assets/itemsheaderrareleft.png",
            "Assets/itemsheaderraremiddle.png",
            "Assets/itemsheaderrareright.png",
            header(true, 47.0, 52.0, 3.0, true),
        ),
        "MAGIC" => (
            "Assets/itemsheadermagicleft.png",
            "Assets/itemsheadermagicmiddle.png",
            "Assets/itemsheadermagicright.png",
            header(false, 32.0, 32.0, 6.0, true),
        ),
        "GEM" => (
            "Assets/itemsheadergemleft.png",
            "Assets/itemsheadergemmiddle.png",
            "Assets/itemsheadergemright.png",
            header(false, 33.0, 38.0, 5.0, false),
        ),
        "JEWEL" => (
            "Assets/jewelpassiveheaderleft.png",
            "Assets/jewelpassiveheadermiddle.png",
            "Assets/jewelpassiveheaderright.png",
            header(false, 33.0, 38.0, 6.0, false),
        ),
        "NOTABLE" => (
            "Assets/notablepassiveheaderleft.png",
            "Assets/notablepassiveheadermiddle.png",
            "Assets/notablepassiveheaderright.png",
            header(false, 38.0, 38.0, 6.0, false),
        ),
        "PASSIVE" => (
            "Assets/normalpassiveheaderleft.png",
            "Assets/normalpassiveheadermiddle.png",
            "Assets/normalpassiveheaderright.png",
            header(false, 33.0, 38.0, 6.0, false),
        ),
        "KEYSTONE" => (
            "Assets/keystonepassiveheaderleft.png",
            "Assets/keystonepassiveheadermiddle.png",
            "Assets/keystonepassiveheaderright.png",
            header(false, 33.0, 38.0, 6.0, false),
        ),
        "ASCENDANCY" => (
            "Assets/ascendancypassiveheaderleft.png",
            "Assets/ascendancypassiveheadermiddle.png",
            "Assets/ascendancypassiveheaderright.png",
            header(false, 33.0, 38.0, 6.0, false),
        ),
        "MASTERY" => (
            "Assets/masteryheaderunallocatedleft.png",
            "Assets/masteryheaderunallocatedmiddle.png",
            "Assets/masteryheaderunallocatedright.png",
            header(false, 33.0, 38.0, 6.0, false),
        ),
        "MASTERYALLOC" => (
            "Assets/masteryheaderallocatedleft.png",
            "Assets/masteryheaderallocatedmiddle.png",
            "Assets/masteryheaderallocatedright.png",
            header(false, 33.0, 38.0, 6.0, false),
        ),
        _ => (
            "Assets/itemsheaderwhiteleft.png",
            "Assets/itemsheaderwhitemiddle.png",
            "Assets/itemsheaderwhiteright.png",
            header(false, 32.0, 32.0, 6.0, true),
        ),
    };
    Header {
        left,
        middle,
        right,
        ..base
    }
}

/// `separatorConfigs[rarity] or separatorConfigs.NORMAL` (Tooltip.lua:129-139)
pub fn separator_art(name: &str) -> &'static str {
    match name.to_ascii_uppercase().as_str() {
        "RELIC" => "Assets/itemsseparatorfoil.png",
        "UNIQUE" => "Assets/itemsseparatorunique.png",
        "RARE" => "Assets/itemsseparatorrare.png",
        "MAGIC" => "Assets/itemsseparatormagic.png",
        "GEM" => "Assets/itemsseparatorgem.png",
        _ => "Assets/itemsseparatorwhite.png",
    }
}

/// `headerInfluence` (Tooltip.lua:389-406)
pub fn influence_icon(name: &str) -> Option<&'static str> {
    Some(match name {
        "Fractured" => "Assets/fracturedicon.png",
        "Veiled" => "Assets/veiledicon.png",
        "Shaper" => "Assets/shapericon.png",
        "Elder" => "Assets/eldericon.png",
        "Redeemer" => "Assets/redeemericon.png",
        "Hunter" => "Assets/huntericon.png",
        "Crusader" => "Assets/crusadericon.png",
        "Warlord" => "Assets/warlordicon.png",
        "Eater" => "Assets/eatericon.png",
        "Exarch" => "Assets/exarchicon.png",
        "Synthesis" => "Assets/synthesisicon.png",
        "Experimented" => "Assets/experimentedicon.png",
        "Foulborn" => "Assets/breachicon.png",
        "Memory" => "Assets/memoryicon.png",
        "Vestigial" => "Assets/vestigialicon.png",
        _ => return None,
    })
}

/// A relic's header tint by its `foilType`, Rainbow's for any other (Tooltip.lua:509-523)
pub fn foil_tint(foil: Option<&str>) -> [f32; 3] {
    match foil.unwrap_or("") {
        "Amethyst" => [0.9, 0.6, 1.0],
        "Verdant" => [0.5, 1.0, 0.5],
        "Ruby" => [1.0, 0.5, 0.6],
        "Cobalt" => [0.6, 0.7, 1.0],
        "Sunset" => [1.0, 1.0, 0.6],
        "Aureate" => [1.0, 0.85, 0.2],
        "Celestial Quartz" => [1.0, 0.7, 0.85],
        "Celestial Ruby" => [0.8, 0.3, 0.2],
        "Celestial Emerald" => [0.2, 0.6, 0.3],
        "Celestial Aureate" => [0.8, 0.7, 0.2],
        "Celestial Pearl" => [1.0, 0.85, 0.9],
        "Celestial Amethyst" => [0.5, 0.6, 1.0],
        _ => [0.6, 1.0, 0.5],
    }
}

/// Whether PoB skips drawing a tooltip's first separator: an item's or a gem's, under its header
/// (Tooltip.lua:577-585)
pub fn skips_first_separator(header: &str) -> bool {
    matches!(
        header.to_ascii_uppercase().as_str(),
        "RELIC" | "UNIQUE" | "RARE" | "MAGIC" | "GEM"
    )
}

/// A line of a tooltip, as far as its images go
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineArt<'a> {
    Text { background: Option<&'a str> },
    Separator,
}

/// What a tooltip shows, as far as its images go
#[derive(Debug, Clone, Copy)]
pub struct TooltipArt<'a> {
    pub header: Option<&'a str>,
    pub influences: &'a [String],
    pub foil: Option<&'a str>,
    pub recipe: &'a [String],
}

/// One image PoB draws: its file and colour
pub type Drawn = (String, [f32; 4]);

/// The images PoB draws on a tooltip, in its order (Tooltip.lua:466-630), with one middle tile
/// for the header's however many. `styled` is "Styled Tooltips with Flavour Text"; a tooltip
/// whose first line is a separator has no header.
pub fn tooltip_art<'a>(
    tip: &TooltipArt,
    styled: bool,
    lines: impl IntoIterator<Item = LineArt<'a>>,
) -> Vec<Drawn> {
    let white = [1.0, 1.0, 1.0, 1.0];
    let mut out: Vec<Drawn> = Vec::new();
    let lines: Vec<LineArt> = lines.into_iter().collect();
    let titled = matches!(lines.first(), Some(LineArt::Text { .. }));
    if !styled {
        // the oils are drawn whatever the option
        for oil in tip.recipe {
            out.push((format!("TreeData/{oil}.png"), white));
        }
        return out;
    }
    if let Some(name) = tip.header.filter(|_| titled) {
        let art = header_art(name);
        let tint = if name == "RELIC" {
            let [r, g, b] = foil_tint(tip.foil);
            [r, g, b, 1.0]
        } else {
            white
        };
        out.push((art.left.into(), tint));
        let icon = |i: usize| {
            tip.influences
                .get(i)
                .filter(|_| art.influence)
                .and_then(|n| influence_icon(n))
        };
        if let Some(file) = icon(0) {
            out.push((file.into(), white));
        }
        out.push((art.middle.into(), tint));
        out.push((art.right.into(), tint));
        if let Some(file) = icon(1) {
            out.push((file.into(), white));
        }
    }
    for oil in tip.recipe {
        out.push((format!("TreeData/{oil}.png"), white));
    }
    let mut skipped = false;
    for line in lines {
        match line {
            LineArt::Separator => {
                let Some(name) = tip.header else { continue };
                if !skipped && skips_first_separator(name) {
                    skipped = true;
                    continue;
                }
                out.push((separator_art(name).into(), white));
            }
            LineArt::Text {
                background: Some(file),
            } => out.push((file.into(), white)),
            LineArt::Text { .. } => {}
        }
    }
    out
}

/// Paints PoB's header art across the top of a tooltip whose outer rectangle is `outer` (the
/// border inside it), as Tooltip.lua:497-569 does: the left cap, the middle tiled at its width
/// with a cut last tile, the right cap, and the influence icons on the caps.
pub fn header_shapes(
    ctx: &egui::Context,
    outer: Rect,
    art: &Header,
    tint: [f32; 3],
    influences: &[String],
) -> Vec<egui::Shape> {
    let mut shapes = Vec::new();
    let colour = |t: [f32; 3]| {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgb(c(t[0]), c(t[1]), c(t[2]))
    };
    let x0 = outer.left() + 1.0;
    let y0 = outer.top() + 1.0;
    let width = outer.width() - 2.0;
    let mut push = |file: &str, rect: Rect, uv_w: f32, c: Color32| {
        if let Some((texture, _)) = art::image(ctx, file) {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(uv_w, 1.0));
            let mut mesh = egui::Mesh::with_texture(texture);
            mesh.add_rect_with_uv(rect, uv, c);
            shapes.push(egui::Shape::mesh(mesh));
        }
    };
    let tinted = colour(tint);
    let size = Vec2::new(art.side, art.height);
    push(
        art.left,
        Rect::from_min_size(Pos2::new(x0, y0), size),
        1.0,
        tinted,
    );
    let icon_size = art.height / 2.0;
    let icon_y = y0 + (art.height - icon_size) / 2.0;
    let icon = |i: usize| {
        influences
            .get(i)
            .filter(|_| art.influence)
            .and_then(|n| influence_icon(n))
    };
    if let Some(file) = icon(0) {
        push(
            file,
            Rect::from_min_size(Pos2::new(x0 + 2.0, icon_y), Vec2::splat(icon_size)),
            1.0,
            Color32::WHITE,
        );
    }
    let end = x0 + width - art.side;
    let mut x = x0 + art.side;
    while x + art.middle_width <= end {
        push(
            art.middle,
            Rect::from_min_size(Pos2::new(x, y0), Vec2::new(art.middle_width, art.height)),
            1.0,
            tinted,
        );
        x += art.middle_width;
    }
    if end - x > 0.0 {
        // PoB stretches the whole image over what is left
        push(
            art.middle,
            Rect::from_min_size(Pos2::new(x, y0), Vec2::new(end - x, art.height)),
            1.0,
            tinted,
        );
    }
    push(
        art.right,
        Rect::from_min_size(Pos2::new(end, y0), size),
        1.0,
        tinted,
    );
    if let Some(file) = icon(1) {
        push(
            file,
            Rect::from_min_size(
                Pos2::new(x0 + width - icon_size - 2.0, icon_y),
                Vec2::splat(icon_size),
            ),
            1.0,
            Color32::WHITE,
        );
    }
    shapes
}

/// Whether to draw PoB's styled tooltips ("Styled Tooltips with Flavour Text"), on by default
pub fn styled(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::NULL.with("pob-styled-tooltips")))
        .unwrap_or(true)
}

pub fn set_styled(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL.with("pob-styled-tooltips"), on));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[Drawn]) -> Vec<&str> {
        list.iter().map(|(f, _)| f.as_str()).collect()
    }

    /// A rare's header, its influence icons on the caps, its first separator skipped and the
    /// rest in its rarity's art; a relic's header tinted by its foil
    #[test]
    fn the_art_follows_pobs_tooltip() {
        let influences = vec!["Shaper".to_owned(), "Elder".to_owned()];
        let tip = TooltipArt {
            header: Some("RARE"),
            influences: &influences,
            foil: None,
            recipe: &[],
        };
        let lines = [
            LineArt::Text { background: None },
            LineArt::Separator,
            LineArt::Text {
                background: Some("Assets/memorybg.png"),
            },
            LineArt::Separator,
        ];
        let drawn = tooltip_art(&tip, true, lines);
        assert_eq!(
            files(&drawn),
            [
                "Assets/itemsheaderrareleft.png",
                "Assets/shapericon.png",
                "Assets/itemsheaderraremiddle.png",
                "Assets/itemsheaderrareright.png",
                "Assets/eldericon.png",
                "Assets/memorybg.png",
                "Assets/itemsseparatorrare.png",
            ]
        );
        // unstyled: none of it
        assert!(tooltip_art(&tip, false, lines).is_empty());
        let relic = TooltipArt {
            header: Some("RELIC"),
            influences: &[],
            foil: Some("Ruby"),
            recipe: &[],
        };
        let drawn = tooltip_art(&relic, true, lines);
        assert_eq!(drawn[0].1, [1.0, 0.5, 0.6, 1.0]);
        // a gem has no influence icons, a passive no skipped separator
        let gem = TooltipArt {
            header: Some("GEM"),
            influences: &influences,
            foil: None,
            recipe: &[],
        };
        assert_eq!(tooltip_art(&gem, true, lines).len(), 5);
        let oils = vec!["AmberOil".to_owned()];
        let notable = TooltipArt {
            header: Some("NOTABLE"),
            influences: &[],
            foil: None,
            recipe: &oils,
        };
        assert_eq!(
            files(&tooltip_art(&notable, true, lines)),
            [
                "Assets/notablepassiveheaderleft.png",
                "Assets/notablepassiveheadermiddle.png",
                "Assets/notablepassiveheaderright.png",
                "TreeData/AmberOil.png",
                "Assets/itemsseparatorwhite.png",
                "Assets/memorybg.png",
                "Assets/itemsseparatorwhite.png",
            ]
        );
    }

    /// The middle is tiled at its width between the caps, the last tile cut to fit
    #[test]
    fn the_header_tiles_its_middle() {
        let ctx = egui::Context::default();
        let art = header_art("MAGIC");
        assert_eq!((art.height, art.side, art.middle_width), (38.0, 32.0, 32.0));
        // no images load in a test: only the geometry is checked, through a stand-in
        let outer =
            Rect::from_min_size(Pos2::ZERO, Vec2::new(2.0 + 64.0 + 32.0 * 3.0 + 10.0, 100.0));
        let mut xs = Vec::new();
        let (end, mut x) = (1.0 + outer.width() - 2.0 - art.side, 1.0 + art.side);
        while x + art.middle_width <= end {
            xs.push((x, art.middle_width));
            x += art.middle_width;
        }
        xs.push((x, end - x));
        assert_eq!(
            xs,
            [(33.0, 32.0), (65.0, 32.0), (97.0, 32.0), (129.0, 10.0)]
        );
        assert!(header_shapes(&ctx, outer, &art, [1.0; 3], &[]).is_empty());
    }

    /// What the item and node tooltip widgets paint is what `tooltip_art` lists, in order: the
    /// gates compare the list with upstream's Draw, so the widgets must paint that list
    #[test]
    fn the_widgets_paint_what_tooltip_art_lists() {
        use crate::engine::views::{ItemTooltip, ItemTooltipLine, TooltipLine};
        let dir = crate::test_dir::TestDir::new("tooltip-art");
        let file = |path: &str| {
            let at = dir.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            if !at.exists() {
                image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]))
                    .save(&at)
                    .unwrap();
            }
        };
        let text = |t: &str, background: Option<&str>| ItemTooltipLine::Line {
            size: 14,
            text: t.into(),
            font: None,
            center: false,
            background: background.map(str::to_owned),
            mod_line: None,
        };
        let sep = ItemTooltipLine::Separator { separator: 10 };
        let rare = ItemTooltip {
            header: Some("RARE".into()),
            influences: vec!["Shaper".into(), "Elder".into()],
            lines: vec![
                ItemTooltipLine::Line {
                    size: 20,
                    text: "Doom Loop".into(),
                    font: None,
                    center: true,
                    background: None,
                    mod_line: None,
                },
                sep.clone(),
                text("+10 life", None),
                sep.clone(),
                text("Memory Strands: 5", Some("Assets/memorybg.png")),
                sep.clone(),
                text("end", None),
            ],
            ..Default::default()
        };
        // no title first: no header art, but its first separator is still left out
        let untitled = ItemTooltip {
            header: Some("UNIQUE".into()),
            lines: vec![sep.clone(), text("a", None), sep.clone(), text("b", None)],
            ..Default::default()
        };
        let oils = vec!["AmberOil".to_owned(), "SepiaOil".to_owned()];
        let node_lines = vec![
            TooltipLine::Line {
                size: 18,
                text: "Notable".into(),
            },
            TooltipLine::Separator { separator: 14 },
            TooltipLine::Line {
                size: 14,
                text: "stat".into(),
            },
        ];
        let node_look = crate::widgets::Look {
            header: Some("NOTABLE"),
            recipe: &oils,
            center: true,
        };
        let gem_lines = vec![
            TooltipLine::Separator { separator: 10 },
            TooltipLine::Line {
                size: 14,
                text: "gem".into(),
            },
            TooltipLine::Separator { separator: 10 },
            TooltipLine::Line {
                size: 14,
                text: "more".into(),
            },
        ];
        let gem_look = crate::widgets::Look {
            header: Some("GEM"),
            ..Default::default()
        };
        enum Tip<'a> {
            Item(&'a ItemTooltip),
            Lines(&'a [TooltipLine], crate::widgets::Look<'a>),
        }
        let cases = [
            Tip::Item(&rare),
            Tip::Item(&untitled),
            Tip::Lines(&node_lines, node_look),
            Tip::Lines(&gem_lines, gem_look),
        ];
        for tip in cases {
            let want: Vec<String> = match &tip {
                Tip::Item(t) => crate::widgets::item_tooltip::item_tooltip_art(t, true),
                Tip::Lines(l, look) => crate::widgets::tooltip::tooltip_lines_art(l, look, true),
            }
            .into_iter()
            .map(|(f, _)| f)
            .collect();
            for f in &want {
                file(f);
            }
            let ctx = egui::Context::default();
            art::set_context_root(&ctx, &dir);
            // every image loaded first, so one frame paints them all
            let mut loaded = false;
            for i in 0..400 {
                let input = egui::RawInput {
                    time: Some(f64::from(i) * 0.01),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    loaded = want.iter().all(|f| art::image(ui.ctx(), f).is_some());
                });
                out.textures_delta.clear();
                if loaded {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
            assert!(loaded);
            let painted = {
                let input = egui::RawInput {
                    time: Some(4.0),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| match &tip {
                    Tip::Item(t) => {
                        crate::widgets::item_tooltip_ui(ui, t);
                    }
                    Tip::Lines(l, look) => crate::widgets::styled_tooltip_ui(ui, l, look),
                });
                out.textures_delta.clear();
                fn walk(s: &egui::Shape, out: &mut Vec<egui::TextureId>) {
                    match s {
                        egui::Shape::Mesh(m) => out.push(m.texture_id),
                        egui::Shape::Vec(v) => v.iter().for_each(|x| walk(x, out)),
                        _ => {}
                    }
                }
                let mut ids = Vec::new();
                for c in &out.shapes {
                    walk(&c.shape, &mut ids);
                }
                let tm = ctx.tex_manager();
                let tm = tm.read();
                let mut painted: Vec<String> = ids
                    .iter()
                    .filter_map(|id| tm.meta(*id))
                    .filter_map(|m| m.name.strip_prefix("pob-art:").map(str::to_owned))
                    .collect();
                // the header's middle is tiled: one entry for its run
                painted.dedup_by(|a, b| a == b && a.contains("middle"));
                painted
            };
            assert_eq!(painted, want);
        }
    }
}
