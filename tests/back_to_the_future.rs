use std::{future::Future, task::Poll};

#[test]
#[ignore = "requires a native display"]
fn back_to_the_future() {
    rayquad::Window::new("test", back_to_the_future_async());
}

async fn back_to_the_future_async() {
    struct Kaboom;
    impl Future for Kaboom {
        type Output = ();

        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<Self::Output> {
            let cloned = cx.waker().clone(); // segmentation fault
            drop(cloned);
            Poll::Ready(())
        }
    }
    Kaboom.await;
}
