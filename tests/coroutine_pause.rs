use rayquad::{
    experimental::coroutines::{start_coroutine, wait_seconds},
    window::next_frame,
};
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};

#[test]
#[ignore = "requires a native display"]
fn coroutine_execution_order() {
    rayquad::Window::new("test", coroutine_execution_order_async());
}

async fn coroutine_execution_order_async() {
    start_coroutine(async move {
        println!("a");
        next_frame().await;
        println!("b");
    });
    println!("c");
    next_frame().await;
    println!("d");
    next_frame().await;
}

#[test]
#[ignore = "requires a native display"]
fn coroutine_manual_poll() {
    rayquad::Window::new("test", coroutine_manual_poll_async());
}

async fn coroutine_manual_poll_async() {
    let state = Arc::new(AtomicI32::new(0));
    let coroutine_state = state.clone();

    let mut coroutine = start_coroutine(async move {
        loop {
            coroutine_state.fetch_add(1, Ordering::Relaxed);
            next_frame().await;
        }
    });

    // make sure that coroutine is not yet polled
    assert_eq!(state.load(Ordering::Relaxed), 0);

    coroutine.set_manual_poll();

    // still not polled
    assert_eq!(state.load(Ordering::Relaxed), 0);

    coroutine.poll(0.1);
    assert_eq!(state.load(Ordering::Relaxed), 1);

    next_frame().await;
    next_frame().await;

    // make sure that after main loop's next_frame coroutine was not polled
    assert_eq!(state.load(Ordering::Relaxed), 1);

    // and that we still can poll
    coroutine.poll(0.1);
    assert_eq!(state.load(Ordering::Relaxed), 2);
}

#[test]
#[ignore = "requires a native display"]
fn coroutine_manual_poll_delay() {
    rayquad::Window::new("test", coroutine_manual_poll_delay_async());
}

async fn coroutine_manual_poll_delay_async() {
    let state = Arc::new(AtomicI32::new(0));
    let coroutine_state = state.clone();

    let mut coroutine = start_coroutine(async move {
        wait_seconds(1.).await;
        coroutine_state.store(1, Ordering::Relaxed);
    });

    coroutine.set_manual_poll();

    assert_eq!(state.load(Ordering::Relaxed), 0);

    // not 1 second yet, coroutine will have "now": 0.0, "delta": 0.9, (0.0 + 0.9) < 1.0
    coroutine.poll(0.9);

    assert_eq!(state.load(Ordering::Relaxed), 0);

    // coroutine will have "now": 0.1, delta: 0.11, (0.9 + 0.11) > 1.0, wait_for_seconds pass
    coroutine.poll(0.11);

    assert_eq!(state.load(Ordering::Relaxed), 1);
}
