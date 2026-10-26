use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

use crate::Error;

// Returns Pending as long as its inner bool is false.
#[derive(Default)]
pub struct FrameFuture {
    pub done: bool,
}

impl Future for FrameFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context) -> Poll<Self::Output> {
        if self.done {
            // We were told to step, meaning this future gets destroyed and we run
            // the main future until we call next_frame again and end up in this poll
            // function again.
            Poll::Ready(())
        } else {
            self.done = true;
            Poll::Pending
        }
    }
}

pub struct FileLoadingFuture {
    pub contents: Arc<Mutex<Option<Result<Vec<u8>, Error>>>>,
}

impl Future for FileLoadingFuture {
    type Output = Result<Vec<u8>, Error>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context) -> Poll<Self::Output> {
        let mut contents = self
            .contents
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(contents) = contents.take() {
            Poll::Ready(contents)
        } else {
            Poll::Pending
        }
    }
}

struct UnsupportedWaker;

impl Wake for UnsupportedWaker {
    fn wake(self: Arc<Self>) {
        panic!(
            "rayquad does not support waking futures, please use coroutines, \
            otherwise your pending future will block until the next frame"
        )
    }

    fn wake_by_ref(self: &Arc<Self>) {
        panic!(
            "rayquad does not support waking futures, please use coroutines, \
            otherwise your pending future will block until the next frame"
        )
    }
}

fn waker() -> &'static Waker {
    static WAKER: OnceLock<Waker> = OnceLock::new();
    WAKER.get_or_init(|| Waker::from(Arc::new(UnsupportedWaker)))
}

/// returns Some(T) if future is done, None if it would block
pub(crate) fn resume<T>(future: &mut Pin<Box<dyn Future<Output = T>>>) -> Option<T> {
    let mut futures_context = std::task::Context::from_waker(waker());
    match future.as_mut().poll(&mut futures_context) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_future_yields_exactly_once() {
        let mut future: Pin<Box<dyn Future<Output = u8>>> = Box::pin(async {
            FrameFuture::default().await;
            42
        });

        assert_eq!(resume(&mut future), None);
        assert_eq!(resume(&mut future), Some(42));
    }

    #[test]
    #[should_panic(expected = "rayquad does not support waking futures")]
    fn waking_remains_explicitly_unsupported() {
        struct Wakes;

        impl Future for Wakes {
            type Output = ();

            fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
                context.waker().wake_by_ref();
                Poll::Pending
            }
        }

        let mut future: Pin<Box<dyn Future<Output = ()>>> = Box::pin(Wakes);
        resume(&mut future);
    }
}
