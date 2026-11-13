//! PoB's Ctrl+click on a text box that holds a link: the box opens its whole text as a URL
//! instead of placing the caret (EditControl.lua:485-486, `[a-z]*://[^ >,;]*` anywhere in it).

use eframe::egui;

/// Whether PoB's box would open `text` on a Ctrl+click: it has `://` somewhere, which its
/// pattern matches (the scheme part may be empty)
pub fn holds_url(text: &str) -> bool {
    text.contains("://")
}

/// Opens `text` as PoB does when the box under `response` is pressed with Ctrl held. The user's
/// own click asked for it (R20). True when it opened.
pub fn open_on_ctrl_click(ui: &egui::Ui, response: &egui::Response, text: &str) -> bool {
    let pressed = ui.input(|i| i.pointer.primary_pressed() && i.modifiers.command);
    if pressed && response.contains_pointer() && response.enabled() && holds_url(text) {
        ui.ctx().open_url(egui::OpenUrl::new_tab(text));
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    fn run(text: &str, modifiers: Modifiers) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let mut opened = Vec::new();
        let mut events = vec![];
        for step in 0..2 {
            let mut buf = text.to_owned();
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                    events: std::mem::take(&mut events),
                    ..Default::default()
                },
                |ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut buf));
                    rect = r.rect;
                    open_on_ctrl_click(ui, &r, &buf);
                },
            );
            let commands = out.platform_output.commands.clone();
            out.drop_without_applying_deltas();
            for command in commands {
                if let egui::OutputCommand::OpenUrl(url) = command {
                    opened.push(url.url);
                }
            }
            if step == 0 {
                let pos = rect.center();
                events = vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    },
                ];
            }
        }
        opened
    }

    #[test]
    fn ctrl_click_opens_a_box_that_holds_a_link() {
        let link = "see https://pobb.in/abc";
        // PoB opens the whole text, not only the link part
        assert_eq!(run(link, Modifiers::COMMAND), [link]);
        assert!(run(link, Modifiers::NONE).is_empty());
        assert!(run("no link here", Modifiers::COMMAND).is_empty());
        assert!(holds_url("://x"), "PoB's scheme part may be empty");
    }
}
