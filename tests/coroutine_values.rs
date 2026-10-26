use rayquad::{experimental::coroutines::start_coroutine, window::next_frame};

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

    let mut coroutines = Vec::new();
    for _ in 0..20 {
        coroutines.push(start_coroutine(async move {
            next_frame().await;
        }));

        next_frame().await;
    }

    // wait for the last one to finish
    next_frame().await;

    assert!(coroutines.iter().all(|coroutine| coroutine.is_done()));
}
