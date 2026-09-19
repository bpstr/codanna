//! HTTPS server implementation for MCP using streamable HTTP transport with TLS
//!
//! Provides a secure HTTPS server with TLS support for MCP communication.
//! Uses streamable HTTP transport which is compatible with Claude Code.

#[cfg(feature = "https-server")]
pub async fn serve_https(config: crate::Settings, watch: bool, bind: String) -> anyhow::Result<()> {
    use crate::IndexPersistence;
    use crate::indexing::facade::IndexFacade;
    use crate::mcp::notifications::NotificationBroadcaster;
    use crate::watcher::HotReloadWatcher;
    use anyhow::Context;
    use axum_server::tls_rustls::RustlsConfig;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tokio_util::sync::CancellationToken;

    let auth = crate::mcp::auth::NetworkAuth::from_env(&config)?;
    let validated_bind = crate::mcp::auth::validate_bind(&bind, true)?;

    let _code_write_lease = if watch || config.file_watch.enabled {
        Some(crate::storage::write_lease::CodeWriteLease::acquire(
            &config.index_path,
        )?)
    } else {
        None
    };

    // Initialize logging with config
    crate::logging::init_with_config(&config.logging);

    crate::log_event!("https", "starting", "MCP server on {bind}");

    // Create notification broadcaster for file change events
    let broadcaster = Arc::new(NotificationBroadcaster::new(100));

    // Create shared facade
    let settings = Arc::new(config.clone());
    let persistence = IndexPersistence::new(config.index_path.clone());

    let mut facade = if persistence.exists() {
        persistence.load_facade(settings.clone())?
    } else {
        crate::log_event!("https", "starting", "no existing index");
        IndexFacade::new(settings.clone())?
    };
    auth.restrict_facade(&mut facade)?;
    let indexer = Arc::new(RwLock::new(facade));

    // Create cancellation token for graceful shutdown
    let ct = CancellationToken::new();
    let _cancel_on_drop = ct.clone().drop_guard();

    // Load document store once (shared between MCP server and watcher)
    let document_store_arc = crate::documents::load_from_settings(&config);
    if let Some(store) = &document_store_arc {
        auth.validate_documents(&*store.read().await)?;
    }
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
            Ok(mut unified_watcher) => {
                if let Err(error) = unified_watcher.prepare().await {
                    ct.cancel();
                    return Err(error.into());
                }
                let watcher_ct = ct.clone();
                let watcher_lease = _code_write_lease.clone();
                tokio::spawn(async move {
                    // Retain ownership through the actual mutation, including
                    // cancellation/error paths in the enclosing server future.
                    let _lease = watcher_lease;
                    if let Err(error) = unified_watcher.watch_until(watcher_ct).await {
                        tracing::error!("[watcher] error: {error}");
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
                ct.cancel();
                return Err(e.into());
            }
        }
    }

    // Start index watcher if watch mode is enabled
    if watch {
        let hot_reload_indexer = indexer.clone();
        let hot_reload_settings = Arc::new(config.clone());
        let hot_reload_broadcaster = broadcaster.clone();
        let hot_reload_ct = ct.clone();

        let watch_interval = config.server.watch_interval;

        let hot_reload_watcher = HotReloadWatcher::new(
            hot_reload_indexer,
            hot_reload_settings,
            Duration::from_secs(watch_interval),
        )
        .with_broadcaster(hot_reload_broadcaster);

        tokio::spawn(async move {
            tokio::select! {
                _ = hot_reload_watcher.watch() => {
                    crate::log_event!("hot-reload", "ended");
                }
                _ = hot_reload_ct.cancelled() => {
                    crate::log_event!("hot-reload", "stopped");
                }
            }
        });

        crate::log_event!("hot-reload", "started", "polling every {watch_interval}s");
    }

    let router = crate::mcp::network::NetworkService::new(
        indexer,
        Arc::new(config.clone()),
        document_store_arc,
        broadcaster,
        ct.clone(),
        auth,
    )
    .router;

    // Get or create TLS certificates
    let (cert_pem, key_pem) = get_or_create_certificate(&bind)
        .await
        .context("Failed to get or create TLS certificate")?;

    // Configure TLS
    let tls_config = RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .context("Failed to configure TLS")?;

    // Bind once and report the actual port, including an OS-assigned :0 socket.
    let listener = std::net::TcpListener::bind(validated_bind)?;
    let bound = listener.local_addr()?;
    eprintln!("HTTPS MCP server listening on https://{bound}");
    eprintln!("MCP endpoint: https://{bound}/mcp");
    eprintln!("Health check: https://{bound}/health");
    eprintln!(
        "Using a self-signed certificate. Configure your client to trust its CA/certificate."
    );
    eprintln!("Press Ctrl+C to stop the server");
    let server =
        axum_server::from_tcp_rustls(listener, tls_config)?.serve(router.into_make_service());

    // Handle graceful shutdown
    tokio::select! {
        result = server => {
            result?;
        }
        _ = shutdown_signal() => {
            eprintln!("Shutting down HTTPS server...");
            ct.cancel();
        }
    }

    eprintln!("HTTPS server shut down gracefully");
    Ok(())
}

/// Helper function for shutdown signal
#[cfg(feature = "https-server")]
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    eprintln!("Received shutdown signal");
}

