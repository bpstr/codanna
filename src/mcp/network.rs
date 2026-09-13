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
    #[cfg(all(test, feature = "https-server"))]
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
            #[cfg(all(test, feature = "https-server"))]
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

    // A client-side Response drop is not an acknowledgement that Hyper has
    // dropped the server body. rmcp deliberately creates an idle shadow GET
    // while an earlier common stream is still active. Observe the actual body
    // lifetime rather than sleeping or treating a duplicate GET as a reconnect.
    struct ObservedBody {
        inner: Option<axum::body::Body>,
        session: String,
        closed: tokio::sync::mpsc::UnboundedSender<String>,
    }
    impl http_body::Body for ObservedBody {
        type Data = axum::body::Bytes;
        type Error = axum::Error;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            std::pin::Pin::new(self.inner.as_mut().expect("body lives until drop")).poll_frame(cx)
        }
        fn is_end_stream(&self) -> bool {
            self.inner
                .as_ref()
                .expect("body lives until drop")
                .is_end_stream()
        }
        fn size_hint(&self) -> http_body::SizeHint {
            self.inner
                .as_ref()
                .expect("body lives until drop")
                .size_hint()
        }
    }
    impl Drop for ObservedBody {
        fn drop(&mut self) {
            // Release rmcp's stream receiver before acknowledging closure.
            drop(self.inner.take());
            let _ = self.closed.send(self.session.clone());
        }
    }
    async fn observe_stream_closure(
        axum::extract::State(closed): axum::extract::State<
            tokio::sync::mpsc::UnboundedSender<String>,
        >,
        request: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let session = if request.method() == axum::http::Method::GET {
            request
                .headers()
                .get("Mcp-Session-Id")
                .map(|value| value.to_str().unwrap().to_owned())
        } else {
            None
        };
        let response = next.run(request).await;
        if let Some(session) = session.filter(|_| response.status() == StatusCode::OK) {
            response.map(|body| {
                axum::body::Body::new(ObservedBody {
                    inner: Some(body),
                    session,
                    closed,
                })
            })
        } else {
            response
        }
    }

    struct EventStream {
        response: Response,
        buffered: Vec<u8>,
    }
    impl EventStream {
        async fn changed_after(&mut self, previous: Option<usize>) -> usize {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    while let Some(end) = self.buffered.windows(2).position(|pair| pair == b"\n\n")
                    {
                        let event: Vec<_> = self.buffered.drain(..end + 2).collect();
                        let text =
                            std::str::from_utf8(&event).expect("complete SSE event is UTF-8");
                        let mut id = None;
                        let mut data = String::new();
                        for line in text.lines() {
                            if let Some(value) = line.strip_prefix("id:") {
                                id = Some(
                                    value
                                        .trim()
                                        .parse::<usize>()
                                        .expect("common stream event id"),
                                );
                            }
                            if let Some(value) = line.strip_prefix("data:") {
                                data.push_str(value.trim_start());
                                data.push('\n');
                            }
                        }
                        if data.is_empty() {
                            continue;
                        }
                        let message: Value =
                            serde_json::from_str(&data).expect("SSE data is JSON-RPC");
                        if message["method"] == "notifications/resources/list_changed" {
                            let id = id.expect("legacy SSE events have replay identity");
                            // The SDK may replay its last cached event inclusively.
                            // An old event must never satisfy the next delivery assertion.
                            if previous.is_none_or(|previous| id > previous) {
                                return id;
                            }
                        }
                    }
                    let chunk = self
                        .response
                        .chunk()
                        .await
                        .unwrap()
                        .expect("SSE must remain open");
                    self.buffered.extend_from_slice(&chunk);
                    assert!(
                        self.buffered.len() < 64 * 1024,
                        "unbounded incomplete SSE event"
                    );
                }
            })
            .await
            .expect("each active session must receive a new notification")
        }
    }

    struct Fixture {
        url: String,
        client: Client,
        events: Arc<NotificationBroadcaster>,
        closed: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<String>>,
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
            let (closed_tx, closed_rx) = tokio::sync::mpsc::unbounded_channel();
            let router = service.router.layer(axum::middleware::from_fn_with_state(
                closed_tx,
                observe_stream_closure,
            ));
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
                        .serve(router.into_make_service())
                        .await
                        .unwrap();
                })
            } else {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                tokio::spawn(async move {
                    axum::serve(listener, router).await.unwrap();
                })
            };
            Self {
                url: format!("{}://{address}/mcp", if tls { "https" } else { "http" }),
                client: client.build().unwrap(),
                events,
                closed: tokio::sync::Mutex::new(closed_rx),
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
        async fn stream(&self, token: &str, sid: &str, previous: Option<usize>) -> EventStream {
            let mut request = self
                .client
                .get(&self.url)
                .bearer_auth(token)
                .header("Accept", "text/event-stream")
                .header("Mcp-Session-Id", sid)
                .header("MCP-Protocol-Version", PROTOCOL);
            if let Some(previous) = previous {
                request = request.header("Last-Event-ID", previous.to_string());
            }
            let response = tokio::time::timeout(Duration::from_secs(5), request.send())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            EventStream {
                response,
                buffered: Vec::new(),
            }
        }
        async fn disconnected(&self, expected: &str) {
            let actual = tokio::time::timeout(Duration::from_secs(5), async {
                self.closed
                    .lock()
                    .await
                    .recv()
                    .await
                    .expect("closure observer stays alive")
            })
            .await
            .expect("the server must observe client SSE disconnection");
            assert_eq!(actual, expected, "only the disconnected stream may close");
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
        let mut first = server.stream(READER, &a, None).await;
        let mut second = server.stream(WRITER, &b, None).await;
        server.events.send(FileChangeEvent::IndexReloaded);
        let first_id = first.changed_after(None).await;
        let second_id = second.changed_after(None).await;
        drop(first);
        // No artificial close/retry/sleep: await Hyper dropping the actual
        // response body, while both Codanna session listeners must stay alive.
        server.disconnected(&a).await;
        server.subscribers(2).await;
        let mut first = server.stream(READER, &a, Some(first_id)).await;
        server.events.send(FileChangeEvent::IndexReloaded);
        first.changed_after(Some(first_id)).await;
        let second_id = second.changed_after(Some(second_id)).await;
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
        second.changed_after(Some(second_id)).await;
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
