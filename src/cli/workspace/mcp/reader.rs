//! Private subprocess reader: no auto-init, provider-cache rebuild, or model load
//! on lexical requests. It exposes tools only and opens an existing index strictly.
use crate::init::workspaces::{confined_index_path, read_settings};
use crate::mcp::server::CodeIntelligenceServer;
use crate::storage::{IndexMetadata, IndexPersistence};
use crate::{IndexError, Settings};
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{OnceCell, RwLock};

type Documents = Option<Arc<RwLock<crate::documents::DocumentStore>>>;
#[derive(Clone)]
struct Reader {
    code: CodeIntelligenceServer,
    settings: Arc<Settings>,
    semantic: Arc<OnceCell<Result<bool, String>>>,
    documents: Arc<OnceCell<Documents>>,
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
        if matches!(
            request.name.as_ref(),
            "semantic_search_docs" | "semantic_search_with_context"
        ) && self.settings.semantic_search.enabled
        {
            let facade = self.code.facade.clone();
            let path = self.settings.index_path.join("semantic");
            let outcome = self
                .semantic
                .get_or_init(|| async move {
                    crate::runtime::mutate(&facade, move |index| {
                        // A cancelled waiter cannot cause a second successful load:
                        // the mutation lane retains ownership until publication.
                        if index.has_semantic_search() {
                            Ok(true)
                        } else {
                            index.load_semantic_search(&path)
                        }
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())
                })
                .await;
            outcome.clone().map_err(super::internal)?;
        }
        let mut server = self.code.clone();
        if matches!(request.name.as_ref(), "search_context" | "search_documents") {
            let settings = self.settings.clone();
            let documents = self
                .documents
                .get_or_init(|| async move {
                    crate::runtime::blocking(move || {
                        crate::documents::load_from_settings(&settings)
                    })
                    .await
                    .unwrap_or(None)
                })
                .await;
            if let Some(store) = documents {
                server = server.with_document_store_arc(store.clone());
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
        .map_err(|e| IndexError::General(e.to_string()))?
        .canonicalize()
        .map_err(|e| IndexError::General(e.to_string()))?
        != root
    {
        return Err(IndexError::General(
            "Workspace reader must start in its selected root".into(),
        ));
    }
    let mut settings = read_settings(&root)?;
    settings.index_path = confined_index_path(&root, &settings)?;
    settings.workspace_root = Some(root.clone());
    settings.indexed_paths_cache = settings
        .indexing
        .indexed_paths
        .iter()
        .map(|path| root.join(path).canonicalize())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| IndexError::General(e.to_string()))?;
    let metadata = IndexMetadata::load(&settings.index_path)?;
    if metadata.emission_version != Some(crate::storage::metadata::EMISSION_SEMANTICS_VERSION) {
        return Err(IndexError::General("Index semantics changed. Run codanna index in this workspace; reads will not rebuild it.".into()));
    }
    let settings = Arc::new(settings);
    let owned = settings.clone();
    let facade = crate::runtime::blocking(move || {
        IndexPersistence::new(owned.index_path.clone()).load_facade_lite(owned)
    })
    .await??;
    let reader = Reader {
        code: CodeIntelligenceServer::new(facade),
        settings,
        semantic: Arc::new(OnceCell::new()),
        documents: Arc::new(OnceCell::new()),
    };
    let running = reader
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| IndexError::General(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| IndexError::General(e.to_string()))?;
    Ok(0)
}