/// Get or create self-signed certificate for HTTPS
#[cfg(feature = "https-server")]
async fn get_or_create_certificate(bind: &str) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    use anyhow::Context;
    use rcgen::generate_simple_self_signed;

    // Determine certificate storage directory
    let cert_dir = dirs::config_dir()
        .context("Failed to get config directory")?
        .join("codanna")
        .join("certs");

    let cert_path = cert_dir.join("server.pem");
    let key_path = cert_dir.join("server.key");

    // Create directory if it doesn't exist
    tokio::fs::create_dir_all(&cert_dir)
        .await
        .context("Failed to create certificate directory")?;

    // Check if server certificate already exists
    if cert_path.exists() && key_path.exists() {
        eprintln!("Loading existing certificates from {cert_dir:?}");
        let cert = tokio::fs::read(&cert_path)
            .await
            .context("Failed to read certificate file")?;
        let key = tokio::fs::read(&key_path)
            .await
            .context("Failed to read key file")?;
        return Ok((cert, key));
    }

    eprintln!("Generating new enhanced self-signed certificate...");

    // Build list of Subject Alternative Names
    let mut subject_alt_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];

    // If binding to 0.0.0.0, include local network IP
    if bind.starts_with("0.0.0.0") {
        if let Ok(local_ip) = local_ip_address::local_ip() {
            eprintln!("Including local network IP in certificate: {local_ip}");
            subject_alt_names.push(local_ip.to_string());
        }
    }

    // Generate certificate using the simpler API but with better parameters
    let cert = generate_simple_self_signed(subject_alt_names.clone())
        .context("Failed to generate self-signed certificate")?;

    let cert_pem = cert.cert.pem().into_bytes();
    let key_pem = cert.signing_key.serialize_pem().into_bytes();

    // Save certificate and key
    tokio::fs::write(&cert_path, &cert_pem)
        .await
        .context("Failed to write server certificate")?;
    tokio::fs::write(&key_path, &key_pem)
        .await
        .context("Failed to write server key")?;

    // Calculate fingerprint
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    cert.cert.der().hash(&mut hasher);
    let fingerprint = hasher.finish();
    let fingerprint_hex = format!("{fingerprint:016X}");

    eprintln!();
    eprintln!("🔐 Certificate Details:");
    eprintln!("   - Type: Self-Signed TLS Certificate");
    eprintln!(
        "   - Location: {}",
        crate::parsing::paths::render_absolute_path(&cert_path).display()
    );
    eprintln!("   - Fingerprint: {fingerprint_hex}");
    eprintln!("   - Valid for: {}", subject_alt_names.join(", "));
    eprintln!();
    eprintln!("🔧 To trust this certificate on macOS:");
    eprintln!();
    eprintln!("   Option 1: Command line (requires sudo):");
    eprintln!(
        "   sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain {}",
        crate::parsing::paths::render_absolute_path(&cert_path).display()
    );
    eprintln!();
    eprintln!("   Option 2: GUI (recommended):");
    eprintln!(
        "   1. Open Finder and navigate to: {}",
        crate::parsing::paths::render_absolute_path(&cert_dir).display()
    );
    eprintln!("   2. Double-click 'server.pem'");
    eprintln!("   3. Add to 'System' keychain");
    eprintln!("   4. Set to 'Always Trust' for SSL");
    eprintln!();
    eprintln!("   Option 3: Open in browser first:");
    eprintln!("   1. Visit https://127.0.0.1:8443/health in Safari/Chrome");
    eprintln!("   2. Click 'Advanced' and proceed anyway");
    eprintln!("   3. This may help some clients accept the certificate");
    eprintln!();
    eprintln!("⚠️  After trusting the certificate, restart Claude Code to reconnect");
    eprintln!();

    Ok((cert_pem, key_pem))
}

/// Helper function to detect local IP address
#[cfg(feature = "https-server")]
mod local_ip_address {
    use std::net::{IpAddr, UdpSocket};

    pub fn local_ip() -> Result<IpAddr, Box<dyn std::error::Error>> {
        // Connect to a dummy address to determine local IP
        // This doesn't actually send any packets, just determines
        // which network interface would be used for external traffic
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect("8.8.8.8:80")?;
        let addr = socket.local_addr()?;
        Ok(addr.ip())
    }
}

#[cfg(not(feature = "https-server"))]
pub async fn serve_https(
    _config: crate::Settings,
    _watch: bool,
    _bind: String,
) -> anyhow::Result<()> {
    eprintln!("HTTPS server support is not compiled in.");
    eprintln!("Please rebuild with: cargo build --features https-server");
    std::process::exit(1);
}
