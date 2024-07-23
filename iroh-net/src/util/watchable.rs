//! Watchable values.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock, RwLockReadGuard,
    },
    task::{self, Poll, Waker},
};

use futures_lite::stream::Stream;

#[derive(Debug)]
struct Shared<T> {
    value: RwLock<T>,
    epoch: AtomicU64,
    watchers: RwLock<VecDeque<Waker>>,
}

impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Shared {
            value: Default::default(),
            epoch: INITIAL_EPOCH.into(),
            watchers: Default::default(),
        }
    }
}

/// A value, whos changes can be observed.
#[derive(Debug, Default)]
pub struct Watchable<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for Watchable<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// The watcher watching the watchable.
#[derive(Debug, Clone)]
pub struct Watcher<T> {
    last_epoch: u64,
    shared: Arc<Shared<T>>,
}

const INITIAL_EPOCH: u64 = 1;

impl<T: Clone + Eq> Watchable<T> {
    /// Creates a new entity, with the provided initial value.
    pub fn new(value: T) -> Self {
        let shared = Shared {
            value: RwLock::new(value),
            epoch: AtomicU64::new(INITIAL_EPOCH),
            watchers: Default::default(),
        };
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Set the current value.
    ///
    /// If the value changed, returns the old value, otherwise `None`.
    /// Watchers are only notified if the value is changed.
    pub fn set(&self, value: T) -> Option<T> {
        if value == *self.shared.value.read().unwrap() {
            return None;
        }
        let old = std::mem::replace(&mut *self.shared.value.write().unwrap(), value);
        self.shared.epoch.fetch_add(1, Ordering::SeqCst);
        for watcher in self.shared.watchers.write().unwrap().drain(..) {
            watcher.wake();
        }
        Some(old)
    }

    /// Creates a watcher that yield new values only.
    pub fn watch(&self) -> Watcher<T> {
        Watcher {
            last_epoch: self.shared.epoch.load(Ordering::SeqCst),
            shared: Arc::clone(&self.shared),
        }
    }

    /// Creates a watcher, that will yield the initial value immediately
    /// and then new values.
    pub fn watch_initial(&self) -> Watcher<T> {
        let last_epoch = self.shared.epoch.load(Ordering::SeqCst);
        debug_assert!(last_epoch > 0);
        Watcher {
            last_epoch: last_epoch - 1,
            shared: Arc::clone(&self.shared),
        }
    }

    /// Returns a reference to the currently held value.
    pub fn get(&self) -> RwLockReadGuard<'_, T> {
        self.shared.value.read().unwrap()
    }
}

impl<T> Watcher<T> {
    /// Returns a reference to the currently held value.
    pub fn get(&self) -> RwLockReadGuard<'_, T> {
        self.shared.value.read().unwrap()
    }
}

impl<T: Clone + Eq> Future for Watcher<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let epoch = self.shared.epoch.load(Ordering::SeqCst);
        if self.last_epoch == epoch {
            self.shared
                .watchers
                .write()
                .unwrap()
                .push_back(cx.waker().to_owned());
            Poll::Pending
        } else {
            self.as_mut().last_epoch = epoch;
            Poll::Ready(self.shared.value.read().unwrap().clone())
        }
    }
}

impl<T: Clone + Eq> Stream for Watcher<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll(cx).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, Instant};

    use futures_lite::StreamExt;
    use rand::{thread_rng, Rng};
    use tokio::task::JoinSet;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn test_watcher() {
        let cancel = CancellationToken::new();
        let watchable = Watchable::new(17);

        assert_eq!(watchable.watch_initial().next().await.unwrap(), 17);

        let start = Instant::now();
        // spawn watchers
        let mut tasks = JoinSet::new();
        for i in 0..3 {
            let mut watch = watchable.watch();
            let cancel = cancel.clone();
            tasks.spawn(async move {
                println!("[{i}] spawn");
                let mut expected_value = 0;
                loop {
                    tokio::select! {
                        biased;
                        value = &mut watch => {
                            println!("{:?} [{i}] update: {value}", start.elapsed());
                            assert_eq!(value, expected_value);
                            expected_value += 1;
                        },
                        _ = cancel.cancelled() => {
                            println!("{:?} [{i}] cancel", start.elapsed());
                            assert_eq!(expected_value, 10);
                            break;
                        }
                    }
                }
            });
        }
        for i in 0..3 {
            let mut watch = watchable.watch();
            let cancel = cancel.clone();
            tasks.spawn(async move {
                println!("[{i}] spawn");
                let mut expected_value = 0;
                loop {
                    tokio::select! {
                        biased;
                        Some(value) = watch.next() => {
                            println!("{:?} [{i}] stream update: {value}", start.elapsed());
                            assert_eq!(value, expected_value);
                            expected_value += 1;
                        },
                        _ = cancel.cancelled() => {
                            println!("{:?} [{i}] cancel", start.elapsed());
                            assert_eq!(expected_value, 10);
                            break;
                        }
                        else => {
                            panic!("stream died");
                        }
                    }
                }
            });
        }

        // set value
        for next_value in 0..10 {
            let sleep = Duration::from_nanos(thread_rng().gen_range(0..100_000_000));
            println!("{:?} sleep {sleep:?}", start.elapsed());
            tokio::time::sleep(sleep).await;

            let changed = watchable.set(next_value);
            println!("{:?} set {next_value} changed={changed:?}", start.elapsed());
        }

        println!("cancel");
        cancel.cancel();
        while let Some(res) = tasks.join_next().await {
            res.expect("task failed");
        }
    }
}
