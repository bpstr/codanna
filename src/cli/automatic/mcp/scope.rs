//! Immutable per-call scope, including legacy and stateless MCP client roots.

use crate::init::workspaces::{Workspace, WorkspaceRegistry};
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const ROOT_TIMEOUT: Duration = Duration::from_secs(3);
const STATE_LIFETIME: Duration = Duration::from_secs(30);
const MAX_PENDING: usize = 32;
const MAX_ROOTS: usize = 64;
const ROOT_INPUT: &str = "codanna-workspace-roots";

pub(super) enum Route {
    Ready(Workspace, Map<String, Value>),
    InputRequired(InputRequiredResult),
}

struct PendingRoots {
    tool: String,
    arguments: Map<String, Value>,
    expires: Instant,
}

pub(super) struct ScopeResolver {
    registry: WorkspaceRegistry,
    cwd: PathBuf,
    home: Option<PathBuf>,
    pending: Mutex<HashMap<String, PendingRoots>>,
}

impl ScopeResolver {
    pub(super) fn new(registry: WorkspaceRegistry, cwd: PathBuf, home: Option<PathBuf>) -> Self {
        Self { registry, cwd, home, pending: Mutex::new(HashMap::new()) }
    }

    /// Roots are requested from a tool handler, never during the handshake. A
    /// supplied scope wins; roots win over cwd so a HOME/launcher cwd cannot leak
    /// another project's default. Modern requests use MRTR, not deprecated RPC.
    #[allow(deprecated)] // Legacy roots RPC is used only with pre-2026-07-28 clients.
    pub(super) async fn resolve(
        &self,
        request: &CallToolRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> Result<Route, ErrorData> {
        let mut arguments = request.arguments.clone().unwrap_or_default();
        let workspace = take_selector(&mut arguments, "workspace", 256)?;
        let project_path = take_selector(&mut arguments, "project_path", 4096)?;
        if workspace.is_some() && project_path.is_some() {
            return Err(invalid("Supply workspace OR project_path, not both"));
        }
        if request.request_state.is_some() && (workspace.is_some() || project_path.is_some()) {
            return Err(invalid("Do not change the scope while continuing a roots request"));
        }
        if let Some(selector) = workspace {
            reject_unsolicited_input(request)?;
            let registry = self.registry.clone();
            let workspace = tokio::task::spawn_blocking(move || registry.get(&selector))
                .await.map_err(super::internal)?.map_err(super::internal)?;
            return Ok(Route::Ready(workspace, arguments));
        }
        if let Some(path) = project_path {
            reject_unsolicited_input(request)?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(invalid("project_path must be absolute"));
            }
            return self.resolve_paths(vec![path], arguments).await;
        }
        if let Some(state) = &request.request_state {
            // Tokens are random, short-lived, single-use handles to server-side
            // state, bound to the exact original tool and arguments. No paths or
            // permissions are trusted merely because a client echoes them.
            if state.len() != 64 {
                return Err(invalid("Unknown or expired workspace request state"));
            }
            let pending = self.pending.lock().await.remove(state)
                .ok_or_else(|| invalid("Unknown or expired workspace request state"))?;
            if pending.expires <= Instant::now()
                || pending.tool != request.name.as_ref()
                || pending.arguments != arguments
            {
                return Err(invalid("Workspace continuation expired or its tool arguments changed"));
            }
            let responses = request.input_responses.as_ref()
                .ok_or_else(|| invalid("Missing roots response"))?;
            if responses.len() != 1 {
                return Err(invalid("Expected exactly one roots response"));
            }
            let roots = responses.get(ROOT_INPUT).ok_or_else(|| invalid("Missing roots response"))?;
            return self.resolve_paths(root_paths(roots)?, arguments).await;
        }
        reject_unsolicited_input(request)?;
        let capabilities = context.client_capabilities()
            .map(serde_json::to_value).transpose().map_err(super::internal)?;
        let supports_roots = capabilities.as_ref()
            .and_then(|caps| caps.get("roots")).is_some_and(|roots| roots.is_object());
        if !supports_roots {
            return self.resolve_paths(vec![self.cwd.clone()], arguments).await;
        }
        if context.protocol_version().is_some_and(|v| v >= ProtocolVersion::V_2026_07_28) {
            let mut pending = self.pending.lock().await;
            pending.retain(|_, state| state.expires > Instant::now());
            if pending.len() >= MAX_PENDING {
                return Err(invalid("Too many pending workspace-root requests; supply workspace explicitly"));
            }
            let state = hex::encode(rand::random::<[u8; 32]>());
            pending.insert(state.clone(), PendingRoots {
                tool: request.name.to_string(), arguments,
                expires: Instant::now() + STATE_LIFETIME,
            });
            let roots: ListRootsRequest = serde_json::from_value(serde_json::json!({"method": "roots/list"}))
                .map_err(super::internal)?;
            let mut inputs = InputRequests::new();
            inputs.insert(ROOT_INPUT.into(), InputRequest::ListRoots(roots));
            return Ok(Route::InputRequired(InputRequiredResult::new(Some(inputs), Some(state))));
        }
        // Query on each unscoped legacy call. Root-change notifications need no
        // mutable "current workspace" and cannot retarget an in-flight operation.
        let roots = tokio::select! {
            _ = context.ct.cancelled() => return Err(invalid("Workspace selection cancelled")),
            result = tokio::time::timeout(ROOT_TIMEOUT, context.peer.list_roots()) => {
                result.map_err(|_| invalid("Client roots timed out; supply workspace explicitly"))?
                    .map_err(|_| invalid("Client roots unavailable; supply workspace explicitly"))?
            }
        };
        let roots = serde_json::to_value(roots).map_err(super::internal)?;
        self.resolve_paths(root_paths(&roots)?, arguments).await
    }

