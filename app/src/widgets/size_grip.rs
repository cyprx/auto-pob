//! PoB's ResizableEditControl grip (ResizableEditControl.lua:16-45): the "//" corner of a
//! box. Dragging it sets the box's height between its least and most; a right-click puts the
//! box at its least, or at its most when it is at its least already.

use eframe::egui;

/// The grip's side, as PoB's DraggerControl (14 by 14)
pub const GRIP: f32 = 14.0;

/// Draws the grip on the bottom-right corner of `rect` (the box) and applies its drag or
/// right-click to `height`. Returns the grip's response.
pub fn size_grip(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    height: &mut f32,
    (min, max): (f32, f32),
) -> egui::Response {
    let grip = egui::Rect::from_min_size(
        rect.right_bottom() - egui::vec2(GRIP, GRIP),
        egui::vec2(GRIP, GRIP),
    );
    let response = ui
        .interact(grip, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    ui.painter().text(
        grip.center(),
        egui::Align2::CENTER_CENTER,
        "//",
        egui::FontId::monospace(10.0),
        ui.visuals().weak_text_color(),
    );
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        // SetBoundedDrag: the cursor's distance from the box's top
        *height = (pointer.y - rect.top()).clamp(min, max);
    }
    if response.secondary_clicked() {
        *height = if *height != min { min } else { max };
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    fn frame(ctx: &egui::Context, height: &mut f32, events: Vec<Event>) -> Rect {
        let mut grip = Rect::NOTHING;
        ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(344.0, *height));
                grip = size_grip(ui, ui.make_persistent_id("g"), rect, height, (40.0, 600.0)).rect;
            },
        )
        .drop_without_applying_deltas();
        grip
    }

    fn click(button: PointerButton, pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn right_click_toggles_least_and_most_and_drag_resizes() {
        let ctx = egui::Context::default();
        let mut height = 80.0;
        let grip = frame(&ctx, &mut height, vec![]).center();
        let right = |h: &mut f32| {
            frame(&ctx, h, vec![Event::PointerMoved(grip)]);
            frame(&ctx, h, vec![click(PointerButton::Secondary, grip, true)]);
            frame(&ctx, h, vec![click(PointerButton::Secondary, grip, false)]);
        };
        right(&mut height);
        assert_eq!(height, 40.0);
        let grip = frame(&ctx, &mut height, vec![]).center();
        frame(&ctx, &mut height, vec![Event::PointerMoved(grip)]);
        frame(
            &ctx,
            &mut height,
            vec![click(PointerButton::Secondary, grip, true)],
        );
        frame(
            &ctx,
            &mut height,
            vec![click(PointerButton::Secondary, grip, false)],
        );
        assert_eq!(height, 600.0);
        // a drag puts the bottom under the pointer, within the bounds
        let grip = frame(&ctx, &mut height, vec![]).center();
        frame(&ctx, &mut height, vec![Event::PointerMoved(grip)]);
        frame(
            &ctx,
            &mut height,
            vec![click(PointerButton::Primary, grip, true)],
        );
        for y in [400.0, 210.0] {
            frame(
                &ctx,
                &mut height,
                vec![Event::PointerMoved(Pos2::new(grip.x, y))],
            );
        }
        frame(
            &ctx,
            &mut height,
            vec![click(
                PointerButton::Primary,
                Pos2::new(grip.x, 210.0),
                false,
            )],
        );
        assert_eq!(height, 200.0);
        let _ = right;
    }
}
