//! Bounded blocking work and serialized mutation ownership for asynchronous callers.
use crate::{IndexError, indexing::facade::IndexFacade};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{RwLock, Semaphore};

fn workers() -> Arc<Semaphore> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    Arc::clone(WORKERS.get_or_init(|| Arc::new(Semaphore::new(4))))
}
async fn on_pool<T: Send + 'static>(
    pool: Arc<Semaphore>,
    task: impl FnOnce() -> T + Send + 'static,
) -> Result<T, IndexError> {
    let permit = pool
        .acquire_owned()
        .await
        .map_err(|_| IndexError::General("blocking worker pool closed".into()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        task()
    })
    .await
    .map_err(|error| IndexError::General(format!("blocking worker failed: {error}")))
}
pub(crate) async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> T + Send + 'static,
) -> Result<T, IndexError> {
    on_pool(workers(), task).await
}

fn mutation_gate<T>(owner: &Arc<RwLock<T>>) -> Result<Arc<Semaphore>, IndexError> {
    static GATES: OnceLock<Mutex<HashMap<usize, Weak<Semaphore>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| IndexError::MutexPoisoned)?;
    gates.retain(|_, gate| gate.strong_count() > 0);
    let key = Arc::as_ptr(owner) as usize;
    if let Some(gate) = gates.get(&key).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    let gate = Arc::new(Semaphore::new(1));
    gates.insert(key, Arc::downgrade(&gate));
    Ok(gate)
}

/// Snapshot only Arc handles and small facade metadata under a short read guard.
/// Expensive queries run after that guard is released, on the bounded pool.
pub(crate) async fn read<T: Send + 'static>(
    owner: &Arc<RwLock<IndexFacade>>,
    task: impl FnOnce(IndexFacade) -> T + Send + 'static,
) -> Result<T, IndexError> {
    let snapshot = owner.read().await.clone();
    blocking(move || task(snapshot)).await
}

/// Serialize changes to one facade, without locking out read-only snapshots for
/// the duration of walking, embedding, committing or saving. The worker owns its
/// mutation permit even when its awaiting request is cancelled. Partial state is
/// published on errors too; callers still receive the error, not false success.
pub(crate) async fn mutate<T: Send + 'static>(
    owner: &Arc<RwLock<IndexFacade>>,
    task: impl FnOnce(&mut IndexFacade) -> T + Send + 'static,
) -> Result<T, IndexError> {
    let gate = mutation_gate(owner)?;
    let permit = gate
        .acquire_owned()
        .await
        .map_err(|_| IndexError::General("index mutation lane closed".into()))?;
    let owner = Arc::clone(owner);
    blocking(move || {
        let _permit = permit;
        let mut snapshot = owner.blocking_read().clone();
        let workspace = snapshot.network_workspace.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(&mut snapshot)));
        if let Some(root) = workspace {
            // A normal mutation retains the exact boundary and has already
            // checked paths. A replaced facade must revalidate all provenance.
            if snapshot.network_workspace.as_ref() != Some(&root) {
                snapshot.restrict_workspace(root)?;
            }
        }
        *owner.blocking_write() = snapshot;
        match result {
            Ok(value) => Ok(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn hardening_final_mutation_releases_facade_guard_during_blocking_work() {
        let dir = tempfile::tempdir().unwrap();
        let settings = crate::Settings {
            workspace_root: Some(dir.path().to_path_buf()),
            index_path: dir.path().join("index"),
            ..crate::Settings::default()
        };
        let owner = Arc::new(RwLock::new(IndexFacade::new(Arc::new(settings)).unwrap()));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let job = tokio::spawn({
            let owner = Arc::clone(&owner);
            async move {
                mutate(&owner, move |facade| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    facade.set_indexed_paths(vec![PathBuf::from("changed")]);
                })
                .await
            }
        });
        started_rx.await.unwrap();
        let read = tokio::time::timeout(std::time::Duration::from_secs(2), owner.read()).await;
        // Always unblock the worker even when the assertion fails.
        release_tx.send(()).unwrap();
        assert!(
            read.is_ok(),
            "mutation must not retain the facade write guard"
        );
        drop(read);
        job.await.unwrap().unwrap();
        assert!(
            owner
                .read()
                .await
                .get_indexed_paths()
                .contains(&PathBuf::from("changed"))
        );
    }
    use std::path::PathBuf;
    #[tokio::test(flavor = "current_thread")]
    async fn hardening_final_cancelled_waiter_does_not_release_running_worker_permit() {
        let pool = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        let job = tokio::spawn(on_pool(Arc::clone(&pool), move || {
            let _ = started_tx.send(());
            rx.recv().unwrap();
        }));
        started_rx.await.unwrap();
        job.abort();
        assert_eq!(
            pool.available_permits(),
            0,
            "aborting the awaiter cannot admit a second running worker"
        );
        tx.send(()).unwrap();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(2), pool.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert_eq!(pool.available_permits(), 1);
    }
}
