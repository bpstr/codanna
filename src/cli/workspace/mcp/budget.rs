//! Admission covers discovery, diagnostics, refresh, and queries. A blocking
//! operation owns its permit until its closure exits, even if its caller cancels.
use rmcp::model::ErrorData;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct Budget {
    requests: Arc<Semaphore>,
    blocking: Arc<Semaphore>,
}
impl Budget {
    pub(super) fn new() -> Self {
        Self {
            requests: Arc::new(Semaphore::new(16)),
            blocking: Arc::new(Semaphore::new(4)),
        }
    }
    pub(super) fn enter(&self) -> Result<OwnedSemaphorePermit, ErrorData> {
        self.requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| super::internal("Workspace request queue is full"))
    }
    pub(super) async fn run<T, F>(&self, ct: &CancellationToken, work: F) -> Result<T, ErrorData>
    where
        T: Send + 'static,
        F: FnOnce(CancellationToken) -> Result<T, ErrorData> + Send + 'static,
    {
        let permit = tokio::select! {
            _ = ct.cancelled() => return Err(super::internal("Workspace operation cancelled")),
            result = tokio::time::timeout(Duration::from_secs(3), self.blocking.clone().acquire_owned()) => result.map_err(super::internal)?.map_err(super::internal)?,
        };
        let work_ct = ct.child_token();
        let cancel_on_drop = work_ct.clone().drop_guard();
        let mut task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if work_ct.is_cancelled() {
                return Err(super::internal("Workspace operation cancelled"));
            }
            work(work_ct)
        });
        let result = tokio::select! {
            _ = ct.cancelled() => Err(super::internal("Workspace operation cancelled")),
            result = tokio::time::timeout(Duration::from_secs(5), &mut task) => result.map_err(super::internal)?.map_err(super::internal)?,
        };
        drop(cancel_on_drop);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hardening_workspace_blocking_permit_outlives_cancelled_waiter() {
        let budget = Budget::new();
        let ct = CancellationToken::new();
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let clone = budget.clone();
        let token = ct.clone();
        let caller = tokio::spawn(async move {
            clone
                .run(&token, move |_| {
                    let _ = started.send(());
                    wait.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(())
                })
                .await
        });
        started_rx.await.unwrap();
        ct.cancel();
        assert!(caller.await.unwrap().is_err());
        assert_eq!(budget.blocking.available_permits(), 3);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while budget.blocking.available_permits() != 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
