//! What is drawn on top of the nodes: search rings, compare tint, jewel radii, the heatmap
//! glow and the legend.

use crate::engine::{JewelSocket, NodePower, NodePowerEntry};
use crate::theme;
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

pub const ALLOCATED: Color32 = Color32::from_rgb(230, 185, 80);
/// PoB tints its search ring red: its `rgbColor` is never set (PassiveTreeView.lua:1013-1014)
pub const SEARCH_RING: Color32 = Color32::from_rgb(255, 0, 0);
pub const JEWEL_RING: Color32 = Color32::from_rgb(120, 190, 230);

/// PoB's heat strength: `(max(value, 0) / max * 1.5) ^ 0.5` (PassiveTreeView.lua:911),
/// limited to what a colour channel can show.
pub fn heat_strength(value: f64, max: f64) -> f32 {
    if max <= 0.0 {
        return 0.0;
    }
    ((value.max(0.0) / max * 1.5).sqrt() as f32).clamp(0.0, 1.0)
}

/// PoB's "Node Power colours" option (`main.nodePowerTheme`, Main.lua:957-964): the colours of
/// offence and of defence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerTheme {
    #[default]
    RedBlue,
    RedGreen,
    GreenBlue,
}

impl PowerTheme {
    /// The setting's value; anything else is PoB's default
    pub fn parse(value: &str) -> PowerTheme {
        match value {
            "RED/GREEN" => PowerTheme::RedGreen,
            "GREEN/BLUE" => PowerTheme::GreenBlue,
            _ => PowerTheme::RedBlue,
        }
    }

    /// The offence and defence colour names, as the heat map's tooltip says them
    pub fn names(self) -> (&'static str, &'static str) {
        match self {
            PowerTheme::RedBlue => ("red", "blue"),
            PowerTheme::RedGreen => ("red", "green"),
            PowerTheme::GreenBlue => ("green", "blue"),
        }
    }
}

/// The glow colour of a node's power in the theme (PassiveTreeView.lua:907-934). `None` when
/// there is nothing to show.
pub fn heat_color(power: &NodePower, entry: &NodePowerEntry, theme: PowerTheme) -> Option<Color32> {
    let channel = |v: f32| (v * 255.0).round() as u8;
    let [r, g, b] = heat_rgb(power, Some(entry), theme);
    (r.max(g).max(b) > 0.02).then(|| Color32::from_rgb(channel(r), channel(g), channel(b)))
}

/// The colour PoB draws an unallocated node's art with while the heat map is on
/// (PassiveTreeView.lua:907-934); a node without an entry has no power
pub fn heat_rgb(power: &NodePower, entry: Option<&NodePowerEntry>, theme: PowerTheme) -> [f32; 3] {
    let single = entry.map_or(0.0, |e| e.single);
    let offence = entry.and_then(|e| e.offence).unwrap_or(0.0);
    let defence = entry.and_then(|e| e.defence).unwrap_or(0.0);
    let (r, g, b) = if power.stat.is_some() {
        let stat = heat_strength(single, power.max_single);
        match theme {
            PowerTheme::RedBlue => (stat, 0.0, 0.0),
            PowerTheme::RedGreen => (0.0, stat, 0.0),
            PowerTheme::GreenBlue => (0.0, 0.0, stat),
        }
    } else {
        let dps = heat_strength(offence, power.max_offence);
        let def = heat_strength(defence, power.max_defence);
        let mix = ((dps - 0.5).max(0.0) + (def - 0.5).max(0.0)) / 2.0;
        match theme {
            PowerTheme::RedBlue => (dps, mix, def),
            PowerTheme::RedGreen => (dps, def, mix),
            PowerTheme::GreenBlue => (mix, dps, def),
        }
    };
    [r, g, b]
}

pub fn draw_search_ring(painter: &Painter, pos: Pos2, radius: f32) {
    painter.circle_stroke(pos, radius + 4.0, Stroke::new(2.0, SEARCH_RING));
}

pub fn draw_compare_tint(painter: &Painter, pos: Pos2, radius: f32, rgb: [f32; 3]) {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let colour = Color32::from_rgba_unmultiplied(c(rgb[0]), c(rgb[1]), c(rgb[2]), 150);
    painter.circle_filled(pos, radius * 1.5, colour);
}

/// The jewel's radius around its socket: the outer ring, and the inner one when it has a hole.
pub fn draw_jewel_ring(painter: &Painter, view: Rect, pos: Pos2, jewel: &JewelSocket, zoom: f32) {
    let Some([inner, outer]) = jewel.ring else {
        return;
    };
    let outer = outer * zoom;
    if !view.expand(outer).contains(pos) {
        return;
    }
    let stroke = Stroke::new(1.5, JEWEL_RING.gamma_multiply(0.7));
    painter.circle_stroke(pos, outer, stroke);
    if inner > 0.0 {
        painter.circle_stroke(pos, inner * zoom, stroke);
    }
}

/// What the legend has to explain.
pub struct Legend<'a> {
    pub compare: bool,
    pub searching: bool,
    pub heat_stat: Option<&'a str>,
    pub heat: bool,
    pub progress: Option<u32>,
    pub busy: bool,
}

