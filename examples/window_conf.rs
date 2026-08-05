use rayquad::prelude::*;

fn window_conf() -> Conf {
    Conf {
        window_title: "Window Conf".to_owned(),
        fullscreen: true,
        platform: miniquad::conf::Platform {
            linux_backend: miniquad::conf::LinuxBackend::WaylandOnly,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn main() {
    rayquad::Window::from_config(window_conf(), game());
}

async fn game() {
    loop {
        clear_background(WHITE);
        next_frame().await
    }
}
