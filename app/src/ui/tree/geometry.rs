//! Camera, hit testing and arc geometry for the passive tree canvas.

use crate::engine::TreeLayout;
use eframe::egui::{Pos2, Rect, Vec2};
use std::collections::HashMap;

const MIN_ZOOM: f32 = 0.01;
/// PoB's deepest zoom level: `zoom = 1.2 ^ zoomLevel` from 0 to 12 (PassiveTreeView.lua:1299)
pub const MAX_LEVEL: i32 = 12;
/// PoB's zoom step
const STEP: f32 = 1.2;
const MAX_ZOOM: f32 = 2.0;
/// A node can be hit this many screen points away from its centre, even when drawn smaller
pub const MIN_HIT_POINTS: f32 = 6.0;
/// Side of a hit grid bucket, in tree units
const CELL: f32 = 200.0;

/// Camera over the tree.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Camera {
    /// Tree position at the centre of the view
    pub center: Vec2,
    /// Screen points per tree unit. With a tree it follows `level` and the view (see `limit`).
    pub zoom: f32,
    /// `tree.size` of the tree shown, which sets PoB's zoom range; 1 or less sets none
    pub tree_size: f32,
    /// PoB's `zoomLevel`, 0 to [`MAX_LEVEL`]: the tree's size across the view's smaller side,
    /// times 1.2 to the level (PassiveTreeView.lua:271, 1297-1301)
    pub level: i32,
}

impl Camera {
    pub fn to_screen(&self, rect: Rect, tree: [f32; 2]) -> Pos2 {
        rect.center() + (Vec2::from(tree) - self.center) * self.zoom
    }

    pub fn to_tree(&self, rect: Rect, screen: Pos2) -> [f32; 2] {
        (self.center + (screen - rect.center()) / self.zoom).into()
    }

    /// The zoom at level 0 for the view, when a tree is shown
    fn base(&self, rect: Rect) -> Option<f32> {
        let side = rect.width().min(rect.height());
        (self.tree_size > 1.0 && side > 0.0).then(|| side / self.tree_size)
    }

    /// Zooms `steps` of PoB's levels, within its 0 to 12, keeping the tree point under `cursor`
    /// where it is (PassiveTreeView:Zoom). A step past a limit changes nothing. Without a tree
    /// the zoom is free, each step still 1.2.
    pub fn zoom_at(&mut self, rect: Rect, cursor: Pos2, steps: i32) {
        let under_cursor = Vec2::from(self.to_tree(rect, cursor));
        match self.base(rect) {
            Some(base) => {
                let level = (self.level + steps).clamp(0, MAX_LEVEL);
                if level == self.level {
                    return;
                }
                self.level = level;
                self.zoom = base * STEP.powi(level);
            }
            None => self.zoom = (self.zoom * STEP.powi(steps)).clamp(MIN_ZOOM, MAX_ZOOM),
        }
        self.center = under_cursor - (cursor - rect.center()) / self.zoom;
    }

    /// Keeps the view within PoB's for a tree of `size`: the zoom at its level for this view,
    /// and the tree's centre no farther from the view's than 2/3 of the view at PoB's zoom
    /// (PassiveTreeView.lua:265-271). A size of 1 or less is no tree's: nothing is limited.
    pub fn limit(&mut self, rect: Rect, size: f32) {
        self.tree_size = size;
        let side = rect.width().min(rect.height());
        let Some(base) = self.base(rect) else {
            return;
        };
        if self.zoom <= 0.0 {
            return;
        }
        self.level = self.level.clamp(0, MAX_LEVEL);
        self.zoom = base * STEP.powi(self.level);
        let reach = Vec2::new(rect.width(), rect.height()) * (2.0 / 3.0) * size / side;
        self.center = self.center.clamp(-reach, reach);
    }

    /// Shows the whole tree: at its centre, at the deepest of PoB's levels that still shows
    /// every node. An empty, single-node or flat layout gets the centre of its nodes and a
    /// usable zoom, so the zoom is never 0.
    pub fn fit(&mut self, rect: Rect, layout: &TreeLayout) {
        let bounds = layout.nodes.iter().fold(Rect::NOTHING, |r, n| {
            r.union(Rect::from_pos(Pos2::from(n.pos)))
        });
        if bounds.is_negative() {
            self.center = Vec2::ZERO;
            self.zoom = 0.3;
            return;
        }
        self.center = bounds.center().to_vec2();
        let (w, h) = (bounds.width().max(1.0), bounds.height().max(1.0));
        let whole = (rect.width() / w).min(rect.height() / h) * 0.95;
        self.tree_size = layout.size;
        match self.base(rect) {
            Some(base) => {
                let level = (whole / base).ln() / STEP.ln();
                self.level = (level.floor() as i32).clamp(0, MAX_LEVEL);
                self.zoom = base * STEP.powi(self.level);
            }
            None => self.zoom = whole.clamp(MIN_ZOOM, MAX_ZOOM),
        }
    }

