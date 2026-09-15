//! Local HTTP sessions with short map guards and cancellation-owned streams.
//!
//! rmcp owns message routing and replay. This adapter never holds the session map
//! across a handle RPC and never queues DELETE behind a slow client's traffic.
use parking_lot::Mutex;
use rmcp::RoleServer;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    ServerSseMessage, SessionId, SessionManager,
    local::{
        LocalSessionHandle, LocalSessionManagerError, LocalSessionWorker, SessionConfig,
        SessionError, create_local_session,
    },
};
use rmcp::transport::worker::Worker;
use rmcp::transport::{Transport, WorkerTransport};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll, Waker};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;

type Error = LocalSessionManagerError;
const MAX_STREAMS: usize = 64;
const MAX_SESSIONS: usize = 256;
type SharedStream = Arc<Mutex<StreamState>>;

struct StreamState {
    receiver: Option<mpsc::Receiver<ServerSseMessage>>,
    waker: Option<Waker>,
    permit: Option<OwnedSemaphorePermit>,
}

/// Closing an entry drops its receivers even if the HTTP client never polls
/// again. Merely cancelling a token in a stream's poll method is insufficient:
/// a full channel can otherwise keep the session worker stuck in send().await.
pub(crate) struct SessionStream(SharedStream);
impl Stream for SessionStream {
    type Item = ServerSseMessage;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self.0.lock();
        let Some(receiver) = state.receiver.as_mut() else {
            return Poll::Ready(None);
        };
        let result = receiver.poll_recv(cx);
        if result.is_pending() {
            if state
                .waker
                .as_ref()
                .is_none_or(|waker| !waker.will_wake(cx.waker()))
            {
                state.waker = Some(cx.waker().clone());
            }
        } else {
            state.waker = None;
            if matches!(result, Poll::Ready(None)) {
                state.receiver.take();
                state.permit.take();
            }
        }
        result
    }
}

struct Entry {
    handle: LocalSessionHandle,
    cancel: CancellationToken,
    streams: Mutex<Vec<Weak<Mutex<StreamState>>>>,
    capacity: Arc<Semaphore>,
}
impl Entry {
    async fn run<T>(
        &self,
        operation: impl Future<Output = Result<T, SessionError>>,
    ) -> Result<T, Error> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(SessionError::SessionServiceTerminated.into()),
            result = operation => result.map_err(Into::into),
        }
    }
    fn reserve(&self) -> Result<OwnedSemaphorePermit, Error> {
        self.capacity.clone().try_acquire_owned().map_err(|_| {
            SessionError::Io(std::io::Error::other("session stream limit reached")).into()
        })
    }
    fn stream(
        &self,
        receiver: mpsc::Receiver<ServerSseMessage>,
        permit: OwnedSemaphorePermit,
    ) -> Result<SessionStream, Error> {
        let mut streams = self.streams.lock();
        streams.retain(|stream| stream.strong_count() > 0);
        if self.cancel.is_cancelled() {
            return Err(SessionError::SessionServiceTerminated.into());
        }
        let shared = Arc::new(Mutex::new(StreamState {
            receiver: Some(receiver),
            waker: None,
            permit: Some(permit),
        }));
        streams.push(Arc::downgrade(&shared));
        Ok(SessionStream(shared))
    }
    fn close(&self) {
        self.cancel.cancel();
        let streams = std::mem::take(&mut *self.streams.lock());
        for stream in streams.into_iter().filter_map(|stream| stream.upgrade()) {
            let (receiver, permit, waker) = {
                let mut state = stream.lock();
                (
                    state.receiver.take(),
                    state.permit.take(),
                    state.waker.take(),
                )
            };
            drop(receiver);
            drop(permit);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.close();
    }
}

/// Interrupt the handler-side wait as well as the HTTP-side wait. The pinned SDK
/// does not select its cancellation token while waiting for an initialize reply.
/// Dropping its transport closes both handler channels, releasing that wait.
pub(crate) struct SessionTransport {
    inner: Option<WorkerTransport<LocalSessionWorker>>,
    entry: Arc<Entry>,
}
impl Transport<RoleServer> for SessionTransport {
    type Error = <WorkerTransport<LocalSessionWorker> as Transport<RoleServer>>::Error;
    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let send = self.inner.as_mut().map(|inner| inner.send(message));
        let cancel = self.entry.cancel.clone();
        async move {
            let Some(send) = send else {
                return Err(LocalSessionWorker::err_closed());
            };
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(LocalSessionWorker::err_closed()),
                result = send => result,
            }
        }
    }
    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        let inner = self.inner.as_mut()?;
        tokio::select! {
            biased;
            _ = self.entry.cancel.cancelled() => None,
            message = inner.receive() => message,
        }
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.entry.close();
        drop(self.inner.take());
        Ok(())
    }
}
impl Drop for SessionTransport {
    fn drop(&mut self) {
        self.entry.close();
    }
}

