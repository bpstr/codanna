//! Notification broadcasting for MCP servers
//!
//! This module provides a broadcast channel for file change events
//! that can be shared between file watchers and multiple MCP server instances.

use std::path::PathBuf;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum FileChangeEvent {
    FileReindexed { path: PathBuf },
    FileCreated { path: PathBuf },
    FileDeleted { path: PathBuf },
    IndexReloaded, // Entire index was reloaded from disk
}

/// The resource URI for a change-event path: an emitted relative path
/// on the MCP wire, portable-form per the emission contract. Clients
/// subscribe by URI and rmcp filters by exact membership — the URI
/// must byte-match the subscription on every platform. Non-Normal
/// path shapes fall back to display text.
pub fn resource_uri(path: &std::path::Path) -> String {
    let portable = crate::parsing::paths::portable_join(path).unwrap_or_else(|| {
        crate::parsing::paths::render_absolute_path(path)
            .display()
            .to_string()
    });
    format!("file://{portable}")
}

/// Manages notification broadcasting to multiple MCP server instances
#[derive(Clone)]
pub struct NotificationBroadcaster {
    sender: broadcast::Sender<FileChangeEvent>,
}

impl NotificationBroadcaster {
    /// Create a new broadcaster with specified channel capacity
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Send a file change event to all subscribers
    pub fn send(&self, event: FileChangeEvent) {
        match self.sender.send(event.clone()) {
            Ok(count) => {
                crate::debug_event!("broadcast", "sent", "{event:?} to {count} subscribers");
            }
            Err(_) => {
                // No receivers, this is fine
                crate::debug_event!("broadcast", "dropped", "no subscribers for {event:?}");
            }
        }
    }

    #[cfg(all(test, feature = "https-server"))]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Subscribe to receive notifications
    pub fn subscribe(&self) -> broadcast::Receiver<FileChangeEvent> {
        self.sender.subscribe()
    }
}

/// Shared only by clones belonging to one MCP session. The listener holds the
/// token, never this owner or the server/facade; final session drop cancels it.
#[derive(Default)]
pub(super) struct NotificationSession {
    cancellation: tokio_util::sync::CancellationToken,
}

impl NotificationSession {
    pub(super) fn token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation.clone()
    }
}

impl Drop for NotificationSession {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub(super) async fn notify_change(
    peer: &rmcp::service::Peer<rmcp::RoleServer>,
    event: FileChangeEvent,
) -> Result<(), rmcp::service::ServiceError> {
    #[allow(deprecated)]
    use rmcp::model::{
        LoggingLevel, LoggingMessageNotificationParam, ResourceUpdatedNotificationParam,
    };
    match event {
        FileChangeEvent::FileReindexed { path } => {
            peer.notify_resource_updated(ResourceUpdatedNotificationParam::new(resource_uri(
                &path,
            )))
            .await?;
            #[allow(deprecated)]
            peer.notify_logging_message(
                LoggingMessageNotificationParam::new(
                    LoggingLevel::Info,
                    serde_json::json!({"action": "re-indexed", "file": resource_uri(&path)}),
                )
                .with_logger("codanna"),
            )
            .await?;
        }
        _ => peer.notify_resource_list_changed().await?,
    }
    Ok(())
}

// The same lifecycle loop is exercised with deterministic in-memory transports
// in tests. No facade/store guard is retained while notification I/O is pending.
async fn forward_changes<F, Fut, E>(
    mut receiver: broadcast::Receiver<FileChangeEvent>,
    cancellation: tokio_util::sync::CancellationToken,
    mut send: F,
) where
    F: FnMut(FileChangeEvent) -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
{
    loop {
        let event = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            event = receiver.recv() => match event {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(target: "mcp", count, "notification listener lagged; requesting resync");
                    FileChangeEvent::IndexReloaded
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        };
        let sent = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            result = tokio::time::timeout(std::time::Duration::from_secs(5), send(event)) => result,
        };
        match sent {
            Ok(Ok(())) => crate::debug_event!("mcp-notify", "sent"),
            _ => {
                // Transport errors can contain payloads. Do not log them.
                tracing::warn!(target: "mcp", "notification delivery failed or timed out; closing listener");
                break;
            }
        }
    }
}

