//! Shared authenticated transport construction for HTTP and HTTPS.
//! Session identity belongs to a freshly constructed server, never a clone shared
//! across clients. Idle authorization expiry also closes the rmcp transport.

use super::{CodeIntelligenceServer, auth::NetworkAuth, notifications::NotificationBroadcaster};
use crate::{Settings, documents::DocumentStore, indexing::facade::IndexFacade};
use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
    session::{SessionManager, local::LocalSessionManager},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub(crate) struct NetworkService {
    pub router: Router,
    #[cfg(test)]
    manager: Arc<LocalSessionManager>,
}

impl NetworkService {
    pub(crate) fn new(
        indexer: Arc<RwLock<IndexFacade>>,
        settings: Arc<Settings>,
        documents: Option<Arc<RwLock<DocumentStore>>>,
        broadcaster: Arc<NotificationBroadcaster>,
        cancellation: CancellationToken,
        auth: NetworkAuth,
    ) -> Self {
        let manager = Arc::new(LocalSessionManager::default());
        let mut transport = StreamableHttpServerConfig::default()
            .with_cancellation_token(cancellation.child_token())
            .with_sse_keep_alive(Some(Duration::from_secs(15)))
            .with_sse_retry(None)
            .with_legacy_session_mode(true)
            .with_json_response(false);
        if let Some(hosts) = &settings.mcp.allowed_hosts {
            transport = transport.with_allowed_hosts(hosts.clone());
        }
        if let Some(origins) = &settings.mcp.allowed_origins {
            transport = transport.with_allowed_origins(origins.clone());
        }
        let service = StreamableHttpService::new(
            move || {
                let server =
                    CodeIntelligenceServer::new_with_facade(indexer.clone(), settings.clone())
                        .with_broadcaster(broadcaster.clone());
                Ok(match &documents {
                    Some(store) => server.with_document_store_arc(store.clone()),
                    None => server,
                })
            },
            manager.clone(),
            transport,
        );
        let expiry_manager = manager.clone();
        let expiry_auth = auth.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break,
                    _ = tick.tick() => {
                        tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => break,
                            result = reap_sessions(&expiry_auth, &expiry_manager, Instant::now()) => {
                                if result.is_err() {
                                    tracing::warn!(target: "mcp", "session expiry cleanup failed");
                                }
                            }
                        }
                    }
                }
            }
        });
        Self {
            router: super::auth::network_router(Router::new().nest_service("/mcp", service), auth),
            #[cfg(test)]
            manager,
        }
    }
}

async fn reap_sessions(
    auth: &NetworkAuth,
    manager: &LocalSessionManager,
    now: Instant,
) -> anyhow::Result<()> {
    let mut failures = 0;
    for id in auth.expired_sessions(now)? {
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), manager.close_session(&id.into())).await,
            Ok(Ok(()))
        ) {
            failures += 1;
        }
    }
    anyhow::ensure!(
        failures == 0,
        "MCP session expiry cleanup failed for {failures} sessions"
    );
    Ok(())
}

#[cfg(all(test, feature = "https-server"))]
mod tests {
    use super::*;
    use crate::mcp::notifications::FileChangeEvent;
    use reqwest::{Client, Response, StatusCode};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    const READER: &str = "reader-fixture-secret-not-for-use-1234567890";
    const WRITER: &str = "writer-fixture-secret-not-for-use-1234567890";
    const PROTOCOL: &str = "2025-11-25";

