//! Lightweight, persistent CLI run diagnostics, independent of index/model loading.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::IndexProgress;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatus {
    pub pid: u32,
    pub started_at: u64,
    pub heartbeat_at: u64,
    pub progress_at: u64,
    pub state: String,
    pub collection: String,
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub file: Option<String>,
    pub embeddings_enabled: bool,
    pub force: bool,
    pub error: Option<String>,
    pub embedded_chunks: usize,
    pub embedding_total: usize,
}

impl RunStatus {
    pub fn summary(&self) -> String {
        format!(
            "pid={} {} collection={} phase={} {}/{} embedded={}/{} elapsed={}s without-progress={}s heartbeat-age={}s embeddings={} force={}{}{}",
            self.pid,
            self.state,
            self.collection,
            self.phase,
            self.current,
            self.total,
            self.embedded_chunks,
            self.embedding_total,
            self.heartbeat_at.saturating_sub(self.started_at),
            self.heartbeat_at.saturating_sub(self.progress_at),
            now().saturating_sub(self.heartbeat_at),
            self.embeddings_enabled,
            self.force,
            self.file
                .as_ref()
                .map(|p| format!(" file={p}"))
                .unwrap_or_default(),
            self.error
                .as_ref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default(),
        )
    }
}

fn persist(path: &Path, status: &RunStatus) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(status)?)?;
    std::fs::rename(temporary, path)
}

/// A heartbeat remains active while a synchronous embedding batch is running.
pub struct RunMonitor {
    status: Arc<Mutex<RunStatus>>,
    path: PathBuf,
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl RunMonitor {
    pub fn start(index: &Path, embeddings_enabled: bool, force: bool) -> std::io::Result<Self> {
        Self::start_with_interval(index, embeddings_enabled, force, Duration::from_secs(10))
    }

    fn start_with_interval(
        index: &Path,
        embeddings_enabled: bool,
        force: bool,
        interval: Duration,
    ) -> std::io::Result<Self> {
        let directory = index.join("runs");
        std::fs::create_dir_all(&directory)?;
        let timestamp = now();
        let path = directory.join(format!("{timestamp}-{}.json", std::process::id()));
        let status = RunStatus {
            pid: std::process::id(),
            started_at: timestamp,
            heartbeat_at: timestamp,
            progress_at: timestamp,
            state: "running".into(),
            collection: String::new(),
            phase: "initializing model and store".into(),
            current: 0,
            total: 0,
            file: None,
            embeddings_enabled,
            force,
            error: None,
            embedded_chunks: 0,
            embedding_total: 0,
        };
        persist(&path, &status)?;
        eprintln!("Index status: {}", path.display());
        eprintln!("{}", status.summary());
        let status = Arc::new(Mutex::new(status));
        let shared = Arc::clone(&status);
        let destination = path.clone();
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            while receiver.recv_timeout(interval) == Err(mpsc::RecvTimeoutError::Timeout) {
                let mut snapshot = shared.lock().unwrap();
                snapshot.heartbeat_at = now();
                if let Err(error) = persist(&destination, &snapshot) {
                    eprintln!("Cannot save indexing status: {error}");
                }
                eprintln!("{}", snapshot.summary());
            }
        });
        Ok(Self {
            status,
            path,
            stop,
            worker: Some(worker),
        })
    }

    pub fn update(&self, collection: &str, event: &IndexProgress<'_>) {
        let mut status = self.status.lock().unwrap();
        let old_phase = status.phase.clone();
        if status.collection != collection {
            status.embedded_chunks = 0;
            status.embedding_total = 0;
        }
        status.collection = collection.into();
        status.progress_at = now();
        status.file = None;
        match event {
            IndexProgress::Phase { name } => {
                status.phase = (*name).into();
                status.current = 0;
                status.total = 0;
            }
            IndexProgress::ProcessingFile {
                current,
                total,
                path,
            } => {
                status.phase = "processing files".into();
                status.current = *current;
                status.total = *total;
                status.file = Some(path.display().to_string());
            }
            IndexProgress::GeneratingEmbeddings { current, total } => {
                status.embedded_chunks = *current;
                status.embedding_total = *total;
                status.phase = "embedding chunks (completed)".into();
                status.current = *current;
                status.total = *total;
            }
        }
        if status.phase != old_phase {
            status.heartbeat_at = now();
            if let Err(error) = persist(&self.path, &status) {
                eprintln!("Cannot save indexing status: {error}");
            }
            eprintln!("{}", status.summary());
        }
    }

    pub fn finish(&self, state: &str) {
        let mut status = self.status.lock().unwrap();
        status.state = state.into();
        status.heartbeat_at = now();
        if let Err(error) = persist(&self.path, &status) {
            eprintln!("Cannot save indexing status: {error}");
        }
    }

    pub fn fail(&self, error: impl ToString) {
        self.status.lock().unwrap().error = Some(error.to_string());
        self.finish("failed");
    }
}

impl Drop for RunMonitor {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn read_runs(index: &Path) -> std::io::Result<Vec<RunStatus>> {
    let directory = index.join("runs");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut runs = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let mut status: RunStatus = serde_json::from_slice(&std::fs::read(path)?)?;
            if status.state == "running" && now().saturating_sub(status.heartbeat_at) > 30 {
                status.state = "stale: possibly interrupted; completion unconfirmed".into();
            }
            runs.push(status);
        }
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    runs.truncate(10);
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_progress_and_completion_without_a_model() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = RunMonitor::start(dir.path(), true, false).unwrap();
        monitor.update(
            "docs",
            &IndexProgress::GeneratingEmbeddings {
                current: 0,
                total: 128,
            },
        );
        let status = read_runs(dir.path()).unwrap().remove(0);
        assert_eq!(status.total, 128);
        assert_eq!(status.current, 0);
        monitor.update(
            "docs",
            &IndexProgress::GeneratingEmbeddings {
                current: 64,
                total: 128,
            },
        );
        monitor.finish("failed");
        drop(monitor);
        let status = read_runs(dir.path()).unwrap().remove(0);
        assert_eq!(status.state, "failed");
        assert_eq!(status.current, 64);
    }

    #[test]
    fn stale_heartbeat_never_claims_completion() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = RunMonitor::start(dir.path(), false, false).unwrap();
        let mut status = monitor.status.lock().unwrap().clone();
        status.heartbeat_at = now() - 60;
        persist(&monitor.path, &status).unwrap();
        assert!(read_runs(dir.path()).unwrap()[0].state.starts_with("stale"));
    }

    #[test]
    fn heartbeat_persists_progress_without_another_callback() {
        let dir = tempfile::tempdir().unwrap();
        let monitor =
            RunMonitor::start_with_interval(dir.path(), true, false, Duration::from_millis(10))
                .unwrap();
        monitor.update(
            "docs",
            &IndexProgress::GeneratingEmbeddings {
                current: 0,
                total: 128,
            },
        );
        monitor.update(
            "docs",
            &IndexProgress::GeneratingEmbeddings {
                current: 64,
                total: 128,
            },
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if read_runs(dir.path()).unwrap()[0].current == 64 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "heartbeat did not persist progress"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        monitor.finish("completed");
        drop(monitor);
        assert_eq!(read_runs(dir.path()).unwrap()[0].state, "completed");
    }
}
