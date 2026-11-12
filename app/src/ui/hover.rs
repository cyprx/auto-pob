//! "What if" hover tooltips (R22): a control hovered for PoB's delay asks the engine once for
//! what PoB's `tooltipFunc` shows, and shows the answer while it stays hovered.

use crate::engine::views::HoverTooltipView;
use crate::engine::{Command, HoverTarget};
use crate::widgets::tooltip_ui;
use eframe::egui;
use std::sync::Arc;

/// PoB shows a control's tooltip at once; the 100 ms keeps a pointer passing over a row of
/// boxes from asking for each one's calculation.
pub(crate) const HOVER_DELAY: f64 = 0.1;

#[derive(Default)]
pub(crate) struct HoverTip {
    /// The control the pointer is over, and since when
    hover: Option<(HoverTarget, f64)>,
    /// The control the engine was asked about
    asked: Option<HoverTarget>,
    /// The engine's answer for `asked`
    view: Option<Arc<HoverTooltipView>>,
}

impl HoverTip {
    /// Keeps the answer to the question still open; any other is stale.
    pub(crate) fn on_view(&mut self, view: &Arc<HoverTooltipView>) {
        if self.asked.as_ref() == Some(&view.target) {
            self.view = Some(view.clone());
        }
    }

    /// The build changed under the answer: it is asked for again while still hovered.
    pub(crate) fn forget(&mut self) {
        self.asked = None;
        self.view = None;
    }

    /// The question this tip has open, if any (for tests)
    #[cfg(test)]
    pub(crate) fn asked(&self) -> Option<&HoverTarget> {
        self.asked.as_ref()
    }

    /// Runs for `response` every frame it is drawn. After `HOVER_DELAY` of hovering it pushes
    /// one `HoverTooltip` query; once answered, the lines show next to the control. Returns
    /// whether PoB's lines were shown, so a caller can leave out its own static tooltip.
    pub(crate) fn track(
        &mut self,
        response: &egui::Response,
        target: HoverTarget,
        now: f64,
        queries: &mut Vec<Command>,
    ) -> bool {
        if !response.hovered() {
            if self.hover.as_ref().is_some_and(|(t, _)| *t == target) {
                self.hover = None;
            }
            if self.asked.as_ref() == Some(&target) {
                self.forget();
            }
            return false;
        }
        let since = match &self.hover {
            Some((t, since)) if *t == target => *since,
            _ => {
                self.hover = Some((target.clone(), now));
                now
            }
        };
        if self.asked.as_ref() != Some(&target) {
            if now - since >= HOVER_DELAY - 1e-6 {
                self.asked = Some(target.clone());
                self.view = None;
                queries.push(Command::HoverTooltip(target));
            } else {
                response
                    .ctx
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
            return false;
        }
        match &self.view {
            Some(view) if !view.lines.is_empty() => {
                response
                    .clone()
                    .on_hover_ui(|ui| tooltip_ui(ui, &view.lines));
                true
            }
            _ => false,
        }
    }
}
