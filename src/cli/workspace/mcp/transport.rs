//! Observe scope changes in wire order, not in asynchronously scheduled handlers.
//!
//! RMCP spawns request and notification handlers independently. Invalidation in
//! `on_roots_list_changed` can therefore run after the next tool request reads
//! cached roots. Intercept the already-decoded message before it is dispatched;
//! keep the SDK's framing, cancellation safety, and all other protocol behavior.
use super::scope::ScopeResolver;
use rmcp::model::{ClientNotification, JsonRpcMessage};
use rmcp::service::{RoleServer, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use std::sync::Arc;

pub(super) struct ScopeTransport<T> {
    inner: T,
    scope: Arc<ScopeResolver>,
}
impl<T> ScopeTransport<T> {
    pub(super) fn new(inner: T, scope: Arc<ScopeResolver>) -> Self {
        Self { inner, scope }
    }
}
impl<T: Transport<RoleServer>> Transport<RoleServer> for ScopeTransport<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }

    #[allow(deprecated)] // Roots remains supported by the negotiated MCP versions.
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        let message = self.inner.receive().await;
        if let Some(JsonRpcMessage::Notification(notification)) = &message
            && matches!(
                &notification.notification,
                ClientNotification::RootsListChangedNotification(_)
            )
        {
            // No await after consumption: cancellation cannot lose this message
            // or expose a later request before this revision has been published.
            self.scope.invalidate_roots();
        }
        message
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::workspace::mcp::budget::Budget;
    use crate::init::workspaces::WorkspaceRegistry;
    use serde_json::json;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    /// Deliberately run no notification handler. Two messages in one write must
    /// still invalidate routing before the following request can be dispatched.
    #[tokio::test]
    async fn hardening_workspace_transport_invalidates_roots_before_dispatch() {
        let temporary = tempfile::tempdir().unwrap();
        let scope = Arc::new(ScopeResolver::new(
            WorkspaceRegistry::new(temporary.path().join("registry.json")),
            temporary.path().to_path_buf(),
            None,
            Budget::new(),
        ));
        let (mut client, server) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(server);
        let mut transport = ScopeTransport::new(
            rmcp::transport::async_rw::AsyncRwTransport::new_server(read, write),
            scope.clone(),
        );
        let notification = json!({
            "jsonrpc": "2.0", "method": "notifications/roots/list_changed"
        });
        let request = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_workspace", "arguments": {}}
        });
        client
            .write_all(format!("{notification}\n{request}\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(scope.generation(), 0);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), transport.receive())
                .await
                .expect("transport receive deadline"),
            Some(JsonRpcMessage::Notification(_))
        ));
        assert_eq!(scope.generation(), 1);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), transport.receive())
                .await
                .expect("transport receive deadline"),
            Some(JsonRpcMessage::Request(_))
        ));
        assert_eq!(
            scope.generation(),
            1,
            "a request must not invalidate the cache"
        );

        // Cancellation and unrelated notifications must retain their SDK path.
        client
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":1}}\n")
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), transport.receive())
                .await
                .expect("transport receive deadline"),
            Some(JsonRpcMessage::Notification(_))
        ));
        assert_eq!(scope.generation(), 1);
        assert!(!temporary.path().join(".codanna").exists());
        transport.close().await.unwrap();
    }
}
