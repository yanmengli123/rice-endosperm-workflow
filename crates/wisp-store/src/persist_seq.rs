//! Incremental message persistence: seq policy and worker teardown.
//!
//! `last_seq` is the durable `COALESCE(MAX(seq), 0)` of persisted rows, not an
//! in-memory message count. The worker stops at the first append failure so
//! later messages cannot be written after a hole. On timeout the JoinHandle
//! must be aborted and joined before replace/compaction can run.

use std::fmt;
use std::future::Future;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

/// Persist items with monotonically increasing seqs starting after `start_seq`.
///
/// Stops at the first append error and returns it. Later items stay in the
/// channel and are dropped with the receiver — they are not written.
pub async fn persist_seq_loop<T, E, F, Fut>(
    mut seq: i64,
    mut rx: UnboundedReceiver<T>,
    mut append: F,
) -> Result<i64, E>
where
    F: FnMut(i64, T) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    while let Some(item) = rx.recv().await {
        seq += 1;
        append(seq, item).await?;
    }
    Ok(seq)
}

/// Why [`join_or_abort_persist`] could not return the task's output.
#[derive(Debug)]
pub enum PersistJoinError {
    /// The task was aborted and has exited.
    Aborted,
    /// The task panicked.
    Join(tokio::task::JoinError),
    /// `abort()` was called but the task did not exit within `wait`.
    AbortWaitTimeout,
}

impl fmt::Display for PersistJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aborted => write!(f, "persist task was aborted"),
            Self::Join(error) => write!(f, "persist task panicked: {error}"),
            Self::AbortWaitTimeout => {
                write!(f, "persist task did not exit after abort")
            }
        }
    }
}

impl std::error::Error for PersistJoinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Join(error) => Some(error),
            Self::Aborted | Self::AbortWaitTimeout => None,
        }
    }
}

/// Wait for a persist task. On timeout, abort it and wait for the task to exit
/// so a late INSERT cannot race with replace/compaction.
pub async fn join_or_abort_persist<T>(
    mut handle: JoinHandle<T>,
    wait: Duration,
) -> Result<T, PersistJoinError> {
    match tokio::time::timeout(wait, &mut handle).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if error.is_cancelled() => Err(PersistJoinError::Aborted),
        Ok(Err(error)) => Err(PersistJoinError::Join(error)),
        Err(_elapsed) => {
            handle.abort();
            match tokio::time::timeout(wait, handle).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) if error.is_cancelled() => Err(PersistJoinError::Aborted),
                Ok(Err(error)) => Err(PersistJoinError::Join(error)),
                Err(_elapsed) => Err(PersistJoinError::AbortWaitTimeout),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn first_append_failure_stops_worker() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let recorded = attempts.clone();
        let task = tokio::spawn(persist_seq_loop(0, rx, move |seq, item: &'static str| {
            let recorded = recorded.clone();
            async move {
                recorded.lock().unwrap().push((seq, item));
                if seq == 1 {
                    Err("first append failed")
                } else {
                    Ok(())
                }
            }
        }));
        tx.send("first").unwrap();
        tx.send("second").unwrap();
        drop(tx);
        let result = task.await.unwrap();
        assert_eq!(result, Err("first append failed"));
        assert_eq!(*attempts.lock().unwrap(), vec![(1, "first")]);
    }

    #[tokio::test]
    async fn successful_loop_returns_final_seq() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(persist_seq_loop(5, rx, |_seq, _item: u8| async {
            Ok::<(), ()>(())
        }));
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        drop(tx);
        let seq = join_or_abort_persist(handle, Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(seq, 7);
    }

    struct ExitFlag(Arc<AtomicBool>);

    impl Drop for ExitFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn abort_prevents_late_append_and_waits_for_exit() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let appended = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let append_count = Arc::new(AtomicUsize::new(0));
        let mut started_tx = Some(started_tx);
        let mut gate_rx = Some(gate_rx);
        let handle = tokio::spawn({
            let appended = appended.clone();
            let exited = exited.clone();
            let append_count = append_count.clone();
            async move {
                let _exit = ExitFlag(exited);
                persist_seq_loop(0, rx, move |_seq, _item| {
                    let started_tx = started_tx.take();
                    let gate_rx = gate_rx.take();
                    let appended = appended.clone();
                    let append_count = append_count.clone();
                    async move {
                        append_count.fetch_add(1, Ordering::SeqCst);
                        if let Some(started_tx) = started_tx {
                            let _ = started_tx.send(());
                        }
                        if let Some(gate_rx) = gate_rx {
                            let _ = gate_rx.await;
                        }
                        appended.store(true, Ordering::SeqCst);
                        Ok::<(), ()>(())
                    }
                })
                .await
            }
        });
        tx.send(()).unwrap();
        started_rx.await.unwrap();
        let outcome = join_or_abort_persist(handle, Duration::from_millis(20)).await;
        assert!(matches!(outcome, Err(PersistJoinError::Aborted)));
        assert!(
            exited.load(Ordering::SeqCst),
            "teardown must wait until the persist task has exited"
        );
        // Releasing the gate would complete a late write if the task were still
        // running after we returned (the previous timeout-only JoinHandle drop).
        let _ = gate_tx.send(());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!appended.load(Ordering::SeqCst));
        assert_eq!(append_count.load(Ordering::SeqCst), 1);
        drop(tx);
    }
}