    struct Fixture {
        url: String,
        client: Client,
        events: Arc<NotificationBroadcaster>,
        manager: Arc<LocalSessionManager>,
        auth: NetworkAuth,
        stop: CancellationToken,
        task: tokio::task::JoinHandle<()>,
        _dir: tempfile::TempDir,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.cancel();
            self.task.abort();
        }
    }
    impl Fixture {
        async fn new(tls: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let settings = Arc::new(Settings {
                workspace_root: Some(dir.path().canonicalize().unwrap()),
                index_path: dir.path().join("index"),
                semantic_search: crate::config::SemanticSearchConfig {
                    enabled: false,
                    ..Default::default()
                },
                ..Settings::default()
            });
            let principal = |subject: &str, token: &str, scopes: Vec<&str>| {
                json!({
                    "subject":subject,"token_sha256":hex::encode(Sha256::digest(token.as_bytes())),
                    "workspaces":[settings.workspace_root],"scopes":scopes
                })
            };
            let policy = json!({"principals":[
                principal("reader", READER, vec!["read"]),
                principal("writer", WRITER, vec!["read", "reindex"])
            ]});
            let auth = NetworkAuth::from_policy(&serde_json::to_vec(&policy).unwrap(), dir.path())
                .unwrap();
            let mut index = IndexFacade::new(settings.clone()).unwrap();
            auth.restrict_facade(&mut index).unwrap();
            let events = Arc::new(NotificationBroadcaster::new(16));
            let stop = CancellationToken::new();
            let service = NetworkService::new(
                Arc::new(RwLock::new(index)),
                settings,
                None,
                events.clone(),
                stop.clone(),
                auth.clone(),
            );
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let mut client = Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5));
            let task = if tls {
                let certificate =
                    rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
                let pem = certificate.cert.pem();
                client = client
                    .add_root_certificate(reqwest::Certificate::from_pem(pem.as_bytes()).unwrap());
                let config = axum_server::tls_rustls::RustlsConfig::from_pem(
                    pem.into_bytes(),
                    certificate.signing_key.serialize_pem().into_bytes(),
                )
                .await
                .unwrap();
                tokio::spawn(async move {
                    axum_server::from_tcp_rustls(listener, config)
                        .unwrap()
                        .serve(service.router.into_make_service())
                        .await
                        .unwrap();
                })
            } else {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                tokio::spawn(async move {
                    axum::serve(listener, service.router).await.unwrap();
                })
            };
            Self {
                url: format!("{}://{address}/mcp", if tls { "https" } else { "http" }),
                client: client.build().unwrap(),
                events,
                manager: service.manager,
                auth,
                stop,
                task,
                _dir: dir,
            }
        }
        async fn request(&self, token: &str, sid: Option<&str>, body: Value) -> Response {
            let mut request = self
                .client
                .post(&self.url)
                .bearer_auth(token)
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROTOCOL);
            if let Some(sid) = sid {
                request = request.header("Mcp-Session-Id", sid);
            }
            tokio::time::timeout(Duration::from_secs(5), request.json(&body).send())
                .await
                .unwrap()
                .unwrap()
        }
        async fn initialize(&self, token: &str) -> String {
            let response = self.request(token, None, json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"protocolVersion":PROTOCOL,"capabilities":{},"clientInfo":{"name":"session-fixture","version":"0"}}})).await;
            assert_eq!(response.status(), StatusCode::OK);
            let sid = response.headers()["mcp-session-id"]
                .to_str()
                .unwrap()
                .to_string();
            let body = tokio::time::timeout(Duration::from_secs(5), response.text())
                .await
                .unwrap()
                .unwrap();
            assert!(body.contains("serverInfo"), "{body}");
            assert!(
                self.request(
                    token,
                    Some(&sid),
                    json!({"jsonrpc":"2.0","method":"notifications/initialized"})
                )
                .await
                .status()
                .is_success()
            );
            sid
        }
        async fn stream(&self, token: &str, sid: &str) -> Response {
            let response = tokio::time::timeout(
                Duration::from_secs(5),
                self.client
                    .get(&self.url)
                    .bearer_auth(token)
                    .header("Accept", "text/event-stream")
                    .header("Mcp-Session-Id", sid)
                    .header("MCP-Protocol-Version", PROTOCOL)
                    .send(),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            response
        }
        async fn subscribers(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(5), async {
                while self.events.subscriber_count() != expected {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "expected {expected} listeners, found {}",
                    self.events.subscriber_count()
                )
            });
        }
    }
    async fn changed(response: &mut Response) {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut body = String::new();
            loop {
                let chunk = response
                    .chunk()
                    .await
                    .unwrap()
                    .expect("SSE must remain open");
                body.push_str(&String::from_utf8_lossy(&chunk));
                if body.contains("notifications/resources/list_changed") {
                    break;
                }
                assert!(body.len() < 64 * 1024, "unexpected unbounded SSE response");
            }
        })
        .await
        .expect("each active session must receive the notification");
    }
    async fn exercise(tls: bool) {
        let server = Fixture::new(tls).await;
        let a = server.initialize(READER).await;
        let b = server.initialize(WRITER).await;
        assert_ne!(a, b);
        server.subscribers(2).await;
        let stolen = server
            .request(
                READER,
                Some(&b),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            )
            .await;
        assert_eq!(stolen.status(), StatusCode::FORBIDDEN);
        let mutation = server.request(READER, Some(&a), json!({"jsonrpc":"2.0","id":3,"method":"requests/codanna/force-reindex","params":{"paths":[]}})).await;
        assert_eq!(mutation.status(), StatusCode::FORBIDDEN);
        let mut first = server.stream(READER, &a).await;
        let mut second = server.stream(WRITER, &b).await;
        server.events.send(FileChangeEvent::IndexReloaded);
        changed(&mut first).await;
        changed(&mut second).await;
        drop(first); // temporary SSE disconnect keeps exactly one reconnectable session
        let mut first = server.stream(READER, &a).await;
        server.subscribers(2).await;
        server.events.send(FileChangeEvent::IndexReloaded);
        changed(&mut first).await;
        changed(&mut second).await;
        let deleted = server
            .client
            .delete(&server.url)
            .timeout(Duration::from_secs(5))
            .bearer_auth(READER)
            .header("Mcp-Session-Id", &a)
            .header("MCP-Protocol-Version", PROTOCOL)
            .send()
            .await
            .unwrap();
        assert!(deleted.status().is_success());
        drop(first);
        server.subscribers(1).await;
        server.events.send(FileChangeEvent::IndexReloaded);
        changed(&mut second).await;
        // Inject the clock into the real reaper rather than waiting an hour.
        reap_sessions(
            &server.auth,
            &server.manager,
            Instant::now() + Duration::from_secs(3601),
        )
        .await
        .unwrap();
        server.subscribers(0).await;
        assert!(!server.manager.has_session(&b.clone().into()).await.unwrap());
        let expired = server
            .request(
                WRITER,
                Some(&b),
                json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}),
            )
            .await;
        assert_eq!(expired.status(), StatusCode::NOT_FOUND);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hardening_final_https_sessions_reconnect_close_expire_and_enforce_ownership() {
        exercise(true).await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hardening_final_http_sessions_reconnect_close_expire_and_enforce_ownership() {
        exercise(false).await;
    }
}
