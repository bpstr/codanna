//! Principal authentication, whole-workspace ACLs, mutation scopes and session ownership.
//!
//! Policies are supplied by the operator. One process exposes one complete workspace;
//! a reader cannot access a different workspace or invoke the reindex mutation.
//! This is not an OAuth issuer. Stdio uses the local process boundary.

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub(crate) struct NetworkAuth {
    workspace: Option<std::path::PathBuf>,
    requests: std::sync::Arc<tokio::sync::Semaphore>,
    initialization: std::sync::Arc<tokio::sync::Semaphore>,
    principals: std::sync::Arc<Vec<Principal>>,
    sessions: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, (String, std::time::Instant)>>,
    >,
}
#[derive(Clone)]
struct Principal {
    subject: String,
    digest: [u8; 32],
    reindex: bool,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    principals: Vec<PolicyPrincipal>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyPrincipal {
    subject: String,
    token_sha256: String,
    workspaces: Vec<std::path::PathBuf>,
    scopes: Vec<Scope>,
}
#[derive(serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Scope {
    Read,
    Reindex,
}
const SESSION_LIMIT: usize = 1024;
const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

impl NetworkAuth {
    pub(crate) fn from_env(settings: &crate::Settings) -> anyhow::Result<Self> {
        if let Some(path) = std::env::var_os("CODANNA_MCP_AUTH_FILE") {
            // Policy is operator supplied, never a repository-selected automatic file.
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(128 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 128 * 1024, "MCP auth policy exceeds 128 KiB");
            let workspace = settings
                .workspace_root
                .clone()
                .map(Ok)
                .unwrap_or_else(std::env::current_dir)?
                .canonicalize()?;
            return Self::from_policy(&bytes, &workspace);
        }
        let token = std::env::var("CODANNA_MCP_TOKEN").map_err(|_| {
            anyhow::anyhow!("Network MCP requires CODANNA_MCP_AUTH_FILE or CODANNA_MCP_TOKEN; no default credential is accepted.")
        })?;
        Self::new(&token)
    }

