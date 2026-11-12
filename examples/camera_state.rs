//! Self-checking test for camera state handling. Prints PASS/FAIL and exits with a
//! matching status code.
//!
//! Frame 0 deliberately leaves a render-target camera active at the end of the frame.
//! Frame 1 then checks that:
//!   * the previous camera did not leak: drawing with no camera set goes to the screen,
//!     not into the render target that was left active;
//!   * `pop_camera_state` restores the viewport: a rect drawn after the pop lands in the
//!     left viewport, not the whole render target.
//!
//! Results are read back from render targets, so it does not depend on the screen.

use rayquad::prelude::*;

fn main() {
    rayquad::Window::new("Camera state", game());
}

fn near(c: Color, expected: Color) -> bool {
    [c.r - expected.r, c.g - expected.g, c.b - expected.b]
        .iter()
        .all(|d| d.abs() < 0.02)
}

fn target_camera(rt: &RenderTarget, viewport: Option<(i32, i32, i32, i32)>) -> Camera2D {
    Camera2D {
        render_target: Some(rt.clone()),
        viewport,
        ..Default::default()
    }
}

async fn game() {
    let leak_rt = render_target(64, 64);
    let split_rt = render_target(200, 100);
    let mut frame = 0;
    let mut all_failures: Vec<&str> = vec![];

    loop {
        clear_background(LIGHTGRAY);

        if frame == 0 {
            // Leave a camera active on purpose.
            set_camera(&target_camera(&leak_rt, None));
            draw_rectangle(-1., -1., 2., 2., RED);
        } else if frame == 1 {
            let mut failures: Vec<&str> = vec![];

            // Drawn with no camera set. If frame 0's camera leaked, this ends up in leak_rt.
            draw_rectangle(0., 0., 50., 50., MAGENTA);

            let left = target_camera(&split_rt, Some((0, 0, 100, 100)));
            let right = target_camera(&split_rt, Some((100, 0, 100, 100)));
            set_camera(&left);
            draw_rectangle(-1., -1., 2., 2., RED);
            push_camera_state();
            set_camera(&right);
            draw_rectangle(-1., -1., 2., 2., BLUE);
            pop_camera_state();
            // Small green rect in the centre of the *left* viewport.
            draw_rectangle(-0.25, -0.25, 0.5, 0.5, GREEN);
            set_default_camera(); // flushes

            // world_to_screen / screen_to_world round trip, with rotation, zoom and a viewport.
            let cam = Camera2D {
                rotation: 30.,
                zoom: vec2(0.004, 0.003),
                target: vec2(100., -40.),
                offset: vec2(0.1, 0.2),
                viewport: Some((50, 40, 300, 200)),
                ..Default::default()
            };
            for p in [vec2(0., 0.), vec2(250., 120.), vec2(-80., 60.)] {
                let back = cam.screen_to_world(cam.world_to_screen(p));
                if (back - p).length() > 0.05 {
                    failures.push("world_to_screen / screen_to_world do not round trip");
                }
            }

            let leak = leak_rt.texture.get_texture_data();
            if leak.get_image_data().iter().any(|p| p[0] > 250 && p[1] < 5 && p[2] > 250) {
                failures.push("camera leaked across frames: default drawing hit old target");
            }

            let split = split_rt.texture.get_texture_data();
            if !near(split.get_pixel(50, 50), GREEN) {
                failures.push("green rect missing from centre of left viewport");
            }
            if !near(split.get_pixel(110, 50), BLUE) {
                failures.push("viewport not restored after pop: rect leaked into right half");
            }
            if !near(split.get_pixel(150, 50), BLUE) {
                failures.push("right viewport not blue");
            }

            all_failures = failures;
        }

        // Regression: get_screen_data() used to leave a stale texture binding in the cache,
        // so everything drawn with the white texture was black on the next frame.
        // (The very first read at frame 1 is skipped: it is occasionally black at startup.)
        if frame >= 2 {
            draw_rectangle(0., 0., 50., 50., MAGENTA);
            let img = get_screen_data();
            if frame == 3 {
                let magenta = img.get_image_data().iter().filter(|p| **p == [255, 0, 255, 255]).count();
                if magenta != 2500 {
                    all_failures.push("get_screen_data corrupted texture state for later frames");
                }
                if all_failures.is_empty() {
                    println!("PASS");
                    std::process::exit(0);
                }
                for f in &all_failures {
                    println!("FAIL: {f}");
                }
                std::process::exit(1);
            }
        }

        frame += 1;
        next_frame().await
    }
}
