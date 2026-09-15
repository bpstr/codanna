//! Shared startup, concurrent RPCs, bounded refresh, and explicit child lifetimes.
use super::{bootstrap, budget::Budget};
use crate::init::workspaces::{Workspace, confined_index_path, read_settings};
use parking_lot::Mutex;
use rmcp::model::*;
use rmcp::service::{Peer, PeerRequestOptions, RoleClient, ServiceError};
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const REFRESH: Duration = Duration::from_secs(1);
const LOAD_TIMEOUT: Duration = Duration::from_secs(95);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_WORKERS: usize = 4;

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: SystemTime,
    identity: (u64, u64),
}
fn stamp(path: &Path) -> Result<Stamp, ErrorData> {
    let metadata = fs::metadata(path).map_err(super::internal)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(super::internal(
            "Workspace metadata must be a regular file of at most 1 MiB",
        ));
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = (0, 0);
    Ok(Stamp {
        length: metadata.len(),
        modified: metadata.modified().map_err(super::internal)?,
        identity,
    })
}
#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    root: PathBuf,
    index: PathBuf,
    config: Stamp,
    metadata: Stamp,
    tantivy: Stamp,
    managed: bool,
    ignore: Option<Stamp>,
}
fn snapshot(workspace: &Workspace, previous: Option<&Snapshot>) -> Result<Snapshot, ErrorData> {
    let config = stamp(&workspace.config_path)?;
    let index = match previous.filter(|old| old.root == workspace.root && old.config == config) {
        Some(old) => old.index.clone(),
        None => {
            let settings = read_settings(&workspace.root).map_err(super::internal)?;
            if settings.server.mode == "http" {
                return Err(super::internal(
                    "Use the explicitly configured network server for this workspace",
                ));
            }
            for source in &settings.indexing.indexed_paths {
                let source = workspace
                    .root
                    .join(source)
                    .canonicalize()
                    .map_err(super::internal)?;
                if !source.starts_with(&workspace.root) {
                    return Err(super::internal(
                        "Automatic workspace readers cannot include external sources",
                    ));
                }
            }
            confined_index_path(&workspace.root, &settings).map_err(super::internal)?
        }
    };
    let managed = match previous.filter(|old| old.root == workspace.root && old.config == config) {
        Some(old) => old.managed && !index.join("semantic/metadata.json").exists(),
        None => {
            let mut settings = read_settings(&workspace.root).map_err(super::internal)?;
            settings.index_path = index.clone();
            super::live::eligible(&settings)
        }
    };
    let ignore_path = workspace.root.join(".codannaignore");
    let ignore = if ignore_path.try_exists().map_err(super::internal)? {
        Some(stamp(&ignore_path)?)
    } else {
        None
    };
    Ok(Snapshot {
        managed,
        ignore,
        root: workspace.root.clone(),
        config,
        metadata: stamp(&index.join("index.meta"))?,
        tantivy: stamp(&index.join("tantivy/meta.json"))?,
        index,
    })
}

struct Reader {
    peer: Peer<RoleClient>,
    snapshot: Snapshot,
    queries: Semaphore,
    close: CancellationToken,
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.close.cancel();
    }
}
#[derive(Clone)]
enum Event {
    Loading,
    Indexing,
    Ready(Arc<Reader>),
    Empty,
    Busy,
    Failed(ErrorData),
}
struct Slot {
    event: Option<watch::Receiver<Event>>,
    refresh_at: Instant,
    last_used: Instant,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            event: None,
            refresh_at: Instant::now(),
            last_used: Instant::now(),
        }
    }
}
type Tasks = Arc<Mutex<Vec<JoinHandle<()>>>>;