    pub(crate) fn from_policy(bytes: &[u8], workspace: &std::path::Path) -> anyhow::Result<Self> {
        let workspace = workspace.canonicalize()?;
        let policy: Policy = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            !policy.principals.is_empty() && policy.principals.len() <= 128,
            "MCP policy needs 1..128 principals"
        );
        let mut subjects = std::collections::HashSet::new();
        let mut digests = std::collections::HashSet::new();
        let mut principals = Vec::new();
        for principal in policy.principals {
            anyhow::ensure!(
                !principal.subject.trim().is_empty()
                    && principal.subject.len() <= 128
                    && subjects.insert(principal.subject.clone()),
                "MCP subjects must be unique and nonempty"
            );
            anyhow::ensure!(
                principal.token_sha256.len() == 64,
                "MCP token_sha256 must be 64 hex characters"
            );
            let mut digest = [0u8; 32];
            for (slot, part) in digest
                .iter_mut()
                .zip(principal.token_sha256.as_bytes().chunks_exact(2))
            {
                let text = std::str::from_utf8(part)?;
                *slot = u8::from_str_radix(text, 16)?;
            }
            anyhow::ensure!(
                digest.iter().any(|b| *b != 0) && digests.insert(digest),
                "MCP token hashes must be nonzero and unique"
            );
            anyhow::ensure!(
                principal.workspaces.len() <= 64 && principal.scopes.contains(&Scope::Read),
                "MCP principals require read scope and at most 64 workspaces"
            );
            let mut permitted = false;
            for root in principal.workspaces {
                anyhow::ensure!(
                    root.is_absolute(),
                    "MCP ACL workspaces must be absolute paths"
                );
                if root.canonicalize()? == workspace {
                    permitted = true;
                }
            }
            if permitted {
                principals.push(Principal {
                    subject: principal.subject,
                    digest,
                    reindex: principal.scopes.contains(&Scope::Reindex),
                });
            }
        }
        anyhow::ensure!(
            !principals.is_empty(),
            "No MCP principal is authorized for this server workspace"
        );
        Ok(Self {
            workspace: Some(workspace),
            requests: std::sync::Arc::new(tokio::sync::Semaphore::new(64)),
            initialization: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            principals: std::sync::Arc::new(principals),
            sessions: Default::default(),
        })
    }

    fn new(token: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (32..=512).contains(&token.len())
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)),
            "CODANNA_MCP_TOKEN must contain 32-512 bearer-token characters; generate a random credential"
        );
        Ok(Self {
            workspace: None,
            requests: std::sync::Arc::new(tokio::sync::Semaphore::new(64)),
            initialization: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            principals: std::sync::Arc::new(vec![Principal {
                subject: "operator".into(),
                digest: Sha256::digest(token.as_bytes()).into(),
                reindex: true,
            }]),
            sessions: Default::default(),
        })
    }

    pub(crate) fn restrict_facade(
        &self,
        facade: &mut crate::indexing::facade::IndexFacade,
    ) -> anyhow::Result<()> {
        if let Some(root) = &self.workspace {
            facade.restrict_workspace(root.clone())?;
        }
        Ok(())
    }

    pub(crate) fn validate_documents(
        &self,
        store: &crate::documents::DocumentStore,
    ) -> anyhow::Result<()> {
        if let Some(root) = &self.workspace {
            for path in store.get_indexed_paths() {
                crate::indexing::facade::IndexFacade::contained_source(root, &path)?;
            }
        }
        Ok(())
    }

    fn identity(&self, headers: &HeaderMap) -> Option<Principal> {
        let mut values = headers.get_all(header::AUTHORIZATION).iter();
        let value = values.next()?;
        if values.next().is_some() {
            return None;
        }
        let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("Bearer") || !(32..=512).contains(&token.len()) {
            return None;
        }
        let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut matched = None;
        // Evaluate all configured hashes, without an early exit revealing the match.
        for principal in self.principals.iter() {
            if bool::from(principal.digest.ct_eq(&supplied)) {
                matched = Some(principal.clone());
            }
        }
        matched
    }
    #[cfg(test)]
    fn accepts(&self, headers: &HeaderMap) -> bool {
        self.identity(headers).is_some()
    }

    /// Remove expired authorizations so their underlying transports can be closed
    /// outside this small lock. `now` is injectable for deterministic expiry tests.
    pub(super) fn expired_sessions(&self, now: std::time::Instant) -> anyhow::Result<Vec<String>> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("MCP session ownership unavailable"))?;
        let expired: Vec<_> = sessions
            .iter()
            .filter(|(_, (_, seen))| now.saturating_duration_since(*seen) >= SESSION_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            sessions.remove(id);
        }
        Ok(expired)
    }

    fn check_session(&self, id: Option<&str>, subject: &str) -> Result<(), StatusCode> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if let Some(id) = id {
            match sessions.get_mut(id) {
                Some((_, seen)) if seen.elapsed() >= SESSION_TTL => {
                    return Err(StatusCode::NOT_FOUND);
                }
                Some((owner, seen)) if owner == subject => *seen = std::time::Instant::now(),
                Some(_) => return Err(StatusCode::FORBIDDEN),
                None => return Err(StatusCode::NOT_FOUND),
            }
        }
        Ok(())
    }
}

/// Plain HTTP is local-only even with authentication: a bearer token must not
/// travel in cleartext over a network. TLS termination may proxy to loopback.
pub(crate) fn validate_bind(bind: &str, tls: bool) -> anyhow::Result<SocketAddr> {
    let address: SocketAddr = bind
        .parse()
        .map_err(|_| anyhow::anyhow!("MCP bind must be an IP socket address"))?;
    anyhow::ensure!(
        tls || address.ip().is_loopback(),
        "Non-loopback MCP requires HTTPS; plain HTTP may bind only to loopback"
    );
    Ok(address)
}

