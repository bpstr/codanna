//! Session roots and immutable request scope. Local client context may bootstrap
//! a project; arbitrary tool-argument paths may only select existing registrations.
use super::budget::Budget;
use crate::cli::automatic::{StartupMode, prepare_root, resolve_root, resolve_session_root};
use crate::init::workspaces::{Workspace, WorkspaceRegistry};
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const ROOT_INPUT: &str = "codanna-workspace-roots";
pub(super) enum Route {
    Ready(Workspace, Map<String, Value>),
    InputRequired(InputRequiredResult),
}
struct Pending {
    tool: String,
    arguments: Map<String, Value>,
    expires: Instant,
    generation: u64,
}
pub(super) struct ScopeResolver {
    registry: WorkspaceRegistry,
    cwd: PathBuf,
    home: Option<PathBuf>,
    budget: Budget,
    pending: Mutex<HashMap<String, Pending>>,
    roots: Mutex<Option<(u64, Vec<PathBuf>)>>,
    generation: AtomicU64,
}
impl ScopeResolver {
    pub(super) fn new(
        registry: WorkspaceRegistry,
        cwd: PathBuf,
        home: Option<PathBuf>,
        budget: Budget,
    ) -> Self {
        Self {
            registry,
            cwd,
            home,
            budget,
            pending: Mutex::new(HashMap::new()),
            roots: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }
    /// Called in receive order, before the SDK can dispatch another request.
    /// Revision checks invalidate both cached roots and pending continuations;
    /// clearing the cache asynchronously would reintroduce the routing race.
    pub(super) fn invalidate_roots(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    #[allow(deprecated)] // The old RPC is used only for older protocol versions.
    pub(super) async fn resolve(
        &self,
        request: &CallToolRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> Result<Route, ErrorData> {
        let mut arguments = request.arguments.clone().unwrap_or_default();
        let workspace = take_selector(&mut arguments, "workspace", 256)?;
        let path = take_selector(&mut arguments, "project_path", 4096)?;
        if workspace.is_some() && path.is_some() {
            return Err(invalid("Supply workspace OR project_path, not both"));
        }
        if request.request_state.is_some() && (workspace.is_some() || path.is_some()) {
            return Err(invalid("Do not change scope during a roots continuation"));
        }
        if let Some(selector) = workspace {
            reject_input(request)?;
            let registry = self.registry.clone();
            let selected = self
                .budget
                .run(&context.ct, move |_| {
                    registry.get(&selector).map_err(super::internal)
                })
                .await?;
            return Ok(Route::Ready(selected, arguments));
        }
        if let Some(path) = path {
            reject_input(request)?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(invalid("project_path must be absolute"));
            }
            return self.paths(vec![path], arguments, context, false).await;
        }
        let capabilities = context
            .client_capabilities()
            .map(serde_json::to_value)
            .transpose()
            .map_err(super::internal)?;
        let roots_capability = capabilities.as_ref().and_then(|caps| caps.get("roots"));
        let supported = roots_capability.is_some_and(Value::is_object);
        let cacheable = roots_capability
            .and_then(|caps| caps.get("listChanged"))
            .and_then(Value::as_bool)
            == Some(true);
        let generation = self.generation();
        if let Some(token) = &request.request_state {
            if token.len() != 64 {
                return Err(invalid("Unknown workspace continuation"));
            }
            let pending = self
                .pending
                .lock()
                .await
                .remove(token)
                .ok_or_else(|| invalid("Unknown or expired workspace continuation"))?;
            if pending.expires <= Instant::now()
                || pending.tool != request.name.as_ref()
                || pending.arguments != arguments
                || pending.generation != generation
            {
                return Err(invalid(
                    "Workspace continuation expired, roots changed, or arguments changed",
                ));
            }
            let responses = request
                .input_responses
                .as_ref()
                .ok_or_else(|| invalid("Missing roots response"))?;
            if responses.len() != 1 {
                return Err(invalid("Expected exactly one roots response"));
            }
            let paths = root_paths(
                responses
                    .get(ROOT_INPUT)
                    .ok_or_else(|| invalid("Missing roots response"))?,
            )?;
            if cacheable && generation == self.generation() {
                *self.roots.lock().await = Some((generation, paths.clone()));
            }
            return self.paths(paths, arguments, context, true).await;
        }
        reject_input(request)?;
        if !supported {
            return self
                .paths(vec![self.cwd.clone()], arguments, context, true)
                .await;
        }
        if cacheable {
            let cached = self.roots.lock().await.clone();
            if let Some((revision, paths)) = cached.filter(|(revision, _)| *revision == generation)
            {
                let _ = revision;
                return self.paths(paths, arguments, context, true).await;
            }
        }
        if context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28)
        {
            let mut pending = self.pending.lock().await;
            pending.retain(|_, value| value.expires > Instant::now());
            if pending.len() >= 32 {
                return Err(invalid("Too many pending roots requests"));
            }
            let token = hex::encode(rand::random::<[u8; 32]>());
            pending.insert(
                token.clone(),
                Pending {
                    tool: request.name.to_string(),
                    arguments,
                    expires: Instant::now() + Duration::from_secs(30),
                    generation,
                },
            );
            let roots: ListRootsRequest =
                serde_json::from_value(serde_json::json!({"method": "roots/list"}))
                    .map_err(super::internal)?;
            let mut inputs = InputRequests::new();
            inputs.insert(ROOT_INPUT.into(), InputRequest::ListRoots(roots));
            return Ok(Route::InputRequired(InputRequiredResult::new(
                Some(inputs),
                Some(token),
            )));
        }
        let roots = tokio::select! {
            _ = context.ct.cancelled() => return Err(invalid("Workspace selection cancelled")),
            result = tokio::time::timeout(Duration::from_secs(3), context.peer.list_roots()) => result.map_err(|_| invalid("Client roots timed out"))?.map_err(|_| invalid("Client roots unavailable; supply workspace explicitly"))?,
        };
        let paths = root_paths(&serde_json::to_value(roots).map_err(super::internal)?)?;
        if generation != self.generation() {
            return Err(invalid("Client roots changed during selection; retry"));
        }
        if cacheable {
            *self.roots.lock().await = Some((generation, paths.clone()));
        }
        self.paths(paths, arguments, context, true).await
    }
    async fn paths(
        &self,
        paths: Vec<PathBuf>,
        arguments: Map<String, Value>,
        context: &RequestContext<RoleServer>,
        session: bool,
    ) -> Result<Route, ErrorData> {
        let registry = self.registry.clone();
        let home = self.home.clone();
        let selected = self.budget.run(&context.ct, move |ct| {
            let mut roots = BTreeSet::new();
            for path in paths {
                if ct.is_cancelled() { return Err(invalid("Workspace selection cancelled")); }
                let root = if session { resolve_session_root(&path, home.as_deref()) } else { resolve_root(&path, home.as_deref()).map(|(root, _)| root) }.map_err(super::internal)?;
                roots.insert(root);
            }
            // Validate the complete root set before creating any state.
            if roots.len() != 1 { return Err(invalid("Client roots span multiple workspaces. Select a workspace explicitly; unrelated graphs are never merged.")); }
            let root = roots.into_iter().next().ok_or_else(|| invalid("Workspace selection required"))?;
            let known = registry.list().map_err(super::internal)?;
            let mut matching = known.into_iter().filter(|workspace| workspace.root == root);
            if let Some(workspace) = matching.next() {
                if matching.next().is_some() { return Err(invalid("Duplicate registrations claim the workspace root")); }
                return Ok(workspace);
            }
            if !session { return Err(invalid("project_path may only select an existing workspace; open the project in your MCP client for automatic setup")); }
            if ct.is_cancelled() { return Err(invalid("Workspace setup cancelled")); }
            prepare_root(&registry, &root, StartupMode::Bootstrap).map_err(super::internal)
        }).await?;
        Ok(Route::Ready(selected, arguments))
    }
}
fn reject_input(request: &CallToolRequestParams) -> Result<(), ErrorData> {
    if request.input_responses.is_some() || request.request_state.is_some() {
        return Err(invalid("Unexpected workspace continuation input"));
    }
    Ok(())
}
fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}
fn take_selector(
    arguments: &mut Map<String, Value>,
    key: &str,
    limit: usize,
) -> Result<Option<String>, ErrorData> {
    match arguments.remove(key) {
        None => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= limit => {
            Ok(Some(value))
        }
        _ => Err(invalid(format!(
            "{key} must be a nonempty string of at most {limit} bytes"
        ))),
    }
}
fn root_paths(value: &Value) -> Result<Vec<PathBuf>, ErrorData> {
    let roots = value
        .get("roots")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Invalid roots response"))?;
    if roots.is_empty() || roots.len() > 64 {
        return Err(invalid("Client must supply 1-64 roots"));
    }
    roots
        .iter()
        .map(|root| {
            let uri = root
                .get("uri")
                .and_then(Value::as_str)
                .filter(|uri| uri.len() <= 8192)
                .ok_or_else(|| invalid("Invalid root URI"))?;
            let url = reqwest::Url::parse(uri).map_err(|_| invalid("Invalid root URI"))?;
            if url.scheme() != "file"
                || url.host_str().is_some_and(|host| host != "localhost")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(invalid("Only local file:// roots are supported"));
            }
            url.to_file_path()
                .map_err(|_| invalid("Root is not an absolute local path"))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_mcp_selectors_are_strict_and_removed_before_forwarding() {
        let mut args = serde_json::json!({"workspace":"project-a","query":"topic"})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            take_selector(&mut args, "workspace", 256)
                .unwrap()
                .as_deref(),
            Some("project-a")
        );
        assert_eq!(args.len(), 1);
        for value in [
            Value::Null,
            Value::Bool(true),
            Value::String(" ".into()),
            serde_json::json!(["project-a"]),
        ] {
            args.insert("workspace".into(), value);
            assert!(take_selector(&mut args, "workspace", 256).is_err());
        }
    }
    #[test]
    fn workspace_mcp_roots_reject_remote_empty_and_excessive_inputs() {
        for uri in [
            "https://example.org/repo",
            "file://other-host/repo",
            "file:///repo?x=1",
            "file:///repo#x",
        ] {
            assert!(root_paths(&serde_json::json!({"roots":[{"uri":uri}]})).is_err());
        }
        assert!(root_paths(&serde_json::json!({"roots":[]})).is_err());
        assert!(
            root_paths(
                &serde_json::json!({"roots":vec![serde_json::json!({"uri":"file:///repo"});65]})
            )
            .is_err()
        );
    }
}
