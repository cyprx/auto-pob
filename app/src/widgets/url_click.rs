//! PoB's Ctrl+click on a text box that holds a link (EditControl.lua:485-486). PoB opens the
//! box's whole text when it matches `[a-z]*://[^ >,;]*` anywhere, whatever the scheme. A
//! deliberate safety deviation (controller ruling, review I3): ours opens only the first
//! `http://` or `https://` URL in the text, as that URL alone; any other scheme opens nothing.
//! Notes, Config text and the Import box can come from a shared build.

use eframe::egui;

/// The first http(s) URL in `text`, up to the characters PoB's pattern stops at (` >,;`) or
/// any other whitespace
pub fn first_url(text: &str) -> Option<&str> {
    let lower = text.to_ascii_lowercase();
    let start = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| lower.find(scheme))
        .min()?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '>' | ',' | ';'))
        .unwrap_or(rest.len());
    let url = &rest[..end];
    // a scheme with nothing after it is no link
    (!url.ends_with("://")).then_some(url)
}

/// Opens the first http(s) URL of `text` when the box under `response` is pressed with Ctrl
/// held. The user's own click asked for it (R20). True when it opened.
pub fn open_on_ctrl_click(ui: &egui::Ui, response: &egui::Response, text: &str) -> bool {
    let pressed = ui.input(|i| i.pointer.primary_pressed() && i.modifiers.command);
    if !pressed || !response.contains_pointer() || !response.enabled() {
        return false;
    }
    let Some(url) = first_url(text) else {
        return false;
    };
    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    true
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
    fn ctrl_click_opens_only_the_first_http_url() {
        // the link alone, not the text around it (a safety deviation from PoB, review I3)
        assert_eq!(
            run("see https://pobb.in/abc, then x", Modifiers::COMMAND),
            ["https://pobb.in/abc"]
        );
        assert_eq!(
            run("a http://x.y/z\nhttps://later", Modifiers::COMMAND),
            ["http://x.y/z"]
        );
        assert!(run("see https://pobb.in/abc", Modifiers::NONE).is_empty());
        assert!(run("no link here", Modifiers::COMMAND).is_empty());
        // other schemes open nothing
        for text in [
            "file:///etc/passwd",
            "javascript://x",
            "://x",
            "steam://run/1",
        ] {
            assert!(run(text, Modifiers::COMMAND).is_empty(), "{text}");
        }
        assert_eq!(first_url("https://"), None);
    }
}
