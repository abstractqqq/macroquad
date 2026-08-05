use rayquad::prelude::*;

fn main() {
    rayquad::Window::new("Texture", game());
}

async fn game() {
    let texture: Texture2D = load_texture("examples/ferris.png").await.unwrap();

    loop {
        clear_background(LIGHTGRAY);
        draw_texture(&texture, 0., 0., WHITE);
        next_frame().await
    }
}