pub(super) async fn forward_notifications(
    peer: rmcp::service::Peer<rmcp::RoleServer>,
    receiver: broadcast::Receiver<FileChangeEvent>,
    cancellation: tokio_util::sync::CancellationToken,
) {
    forward_changes(receiver, cancellation, move |event| {
        let peer = peer.clone();
        async move { notify_change(&peer, event).await }
    })
    .await;
}

impl super::CodeIntelligenceServer {
    /// Forward changes for an initialized session. Normal transports install
    /// this listener during initialize, without a sleep or shared global peer.
    pub async fn start_notification_listener(
        &self,
        receiver: broadcast::Receiver<FileChangeEvent>,
    ) {
        let peer = self.peer.lock().await.clone();
        if let Some(peer) = peer {
            forward_notifications(peer, receiver, self.notification_session.token()).await;
        } else {
            tracing::warn!(target: "mcp", "notification listener requires an initialized session");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The notification URI is an emitted relative path on the MCP wire:
    // portable-form on every platform. rmcp filters resource-updated
    // notifications by exact URI membership, so a native-separator URI
    // never matches the client's subscription.
    #[test]
    fn resource_uri_is_portable_form_on_every_platform() {
        let path = std::path::Path::new("src").join("alpha.rs");
        assert_eq!(resource_uri(&path), "file://src/alpha.rs");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn hardening_review_notification_failure_drops_subscription() {
        let broadcaster = NotificationBroadcaster::new(8);
        let receiver = broadcaster.subscribe();
        broadcaster.send(FileChangeEvent::IndexReloaded);
        forward_changes(
            receiver,
            tokio_util::sync::CancellationToken::new(),
            |_| async { Err::<(), _>("closed mock transport") },
        )
        .await;
        assert_eq!(broadcaster.sender.receiver_count(), 0);
    }

    #[tokio::test]
    async fn hardening_review_notification_session_drop_cancels_idle_listener() {
        let broadcaster = NotificationBroadcaster::new(8);
        let session = Arc::new(NotificationSession::default());
        let other_owner = session.clone();
        let token = session.token();
        drop(session);
        assert!(!token.is_cancelled());
        let receiver = broadcaster.subscribe();
        let task = tokio::spawn(forward_changes(receiver, token.clone(), |_| async {
            Ok::<(), ()>(())
        }));
        drop(other_owner);
        assert!(token.is_cancelled());
        task.await.unwrap();
        assert_eq!(broadcaster.sender.receiver_count(), 0);
    }

    #[tokio::test]
    async fn hardening_review_notification_sessions_are_independent() {
        let broadcaster = NotificationBroadcaster::new(8);
        let first = NotificationSession::default();
        let second = NotificationSession::default();
        let first_receiver = broadcaster.subscribe();
        let second_receiver = broadcaster.subscribe();
        let deliveries = Arc::new(AtomicUsize::new(0));
        let first_token = first.token();
        drop(first);
        broadcaster.send(FileChangeEvent::IndexReloaded);
        let count = deliveries.clone();
        let token = second.token();
        forward_changes(first_receiver, first_token, |_| async {
            panic!("closed session received an event");
            #[allow(unreachable_code)]
            Ok::<(), ()>(())
        })
        .await;
        forward_changes(second_receiver, token.clone(), move |_| {
            let count = count.clone();
            let token = token.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                token.cancel();
                Ok::<(), ()>(())
            }
        })
        .await;
        assert_eq!(deliveries.load(Ordering::SeqCst), 1);
        assert_eq!(broadcaster.sender.receiver_count(), 0);
    }
}