async fn authorize(State(auth): State<NetworkAuth>, request: Request, next: Next) -> Response {
    let Some(principal) = auth.identity(request.headers()) else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    };
    let mut session_headers = request.headers().get_all("mcp-session-id").iter();
    let session = match session_headers.next() {
        Some(header) => match header.to_str() {
            Ok(value) if !value.is_empty() && value.len() <= 128 => Some(value.to_owned()),
            _ => return StatusCode::BAD_REQUEST.into_response(),
        },
        None => None,
    };
    if session_headers.next().is_some() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let Err(status) = auth.check_session(session.as_deref(), &principal.subject) {
        return status.into_response();
    }
    let Ok(_permit) = auth.requests.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let is_delete = request.method() == axum::http::Method::DELETE;
    let mut request = request;
    // Hold admission through transport creation and ownership registration. A
    // preflight count alone races with other initializing clients and can leave
    // allocated transports untracked when the session limit is reached.
    let mut _initialization_permit = None;
    if request.method() == axum::http::Method::POST {
        let (parts, body) = request.into_parts();
        let bytes = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            axum::body::to_bytes(body, 1024 * 1024),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
            Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        };
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        let method = value.get("method").and_then(|m| m.as_str());
        // Client responses to server-initiated requests have no method. Session
        // ownership was already checked; rmcp validates the outstanding request ID.
        let is_response = method.is_none()
            && value.get("id").is_some()
            && (value.get("result").is_some() ^ value.get("error").is_some());
        let method = match method {
            Some(method) => method,
            None if is_response => "__response",
            None => return StatusCode::BAD_REQUEST.into_response(),
        };
        if method == "initialize" && session.is_none() {
            _initialization_permit = match auth.initialization.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
            };
            let Ok(sessions) = auth.sessions.lock() else {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            };
            if sessions.len() >= SESSION_LIMIT {
                return StatusCode::TOO_MANY_REQUESTS.into_response();
            }
        }
        let allowed = match method {
            "requests/codanna/force-reindex" => principal.reindex,
            "tools/call" => value
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .is_some_and(|name| {
                    super::catalog::ToolKind::parse(name).is_some_and(|kind| {
                        kind.access() == super::catalog::Access::Read || principal.reindex
                    })
                }),
            "subscriptions/listen"
            | "__response"
            | "initialize"
            | "ping"
            | "tools/list"
            | "resources/list"
            | "resources/templates/list"
            | "resources/read"
            | "resources/subscribe"
            | "resources/unsubscribe"
            | "prompts/list"
            | "prompts/get"
            | "completion/complete"
            | "logging/setLevel"
            | "notifications/initialized"
            | "notifications/cancelled"
            | "notifications/roots/list_changed"
            | "requests/codanna/index-stats" => true,
            _ => false,
        };
        if !allowed {
            return StatusCode::FORBIDDEN.into_response();
        }
        request = Request::from_parts(parts, axum::body::Body::from(bytes));
    }
    let response = next.run(request).await;
    if let Some(id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
    {
        if id.is_empty() || id.len() > 128 {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let Ok(mut sessions) = auth.sessions.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if sessions
            .get(id)
            .is_some_and(|(owner, _)| owner != &principal.subject)
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        if !sessions.contains_key(id) && sessions.len() >= SESSION_LIMIT {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        sessions.insert(id.into(), (principal.subject, std::time::Instant::now()));
    }
    if is_delete && response.status().is_success() {
        if let (Some(id), Ok(mut sessions)) = (session, auth.sessions.lock()) {
            sessions.remove(&id);
        }
    }
    response
}

/// Only the health endpoint is public. No token issuance, callback redirects,
/// registration or script-bearing authorization pages are exposed.
pub(crate) fn network_router(mcp: Router, auth: NetworkAuth) -> Router {
    let protected = mcp.route_layer(middleware::from_fn_with_state(auth, authorize));
    Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .merge(protected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const TOKEN: &str = "review-fixture-credential-not-for-production-123456";

    #[test]
    fn hardening_review_network_auth_has_no_default_or_ambiguous_header() {
        assert!(NetworkAuth::new("").is_err());
        assert!(NetworkAuth::new("mcp-access-token-dummy").is_err());
        let auth = NetworkAuth::new(TOKEN).unwrap();
        let mut headers = HeaderMap::new();
        assert!(!auth.accepts(&headers));
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(auth.accepts(&headers));
        headers.append(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(!auth.accepts(&headers));
    }

    #[tokio::test]
    async fn hardening_final_concurrent_initialization_does_not_allocate_untracked_sessions() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let auth = NetworkAuth::new(TOKEN).unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let app = network_router(
            Router::new().route(
                "/mcp",
                axum::routing::post({
                    let started = started.clone();
                    let release = release.clone();
                    let calls = calls.clone();
                    move || {
                        let started = started.clone();
                        let release = release.clone();
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            started.notify_one();
                            release.notified().await;
                            ([("mcp-session-id", "owned-session")], "initialized")
                        }
                    }
                }),
            ),
            auth.clone(),
        );
        let request = || {
            axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
                ))
                .unwrap()
        };
        let first = tokio::spawn(app.clone().oneshot(request()));
        tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        let second =
            tokio::time::timeout(std::time::Duration::from_secs(5), app.oneshot(request())).await;
        // Always release the first handler even when admission regresses.
        release.notify_one();
        let first = first.await.unwrap().unwrap();
        assert_eq!(
            second.unwrap().unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(auth.sessions.lock().unwrap().len(), 1);
        assert!(
            auth.check_session(Some("owned-session"), "operator")
                .is_ok()
        );
        assert_eq!(auth.initialization.available_permits(), 1);
    }

    #[test]
    fn hardening_review_network_bind_requires_tls_off_loopback() {
        assert!(validate_bind("127.0.0.1:8080", false).is_ok());
        assert!(validate_bind("[::1]:8080", false).is_ok());
        assert!(validate_bind("0.0.0.0:8080", false).is_err());
        assert!(validate_bind("[::]:8080", false).is_err());
        assert!(validate_bind("0.0.0.0:8443", true).is_ok());
    }

    #[tokio::test]
    async fn hardening_review_network_routes_require_auth_and_remove_oauth() {
        let app = network_router(
            Router::new().route("/mcp", axum::routing::any(|| async { "protected" })),
            NetworkAuth::new(TOKEN).unwrap(),
        );
        for method in ["GET", "POST", "DELETE", "OPTIONS"] {
            for token in [
                None,
                Some("Bearer mcp-access-token-dummy"),
                Some("Bearer wrong-token-that-is-long-enough-123456"),
            ] {
                let mut builder = axum::http::Request::builder().method(method).uri("/mcp");
                if let Some(value) = token {
                    builder = builder.header(header::AUTHORIZATION, value);
                }
                let response = app
                    .clone()
                    .oneshot(builder.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/mcp")
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        for path in [
            "/oauth/token",
            "/oauth/register",
            "/oauth/authorize?redirect_uri=https://invalid.example&state=%3Cscript%3E",
            "/.well-known/oauth-authorization-server",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    fn hash(token: &str) -> String {
        Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    const READER: &str = "fixture-read-principal-secret-0000000001";
    const WRITER: &str = "fixture-write-principal-secret-000000001";
    fn fixture(root: &std::path::Path) -> NetworkAuth {
        let policy = serde_json::json!({"principals":[
            {"subject":"reader","token_sha256":hash(READER),"workspaces":[root],"scopes":["read"]},
            {"subject":"writer","token_sha256":hash(WRITER),"workspaces":[root],"scopes":["read","reindex"]}
        ]});
        NetworkAuth::from_policy(&serde_json::to_vec(&policy).unwrap(), root).unwrap()
    }
    #[tokio::test]
    async fn hardening_final_network_principals_scopes_and_session_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let auth = fixture(&root);
        let app = network_router(
            Router::new().route("/mcp", axum::routing::post(|| async { StatusCode::OK })),
            auth.clone(),
        );
        for (token, method, expected) in [
            (
                READER,
                "requests/codanna/force-reindex",
                StatusCode::FORBIDDEN,
            ),
            (WRITER, "requests/codanna/force-reindex", StatusCode::OK),
            (READER, "requests/codanna/index-stats", StatusCode::OK),
            (WRITER, "unregistered/mutation", StatusCode::FORBIDDEN),
        ] {
            let request = Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"jsonrpc":"2.0","id":1,"method":method}).to_string(),
                ))
                .unwrap();
            assert_eq!(
                app.clone().oneshot(request).await.unwrap().status(),
                expected
            );
        }
        auth.sessions.lock().unwrap().insert(
            "reader-session".into(),
            ("reader".into(), std::time::Instant::now()),
        );
        assert!(auth.check_session(Some("reader-session"), "reader").is_ok());
        assert_eq!(
            auth.check_session(Some("reader-session"), "writer"),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            auth.check_session(Some("unknown"), "reader"),
            Err(StatusCode::NOT_FOUND)
        );
        auth.sessions
            .lock()
            .unwrap()
            .get_mut("reader-session")
            .unwrap()
            .1 = std::time::Instant::now() - SESSION_TTL;
        assert_eq!(
            auth.check_session(Some("reader-session"), "reader"),
            Err(StatusCode::NOT_FOUND)
        );
    }
    #[test]
    fn hardening_final_network_workspace_acl_and_policy_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let principal = serde_json::json!({"subject":"reader","token_sha256":hash(READER),"workspaces":[other.path()],"scopes":["read"]});
        let policy = serde_json::json!({"principals":[principal]});
        assert!(
            NetworkAuth::from_policy(&serde_json::to_vec(&policy).unwrap(), dir.path()).is_err()
        );
        for raw in [b"{}".as_slice(), b"{\"principals\":[]}", b"not-json"] {
            assert!(NetworkAuth::from_policy(raw, dir.path()).is_err());
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn hardening_final_workspace_policy_rejects_foreign_rows_and_future_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let other = tempfile::tempdir().unwrap();
        let settings = std::sync::Arc::new(crate::Settings {
            workspace_root: Some(root.clone()),
            index_path: root.join("index"),
            ..crate::Settings::default()
        });
        let mut facade = crate::indexing::facade::IndexFacade::new(settings).unwrap();
        facade.restrict_workspace(root.clone()).unwrap();
        let foreign = other.path().join("foreign.rs");
        std::fs::write(&foreign, "pub fn private_foreign() {}\n").unwrap();
        assert!(facade.index_file(&foreign).is_err());
        assert_eq!(facade.symbol_count(), 0);
        let path = std::path::Path::new("src/deleted/missing.rs");
        assert!(crate::indexing::facade::IndexFacade::contained_source(&root, path).is_ok());
        assert!(crate::indexing::facade::IndexFacade::contained_source(&root, &foreign).is_err());
        // A previously mixed index is refused too, not merely protected on future writes.
        facade.network_workspace = None;
        facade.index_file(&foreign).unwrap();
        assert!(facade.restrict_workspace(root).is_err());
    }
    #[tokio::test]
    async fn hardening_final_auth_bounds_payload_and_request_admission() {
        use axum::body::Body;
        use tower::ServiceExt;
        let token = "fixture-request-budget-secret-0000000001";
        let auth = NetworkAuth::new(token).unwrap();
        let app = network_router(
            Router::new().route("/mcp", axum::routing::post(|| async { StatusCode::OK })),
            auth.clone(),
        );
        let request = |body| {
            axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap()
        };
        assert_eq!(
            app.clone()
                .oneshot(request(vec![b' '; 1024 * 1024 + 1]))
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let _permits = auth.requests.acquire_many(64).await.unwrap();
        assert_eq!(
            app.oneshot(request(
                br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec()
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
