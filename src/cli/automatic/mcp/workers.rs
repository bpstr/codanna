//! Lazy local readers, bounded to four workspaces and sixteen admitted requests.
//! The only child command is this running Codanna binary with validated config.

use crate::cli::workspace::WorkspaceLaunch;
use crate::init::workspaces::{Workspace, WorkspaceRegistry, confined_index_path, read_settings};
use rmcp::model::{CallToolRequestParams, CallToolResult, ErrorData, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

const MAX_WORKERS: usize = 4;
const MAX_REQUESTS: usize = 16;
const QUEUE_TIMEOUT: Duration = Duration::from_secs(30);
const START_TIMEOUT: Duration = Duration::from_secs(90);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_BACKOFF: Duration = Duration::from_secs(2);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

struct Slot {
    client: Option<RunningService<RoleClient, ()>>,
    fingerprint: Vec<u8>,
    last_used: Instant,
    retry_after: Option<Instant>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            client: None,
            fingerprint: Vec::new(),
            last_used: Instant::now(),
            retry_after: None,
        }
    }
}

pub(super) struct WorkerPool {
    registry: WorkspaceRegistry,
    executable: PathBuf,
    slots: Mutex<HashMap<String, Arc<Mutex<Slot>>>>,
    admission: Semaphore,
}

impl WorkerPool {
    pub(super) fn new(registry: WorkspaceRegistry, executable: PathBuf) -> Self {
        Self {
            registry,
            executable,
            slots: Mutex::new(HashMap::new()),
            admission: Semaphore::new(MAX_REQUESTS),
        }
    }

    async fn slot(&self, id: &str) -> Result<Arc<Mutex<Slot>>, ErrorData> {
        let mut slots = self.slots.lock().await;
        slots.retain(|_, slot| {
            Arc::strong_count(slot) != 1
                || slot
                    .try_lock()
                    .map_or(true, |slot| slot.last_used.elapsed() < IDLE_TIMEOUT)
        });
        if let Some(slot) = slots.get(id) {
            return Ok(Arc::clone(slot));
        }
        if slots.len() >= MAX_WORKERS {
            let oldest = slots
                .iter()
                .filter(|(_, slot)| Arc::strong_count(slot) == 1)
                .filter_map(|(id, slot)| {
                    slot.try_lock()
                        .ok()
                        .map(|slot| (id.clone(), slot.last_used))
                })
                .min_by_key(|(_, used)| *used)
                .map(|(id, _)| id);
            match oldest {
                Some(id) => {
                    slots.remove(&id);
                }
                None => {
                    return Err(super::internal(
                        "All workspace workers are busy; retry the query",
                    ));
                }
            }
        }
        let slot = Arc::new(Mutex::new(Slot::default()));
        slots.insert(id.to_owned(), Arc::clone(&slot));
        Ok(slot)
    }

    pub(super) async fn is_loaded(&self, id: &str) -> bool {
        let slots = self.slots.lock().await;
        slots
            .get(id)
            .is_some_and(|slot| slot.try_lock().is_ok_and(|slot| slot.client.is_some()))
    }