    async fn resolve_paths(&self, paths: Vec<PathBuf>, arguments: Map<String, Value>) -> Result<Route, ErrorData> {
        let registry = self.registry.clone();
        let home = self.home.clone();
        let selected = tokio::task::spawn_blocking(move || registered_owner(&registry, &paths, home.as_deref()))
            .await.map_err(super::internal)??;
        Ok(Route::Ready(selected, arguments))
    }
}

fn reject_unsolicited_input(request: &CallToolRequestParams) -> Result<(), ErrorData> {
    if request.input_responses.is_some() || request.request_state.is_some() {
        return Err(invalid("Unexpected workspace continuation input"));
    }
    Ok(())
}

fn take_selector(arguments: &mut Map<String, Value>, key: &str, limit: usize) -> Result<Option<String>, ErrorData> {
    match arguments.remove(key) {
        None => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= limit => Ok(Some(value)),
        Some(_) => Err(invalid(format!("{key} must be a nonempty string of at most {limit} bytes"))),
    }
}

fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}

fn root_paths(value: &Value) -> Result<Vec<PathBuf>, ErrorData> {
    let roots = value.get("roots").and_then(Value::as_array)
        .ok_or_else(|| invalid("Client did not return a valid roots list"))?;
    if roots.is_empty() || roots.len() > MAX_ROOTS {
        return Err(invalid("Client must supply 1-64 roots, or select a workspace explicitly"));
    }
    roots.iter().map(|root| {
        let uri = root.get("uri").and_then(Value::as_str)
            .filter(|uri| uri.len() <= 8192)
            .ok_or_else(|| invalid("Invalid client root URI"))?;
        let url = reqwest::Url::parse(uri).map_err(|_| invalid("Invalid client root URI"))?;
        if url.scheme() != "file"
            || url.host_str().is_some_and(|host| host != "localhost")
            || !url.username().is_empty() || url.password().is_some()
            || url.port().is_some() || url.query().is_some() || url.fragment().is_some()
        {
            return Err(invalid("Only local file:// client roots are supported"));
        }
        url.to_file_path().map_err(|_| invalid("Client root is not a local absolute path"))
    }).collect()
}

/// Resolve every root. Do not discard unknown roots then accidentally select an
/// unrelated remaining workspace. The registry is authorization for this local
/// router; client roots cannot create registrations or open arbitrary indexes.
fn registered_owner(registry: &WorkspaceRegistry, paths: &[PathBuf], home: Option<&Path>) -> Result<Workspace, ErrorData> {
    let known = registry.list().map_err(super::internal)?;
    let mut selected = BTreeMap::new();
    for path in paths {
        let (root, _) = super::super::resolve_root(path, home)
            .map_err(|error| invalid(format!("Cannot select workspace: {error}. Use list_workspaces or run codanna index in the intended product root.")))?;
        let matches: Vec<_> = known.iter().filter(|workspace| {
            workspace.root.canonicalize().ok().as_deref() == Some(root.as_path())
        }).collect();
        match matches.as_slice() {
            [workspace] => { selected.insert(workspace.id.as_str().to_owned(), (*workspace).clone()); }
            [] => return Err(invalid("Client root is not registered. Run codanna index once in the intended product root; setup is automatic.")),
            _ => return Err(invalid("Multiple registrations claim a client root; repair duplicate registrations by ID")),
        }
    }
    if selected.len() != 1 {
        return Err(invalid("Client roots span multiple workspaces. Supply workspace or project_path explicitly; Codanna will not guess."));
    }
    selected.into_values().next().ok_or_else(|| invalid("Workspace selection required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_mcp_selectors_are_strict_and_removed_before_forwarding() {
        let mut args = serde_json::json!({"workspace": "assign", "query": "task"}).as_object().unwrap().clone();
        assert_eq!(take_selector(&mut args, "workspace", 256).unwrap().as_deref(), Some("assign"));
        assert_eq!(args.len(), 1);
        for value in [Value::Null, Value::Bool(true), Value::String(" ".into()), serde_json::json!(["assign"])] {
            args.insert("workspace".into(), value);
            assert!(take_selector(&mut args, "workspace", 256).is_err());
        }
    }

    #[test]
    fn workspace_mcp_roots_reject_remote_empty_and_excessive_inputs() {
        for uri in ["https://example.org/repo", "file://other-host/repo", "file:///repo?query=yes", "file:///repo#fragment"] {
            assert!(root_paths(&serde_json::json!({"roots": [{"uri": uri}]})).is_err());
        }
        assert!(root_paths(&serde_json::json!({"roots": []})).is_err());
        assert!(root_paths(&serde_json::json!({"roots": vec![serde_json::json!({"uri": "file:///repo"}); 65]})).is_err());
    }
}