    /// PoB's `Focus`: the deepest level, centred on `at` (PassiveTreeView.lua:1312-1320)
    pub fn focus(&mut self, at: [f32; 2]) {
        self.center = at.into();
        if self.tree_size > 1.0 {
            self.level = MAX_LEVEL;
        }
    }
}

/// Spatial buckets over node positions, so a pointer lookup doesn't scan every node.
#[derive(Debug, Default)]
pub struct HitGrid {
    cells: HashMap<(i32, i32), Vec<usize>>,
}

fn cell_of(pos: [f32; 2]) -> (i32, i32) {
    (
        (pos[0] / CELL).floor() as i32,
        (pos[1] / CELL).floor() as i32,
    )
}

impl HitGrid {
    pub fn new(layout: &TreeLayout) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, node) in layout.nodes.iter().enumerate() {
            cells.entry(cell_of(node.pos)).or_default().push(i);
        }
        Self { cells }
    }

    /// The node nearest to `tree_pos` among those whose hit radius contains it.
    pub fn node_at(&self, layout: &TreeLayout, tree_pos: [f32; 2], zoom: f32) -> Option<usize> {
        if zoom.is_nan() || zoom <= 0.0 {
            return None;
        }
        let floor = MIN_HIT_POINTS / zoom;
        let reach = floor.max(node_radius("Keystone"));
        let (x0, y0) = cell_of([tree_pos[0] - reach, tree_pos[1] - reach]);
        let (x1, y1) = cell_of([tree_pos[0] + reach, tree_pos[1] + reach]);
        let mut best: Option<(usize, f32)> = None;
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                for &i in self.cells.get(&(cx, cy)).into_iter().flatten() {
                    let node = &layout.nodes[i];
                    // PoB's class starts have no size, so no hover (PassiveTree.lua:860-869)
                    if !hoverable(&node.kind) {
                        continue;
                    }
                    let d = (Vec2::from(node.pos) - Vec2::from(tree_pos)).length();
                    if d <= node_radius(&node.kind).max(floor) && best.is_none_or(|(_, b)| d < b) {
                        best = Some((i, d));
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

/// A node's size in tree units, by PoB node type: its frame's width times 1.33, the radius
/// PoB hovers it within (`nodeOverlay` `artWidth`, PassiveTree.lua:423-474). A class start has
/// none in PoB; it gets one for drawing a missing art's dot.
pub fn node_radius(kind: &str) -> f32 {
    let art_width = match kind {
        "Notable" | "Socket" => 58.0,
        "Keystone" => 84.0,
        "Mastery" => 65.0,
        "ClassStart" | "AscendClassStart" => 45.0,
        _ => 40.0,
    };
    art_width * 1.33
}

/// Whether PoB lets the pointer find a node of this type: not a class or ascendancy start
pub fn hoverable(kind: &str) -> bool {
    !matches!(kind, "ClassStart" | "AscendClassStart")
}

/// Points along the shorter arc from `a` to `b` around `center`, in screen space.
pub fn arc_points(
    center: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
    zoom: f32,
    to_screen: impl Fn([f32; 2]) -> Pos2,
) -> Vec<Pos2> {
    let c = Vec2::from(center);
    let (va, vb) = (Vec2::from(a) - c, Vec2::from(b) - c);
    let radius = va.length();
    let start = va.angle();
    let mut sweep = vb.angle() - start;
    // take the short way round
    if sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    } else if sweep < -std::f32::consts::PI {
        sweep += std::f32::consts::TAU;
    }
    let segments = ((sweep.abs() * radius * zoom / 6.0) as usize).clamp(2, 48);
    (0..=segments)
        .map(|i| {
            let angle = start + sweep * i as f32 / segments as f32;
            to_screen((c + Vec2::angled(angle) * radius).into())
        })
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::engine::{NodeArt, TreeNode};

    pub fn node(id: u32, pos: [f32; 2], kind: &str) -> TreeNode {
        TreeNode {
            id,
            pos,
            kind: kind.into(),
            name: format!("Node {id}"),
            stats: vec![format!("stat {id}")],
            orbit: None,
            group_center: None,
            art: NodeArt::default(),
            ascendancy: None,
            replaceable: false,
            mastery_effects: false,
            in_radius: false,
            radius_socket: false,
        }
    }

    pub fn layout(nodes: Vec<TreeNode>, edges: Vec<(usize, usize)>) -> TreeLayout {
        TreeLayout {
            version: "3_28".into(),
            nodes,
            edges,
            sheets: vec![],
            connectors: vec![],
            groups: vec![],
            class_art: vec![],
            assets: Default::default(),
            size: 1.0,
            jewel_radii: vec![],
            key: "k".into(),
        }
    }

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(800.0, 600.0))
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05
    }

    #[test]
    fn to_tree_inverts_to_screen() {
        for zoom in [0.05, 0.3, 2.0] {
            let camera = Camera {
                center: Vec2::new(120.0, -340.0),
                zoom,
                ..Camera::default()
            };
            let p = [500.0, -1200.0];
            assert!(close(
                camera.to_tree(rect(), camera.to_screen(rect(), p)),
                p
            ));
        }
    }

    #[test]
    fn zoom_at_keeps_point_under_cursor() {
        // with a tree and without
        for tree_size in [0.0, 20_000.0] {
            let mut camera = Camera {
                center: Vec2::new(50.0, 80.0),
                zoom: 0.3,
                tree_size,
                level: 5,
            };
            let cursor = Pos2::new(600.0, 100.0);
            let before = camera.to_tree(rect(), cursor);
            camera.zoom_at(rect(), cursor, 2);
            assert!(close(camera.to_tree(rect(), cursor), before));
        }
    }

    /// PoB's 13 zoom levels: 1.2 to the level times the tree across the view's smaller side,
    /// whatever the view's size; a step past 0 or 12 does nothing (PassiveTreeView.lua:271,
    /// 1297-1310)
    #[test]
    fn the_zoom_takes_pobs_levels() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let mut c = Camera {
            center: Vec2::ZERO,
            zoom: 1.0,
            tree_size: 20_000.0,
            level: 3,
        };
        c.limit(rect, 20_000.0);
        let at = |level: i32| 600.0 / 20_000.0 * 1.2_f32.powi(level);
        assert!((c.zoom - at(3)).abs() < 1e-6);
        let cursor = Pos2::new(700.0, 100.0);
        c.zoom_at(rect, cursor, 1);
        assert_eq!(c.level, 4);
        assert!((c.zoom - at(4)).abs() < 1e-6);
        c.zoom_at(rect, cursor, 30);
        assert_eq!(c.level, 12);
        assert!((c.zoom - at(12)).abs() < 1e-6);
        c.zoom_at(rect, cursor, -30);
        assert_eq!(c.level, 0);
        // a larger view keeps the level, as PoB's does
        let wide = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0));
        c.level = 2;
        c.limit(wide, 20_000.0);
        assert!((c.zoom - 1200.0 / 20_000.0 * 1.44).abs() < 1e-6);
        // fitting picks the deepest level that shows every node
        let l = TreeLayout {
            size: 20_000.0,
            ..layout(
                vec![
                    node(1, [-9000.0, -9000.0], "Normal"),
                    node(2, [9000.0, 9000.0], "Normal"),
                ],
                vec![],
            )
        };
        c.fit(rect, &l);
        assert_eq!(c.level, 0);
        let small = TreeLayout {
            size: 20_000.0,
            ..layout(
                vec![
                    node(1, [-2000.0, -2000.0], "Normal"),
                    node(2, [2000.0, 2000.0], "Normal"),
                ],
                vec![],
            )
        };
        c.fit(rect, &small);
        // 600 / 4000 * 0.95 over 0.03 is 4.75, and 1.2^8 = 4.3 < 4.75 < 1.2^9
        assert_eq!(c.level, 8);
        assert!((c.zoom - at(8)).abs() < 1e-6);
        assert_eq!(c.center, Vec2::ZERO);
        // PoB's Focus goes to the deepest level
        c.focus([100.0, 200.0]);
        c.limit(rect, 20_000.0);
        assert_eq!((c.level, c.center), (12, Vec2::new(100.0, 200.0)));
    }

    #[test]
    fn node_at_finds_the_nearest_node_within_radius() {
        let l = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                node(2, [40.0, 0.0], "Normal"),
                node(3, [1000.0, 0.0], "Normal"),
            ],
            vec![],
        );
        let grid = HitGrid::new(&l);
        // between 1 and 2, nearer to 2
        assert_eq!(grid.node_at(&l, [26.0, 0.0], 1.0), Some(1));
        assert_eq!(grid.node_at(&l, [14.0, 0.0], 1.0), Some(0));
        assert_eq!(grid.node_at(&l, [1005.0, 3.0], 1.0), Some(2));
        // empty space
        assert_eq!(grid.node_at(&l, [500.0, 500.0], 1.0), None);
        assert_eq!(grid.node_at(&l, [0.0, 80.0], 1.0), None);
    }

    #[test]
    fn the_view_stays_within_pobs() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let mut c = Camera {
            center: Vec2::ZERO,
            zoom: 0.001,
            tree_size: 0.0,
            level: 0,
        };
        c.limit(rect, 20_000.0);
        assert_eq!(c.zoom, 600.0 / 20_000.0);
        c.center = Vec2::new(1e6, -1e6);
        c.limit(rect, 20_000.0);
        let reach = Vec2::new(800.0, 600.0) * (2.0 / 3.0) * 20_000.0 / 600.0;
        assert_eq!(c.center, Vec2::new(reach.x, -reach.y));
        // no tree: nothing changes
        let mut free = Camera {
            center: Vec2::splat(1e6),
            zoom: 0.001,
            tree_size: 0.0,
            level: 0,
        };
        free.limit(rect, 1.0);
        assert_eq!(free.zoom, 0.001);
        // a notch at a limit leaves the view where it is
        let mut at_limit = Camera {
            center: Vec2::new(100.0, 50.0),
            zoom: 0.03 * 1.2_f32.powi(12),
            tree_size: 20_000.0,
            level: 12,
        };
        let before = at_limit;
        at_limit.zoom_at(rect, Pos2::new(700.0, 100.0), 1);
        assert_eq!(at_limit, before);
        at_limit.level = 0;
        at_limit.zoom = 0.03;
        let before = at_limit;
        at_limit.zoom_at(rect, Pos2::new(700.0, 100.0), -1);
        assert_eq!(at_limit, before);
    }

    /// PoB hovers a node within its frame's size (artWidth * 1.33) and never a class or
    /// ascendancy start, which has no size (PassiveTree.lua:423-474, 860-869)
    #[test]
    fn node_at_follows_pobs_sizes() {
        let l = layout(
            vec![
                node(1, [0.0, 0.0], "Normal"),
                node(2, [1000.0, 0.0], "Keystone"),
                node(3, [2000.0, 0.0], "ClassStart"),
                node(4, [3000.0, 0.0], "AscendClassStart"),
            ],
            vec![],
        );
        let grid = HitGrid::new(&l);
        assert_eq!(grid.node_at(&l, [53.0, 0.0], 1.0), Some(0));
        assert_eq!(grid.node_at(&l, [54.0, 0.0], 1.0), None);
        assert_eq!(grid.node_at(&l, [1111.0, 0.0], 1.0), Some(1));
        assert_eq!(grid.node_at(&l, [1113.0, 0.0], 1.0), None);
        assert_eq!(grid.node_at(&l, [2000.0, 0.0], 1.0), None);
        assert_eq!(grid.node_at(&l, [3000.0, 0.0], 1.0), None);
        assert!((node_radius("Notable") - 77.14).abs() < 1e-3);
        assert!((node_radius("Mastery") - 86.45).abs() < 1e-3);
    }

    #[test]
    fn node_at_is_none_without_a_zoom_and_works_at_extremes() {
        let l = layout(vec![node(1, [0.0, 0.0], "Normal")], vec![]);
        let grid = HitGrid::new(&l);
        assert_eq!(grid.node_at(&l, [0.0, 0.0], 0.0), None);
        assert_eq!(grid.node_at(&l, [0.0, 0.0], f32::NAN), None);
        assert_eq!(grid.node_at(&l, [100.0, 0.0], 0.05), Some(0));
        assert_eq!(grid.node_at(&l, [100.0, 0.0], 2.0), None);
        assert_eq!(grid.node_at(&l, [20.0, 0.0], 2.0), Some(0));
    }

    #[test]
    fn fit_gives_degenerate_layouts_a_positive_zoom() {
        for nodes in [vec![], vec![node(1, [50.0, 70.0], "Normal")]] {
            let l = layout(nodes, vec![]);
            let mut c = Camera::default();
            c.fit(rect(), &l);
            assert!(c.zoom > 0.0 && c.zoom.is_finite());
        }
    }

    #[test]
    fn arc_points_lie_on_the_orbit() {
        let center = [100.0, 100.0];
        let points = arc_points(center, [200.0, 100.0], [100.0, 200.0], 1.0, |p| {
            Pos2::new(p[0], p[1])
        });
        assert!(points.len() >= 3);
        for p in &points {
            let d = (*p - Pos2::new(100.0, 100.0)).length();
            assert!((d - 100.0).abs() < 0.01, "{d}");
        }
        assert!((points[0] - Pos2::new(200.0, 100.0)).length() < 0.01);
        assert!((points[points.len() - 1] - Pos2::new(100.0, 200.0)).length() < 0.01);
    }
}