    pub(super) async fn call(
        &self,
        workspace: &Workspace,
        request: CallToolRequestParams,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        let _admission = self
            .admission
            .try_acquire()
            .map_err(|_| super::internal("Workspace query queue is full; retry later"))?;
        let registry = self.registry.clone();
        let selected = workspace.clone();
        let executable = self.executable.clone();
        let (command, fingerprint) =
            tokio::task::spawn_blocking(move || worker_command(&registry, &selected, &executable))
                .await
                .map_err(super::internal)??;
        let slot_ref = self.slot(workspace.id.as_str()).await?;
        let mut slot = tokio::select! {
            _ = cancellation.cancelled() => return Err(super::internal("Workspace query cancelled")),
            result = tokio::time::timeout(QUEUE_TIMEOUT, slot_ref.lock()) => {
                result.map_err(|_| super::internal("Workspace query queue timed out"))?
            }
        };
        if slot.retry_after.is_some_and(|until| until > Instant::now()) {
            return Err(super::internal(format!(
                "Workspace '{}' reader is cooling down after failure",
                workspace.name
            )));
        }
        if slot.fingerprint != fingerprint {
            close_slot(&mut slot).await;
        }
        if slot.client.is_none() {
            let mut command = tokio::process::Command::from(command);
            command.kill_on_drop(true);
            let transport = TokioChildProcess::new(command).map_err(super::internal)?;
            let started = tokio::select! {
                _ = cancellation.cancelled() => return Err(super::internal("Workspace loading cancelled")),
                result = tokio::time::timeout(START_TIMEOUT, ().serve_with_lifecycle(
                    transport,
                    ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::V_2026_07_28] },
                )) => result,
            };
            match started {
                Ok(Ok(client)) => {
                    slot.client = Some(client);
                    slot.fingerprint = fingerprint;
                    slot.retry_after = None;
                }
                result => {
                    slot.retry_after = Some(Instant::now() + RETRY_BACKOFF);
                    slot.last_used = Instant::now();
                    let detail = match result {
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "loading exceeded 90 seconds".into(),
                        Ok(Ok(_)) => unreachable!(),
                    };
                    return Err(super::internal(format!(
                        "Workspace '{}' could not start: {detail}. Run codanna index in its root if rebuilding is required.",
                        workspace.name
                    )));
                }
            }
        }
        let result = {
            let client = slot.client.as_ref().expect("reader initialized above");
            tokio::select! {
                _ = cancellation.cancelled() => Err(super::internal("Workspace query cancelled")),
                result = tokio::time::timeout(QUERY_TIMEOUT, client.call_tool(request)) => {
                    match result {
                        Ok(result) => result.map_err(super::internal),
                        Err(_) => Err(super::internal("Workspace query exceeded 30 seconds")),
                    }
                }
            }
        };
        slot.last_used = Instant::now();
        if result.is_err() {
            // No other call is active on this serialized reader. Cancellation and
            // timeouts always close it; failed calls are never automatically replayed.
            close_slot(&mut slot).await;
            slot.retry_after = Some(Instant::now() + RETRY_BACKOFF);
        }
        result.map_err(|error| super::internal(format!("Workspace '{}': {error}", workspace.name)))
    }

    pub(super) async fn shutdown(&self) {
        let slots: Vec<_> = self
            .slots
            .lock()
            .await
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        for slot in slots {
            if let Ok(mut slot) = tokio::time::timeout(Duration::from_secs(5), slot.lock()).await {
                close_slot(&mut slot).await;
            }
        }
    }
}

async fn close_slot(slot: &mut Slot) {
    if let Some(client) = slot.client.take() {
        let _ = tokio::time::timeout(Duration::from_secs(3), client.cancel()).await;
    }
}

fn worker_command(
    registry: &WorkspaceRegistry,
    workspace: &Workspace,
    executable: &Path,
) -> Result<(std::process::Command, Vec<u8>), ErrorData> {
    // This is not a configurable executable runner. Even library callers cannot
    // substitute a script or another program for the running Codanna binary.
    let current_executable = std::env::current_exe().map_err(super::internal)?;
    if executable.canonicalize().map_err(super::internal)?
        != current_executable.canonicalize().map_err(super::internal)?
    {
        return Err(super::internal(
            "Workspace readers must use the running Codanna executable",
        ));
    }
    let current = registry
        .get(workspace.id.as_str())
        .map_err(super::internal)?;
    if current.root != workspace.root {
        return Err(super::internal(
            "Workspace moved during selection; retry with its stable ID",
        ));
    }
    let settings = read_settings(&current.root).map_err(super::internal)?;
    if settings.server.mode == "http" {
        return Err(super::internal(
            "Local workspace readers require stdio settings; use the explicitly configured network server instead",
        ));
    }
    let index = confined_index_path(&current.root, &settings).map_err(super::internal)?;
    crate::storage::IndexMetadata::load(&index).map_err(|error| {
        super::internal(format!(
            "Workspace '{}' index is unavailable: {error}. Run codanna index in its root.",
            current.name,
        ))
    })?;
    let args = vec![
        OsString::from("--workspace"),
        current.id.as_str().into(),
        OsString::from("serve"),
    ];
    let plan =
        WorkspaceLaunch::prepare(registry, current.id.as_str(), &args).map_err(super::internal)?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(current.root.as_os_str().as_encoded_bytes());
    for path in [
        current.config_path.clone(),
        index.join("index.meta"),
        index.join("tantivy/meta.json"),
    ] {
        let mut bytes = Vec::new();
        File::open(&path)
            .map_err(super::internal)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(super::internal)?;
        if bytes.len() > 1024 * 1024 {
            return Err(super::internal(
                "Workspace metadata exceeds the 1 MiB routing limit",
            ));
        }
        fingerprint.update((bytes.len() as u64).to_le_bytes());
        fingerprint.update(&bytes);
    }
    let mut command = plan.command(&current_executable);
    command.env("PWD", &current.root);
    Ok((command, fingerprint.finalize().to_vec()))
}
