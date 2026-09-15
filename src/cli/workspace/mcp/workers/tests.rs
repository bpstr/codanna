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
    let (_sender, receiver) = watch::channel(Event::Ready(reader));
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
    pool.shutdown().await;
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