#[derive(Default)]
pub(crate) struct LocalSessions {
    sessions: Mutex<HashMap<SessionId, Arc<Entry>>>,
    config: SessionConfig,
}
impl LocalSessions {
    fn entry(&self, id: &SessionId) -> Result<Arc<Entry>, Error> {
        self.sessions
            .lock()
            .get(id)
            .filter(|entry| !entry.cancel.is_cancelled())
            .cloned()
            .ok_or_else(|| Error::SessionNotFound(id.clone()))
    }
}
impl Drop for LocalSessions {
    fn drop(&mut self) {
        for (_, entry) in self.sessions.get_mut().drain() {
            entry.close();
        }
    }
}
impl SessionManager for LocalSessions {
    type Error = Error;
    type Transport = SessionTransport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Error> {
        let mut sessions = self.sessions.lock();
        sessions.retain(|_, entry| !entry.cancel.is_cancelled());
        if sessions.len() >= MAX_SESSIONS {
            return Err(SessionError::Io(std::io::Error::other("session limit reached")).into());
        }
        let id: SessionId = loop {
            let id = hex::encode(rand::random::<[u8; 32]>()).into();
            if !sessions.contains_key(&id) {
                break id;
            }
        };
        let (handle, worker) = create_local_session(id.clone(), self.config.clone());
        let cancel = CancellationToken::new();
        let transport = WorkerTransport::spawn_with_ct(worker, cancel.clone());
        let entry = Arc::new(Entry {
            handle,
            cancel,
            streams: Mutex::new(Vec::new()),
            capacity: Arc::new(Semaphore::new(MAX_STREAMS)),
        });
        sessions.insert(id.clone(), entry.clone());
        Ok((
            id,
            SessionTransport {
                inner: Some(transport),
                entry,
            },
        ))
    }
    async fn has_session(&self, id: &SessionId) -> Result<bool, Error> {
        Ok(self.entry(id).is_ok())
    }
    async fn close_session(&self, id: &SessionId) -> Result<(), Error> {
        let entry = { self.sessions.lock().remove(id) };
        if let Some(entry) = entry {
            entry.close();
        }
        Ok(())
    }
    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Error> {
        let entry = self.entry(id)?;
        entry.run(entry.handle.initialize(message)).await
    }
    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Error> {
        let entry = self.entry(id)?;
        entry.run(entry.handle.push_message(message, None)).await
    }
    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Error> {
        let entry = self.entry(id)?;
        let permit = entry.reserve()?;
        let receiver = entry
            .run(entry.handle.establish_request_wise_channel())
            .await?;
        let request_id = receiver.http_request_id;
        // Register ownership before sending. DELETE can now close the receiver
        // while this request waits for a saturated session event queue.
        let stream = entry.stream(receiver.inner, permit)?;
        entry
            .run(entry.handle.push_message(message, request_id))
            .await?;
        Ok(stream)
    }
    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Error> {
        let entry = self.entry(id)?;
        let permit = entry.reserve()?;
        let receiver = entry.run(entry.handle.establish_common_channel()).await?;
        entry.stream(receiver.inner, permit)
    }
    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Error> {
        let entry = self.entry(id)?;
        let permit = entry.reserve()?;
        let receiver = entry
            .run(entry.handle.resume(last_event_id.parse()?))
            .await?;
        entry.stream(receiver.inner, permit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::Transport;
    use std::time::Duration;

    #[tokio::test]
    async fn hardening_workspace_session_close_does_not_wait_for_unfinished_initialization() {
        let manager = LocalSessions::default();
        let (a, mut transport_a) = manager.create_session().await.unwrap();
        let message: ClientJsonRpcMessage = serde_json::from_value(serde_json::json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                "protocolVersion":"2025-11-25", "capabilities":{},
                "clientInfo":{"name":"blocked-fixture","version":"1"}
            }
        }))
        .unwrap();
        let mut pending = Box::pin(manager.initialize_session(&a, message));
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let (b, mut transport_b) =
            tokio::time::timeout(Duration::from_secs(1), manager.create_session())
                .await
                .unwrap()
                .unwrap();
        tokio::time::timeout(Duration::from_secs(1), manager.close_session(&b))
            .await
            .unwrap()
            .unwrap();
        assert!(manager.has_session(&a).await.unwrap());
        manager.close_session(&a).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), pending)
                .await
                .unwrap()
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(1), transport_a.close())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), transport_b.close())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn hardening_workspace_session_delete_releases_unpolled_full_stream() {
        let manager = LocalSessions::default();
        let (a, mut transport) = manager.create_session().await.unwrap();
        let entry = manager.entry(&a).unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let mut stream = entry.stream(receiver, entry.reserve().unwrap()).unwrap();
        sender
            .send(ServerSseMessage::retry(Duration::from_secs(1)))
            .await
            .unwrap();
        let mut blocked = Box::pin(sender.send(ServerSseMessage::retry(Duration::from_secs(1))));
        std::future::poll_fn(|cx| {
            assert!(blocked.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        tokio::time::timeout(Duration::from_secs(1), manager.close_session(&a))
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), blocked)
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                .await
                .is_none()
        );
        assert!(!manager.has_session(&a).await.unwrap());
        tokio::time::timeout(Duration::from_secs(1), transport.close())
            .await
            .unwrap()
            .unwrap();
    }
}
