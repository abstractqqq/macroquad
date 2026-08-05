use rayquad::prelude::*;

fn main() {
    rayquad::Window::new("Events", game());
}

async fn game() {
    loop {
        clear_background(WHITE);
        let (mouse_x, mouse_y) = mouse_position();
        let (wheel_x, wheel_y) = mouse_wheel();
        let key = last_key_pressed().map_or_else(|| "None".to_owned(), |key| format!("{key:?}"));
        let buttons = [MouseButton::Left, MouseButton::Right, MouseButton::Middle]
            .into_iter()
            .filter(|button| is_mouse_button_down(*button))
            .map(|button| format!("{button:?}"))
            .collect::<Vec<_>>()
            .join(", ");

        for (line, y) in [
            (format!("Mouse position: {mouse_x:.1}, {mouse_y:.1}"), 40.0),
            (format!("Mouse wheel: {wheel_x:.1}, {wheel_y:.1}"), 75.0),
            (format!("Last key pressed: {key}"), 110.0),
            (format!("Mouse buttons down: {buttons}"), 145.0),
        ] {
            draw_text(&line, 20.0, y, 28.0, DARKGRAY);
        }
        next_frame().await;
    }
}
