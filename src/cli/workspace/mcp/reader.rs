//! Private subprocess reader: strict existing-index loading and lazy facilities.
//! Lexical requests never initialize a semantic backend. A shared initializer
//! belongs to the reader, not to the lifetime of its first requesting client.
use crate::init::workspaces::{confined_index_path, read_settings};
use crate::mcp::server::CodeIntelligenceServer;
use crate::storage::{IndexMetadata, IndexPersistence};
use crate::{IndexError, Settings};
use parking_lot::Mutex;
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{RwLock, watch};

type Documents = Option<Arc<RwLock<crate::documents::DocumentStore>>>;
type LoadResult<T> = Option<Result<T, String>>;

/// At most one task initializes a facility in this child process. Dropping a
/// waiter cannot start duplicate blocking work or discard an initialization error.
struct Lazy<T> {
    state: Mutex<Option<watch::Receiver<LoadResult<T>>>>,
}
impl<T> Default for Lazy<T> {
    fn default() -> Self {
        Self { state: Mutex::new(None) }
    }
}
impl<T: Clone + Send + Sync + 'static> Lazy<T> {
    async fn get(&self, initialize: impl Future<Output = Result<T, String>> + Send + 'static) -> Result<T, String> {
        let mut receiver = {
            let mut state = self.state.lock();
            match state.as_ref() {
                Some(receiver) => receiver.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    *state = Some(receiver.clone());
                    tokio::spawn(async move { sender.send_replace(Some(initialize.await)); });
                    receiver
                }
            }
        };
        loop {
            let result = receiver.borrow().clone();
            if let Some(result) = result { return result; }
            receiver.changed().await.map_err(|_| "Workspace facility initialization stopped unexpectedly".to_owned())?;
        }
    }
}

#[derive(Clone)]
struct Reader {
    code: CodeIntelligenceServer,
    settings: Arc<Settings>,
    semantic: Arc<Lazy<()>>,
    documents: Arc<Lazy<Documents>>,
}
impl ServerHandler for Reader {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("codanna-workspace-reader", env!("CARGO_PKG_VERSION")),
        )
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.code.list_tools(request, context).await
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if matches!(request.name.as_ref(), "semantic_search_docs" | "semantic_search_with_context")
            && self.settings.semantic_search.enabled
        {
            let facade = self.code.facade.clone();
            let path = self.settings.index_path.join("semantic");
            self.semantic.get(async move {
                crate::runtime::mutate(&facade, move |index| -> Result<(), IndexError> {
                    if index.has_semantic_search() || index.load_semantic_search(&path)? {
                        // Initialize on the owned facade before read snapshots are
                        // cloned; otherwise empty OnceLocks could spawn duplicate
                        // query backends in independent facade snapshots.
                        index.ensure_embedding_pool()?;
                    }
                    Ok(())
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())
            }).await.map_err(super::internal)?;
        }
        let mut server = self.code.clone();
        if matches!(request.name.as_ref(), "search_context" | "search_documents") {
            let settings = self.settings.clone();
            let documents = self.documents.get(async move {
                crate::runtime::blocking(move || crate::documents::load_from_settings(&settings))
                    .await.map_err(|error| error.to_string())
            }).await.map_err(super::internal)?;
            if let Some(store) = documents {
                server = server.with_document_store_arc(store);
            }
        }
        server.call_tool(request, context).await
    }
}

pub(crate) async fn run(root: &Path) -> Result<i32, IndexError> {
    let root = root.canonicalize().map_err(|source| IndexError::FileRead {
        path: root.to_path_buf(),
        source,
    })?;
    if std::env::current_dir()
        .map_err(|error| IndexError::General(error.to_string()))?
        .canonicalize()
        .map_err(|error| IndexError::General(error.to_string()))? != root
    {
        return Err(IndexError::General("Workspace reader must start in its selected root".into()));
    }
    let mut settings = read_settings(&root)?;
    settings.index_path = confined_index_path(&root, &settings)?;
    settings.workspace_root = Some(root.clone());
    settings.indexed_paths_cache = settings.indexing.indexed_paths.iter()
        .map(|path| root.join(path).canonicalize())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| IndexError::General(error.to_string()))?;
    let metadata = IndexMetadata::load(&settings.index_path)?;
    if metadata.emission_version != Some(crate::storage::metadata::EMISSION_SEMANTICS_VERSION) {
        return Err(IndexError::General("Index semantics changed. Run codanna index in this workspace; reads will not rebuild it.".into()));
    }
    let settings = Arc::new(settings);
    let owned = settings.clone();
    let facade = crate::runtime::blocking(move || {
        let mut facade = IndexPersistence::new(owned.index_path.clone()).load_facade_lite(owned)?;
        // Validate stored provenance too: copied/shared index contents are not
        // made safe merely by placing the files under the selected root.
        facade.restrict_workspace(root)?;
        Ok::<_, IndexError>(facade)
    }).await??;
    let reader = Reader {
        code: CodeIntelligenceServer::new(facade),
        settings,
        semantic: Arc::new(Lazy::default()),
        documents: Arc::new(Lazy::default()),
    };
    let running = reader.serve(rmcp::transport::stdio()).await
        .map_err(|error| IndexError::General(error.to_string()))?;
    running.waiting().await.map_err(|error| IndexError::General(error.to_string()))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn hardening_workspace_cancelled_lazy_waiter_does_not_restart_initialization() {
        let lazy = Arc::new(Lazy::<u32>::default());
        let count = Arc::new(AtomicUsize::new(0));
        let (started, observe_start) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let owned = lazy.clone();
        let runs = count.clone();
        let waiter = tokio::spawn(async move {
            owned.get(async move {
                runs.fetch_add(1, Ordering::SeqCst);
                let _ = started.send(());
                wait.await.map_err(|error| error.to_string())?;
                Ok(42)
            }).await
        });
        observe_start.await.unwrap();
        waiter.abort();
        let _ = waiter.await;
        release.send(()).unwrap();
        let value = tokio::time::timeout(Duration::from_secs(2), lazy.get(async {
            Err("a second initializer must never execute".to_owned())
        })).await.unwrap().unwrap();
        assert_eq!(value, 42);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hardening_workspace_lazy_errors_are_retained_not_reported_as_missing_data() {
        let lazy = Lazy::<bool>::default();
        assert_eq!(lazy.get(async { Err("fixture failure".to_owned()) }).await.unwrap_err(), "fixture failure");
        assert_eq!(lazy.get(async { Ok(true) }).await.unwrap_err(), "fixture failure");
    }
}
