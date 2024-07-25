use core::future::Future;
use futures_lite::stream::Stream;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pub use gloo_timers::future::IntervalStream;
pub use web_time::{Duration, Instant, SystemTime};

/// Errors returned by `Timeout`.
///
/// This error is returned when a timeout expires before the function was able
/// to finish.
#[derive(Debug, PartialEq, Eq)]
pub struct Elapsed(());
impl std::error::Error for Elapsed {}
impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Elapsed")
    }
}

/// TODO(matheus23): DOCS
pub async fn timeout<T, F>(delay: Duration, fut: F) -> Result<T, Elapsed>
where
    F: std::future::Future<Output = T>,
{
    let mut sleep = sleep(delay);
    tokio::select! {
        _ = &mut sleep => {
            Err(Elapsed(()))
        }
        res = fut => {
            Ok(res)
        }
    }
}

/// TODO(matheus23): DOCS
#[derive(Debug)]
#[must_use = "streams do nothing unless polled or spawned"]
#[pin_project::pin_project]
pub enum Interval {
    /// TODO(matheus23): DOCS
    InOffset(#[pin] gloo_timers::future::TimeoutFuture, u32),
    /// TODO(matheus23): DOCS
    InStream(#[pin] IntervalStream),
}

impl Stream for Interval {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match *self {
            Interval::InOffset(ref mut offset, interval) => match Pin::new(offset).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(()) => {
                    *self.as_mut() = Interval::InStream(IntervalStream::new(interval));
                    Poll::Pending
                }
            },
            Interval::InStream(ref mut stream) => Pin::new(stream).poll_next(cx),
        }
    }
}

/// TODO(matheus23): DOCS
pub fn interval(dur: Duration) -> Interval {
    let interval = u32::try_from(dur.as_millis()).expect("interval too large");
    Interval::InStream(IntervalStream::new(interval))
}

/// TODO(matheus23): DOCS
pub fn interval_at(start: Instant, dur: Duration) -> Interval {
    let interval = u32::try_from(dur.as_millis()).expect("interval too large");
    let offset = start.duration_since(Instant::now());
    Interval::InOffset(gloo_timers::future::sleep(offset), interval)
}

/// TODO(matheus23): DOCS
#[derive(Debug)]
#[pin_project::pin_project]
pub struct Sleep(#[pin] gloo_timers::future::TimeoutFuture);

/// TODO(matheus23): DOCS
pub fn sleep(duration: Duration) -> Sleep {
    Sleep(gloo_timers::future::sleep(duration))
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.project().0.poll(cx)
    }
}

impl Sleep {
    /// TODO(matheus23): DOCS
    pub fn reset(mut self: Pin<&mut Self>, deadline: Instant) {
        let duration = deadline.saturating_duration_since(Instant::now());
        self.set(sleep(duration));
    }
}
