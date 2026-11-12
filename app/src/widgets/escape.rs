//! PoB's popups close on Esc (PopupDialog.lua:72-77), but only once no control has the key:
//! a selected edit box takes the first Esc and only leaves the box (ControlHost.lua:35-42,
//! EditControl.lua:505), and a dropped list takes it to close itself. egui drops a box's
//! focus as the frame with the Esc begins, so whether a box had the keyboard is what the
//! window's last frame ended with: [`note`] records it after the window is drawn, and
//! [`escape`], before the window's contents are drawn, reads it.
//!
//! An outside click does nothing to a PoB popup, so nothing here closes on one.

use eframe::egui::{self, Key, Modifiers};

fn typing_id(window: egui::Id) -> egui::Id {
    window.with("pob_escape_typing")
}

/// Whether Esc closes `window` this frame. Called before the window's contents are drawn.
/// An Esc a box or a dropped list takes is left alone for it; an Esc that only left a box is
/// consumed, so nothing else closes on it either.
pub fn escape(ctx: &egui::Context, window: egui::Id) -> bool {
    if egui::Popup::is_any_open(ctx) {
        return false;
    }
    let now = ctx.cumulative_pass_nr();
    // only the frame before counts: a window closed while typing and opened later starts afresh
    let typed = ctx
        .data(|d| d.get_temp::<(u64, bool)>(typing_id(window)))
        .is_some_and(|(pass, typing)| typing && pass + 1 == now);
    let pressed = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    pressed && !typed
}

/// Records, once `window` is drawn, whether a text field has the keyboard.
pub fn note(ctx: &egui::Context, window: egui::Id) {
    let entry = (ctx.cumulative_pass_nr(), ctx.text_edit_focused());
    ctx.data_mut(|d| d.insert_temp(typing_id(window), entry));
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, RawInput};

    fn esc() -> Vec<Event> {
        vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]
    }

    /// One frame of a window with a box; returns whether Esc closed it
    fn frame(ctx: &egui::Context, text: &mut String, focus: bool, events: Vec<Event>) -> bool {
        let id = egui::Id::new("window");
        let mut closed = false;
        ctx.run_ui(
            RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                closed = escape(ui.ctx(), id);
                let edit = ui.text_edit_singleline(text);
                if focus {
                    edit.request_focus();
                }
                note(ui.ctx(), id);
            },
        )
        .drop_without_applying_deltas();
        closed
    }

    #[test]
    fn the_first_esc_leaves_the_box_and_the_second_closes() {
        let ctx = egui::Context::default();
        let mut text = String::from("typed");
        assert!(!frame(&ctx, &mut text, true, vec![]));
        assert!(ctx.text_edit_focused());
        // the box takes the first: it only leaves the box
        assert!(!frame(&ctx, &mut text, false, esc()));
        assert!(!ctx.text_edit_focused());
        assert_eq!(text, "typed");
        // nothing has the key now: the window closes
        assert!(frame(&ctx, &mut text, false, esc()));
    }
}
