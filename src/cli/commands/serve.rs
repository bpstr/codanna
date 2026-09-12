//! Serve command - MCP server modes (stdio, HTTP, HTTPS).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::Settings;
use crate::indexing::facade::IndexFacade;

/// Lifetime-owned OS lock for stdio MCP servers.
///
/// The inode must remain at this path after release. Unlinking a lockfile lets
/// contenders lock different inodes and defeats mutual exclusion. PID contents
/// are diagnostics only; kernel lock ownership is authoritative.
#[derive(Debug)]
struct ServeLockGuard {
    _file: std::fs::File,
}

#[derive(Debug)]
enum ServeLockError {
    AlreadyRunning { pid: u32, lock_path: PathBuf },
    Io(std::io::Error),
}

impl ServeLockGuard {
    fn acquire(index_path: &Path) -> Result<Self, ServeLockError> {
        let lock_path = index_path.join("serve.lock");
        std::fs::create_dir_all(index_path).map_err(ServeLockError::Io)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(ServeLockError::Io)?;
        match fs4::fs_std::FileExt::try_lock_exclusive(&file) {
            Ok(true) => {}
            Ok(false) => {
                return Err(ServeLockError::AlreadyRunning {
                    pid: read_lock_pid(&lock_path).unwrap_or(0),
                    lock_path,
                });
            }
            Err(err) => return Err(ServeLockError::Io(err)),
        }
        // Only a lock owner may change the diagnostic PID. Closing the handle
        // releases the lock even when writing fails, or the process crashes.
        file.set_len(0).map_err(ServeLockError::Io)?;
        write!(file, "{}", std::process::id()).map_err(ServeLockError::Io)?;
        Ok(Self { _file: file })
    }
}

