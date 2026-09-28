use super::*;
use crate::init::workspaces::WorkspaceId;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use tokio::sync::mpsc;

#[test]
fn hardening_workspace_preserves_backend_invalid_parameter_errors() {
    let error = ErrorData::invalid_params("bad limit", None);
    let mapped = map_rpc_error(ServiceError::McpError(error.clone()));
    assert_eq!(mapped.code, error.code);
    assert_eq!(mapped.message, error.message);
}

/// No query arrives to trigger cleanup. The timer must evict an old idle slot,
/// preserve a pinned slot and a recently used slot, then stop on cancellation.
#[tokio::test]
async fn workspace_idle_cleanup_runs_without_queries_and_preserves_pins() {
    let slots: Slots = Arc::new(Mutex::new(HashMap::new()));
    let old = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
    let pinned = Arc::new(Mutex::new(Slot {
        last_used: old,
        ..Slot::default()
    }));
    {
        let mut entries = slots.lock();
        entries.insert(
            "idle".into(),
            Arc::new(Mutex::new(Slot {
                last_used: old,
                ..Slot::default()
            })),
        );
        entries.insert("pinned".into(), pinned.clone());
        entries.insert("recent".into(), Arc::new(Mutex::new(Slot::default())));
    }
    let stop = CancellationToken::new();
    let cleanup = tokio::spawn(idle_cleanup(
        slots.clone(),
        stop.clone(),
        Duration::from_millis(10),
    ));
    tokio::time::timeout(Duration::from_secs(2), async {
        while slots.lock().contains_key("idle") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("idle slot must expire without a new query");
    assert!(slots.lock().contains_key("pinned"));
    assert!(slots.lock().contains_key("recent"));
    drop(pinned);
    tokio::time::timeout(Duration::from_secs(2), async {
        while slots.lock().contains_key("pinned") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("released slot must become evictable");
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(2), cleanup)
        .await
        .expect("cleanup must stop promptly")
        .unwrap();
    assert!(slots.lock().contains_key("recent"));
}

#[tokio::test]
async fn workspace_idle_cleanup_does_not_keep_dropped_pool_alive() {
    let pool = WorkerPool::new(std::env::current_exe().unwrap(), Budget::new()).unwrap();
    let slots = Arc::downgrade(&pool.slots);
    let cleanup = pool.tasks.lock().pop().expect("pool owns idle cleanup");
    drop(pool);
    tokio::time::timeout(Duration::from_secs(2), cleanup)
        .await
        .expect("dropping the pool must stop idle cleanup")
        .unwrap();
    assert!(slots.upgrade().is_none());
}

#[derive(Clone)]
struct ControlledReader {
    events: mpsc::UnboundedSender<String>,
    release: Arc<Semaphore>,
}
impl ServerHandler for ControlledReader {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name == "invalid" {
            return Err(ErrorData::invalid_params(
                "fixture invalid parameters",
                None,
            ));
        }
        if request.name.starts_with("blocked-") {
            self.events.send(request.name.to_string()).unwrap();
            tokio::select! {
                _ = context.ct.cancelled() => {
                    self.events.send(format!("cancelled:{}", request.name)).unwrap();
                    return Err(ErrorData::internal_error("fixture request cancelled", None));
                }
                permit = self.release.acquire() => { permit.unwrap().forget(); }
            }
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(request.name.to_string())]).into())
    }
}

/// Uses the actual pool and RPC cancellation implementation, with a deterministic
/// in-memory reader instead of source indexes or inference. A held request must
/// not serialize another call, and cancellation must not close their shared peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_workspace_pool_overlaps_queries_and_cancels_only_the_request() {
    let (events, mut observed) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let backend = ControlledReader {
        events,
        release: release.clone(),
    };
    let (server_io, client_io) = tokio::io::duplex(16 * 1024);
    let server_task = tokio::spawn(async move { backend.serve(server_io).await.unwrap() });
    let client = ().serve(client_io).await.unwrap();
    let server = server_task.await.unwrap();
    let pool = Arc::new(WorkerPool::new(std::env::current_exe().unwrap(), Budget::new()).unwrap());
    let temporary = tempfile::tempdir().unwrap();
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "fixture".into(),
        root: temporary.path().to_path_buf(),
        config_path: temporary.path().join("settings.toml"),
    };
    let stamp = Stamp {
        length: 0,
        modified: SystemTime::UNIX_EPOCH,
        identity: (0, 0),
    };
    let reader = Arc::new(Reader {
        peer: client.peer().clone(),
        queries: Semaphore::new(4),
        close: CancellationToken::new(),
        snapshot: Snapshot {
            documents: super::super::documents::Revision::capture(&workspace.root),
            root: workspace.root.clone(),
            index: workspace.root.clone(),
            config: stamp.clone(),
            metadata: stamp.clone(),
            tantivy: stamp,
            managed: false,
            ignore: None,
            git_ignore: None,
        },
    });
    let close = reader.close.clone();
    let (sender, receiver) = watch::channel(Event::Ready(reader));
    drop(sender);
    pool.slots.lock().insert(
        workspace.id.as_str().into(),
        Arc::new(Mutex::new(Slot {
            event: Some(receiver),
            refresh_at: Instant::now() + Duration::from_secs(60),
            last_used: Instant::now(),
        })),
    );
    let cancel_a = CancellationToken::new();
    let a = {
        let pool = pool.clone();
        let workspace = workspace.clone();
        let token = cancel_a.clone();
        tokio::spawn(async move {
            pool.call(&workspace, CallToolRequestParams::new("blocked-a"), token)
                .await
        })
    };
    let b = {
        let pool = pool.clone();
        let workspace = workspace.clone();
        tokio::spawn(async move {
            pool.call(
                &workspace,
                CallToolRequestParams::new("blocked-b"),
                CancellationToken::new(),
            )
            .await
        })
    };
    let mut entered = Vec::new();
    for _ in 0..2 {
        entered.push(
            tokio::time::timeout(Duration::from_secs(2), observed.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    entered.sort();
    assert_eq!(entered, ["blocked-a", "blocked-b"]);
    {
        let mut slots = pool.slots.lock();
        slots.get(workspace.id.as_str()).unwrap().lock().last_used =
            Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        prune_idle(&mut slots);
    }
    assert!(pool.is_loaded(workspace.id.as_str()).await);
    assert!(!close.is_cancelled(), "active queries pin their reader");
    pool.slots
        .lock()
        .get(workspace.id.as_str())
        .unwrap()
        .lock()
        .last_used = Instant::now();
    cancel_a.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), a)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), observed.recv())
            .await
            .unwrap()
            .unwrap(),
        "cancelled:blocked-a"
    );
    release.add_permits(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), b)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    assert!(!client.peer().is_transport_closed());
    let error = pool
        .call(
            &workspace,
            CallToolRequestParams::new("invalid"),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "fixture invalid parameters");
    assert!(
        pool.call(
            &workspace,
            CallToolRequestParams::new("valid"),
            CancellationToken::new()
        )
        .await
        .is_ok()
    );
    assert!(pool.is_loaded(workspace.id.as_str()).await);
    {
        let mut slots = pool.slots.lock();
        slots.get(workspace.id.as_str()).unwrap().lock().last_used =
            Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        prune_idle(&mut slots);
    }
    assert!(!pool.is_loaded(workspace.id.as_str()).await);
    assert!(close.is_cancelled(), "eviction must signal reader shutdown");
    let replacement = pool.slot(workspace.id.as_str()).unwrap();
    assert!(
        replacement.lock().event.is_none(),
        "next use starts a new load"
    );
    drop(replacement);
    pool.shutdown().await;
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