pub(super) struct WorkerPool {
    executable: PathBuf,
    slots: Mutex<HashMap<String, Arc<Mutex<Slot>>>>,
    processes: Arc<Semaphore>,
    indexers: Arc<Semaphore>,
    tasks: Tasks,
    stop: CancellationToken,
    budget: Budget,
}
impl WorkerPool {
    pub(super) fn new(executable: PathBuf, budget: Budget) -> Result<Self, ErrorData> {
        let executable = executable.canonicalize().map_err(super::internal)?;
        if executable
            != std::env::current_exe()
                .map_err(super::internal)?
                .canonicalize()
                .map_err(super::internal)?
        {
            return Err(super::internal(
                "Readers must use the running Codanna executable",
            ));
        }
        Ok(Self {
            executable,
            slots: Mutex::new(HashMap::new()),
            processes: Arc::new(Semaphore::new(MAX_WORKERS)),
            indexers: Arc::new(Semaphore::new(1)),
            tasks: Arc::new(Mutex::new(Vec::new())),
            stop: CancellationToken::new(),
            budget,
        })
    }
    fn slot(&self, id: &str) -> Result<Arc<Mutex<Slot>>, ErrorData> {
        let mut slots = self.slots.lock();
        slots.retain(|_, slot| {
            Arc::strong_count(slot) > 1
                || slot.lock().last_used.elapsed() < Duration::from_secs(300)
        });
        if let Some(slot) = slots.get(id) {
            return Ok(slot.clone());
        }
        if slots.len() >= MAX_WORKERS {
            let oldest = slots
                .iter()
                .filter(|(_, slot)| Arc::strong_count(slot) == 1)
                .min_by_key(|(_, slot)| slot.lock().last_used)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                slots.remove(&id);
            } else {
                return Err(super::internal(
                    "All workspace slots are active; retry shortly",
                ));
            }
        }
        let slot = Arc::new(Mutex::new(Slot::default()));
        slots.insert(id.into(), slot.clone());
        Ok(slot)
    }
    pub(super) async fn is_loaded(&self, id: &str) -> bool {
        self.slots.lock().get(id).is_some_and(|slot| slot.lock().event.as_ref().is_some_and(|rx| matches!(&*rx.borrow(), Event::Ready(reader) if !reader.peer.is_transport_closed())))
    }
    fn subscribe(&self, slot: &Arc<Mutex<Slot>>, workspace: &Workspace) -> watch::Receiver<Event> {
        let mut state = slot.lock();
        state.last_used = Instant::now();
        if let Some(rx) = &state.event {
            let event = rx.borrow().clone();
            if matches!(event, Event::Loading | Event::Indexing)
                || (Instant::now() < state.refresh_at
                    && !matches!(event, Event::Ready(ref reader) if reader.peer.is_transport_closed()))
            {
                return rx.clone();
            }
        }
        let previous = state.event.as_ref().and_then(|rx| match &*rx.borrow() {
            Event::Ready(reader) => Some(reader.clone()),
            _ => None,
        });
        let (tx, rx) = watch::channel(Event::Loading);
        state.event = Some(rx.clone());
        let slot = slot.clone();
        let workspace = workspace.clone();
        let budget = self.budget.clone();
        let executable = self.executable.clone();
        let processes = self.processes.clone();
        let indexers = self.indexers.clone();
        let stop = self.stop.clone();
        let tasks = self.tasks.clone();
        let task = tokio::spawn(async move {
            let result = load(
                workspace, previous, budget, executable, processes, indexers, stop, tasks, &tx,
            )
            .await;
            let event = match result {
                Ok(event) => event,
                Err(error) => Event::Failed(error),
            };
            slot.lock().refresh_at = Instant::now()
                + if matches!(event, Event::Failed(_)) {
                    Duration::from_secs(2)
                } else {
                    REFRESH
                };
            tx.send_replace(event);
        });
        let mut tasks = self.tasks.lock();
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        rx
    }
    pub(super) async fn call(
        &self,
        workspace: &Workspace,
        request: CallToolRequestParams,
        ct: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        // Pin through the entire query. A cancelled follower does not cancel
        // shared initialization, evict its reader, or stop another request.
        let slot = self.slot(workspace.id.as_str())?;
        let mut rx = self.subscribe(&slot, workspace);
        let deadline = tokio::time::Instant::now() + LOAD_TIMEOUT;
        let reader = loop {
            let event = rx.borrow().clone();
            match event {
                Event::Ready(reader) => break reader,
                Event::Failed(error) => return Err(error),
                Event::Indexing => {
                    return status(
                        "indexing",
                        "Initial local code indexing is running. Retry this query shortly; no manual configuration is required.",
                    );
                }
                Event::Empty => {
                    return status(
                        "empty",
                        "No source files yet. Add files and retry; the workspace will initialize automatically.",
                    );
                }
                Event::Busy => {
                    return status(
                        "indexing",
                        "Another local connection is initializing this workspace. Retry shortly.",
                    );
                }
                Event::Loading => {}
            }
            tokio::select! {
                _ = ct.cancelled() => return Err(super::internal("Workspace query cancelled")),
                result = tokio::time::timeout_at(deadline, rx.changed()) => { result.map_err(super::internal)?.map_err(super::internal)?; }
            }
        };
        let _query = tokio::select! {
            _ = ct.cancelled() => return Err(super::internal("Workspace query cancelled")),
            result = tokio::time::timeout(QUERY_TIMEOUT, reader.queries.acquire()) => result.map_err(super::internal)?.map_err(super::internal)?,
        };
        rpc(&reader.peer, request, ct).await
    }
    pub(super) async fn shutdown(&self) {
        self.stop.cancel();
        self.slots.lock().clear();
        loop {
            let tasks = std::mem::take(&mut *self.tasks.lock());
            if tasks.is_empty() {
                break;
            }
            for task in tasks {
                let _ = task.await;
            }
        }
    }
}
impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[allow(clippy::too_many_arguments)]
async fn load(
    workspace: Workspace,
    previous: Option<Arc<Reader>>,
    budget: Budget,
    executable: PathBuf,
    processes: Arc<Semaphore>,
    indexers: Arc<Semaphore>,
    stop: CancellationToken,
    tasks: Tasks,
    tx: &watch::Sender<Event>,
) -> Result<Event, ErrorData> {
    // Existing readers go straight to cheap snapshot refresh. First-use checks
    // must not reparse the complete configuration on every warm refresh.
    if previous.is_none() {
        let selected = workspace.clone();
        let absent = budget
            .run(&stop, move |_| {
                stamp(&selected.config_path)?;
                let settings = read_settings(&selected.root).map_err(super::internal)?;
                Ok(!confined_index_path(&selected.root, &settings)
                    .map_err(super::internal)?
                    .join("index.meta")
                    .is_file())
            })
            .await?;
        if absent {
            tx.send_replace(Event::Indexing);
            match bootstrap::ensure(
                workspace.clone(),
                budget.clone(),
                indexers,
                executable.clone(),
                stop.clone(),
            )
            .await?
            {
                bootstrap::Status::Empty => return Ok(Event::Empty),
                bootstrap::Status::Busy => return Ok(Event::Busy),
                bootstrap::Status::Ready => {}
            }
            tx.send_replace(Event::Loading);
        }
    }
    let old_snapshot = previous.as_ref().map(|reader| reader.snapshot.clone());
    let selected = workspace.clone();
    let current = budget
        .run(&stop, move |_| snapshot(&selected, old_snapshot.as_ref()))
        .await?;
    if let Some(reader) = previous.as_ref().filter(|reader| {
        let same_context = reader.snapshot.root == current.root
            && reader.snapshot.index == current.index
            && reader.snapshot.config == current.config
            && reader.snapshot.ignore == current.ignore;
        (reader.snapshot == current || (same_context && current.managed && reader.snapshot.managed))
            && !reader.peer.is_transport_closed()
    }) {
        return Ok(Event::Ready(reader.clone()));
    }
    drop(previous);
    let permit = tokio::select! {
        _ = stop.cancelled() => return Err(super::internal("Workspace server shutting down")),
        result = tokio::time::timeout(LOAD_TIMEOUT, processes.acquire_owned()) => result.map_err(super::internal)?.map_err(super::internal)?,
    };
    let mut command = tokio::process::Command::new(executable);
    command
        .current_dir(&workspace.root)
        .env("PWD", &workspace.root)
        .args(["workspace", "reader"])
        .arg(&workspace.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("CI_")
        {
            command.env_remove(key);
        }
    }
    command
        .env_remove("CODANNA_RECALL_WORKSPACE")
        .env_remove("CODANNA_RECALL_INDEX");
    let mut child = command.spawn().map_err(super::internal)?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| super::internal("Reader stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| super::internal("Reader stdout unavailable"))?;
    let started = tokio::select! {
        _ = stop.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(90), ().serve_with_lifecycle((stdout, stdin), ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::V_2026_07_28] })) => Some(result),
    };
    let mut service = match started {
        Some(Ok(Ok(service))) => service,
        result => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(super::internal(format!(
                "Workspace reader failed to initialize: {result:?}"
            )));
        }
    };
    let close = CancellationToken::new();
    let reader = Arc::new(Reader {
        peer: service.peer().clone(),
        snapshot: current,
        queries: Semaphore::new(4),
        close: close.clone(),
    });
    let task = tokio::spawn(async move {
        // Retain physical capacity until child exit, not just slot removal.
        let _permit = permit;
        tokio::select! { _ = close.cancelled() => {}, _ = stop.cancelled() => {}, _ = child.wait() => {} }
        let _ = service.close_with_timeout(Duration::from_secs(1)).await;
        // Give a native writer time to drain its in-flight mutation after stdio
        // closes. Physical capacity stays owned until observed process exit.
        if tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .is_err()
        {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    });
    let mut tasks = tasks.lock();
    tasks.retain(|task| !task.is_finished());
    tasks.push(task);
    Ok(Event::Ready(reader))
}
fn status(state: &str, message: &str) -> Result<CallToolResult, ErrorData> {
    let mut result = CallToolResult::success(vec![ContentBlock::text(message)]);
    result.structured_content = Some(serde_json::json!({"status": state, "ready": false}));
    Ok(result)
}
async fn rpc(
    peer: &Peer<RoleClient>,
    request: CallToolRequestParams,
    ct: CancellationToken,
) -> Result<CallToolResult, ErrorData> {
    let mut handle = peer
        .send_request_with_option(
            ClientRequest::CallToolRequest(Request::new(request)),
            PeerRequestOptions::default(),
        )
        .await
        .map_err(map_rpc_error)?;
    // Default options install no progress/subscription watchers. Retain the
    // handle so cancellation addresses this request, never the shared peer.
    let response = tokio::select! {
        _ = ct.cancelled() => { let _ = tokio::time::timeout(Duration::from_secs(1), handle.cancel(Some("caller cancelled".into()))).await; return Err(super::internal("Workspace query cancelled")); }
        result = tokio::time::timeout(QUERY_TIMEOUT, &mut handle.rx) => match result {
            Ok(result) => result.map_err(|_| super::internal("Reader response channel closed"))?.map_err(map_rpc_error)?,
            Err(_) => { let _ = tokio::time::timeout(Duration::from_secs(1), handle.cancel(Some("query deadline".into()))).await; return Err(super::internal("Workspace query exceeded 30 seconds")); }
        }
    };
    match response {
        ServerResult::CallToolResult(result) => Ok(result),
        _ => Err(super::internal("Unexpected workspace reader response")),
    }
}
fn map_rpc_error(error: ServiceError) -> ErrorData {
    match error {
        ServiceError::McpError(error) => error,
        other => super::internal(other),
    }
}

#[cfg(test)]
mod tests;
