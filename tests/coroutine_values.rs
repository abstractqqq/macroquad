use rayquad::{experimental::coroutines::start_coroutine, telemetry, window::next_frame};

#[test]
#[ignore = "requires a native display"]
fn coroutine_value() {
    rayquad::Window::new("test", coroutine_value_async());
}

async fn coroutine_value_async() {
    let mut coroutine = start_coroutine(async move {
        next_frame().await;
        1
    });

    coroutine.set_manual_poll();

    assert_eq!(coroutine.retrieve(), None);

    coroutine.poll(0.0);
    coroutine.poll(0.0);

    assert_eq!(coroutine.retrieve(), Some(1));
}

#[test]
#[ignore = "requires a native display"]
fn coroutine_memory() {
    rayquad::Window::new("test", coroutine_memory_async());
}

async fn coroutine_memory_async() {
    use rayquad::prelude::*;

    for _ in 0..20 {
        start_coroutine(async move {
            next_frame().await;
        });

        next_frame().await;
    }

    // wait for the last one to finish
    next_frame().await;

    assert_eq!(telemetry::active_coroutines_count(), 0);
}