fn read_lock_pid(lock_path: &Path) -> Option<u32> {
    std::fs::read_to_string(lock_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// Arguments for the serve command.
pub struct ServeArgs {
    pub watch: bool,
    pub watch_interval: u64,
    pub http: bool,
    pub https: bool,
    pub bind: String,
}

/// Run the serve command.
pub async fn run(
    args: ServeArgs,
    config: Settings,
    settings: Arc<Settings>,
    facade: IndexFacade,
    index_path: PathBuf,
) {
    let ServeArgs {
        watch,
        watch_interval,
        http,
        https,
        bind,
    } = args;

    // Determine server mode:
    // 1. CLI --https flag takes highest precedence
    // 2. CLI --http flag takes second precedence
    // 3. Otherwise, check config.server.mode
    let server_mode = if https {
        "https"
    } else if http || config.server.mode == "http" {
        "http"
    } else {
        "stdio"
    };

    // Use bind address from CLI if provided, otherwise from config
    // For HTTPS, default to port 8443 if using default bind
    let bind_address = if bind != "127.0.0.1:8080" {
        // CLI flag was explicitly set (not default)
        bind
    } else if https {
        // For HTTPS, use port 8443 by default
        "127.0.0.1:8443".to_string()
    } else {
        // Use config value
        config.server.bind.clone()
    };

    // Use watch interval from CLI if provided, otherwise from config
    let actual_watch_interval = if watch_interval != 5 {
        // CLI flag was explicitly set (not default)
        watch_interval
    } else {
        config.server.watch_interval
    };

    match server_mode {
        "https" => {
            run_https_server(&config, watch, bind_address).await;
        }
        "http" => {
            run_http_server(config, watch, bind_address).await;
        }
        _ => {
            run_stdio_server(
                config,
                settings,
                facade,
                index_path,
                watch,
                actual_watch_interval,
            )
            .await;
        }
    }
}

async fn run_https_server(config: &Settings, watch: bool, bind_address: String) {
    // HTTPS mode - secure server with TLS
    tracing::info!(target: "mcp", "starting HTTPS server on {bind_address}");
    if watch || config.file_watch.enabled {
        tracing::debug!(
            target: "mcp",
            "file watching enabled with {}ms debounce",
            config.file_watch.debounce_ms
        );
    }

    // Use the HTTPS server implementation
    #[cfg(feature = "https-server")]
    {
        use crate::mcp::https_server::serve_https;
        if let Err(e) = serve_https(config.clone(), watch, bind_address).await {
            eprintln!("HTTPS server error: {e}");
            std::process::exit(1);
        }
    }

    #[cfg(not(feature = "https-server"))]
    {
        eprintln!("HTTPS server support is not compiled in.");
        eprintln!("Please rebuild with: cargo build --features https-server");
        std::process::exit(1);
    }
}

async fn run_http_server(config: Settings, watch: bool, bind_address: String) {
    // HTTP mode - persistent server with event-driven file watching
    eprintln!("Starting MCP server in HTTP mode");
    eprintln!("Bind address: {bind_address}");
    if watch || config.file_watch.enabled {
        eprintln!(
            "File watching: ENABLED (event-driven with {}ms debounce)",
            config.file_watch.debounce_ms
        );
    }

    // Use the HTTP server implementation
    use crate::mcp::http_server::serve_http;
    if let Err(e) = serve_http(config, watch, bind_address).await {
        eprintln!("HTTP server error: {e}");
        std::process::exit(1);
    }
}

async fn run_stdio_server(
    config: Settings,
    settings: Arc<Settings>,
    facade: IndexFacade,
    index_path: PathBuf,
    watch: bool,
    actual_watch_interval: u64,
) {
    // Acquire the stdio serve lock before doing anything else. Bound at
    // function scope so the guard releases the OS lock on return / unwind.
    // The process::exit arms below must drop it explicitly: exit skips
    // destructors (the OS still releases the descriptor on process exit).
    let serve_lock = match ServeLockGuard::acquire(&index_path) {
        Ok(guard) => guard,
        Err(ServeLockError::AlreadyRunning { pid, lock_path }) => {
            eprintln!(
                "Another codanna serve is already running for this index (PID {pid}, lock at {}).",
                crate::parsing::paths::render_absolute_path(&lock_path).display()
            );
            eprintln!();
            eprintln!("Subagents and other AI tools may have spawned a duplicate. To run multiple");
            eprintln!("clients against one index, use HTTP mode:");
            eprintln!("  codanna serve --http --watch");
            eprintln!("HTTP mode supports concurrent clients without lock conflicts.");
            eprintln!();
            eprintln!(
                "If you are sure no other codanna serve is running, remove {} and retry.",
                crate::parsing::paths::render_absolute_path(&lock_path).display()
            );
            std::process::exit(1);
        }
        Err(ServeLockError::Io(e)) => {
            eprintln!(
                "Failed to acquire serve lock under {}: {e}",
                crate::parsing::paths::render_absolute_path(&index_path).display()
            );
            std::process::exit(1);
        }
    };

    // stdio mode - current implementation
    eprintln!("Starting MCP server on stdio transport");
    if watch {
        eprintln!("Index watching enabled (interval: {actual_watch_interval}s)");
    }
    eprintln!("To test: npx @modelcontextprotocol/inspector cargo run -- serve");

    // Create MCP server using the already-loaded facade
    tracing::debug!(
        target: "mcp",
        "creating server with facade - symbols: {}, semantic: {}",
        facade.symbol_count(),
        facade.has_semantic_search()
    );
    let broadcaster = Arc::new(crate::mcp::notifications::NotificationBroadcaster::new(100));
    let server =
        crate::mcp::CodeIntelligenceServer::new(facade).with_broadcaster(broadcaster.clone());

    // Load document store and attach to server (shared with watcher later)
    let document_store_arc = crate::documents::load_from_settings(&config);
    let server = if let Some(ref store_arc) = document_store_arc {
        tracing::debug!(target: "mcp", "attaching document store to server");
        server.with_document_store_arc(store_arc.clone())
    } else {
        server
    };

    // Stdio-owned background work must end with the transport. Detached
    // watcher tasks can otherwise survive an EOF long enough to keep the
    // notify backend busy and retain serve.lock during event bursts.
    let mut background_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // If watch mode is enabled, start the hot-reload watcher
    if watch {
        use crate::watcher::HotReloadWatcher;
        use std::time::Duration;

        let facade_arc = server.get_facade_arc();
        let watcher = HotReloadWatcher::new(
            facade_arc,
            settings.clone(),
            Duration::from_secs(actual_watch_interval),
        );

        // Spawn watcher in background, owned by the stdio session.
        background_tasks.push(tokio::spawn(async move {
            watcher.watch().await;
        }));

        eprintln!("Hot-reload watcher started");
    }

    // Start unified file watcher if enabled
    if watch || config.file_watch.enabled {
        use crate::watcher::UnifiedWatcher;
        use crate::watcher::handlers::{CodeFileHandler, ConfigFileHandler, DocumentFileHandler};

        let workspace_root = config
            .workspace_root
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let settings_path = workspace_root.join(".codanna/settings.toml");
        let debounce_ms = config.file_watch.debounce_ms;
        let facade_arc = server.get_facade_arc();

        // Build unified watcher with handlers
        let mut builder = UnifiedWatcher::builder()
            .broadcaster(broadcaster.clone())
            .indexer(facade_arc.clone())
            .index_path(index_path.clone())
            .workspace_root(workspace_root.clone())
            .debounce_ms(debounce_ms);

        // Add code file handler
        builder = builder.handler(CodeFileHandler::new(
            facade_arc.clone(),
            workspace_root.clone(),
        ));

        // Add config file handler
        match ConfigFileHandler::new(settings_path.clone()) {
            Ok(config_handler) => {
                builder = builder.handler(config_handler);
            }
            Err(e) => {
                eprintln!("Failed to create config handler: {e}");
            }
        }

        // Add document handler using shared document store
        if let Some(store_arc) = document_store_arc {
            tracing::debug!(target: "mcp", "adding document handler to watcher");
            builder = builder
                .document_store(store_arc.clone())
                .chunking_config(config.documents.defaults.clone())
                .handler(DocumentFileHandler::new(store_arc, workspace_root.clone()));
        }

        // Build and start the unified watcher
        match builder.build() {
            Ok(unified_watcher) => {
                background_tasks.push(tokio::spawn(async move {
                    if let Err(e) = unified_watcher.watch().await {
                        eprintln!("Unified watcher error: {e}");
                    }
                }));
                eprintln!(
                    "Unified watcher started (debounce: {debounce_ms}ms, config: {})",
                    crate::parsing::paths::render_absolute_path(&settings_path).display()
                );
            }
            Err(e) => {
                eprintln!("Failed to start unified watcher: {e}");
            }
        }
    }

    // Start server with stdio transport
    use rmcp::{ServerHandler, ServiceExt};
    let discover_result = serde_json::to_value(rmcp::model::DiscoverResult::from_server_info(
        server.supported_protocol_versions().into_owned(),
        server.get_info(),
    ))
    .expect("DiscoverResult serializes: closed struct of strings and maps");
    let service = match server.serve(probe_tolerant_stdio(discover_result)).await {
        Ok(service) => service,
        Err(e) => {
            eprintln!("Failed to start MCP server: {e}");
            drop(serve_lock);
            std::process::exit(1);
        }
    };

    // Wait for the stdio transport, then synchronously tear down all work
    // owned by that session before releasing serve.lock. `abort` is sufficient
    // for these async loops because the native notify watcher is owned inside
    // the task and drops when the future is cancelled.
    let service_result = service.waiting().await;
    for task in &background_tasks {
        task.abort();
    }
    for task in background_tasks {
        let _ = task.await;
    }

    if let Err(e) = service_result {
        eprintln!("MCP server error: {e}");
        drop(serve_lock);
        std::process::exit(1);
    }
}

/// Serve a degraded stdio MCP session for a gate-refused index.
/// Completes the handshake with zero tools and heal instructions;
/// never touches the index, so no serve lock is taken and no watcher
/// starts. The caller exits with the gate code when this returns.
pub async fn run_stale_stdio(stored: Option<u32>, current: u32) {
    use rmcp::{ServerHandler, ServiceExt};

    let server = crate::mcp::StaleIndexServer::new(stored, current);
    let discover_result = serde_json::to_value(rmcp::model::DiscoverResult::from_server_info(
        server.supported_protocol_versions().into_owned(),
        server.get_info(),
    ))
    .expect("DiscoverResult serializes: closed struct of strings and maps");
    match server.serve(probe_tolerant_stdio(discover_result)).await {
        Ok(service) => {
            if let Err(e) = service.waiting().await {
                eprintln!("Degraded MCP server error: {e}");
            }
        }
        Err(e) => {
            eprintln!("Failed to start degraded MCP server: {e}");
        }
    }
}

/// Stdio transport that answers bare `server/discover` probes.
///
/// The 2026-07-28 back-compat probe arrives with no `_meta`; rmcp
/// deserializes it as a CustomRequest and `serve()` exits with
/// `ExpectedInitializeRequest` before dispatch. Discover requests that
/// carry `_meta` -- and every other message of either protocol
/// generation -- pass through untouched; rmcp serves both natively.
/// Probes are answered with the same `DiscoverResult` the native
/// handler returns, so both discover forms observe one wire shape.
/// After the first forwarded line every byte passes untouched. Writing
/// to stdout here is safe: rmcp produces no output before its first
/// inbound message.
fn probe_tolerant_stdio(
    discover_result: serde_json::Value,
) -> (tokio::io::DuplexStream, tokio::io::Stdout) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const BUFFER_BYTES: usize = 64 * 1024;
    let (mut inbound, transport_side) = tokio::io::duplex(BUFFER_BYTES);

    tokio::spawn(async move {
        let mut reader = BufReader::new(tokio::io::stdin());
        let mut line = String::new();
        let mut handoff = false;

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }

            if !handoff {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                    let bare_probe = msg.get("method").and_then(|m| m.as_str())
                        == Some("server/discover")
                        && msg
                            .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
                            .is_none();
                    if bare_probe {
                        // Notification-form probes carry nothing to answer.
                        if let Some(id) = msg.get("id") {
                            let response = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": discover_result,
                            });
                            let mut stdout = tokio::io::stdout();
                            // Best-effort: a failed write means the client is
                            // gone; the next read observes EOF and the task
                            // ends.
                            let _ = stdout.write_all(format!("{response}\n").as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                        continue;
                    }
                }
                handoff = true;
            }

            if inbound.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    (transport_side, tokio::io::stdout())
}

#[cfg(test)]
mod serve_lock_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn hardening_review_serve_lock_releases_without_unlinking() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("serve.lock");
        let guard = ServeLockGuard::acquire(dir.path()).unwrap();
        assert_eq!(read_lock_pid(&path), Some(std::process::id()));
        assert!(matches!(
            ServeLockGuard::acquire(dir.path()),
            Err(ServeLockError::AlreadyRunning { .. })
        ));
        drop(guard);
        assert!(path.exists(), "never unlink the lock inode");
        let _next = ServeLockGuard::acquire(dir.path()).unwrap();
    }

    #[test]
    fn hardening_review_serve_lock_ignores_stale_pid_contents() {
        let dir = TempDir::new().unwrap();
        for contents in ["", "garbled", "4294967295"] {
            std::fs::write(dir.path().join("serve.lock"), contents).unwrap();
            let _guard = ServeLockGuard::acquire(dir.path()).unwrap();
            assert!(matches!(
                ServeLockGuard::acquire(dir.path()),
                Err(ServeLockError::AlreadyRunning { .. })
            ));
        }
    }

    #[test]
    fn hardening_review_serve_lock_two_contenders_have_one_owner() {
        use std::sync::{Barrier, mpsc};
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("serve.lock"), "stale").unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let barrier = barrier.clone();
                let tx = tx.clone();
                let path = dir.path();
                scope.spawn(move || {
                    barrier.wait();
                    let guard = ServeLockGuard::acquire(path);
                    tx.send(guard.is_ok()).unwrap();
                    // Keep the winning descriptor alive until both contenders
                    // have completed their acquisition attempt.
                    barrier.wait();
                    drop(guard);
                });
            }
            barrier.wait();
            let owners = usize::from(rx.recv().unwrap()) + usize::from(rx.recv().unwrap());
            barrier.wait();
            assert_eq!(owners, 1);
        });
        let _guard = ServeLockGuard::acquire(dir.path()).unwrap();
    }
}
