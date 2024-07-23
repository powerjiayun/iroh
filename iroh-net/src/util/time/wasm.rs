use core::future::Future;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pub use gloo_timers::future::IntervalStream as Interval;

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

pub async fn timeout<T, F>(delay: std::time::Duration, fut: F) -> Result<T, Elapsed>
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

pub fn interval(dur: std::time::Duration) -> Interval {
    Interval::new(u32::try_from(dur.as_millis()).expect("interval too large"))
}

pub fn interval_at(start: Instant, dur: std::time::Duration) -> Interval {
    todo!()
}

#[pin_project::pin_project]
pub struct Sleep(#[pin] gloo_timers::future::TimeoutFuture);

pub fn sleep(duration: std::time::Duration) -> Sleep {
    Sleep(gloo_timers::future::sleep(duration))
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.project().0.poll(cx)
    }
}

impl Sleep {
    pub fn reset(mut self: Pin<&mut Self>, deadline: Instant) {
        let duration = deadline.saturating_duration_since(Instant::now());
        self.set(sleep(duration));
    }
}
