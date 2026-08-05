use rayquad::{
    experimental::{
        coroutines::{start_coroutine, wait_seconds},
        scene,
    },
    window::next_frame,
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
    struct Player {
        state: i32,
    }
    impl scene::Node for Player {}

    let player = scene::add_node(Player { state: 0 });

    let mut coroutine = start_coroutine(async move {
        loop {
            scene::get_node(player).state += 1;
            next_frame().await;
        }
    });

    // make sure that coroutine is not yet polled
    assert_eq!(scene::get_node(player).state, 0);

    coroutine.set_manual_poll();

    // still not polled
    assert_eq!(scene::get_node(player).state, 0);

    coroutine.poll(0.1);
    assert_eq!(scene::get_node(player).state, 1);

    next_frame().await;
    next_frame().await;

    // make sure that after main loop's next_frame coroutine was not polled
    assert_eq!(scene::get_node(player).state, 1);

    // and that we still can poll
    coroutine.poll(0.1);
    assert_eq!(scene::get_node(player).state, 2);
}

#[test]
#[ignore = "requires a native display"]
fn coroutine_manual_poll_delay() {
    rayquad::Window::new("test", coroutine_manual_poll_delay_async());
}

async fn coroutine_manual_poll_delay_async() {
    struct Player {
        state: i32,
    }
    impl scene::Node for Player {}

    let player = scene::add_node(Player { state: 0 });

    let mut coroutine = start_coroutine(async move {
        wait_seconds(1.).await;
        scene::get_node(player).state = 1;
    });

    coroutine.set_manual_poll();

    assert_eq!(scene::get_node(player).state, 0);

    // not 1 second yet, coroutine will have "now": 0.0, "delta": 0.9, (0.0 + 0.9) < 1.0
    coroutine.poll(0.9);

    assert_eq!(scene::get_node(player).state, 0);

    // coroutine will have "now": 0.1, delta: 0.11, (0.9 + 0.11) > 1.0, wait_for_seconds pass
    coroutine.poll(0.11);

    assert_eq!(scene::get_node(player).state, 1);
}
