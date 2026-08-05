//! RayQuad exposes Miniquad's logging macros.
//! They use the Android console or standard output depending on the platform.
//! Those macros are the recommended way to output debug traces and logs.

use rayquad::prelude::*;

fn main() {
    rayquad::Window::new("Logs", game());
}

async fn game() {
    debug!("This is a debug message");
    info!("and info message");
    error!("and errors, the red ones!");
    warn!("Or warnings, the yellow ones.");

    loop {
        clear_background(LIGHTGRAY);

        debug!("Still alive!");

        next_frame().await
    }
}
