//! In-process task drain for fire-and-forget work.
//!
//! [`BackgroundQueue`] accepts async tasks, schedules them on the Tokio
//! runtime with a bounded concurrency limit, and lets callers wait for the
//! queue to drain. [`crate::did_cache::DidSqliteCache`] uses it to dispatch
//! the cache writes triggered by stale reads.
//!
//! Task failures are logged and swallowed — they never surface to the
//! caller that enqueued the work, because the caller has already returned
//! by the time the task runs. Callers that need to observe failures
//! should track them inside the task itself.
//!
//! **Provenance / temporary duplication:** this is a verbatim copy of
//! `cacos_pds::background` (`pds/src/background.rs`), which `cacos-pds` still
//! uses for blob deref cleanup and other out-of-band work. The DID cache
//! needs the queue but the queue is not identity-specific, so the two copies
//! are distinct types today: `cacos-pds` cannot pass its own queue to
//! [`crate::did_cache::DidSqliteCache::new`] until the queue lands in a shared
//! crate (or this crate exposes a trait that `cacos-pds` implements). The unit
//! tests for the queue stay with the original in `cacos-pds`.

use anyhow::Result;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore};

const DEFAULT_CONCURRENCY: usize = 5;

#[derive(Debug)]
struct QueueState {
    pending: AtomicUsize,
    destroyed: AtomicBool,
    notify: Notify,
    semaphore: Semaphore,
}

/// A simple queue for in-process, out-of-band/backgrounded work.
/// Task failures are logged, never surfaced.
#[derive(Debug, Clone)]
pub struct BackgroundQueue {
    state: Arc<QueueState>,
}

impl Default for BackgroundQueue {
    fn default() -> Self {
        Self::new(DEFAULT_CONCURRENCY)
    }
}

impl BackgroundQueue {
    pub fn new(concurrency: usize) -> Self {
        BackgroundQueue {
            state: Arc::new(QueueState {
                pending: AtomicUsize::new(0),
                destroyed: AtomicBool::new(false),
                notify: Notify::new(),
                semaphore: Semaphore::new(concurrency),
            }),
        }
    }

    pub fn destroyed(&self) -> bool {
        self.state.destroyed.load(Ordering::SeqCst)
    }

    pub fn add<F>(&self, task: F)
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        if self.destroyed() {
            return;
        }
        tracing::debug!("background task added");
        let state = self.state.clone();
        state.pending.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let _permit = state
                .semaphore
                .acquire()
                .await
                .expect("background queue semaphore never closed");
            if let Err(err) = task.await {
                tracing::error!(?err, "background queue task failed");
            }
            drop(_permit);
            state.pending.fetch_sub(1, Ordering::SeqCst);
            state.notify.notify_waiters();
        });
    }

    /// Waits for every queued task to finish.
    pub async fn process_all(&self) {
        loop {
            let notified = self.state.notify.notified();
            if self.state.pending.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }

    /// Stops accepting new tasks and completes all pending/in-progress tasks.
    pub async fn destroy(&self) {
        if self.state.destroyed.swap(true, Ordering::SeqCst) {
            tracing::warn!("BackgroundQueue::destroy() called multiple times");
        }
        tracing::debug!("background queue destroyed");
        self.process_all().await;
    }

    /// Number of tasks currently queued or running (in-flight + waiting on a semaphore permit).
    pub fn pending(&self) -> usize {
        self.state.pending.load(Ordering::SeqCst)
    }

    /// Idiomatic Rust alias for [`BackgroundQueue::pending`].
    pub fn len(&self) -> usize {
        self.pending()
    }

    /// Whether the queue has no in-flight or waiting tasks.
    pub fn is_empty(&self) -> bool {
        self.pending() == 0
    }
}
