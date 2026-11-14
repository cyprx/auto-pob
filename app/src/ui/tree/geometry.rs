//! Camera, hit testing and arc geometry for the passive tree canvas.

use crate::engine::TreeLayout;
use eframe::egui::{Pos2, Rect, Vec2};
use std::collections::HashMap;

const MIN_ZOOM: f32 = 0.01;
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
    /// Screen points per tree unit
    pub zoom: f32,
}

impl Camera {
    pub fn to_screen(&self, rect: Rect, tree: [f32; 2]) -> Pos2 {
        rect.center() + (Vec2::from(tree) - self.center) * self.zoom
    }

    pub fn to_tree(&self, rect: Rect, screen: Pos2) -> [f32; 2] {
        (self.center + (screen - rect.center()) / self.zoom).into()
    }

    /// Zooms by `factor`, keeping the tree point under `cursor` where it is.
    pub fn zoom_at(&mut self, rect: Rect, cursor: Pos2, factor: f32) {
        let under_cursor = Vec2::from(self.to_tree(rect, cursor));
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.center = under_cursor - (cursor - rect.center()) / self.zoom;
    }

    /// Shows the whole tree. An empty, single-node or flat layout gets the centre of its nodes
    /// and a usable zoom, so the zoom is never 0.
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
        self.zoom = ((rect.width() / w).min(rect.height() / h) * 0.95).clamp(MIN_ZOOM, MAX_ZOOM);
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
        let mut camera = Camera {
            center: Vec2::new(50.0, 80.0),
            zoom: 0.3,
        };
        let cursor = Pos2::new(600.0, 100.0);
        let before = camera.to_tree(rect(), cursor);
        camera.zoom_at(rect(), cursor, 1.7);
        assert!((camera.zoom - 0.51).abs() < 1e-4);
        assert!(close(camera.to_tree(rect(), cursor), before));
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
