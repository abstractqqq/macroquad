//! Renders a fixed scene into a render target and prints a hash of the pixels.
//! Used to check that draw-call submission changes do not change the output.
//!
//! Run with:
//!     cargo run --release --example drawcall_check -- [capacity]

use rayquad::prelude::*;
use rayquad::window::miniquad;

fn capacity() -> usize {
    std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000)
}

fn window_conf() -> rayquad::conf::Conf {
    let capacity = capacity();
    rayquad::conf::Conf {
        miniquad_conf: miniquad::conf::Conf {
            window_title: "quad_gl draw-call check".to_owned(),
            window_width: 320,
            window_height: 240,
            ..Default::default()
        },
        draw_call_vertex_capacity: capacity,
        draw_call_index_capacity: capacity * 3 / 2,
        ..Default::default()
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn scene(red: &Texture2D, blue: &Texture2D, inner: &RenderTarget) {
    // Many quads alternating textures: one draw call each.
    for i in 0..400 {
        let texture = if i % 3 == 0 { red } else { blue };
        draw_texture_ex(
            texture,
            (i % 40) as f32 * 6.0,
            (i / 40) as f32 * 6.0,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(5.0, 5.0)),
                ..Default::default()
            },
        );
    }
    // One long batch that exceeds small draw-call capacities.
    for i in 0..2000 {
        let x = (i % 100) as f32 * 2.5;
        let y = 70.0 + (i / 100) as f32 * 2.5;
        draw_rectangle(x, y, 2.0, 2.0, Color::from_rgba((i % 255) as u8, 80, 160, 255));
    }
    draw_circle(200.0, 150.0, 30.0, YELLOW);
    draw_circle_lines(120.0, 160.0, 25.0, 2.0, GREEN);
    draw_line(0.0, 0.0, 256.0, 256.0, 3.0, ORANGE);
    draw_rectangle_lines(10.0, 180.0, 100.0, 40.0, 2.0, PINK);
    draw_text("draw call check", 10.0, 240.0, 24.0, WHITE);

    // Nested render target, then draw it into the outer one.
    set_camera(&Camera2D {
        render_target: Some(inner.clone()),
        ..Camera2D::from_display_rect(Rect::new(0.0, 0.0, 64.0, 64.0))
    });
    clear_background(DARKBLUE);
    draw_circle(32.0, 32.0, 20.0, RED);
    draw_triangle(vec2(0.0, 0.0), vec2(64.0, 0.0), vec2(32.0, 50.0), LIME);
}

fn main() {
    rayquad::Window::from_config(window_conf(), check());
}

async fn check() {
    let red = Texture2D::from_image(&Image::gen_image_color(1, 1, RED));
    let blue = Texture2D::from_image(&Image::gen_image_color(1, 1, BLUE));
    let outer = render_target(256, 256);
    let inner = render_target(64, 64);
    outer.texture.set_filter(FilterMode::Nearest);
    inner.texture.set_filter(FilterMode::Nearest);

    let mut hashes = Vec::new();
    for _ in 0..3 {
        let image = render_to_image(
            &outer,
            Camera2D::from_display_rect(Rect::new(0.0, 0.0, 256.0, 256.0)),
            || {
                clear_background(BLACK);
                push_camera_state();
                scene(&red, &blue, &inner);
                pop_camera_state();
                draw_texture(&inner.texture, 180.0, 180.0, WHITE);
            },
        );
        hashes.push(fnv1a(&image.bytes));
        next_frame().await;
    }
    println!("capacity={} hashes={:016x?}", capacity(), hashes);
    miniquad::window::quit();
}
