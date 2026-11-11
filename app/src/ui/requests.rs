//! The pending-request line: one row per running network request, each with a Cancel button.

use crate::engine::Command;
use crate::engine::views::RequestsView;
use eframe::egui;

/// Draws a row for each pending request. Returns the `CancelRequest` of the row whose Cancel
/// was clicked.
pub fn show(ui: &mut egui::Ui, view: &RequestsView) -> Option<Command> {
    let mut cancel = None;
    for request in &view.pending {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("{}…", request.label));
            let button = ui.button("Cancel");
            super::import::mark(ui, &format!("cancel:{}", request.id), &button);
            if button.clicked() {
                cancel = Some(Command::CancelRequest { id: request.id });
            }
        });
    }
    cancel
}
