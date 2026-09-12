//! HTTP server implementation for MCP
//!
//! Provides a persistent HTTP server with streamable HTTP transport
//! for multiple concurrent clients and real-time updates.

#[cfg(feature = "http-server")]
pub async fn serve_http(config: crate::Settings, watch: bool, bind: String) -> anyhow::Result<()> {
    use crate::IndexPersistence;
    use crate::indexing::facade::IndexFacade;
    use crate::mcp::{CodeIntelligenceServer, notifications::NotificationBroadcaster};
    use crate::watcher::HotReloadWatcher;
    use axum::Router;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tokio_util::sync::CancellationToken;

    let auth = crate::mcp::auth::NetworkAuth::from_env()?;
    let validated_bind = crate::mcp::auth::validate_bind(&bind, false)?;

    // Initialize logging with config
    crate::logging::init_with_config(&config.logging);

    crate::log_event!("http", "starting", "MCP server on {bind}");

    // Create notification broadcaster for file change events
    let broadcaster = Arc::new(NotificationBroadcaster::new(100));

    // Create shared facade
    let settings = Arc::new(config.clone());
    let persistence = IndexPersistence::new(config.index_path.clone());

    let facade = if persistence.exists() {
        persistence.load_facade(settings.clone())?
    } else {
        crate::log_event!("http", "starting", "no existing index");
        IndexFacade::new(settings.clone())?
    };
    let indexer = Arc::new(RwLock::new(facade));

    // Create cancellation token for coordinated shutdown
    let ct = CancellationToken::new();

    // Start index watcher if watch mode is enabled
    if watch {
        let index_watcher_indexer = indexer.clone();
        let index_watcher_settings = Arc::new(config.clone());
        let index_watcher_broadcaster = broadcaster.clone();
        let index_watcher_ct = ct.clone();

        // Default to 5 second interval
        let watch_interval = 5u64;

        let hot_reload_watcher = HotReloadWatcher::new(
            index_watcher_indexer,
            index_watcher_settings,
            Duration::from_secs(watch_interval),
        )
        .with_broadcaster(index_watcher_broadcaster);

        tokio::spawn(async move {
            tokio::select! {
                _ = hot_reload_watcher.watch() => {
                    crate::log_event!("hot-reload", "ended");
                }
                _ = index_watcher_ct.cancelled() => {
                    crate::log_event!("hot-reload", "stopped");
                }
            }
        });

        crate::log_event!("hot-reload", "started", "polling every {watch_interval}s");
    }

    // Load document store once (shared between MCP server instances and watcher)
    let document_store_arc = crate::documents::load_from_settings(&config);
    if document_store_arc.is_some() {
        tracing::debug!(target: "mcp", "document store loaded for MCP server");
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

        // Build unified watcher with handlers
        let mut builder = UnifiedWatcher::builder()
            .broadcaster(broadcaster.clone())
            .indexer(indexer.clone())
            .index_path(config.index_path.clone())
            .workspace_root(workspace_root.clone())
            .debounce_ms(debounce_ms);

        // Add code file handler
        builder = builder.handler(CodeFileHandler::new(
            indexer.clone(),
            workspace_root.clone(),
        ));

        // Add config file handler
        match ConfigFileHandler::new(settings_path.clone()) {
            Ok(config_handler) => {
                builder = builder.handler(config_handler);
            }
            Err(e) => {
                tracing::warn!("[config] failed to create handler: {e}");
            }
        }

        // Add document handler using shared document store
        if let Some(ref store_arc) = document_store_arc {
            tracing::debug!(target: "mcp", "adding document handler to watcher");
            builder = builder
                .document_store(store_arc.clone())
                .chunking_config(config.documents.defaults.clone())
                .handler(DocumentFileHandler::new(
                    store_arc.clone(),
                    workspace_root.clone(),
                ));
        }

        // Build and start the unified watcher
        match builder.build() {
            Ok(unified_watcher) => {
                let watcher_ct = ct.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        result = unified_watcher.watch() => {
                            if let Err(e) = result {
                                tracing::error!("[watcher] error: {e}");
                            }
                        }
                        _ = watcher_ct.cancelled() => {
                            crate::log_event!("watcher", "stopped");
                        }
                    }
                });
                crate::log_event!(
                    "watcher",
                    "started",
                    "debounce: {debounce_ms}ms, config: {}",
                    crate::parsing::paths::render_absolute_path(&settings_path).display()
                );
            }
            Err(e) => {
                tracing::warn!("[watcher] failed to start: {e}");
                tracing::warn!("[watcher] continuing without file watching");
            }
        }
    }

    // Create streamable HTTP service for MCP connections
    let indexer_for_service = indexer.clone();
    let config_for_service = Arc::new(config.clone());
    let document_store_for_service = document_store_arc.clone();

    let mcp_service = StreamableHttpService::new(
        move || {
            crate::debug_event!("mcp", "creating server instance");
            let server = CodeIntelligenceServer::new_with_facade(
                indexer_for_service.clone(),
                config_for_service.clone(),
            )
            .with_broadcaster(broadcaster.clone());

            // Attach document store if available
            let server = if let Some(ref store_arc) = document_store_for_service {
                server.with_document_store_arc(store_arc.clone())
            } else {
                server
            };

            Ok(server)
        },
        LocalSessionManager::default().into(),
        {
            let cfg = StreamableHttpServerConfig::default()
                .with_cancellation_token(ct.child_token())
                .with_sse_keep_alive(Some(Duration::from_secs(15)))
                .with_sse_retry(None)
                .with_legacy_session_mode(true)
                .with_json_response(false);
            let cfg = match config.mcp.allowed_hosts.clone() {
                Some(hosts) => cfg.with_allowed_hosts(hosts),
                None => cfg,
            };
            match config.mcp.allowed_origins.clone() {
                Some(origins) => cfg.with_allowed_origins(origins),
                None => cfg,
            }
        },
    );

    // Helper function for shutdown signal with cancellation token
    async fn shutdown_signal() {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl+c");
        eprintln!("Received shutdown signal");
    }

    let router =
        crate::mcp::auth::network_router(Router::new().nest_service("/mcp", mcp_service), auth);

    // Bind and serve
    let listener = tokio::net::TcpListener::bind(validated_bind).await?;
    // Report the actual socket, including an OS-assigned port for :0. Consumers
    // must not reserve a port and release it before this server binds.
    let bound = listener.local_addr()?;
    eprintln!("HTTP MCP server listening on http://{bound}");
    eprintln!("MCP endpoint: http://{bound}/mcp");
    eprintln!("Health check: http://{bound}/health");
    eprintln!("Press Ctrl+C to stop the server");

    // Create server future
    let server = axum::serve(listener, router);

    // Handle graceful shutdown with tokio::select!
    tokio::select! {
        result = server => {
            result?;
        }
        _ = shutdown_signal() => {
            eprintln!("Shutting down HTTP server...");
            ct.cancel();
        }
    }

    eprintln!("HTTP server shut down gracefully");
    Ok(())
}

#[cfg(not(feature = "http-server"))]
pub async fn serve_http(
    _config: crate::Settings,
    _watch: bool,
    _bind: String,
) -> anyhow::Result<()> {
    eprintln!("HTTP server support is not compiled in.");
    eprintln!("Please rebuild with: cargo build --features http-server");
    std::process::exit(1);
}
