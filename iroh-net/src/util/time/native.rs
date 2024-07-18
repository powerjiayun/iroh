pub use tokio::time::{sleep, timeout, Duration, Instant};
pub use tokio_stream::wrappers::IntervalStream as Interval;

pub fn interval(dur: std::time::Duration) -> Interval {
    let interval = tokio::time::interval(dur);
    Interval::new(interval)
}

pub fn interval_at(start: Instant, dur: std::time::Duration) -> Interval {
    let interval = tokio::time::interval_at(start, dur);
    Interval::new(interval)
}
