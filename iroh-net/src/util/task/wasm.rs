use std::{
    fmt::Debug,
    future::{Future, IntoFuture},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Waker},
};

use futures_lite::{stream::StreamExt, Stream};
use wasm_bindgen::prelude::wasm_bindgen;

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    let handle = JoinHandle::new();

    wasm_bindgen_futures::spawn_local(SpawnFuture {
        handle: handle.clone(),
        fut: future,
    });

    handle
}

#[derive(Debug)]
pub struct JoinHandle<T> {
    cancelled: Arc<AtomicBool>,
    result: Arc<Mutex<Option<T>>>,
    handle_waker: Arc<Mutex<Option<Waker>>>,
    spawn_waker: Arc<Mutex<Option<Waker>>>,
}

impl<T> Clone for JoinHandle<T> {
    fn clone(&self) -> Self {
        Self {
            cancelled: self.cancelled.clone(),
            result: self.result.clone(),
            handle_waker: self.handle_waker.clone(),
            spawn_waker: self.spawn_waker.clone(),
        }
    }
}

impl<T> JoinHandle<T> {
    pub fn new() -> Self {
        JoinHandle {
            cancelled: Arc::new(AtomicBool::new(false)),
            result: Arc::new(Mutex::new(None)),
            handle_waker: Arc::new(Mutex::new(None)),
            spawn_waker: Arc::new(Mutex::new(None)),
        }
    }

    pub fn abort(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.wake();
    }

    pub fn is_finished(&self) -> bool {
        self.result.lock().expect("lock poinsoned").is_some()
    }

    fn register_handle_waker(&self, cx: &mut Context<'_>) {
        let mut guard = self.handle_waker.lock().expect("lock poisoned");
        *guard = Some(cx.waker().clone());
    }

    fn register_spawn_waker(&self, cx: &mut Context<'_>) {
        let mut guard = self.spawn_waker.lock().expect("lock poisoned");
        *guard = Some(cx.waker().clone());
    }

    fn wake(&self) {
        if let Some(waker) = self.handle_waker.lock().expect("lock poisoned").take() {
            waker.wake();
        }
        if let Some(waker) = self.spawn_waker.lock().expect("lock poisoned").take() {
            waker.wake();
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum JoinError {
    Cancelled,
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(Err(JoinError::Cancelled));
        }

        if let Some(result) = self.result.lock().expect("lock poisoned").take() {
            return Poll::Ready(Ok(result));
        }

        self.register_handle_waker(cx);
        Poll::Pending
    }
}

pub struct JoinSet<T> {
    handles: Vec<JoinHandle<T>>,
}

impl<T> JoinSet<T> {
    fn poll_next_with_index(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<(usize, Result<T, JoinError>)> {
        for (idx, handle) in self.handles.iter_mut().enumerate() {
            match Pin::new(handle).poll(cx) {
                Poll::Ready(result) => return Poll::Ready((idx, result)),
                Poll::Pending => {}
            }
        }

        Poll::Pending
    }
}

impl<T> Stream for JoinSet<T> {
    type Item = Result<T, JoinError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<T, JoinError>>> {
        if self.handles.is_empty() {
            return Poll::Ready(None);
        }

        match self.poll_next_with_index(cx) {
            Poll::Ready((idx, result)) => {
                self.handles.remove(idx);
                Poll::Ready(Some(result))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> JoinSet<T> {
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    /// Returns the number of tasks currently in the `JoinSet`.
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// Returns whether the `JoinSet` is empty.
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    pub fn spawn(&mut self, fut: impl Future<Output = T> + 'static) -> JoinHandle<T>
    where
        T: 'static,
    {
        let handle = spawn(fut);
        self.handles.push(handle.clone());
        handle
    }

    pub fn abort_all(&self) {
        for handle in self.handles.iter() {
            handle.abort();
        }
    }

    pub async fn join_next(&mut self) -> Option<Result<T, JoinError>> {
        self.next().await
    }
}

impl<T> Drop for JoinSet<T> {
    fn drop(&mut self) {
        self.abort_all()
    }
}

// Private:

#[pin_project::pin_project]
struct SpawnFuture<Fut: Future<Output = T>, T> {
    handle: JoinHandle<T>,
    #[pin]
    fut: Fut,
}

impl<Fut: Future<Output = T>, T> Future for SpawnFuture<Fut, T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let handle = self.handle.clone();
        if self.handle.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(());
        }

        match self.project().fut.poll(cx) {
            Poll::Ready(value) => {
                *handle.result.lock().expect("lock poisoned") = Some(value);
                handle.wake();
                Poll::Ready(())
            }
            Poll::Pending => {
                handle.register_spawn_waker(cx);
                Poll::Pending
            }
        }
    }
}
