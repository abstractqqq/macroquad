use rayquad::prelude::*;

fn main() {
    rayquad::Window::new("Exit dialog", game());
}

async fn game() {
    prevent_quit();

    let mut show_exit_dialog = false;
    let mut user_decided_to_exit = false;

    loop {
        clear_background(GRAY);

        if is_quit_requested() {
            show_exit_dialog = true;
        }

        if show_exit_dialog {
            let dialog_size = vec2(360., 110.);
            let screen_size = vec2(screen_width(), screen_height());
            let dialog_position = screen_size / 2. - dialog_size / 2.;
            draw_rectangle(
                dialog_position.x,
                dialog_position.y,
                dialog_size.x,
                dialog_size.y,
                WHITE,
            );
            draw_text(
                "Do you really want to quit?",
                dialog_position.x + 20.0,
                dialog_position.y + 38.0,
                24.0,
                BLACK,
            );
            draw_text(
                "Y: yes    N or Escape: no",
                dialog_position.x + 20.0,
                dialog_position.y + 78.0,
                22.0,
                DARKGRAY,
            );
            if is_key_pressed(KeyCode::Y) {
                user_decided_to_exit = true;
            }
            if is_key_pressed(KeyCode::N) || is_key_pressed(KeyCode::Escape) {
                show_exit_dialog = false;
            }
        }

        if user_decided_to_exit {
            break;
        }

        next_frame().await
    }
}