pub fn draw_legend(painter: &Painter, view: Rect, legend: &Legend) {
    let mut rows: Vec<(Color32, String)> = vec![(ALLOCATED, "Allocated".into())];
    rows.push((
        Color32::from_rgb(120, 200, 255),
        "Path to hovered node".into(),
    ));
    if legend.searching {
        rows.push((SEARCH_RING, "Search match".into()));
    }
    if legend.compare {
        rows.push((
            Color32::from_rgb(255, 160, 60),
            "Differs in compared tree".into(),
        ));
    }
    if legend.heat {
        let what = legend.heat_stat.unwrap_or("Damage / defence");
        let text = match legend.progress {
            Some(p) => format!("Node power: {what} ({p}%)"),
            None => format!("Node power: {what}"),
        };
        rows.push((Color32::from_rgb(255, 40, 40), text));
    }
    if legend.busy {
        rows.push((theme::TEXT_DIM, "Working...".into()));
    }
    let font = FontId::proportional(12.0);
    let line = 16.0;
    let mut y = view.left_bottom().y - 8.0 - line * rows.len() as f32;
    for (colour, text) in rows {
        let dot = Pos2::new(view.left() + 14.0, y + line / 2.0);
        painter.circle_filled(dot, 4.0, colour);
        painter.text(
            dot + Vec2::new(10.0, 0.0),
            Align2::LEFT_CENTER,
            text,
            font.clone(),
            egui::Color32::from_gray(170),
        );
        y += line;
    }
}

/// A ring that grows and fades around a node the user was pointed to. `age` is in seconds.
pub fn draw_pulse(painter: &Painter, pos: Pos2, radius: f32, age: f64) {
    let t = (age / super::PULSE_SECONDS).clamp(0.0, 1.0) as f32;
    let grow = radius + 6.0 + 40.0 * t;
    let colour = SEARCH_RING.gamma_multiply(1.0 - t);
    painter.circle_stroke(pos, grow, Stroke::new(3.0, colour));
    painter.circle_stroke(pos, radius + 4.0, Stroke::new(2.0, colour));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power(stat: Option<&str>) -> NodePower {
        NodePower {
            stat: stat.map(String::from),
            nodes: vec![],
            max_single: 100.0,
            max_offence: 10.0,
            max_defence: 20.0,
            finished: true,
        }
    }

    fn entry(single: f64, offence: f64, defence: f64) -> NodePowerEntry {
        NodePowerEntry {
            id: 1,
            single,
            path: None,
            offence: Some(offence),
            defence: Some(defence),
        }
    }

    #[test]
    fn strength_follows_pobs_rule() {
        assert_eq!(heat_strength(-5.0, 10.0), 0.0);
        assert_eq!(heat_strength(5.0, 0.0), 0.0);
        assert!((heat_strength(4.0, 10.0) - 0.6_f64.sqrt() as f32).abs() < 1e-6);
        // the 1.5 factor overshoots for the top nodes; the channel stops at 1
        assert_eq!(heat_strength(10.0, 10.0), 1.0);
    }

    #[test]
    fn single_stat_uses_the_red_channel_only() {
        let c = heat_color(
            &power(Some("Life")),
            &entry(100.0, 0.0, 0.0),
            PowerTheme::RedBlue,
        )
        .unwrap();
        assert_eq!((c.g(), c.b()), (0, 0));
        assert!(c.r() > 200);
        assert_eq!(
            heat_color(
                &power(Some("Life")),
                &entry(-3.0, 0.0, 0.0),
                PowerTheme::RedBlue
            ),
            None
        );
    }

    #[test]
    fn without_a_stat_offence_and_defence_mix() {
        let c = heat_color(&power(None), &entry(0.0, 10.0, 20.0), PowerTheme::RedBlue).unwrap();
        assert_eq!((c.r(), c.b()), (255, 255));
        assert!(c.g() > 100);
    }

    #[test]
    fn the_theme_picks_the_channels() {
        // PassiveTreeView.lua:913-933: the single stat and offence/defence/mix per theme
        let single = |t| heat_color(&power(Some("Life")), &entry(100.0, 0.0, 0.0), t).unwrap();
        assert!(single(PowerTheme::RedGreen).g() > 200);
        assert_eq!(single(PowerTheme::RedGreen).r(), 0);
        assert!(single(PowerTheme::GreenBlue).b() > 200);
        let both = |t| heat_color(&power(None), &entry(0.0, 10.0, 0.0), t).unwrap();
        // offence only: red in red/blue and red/green, green in green/blue
        assert!(both(PowerTheme::RedGreen).r() > 200);
        assert_eq!(both(PowerTheme::RedGreen).g(), 0);
        assert!(both(PowerTheme::GreenBlue).g() > 200);
        // the mix goes to the theme's third channel: blue in red/green, red in green/blue
        assert_eq!(
            both(PowerTheme::GreenBlue).r(),
            both(PowerTheme::RedGreen).b()
        );
        assert!(both(PowerTheme::GreenBlue).r() < 100);
        assert_eq!(PowerTheme::parse("GREEN/BLUE"), PowerTheme::GreenBlue);
        assert_eq!(PowerTheme::parse("x"), PowerTheme::RedBlue);
    }
}
