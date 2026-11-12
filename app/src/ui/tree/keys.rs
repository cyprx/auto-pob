//! The Tree tab's keys (R25): TreeTab.lua:359-384 and PassiveTreeView.lua:198-218.

use super::TreeTab;
use crate::engine::Command;
use crate::ui::shortcuts::ShortcutAction;
use eframe::egui;

/// PoB's zoom step: `zoom = 1.2 ^ zoomLevel` (PassiveTreeView.lua:1297-1301)
const ZOOM_STEP: f32 = 1.2;

impl TreeTab {
    /// The pointer is over the tree selector or its list is open (TreeTab.lua:367-371)
    pub fn spec_select_busy(&self) -> bool {
        self.toolbar.spec_busy
    }

    /// Ctrl+F: the search box takes the keyboard (TreeTab.lua:380-381)
    pub fn focus_search(&mut self) {
        self.toolbar.focus_search = true;
    }

    /// The node under the pointer, from the layout
    fn hovered_node(&self) -> Option<&crate::engine::TreeNode> {
        let layout = self.layout.as_ref()?;
        let &i = self.index.get(&self.hover?)?;
        layout.nodes.get(i)
    }

    /// What Ctrl+C copies over a node that isn't a jewel socket: "# " and its name, then its
    /// stat lines, each ending a line (PassiveTreeView.lua:208-213)
    pub fn hovered_node_text(&self) -> Option<String> {
        let node = self.hovered_node().filter(|n| n.kind != "Socket")?;
        let mut text = format!("# {}\n", node.name);
        for line in &node.stats {
            text.push_str(line);
            text.push('\n');
        }
        Some(text)
    }

    /// Puts the pointer on a node, as the canvas would
    #[cfg(test)]
    pub fn hover_for_test(&mut self, id: Option<u32>) {
        self.hover = id;
    }

    #[cfg(test)]
    pub fn zoom_for_test(&self) -> f32 {
        self.camera.zoom
    }

    /// The wiki page name of the node under the pointer (PassiveTreeView.lua:216-217)
    pub fn hovered_wiki(&self) -> Option<String> {
        self.hovered_node().map(|n| n.name.clone())
    }

    /// Carries out a Tree tab key; returns the command it sends, if any.
    pub fn on_shortcut(&mut self, action: &ShortcutAction, ctx: &egui::Context) -> Option<Command> {
        let state = self.state.clone()?;
        match action {
            // PoB's "Manage trees..." popup; ours is the tree selector with its buttons
            ShortcutAction::SpecManager => {
                if let Some(id) = self.toolbar.spec_combo {
                    egui::Popup::open_id(ctx, id.with("popup"));
                }
                None
            }
            ShortcutAction::PrevSpec => state
                .active_spec
                .checked_sub(1)
                .map(|index| Command::SelectSpec { index }),
            ShortcutAction::NextSpec => {
                (state.active_spec + 1 < state.specs.len()).then(|| Command::SelectSpec {
                    index: state.active_spec + 1,
                })
            }
            ShortcutAction::Rename => {
                self.toolbar.rename_active(&state);
                None
            }
            ShortcutAction::Heatmap => Some(Command::ToggleHeatmap),
            ShortcutAction::StatDifferences => Some(Command::SetTreeStatDifferences {
                on: !state.show_stat_differences,
            }),
            ShortcutAction::Zoom(level) => {
                if let Some(rect) = self.rect {
                    // around the pointer, as PoB's Zoom keeps the point under the cursor
                    let at = ctx.pointer_hover_pos().unwrap_or(rect.center());
                    self.camera.zoom_at(rect, at, ZOOM_STEP.powi(*level));
                }
                None
            }
            _ => None,
        }
    }
}
