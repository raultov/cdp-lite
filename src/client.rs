use crate::error::{CdpError, CdpResult};
use crate::event_filter::EventFilter;
use crate::protocol::{WsCommand, WsResponse};
use crate::rest_client::get_websocket_url;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, error, trace};

type CdpWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Commands awaiting the response that carries their `id`.
///
/// One map per connection, shared with the reader task: a browser-level
/// connection routes the responses of every attached tab through it, which is
/// why command ids are allocated globally rather than per session.
type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<WsResponse>>>>;

#[derive(Clone)]
pub struct CdpClient {
    command_tx: mpsc::UnboundedSender<String>,
    pending_requests: PendingRequests,
    event_tx: broadcast::Sender<WsResponse>,
    next_id: Arc<AtomicU64>,
    default_timeout: Duration,
    is_alive: Arc<AtomicBool>,
}

impl CdpClient {
    /// Connects to the first suitable **page** target advertised by
    /// `GET /json/list` on `host`.
    ///
    /// The returned client is bound to that single tab for its whole lifetime
    /// and never puts a `sessionId` on the wire. To drive several tabs over a
    /// single connection, use [`crate::browser::BrowserClient`] instead.
    pub async fn new(host: &str, default_timeout: Duration) -> CdpResult<Self> {
        let ws_url = get_websocket_url(host).await?;
        Self::connect(&ws_url, default_timeout).await
    }

    /// Connects to an already known WebSocket debugger endpoint.
    ///
    /// Useful when the endpoint was resolved elsewhere, and the building block
    /// [`crate::browser::BrowserClient`] uses to open the browser-level
    /// connection.
    pub async fn connect(ws_url: &str, default_timeout: Duration) -> CdpResult<Self> {
        let (ws_stream, _) = connect_async(ws_url).await?;
        let (ws_sink, ws_source) = ws_stream.split();

        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (command_tx, command_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, _) = broadcast::channel(128);
        let is_alive = Arc::new(AtomicBool::new(true));

        tokio::spawn(write_commands(ws_sink, command_rx));
        tokio::spawn(read_messages(
            ws_source,
            Arc::clone(&pending_requests),
            event_tx.clone(),
            Arc::clone(&is_alive),
        ));

        let next_id = Arc::new(AtomicU64::new(1));
        Ok(Self {
            command_tx,
            pending_requests,
            event_tx,
            next_id,
            default_timeout,
            is_alive,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsResponse> {
        self.event_tx.subscribe()
    }

    pub fn on_domain(&self, domain: &'static str) -> EventFilter {
        EventFilter::new(self.subscribe(), domain)
    }

    /// Same as [`CdpClient::on_domain`], but restricted to the events produced
    /// by one attached tab.
    ///
    /// On a browser-level connection every attached tab shares the same event
    /// stream, so filtering by domain alone would mix tabs together.
    pub fn on_domain_for_session(&self, domain: &'static str, session_id: &str) -> EventFilter {
        EventFilter::for_session(self.subscribe(), domain, session_id)
    }

    pub async fn send_raw_command<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> CdpResult<WsResponse> {
        self.dispatch(method, params, None).await
    }

    /// Sends a command scoped to a single attached tab.
    ///
    /// `session_id` comes from [`crate::tab::Tab::session_id`]; prefer
    /// [`crate::tab::Tab::send_raw_command`], which fills it in for you.
    pub async fn send_raw_command_to_session<P: Serialize>(
        &self,
        session_id: &str,
        method: &str,
        params: P,
    ) -> CdpResult<WsResponse> {
        self.dispatch(method, params, Some(session_id.to_string()))
            .await
    }

    async fn dispatch<P: Serialize>(
        &self,
        method: &str,
        params: P,
        session_id: Option<String>,
    ) -> CdpResult<WsResponse> {
        if !self.is_alive.load(Ordering::SeqCst) {
            return Err(CdpError::Disconnected);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut map = self.pending_requests.lock().map_err(|_| {
                CdpError::InternalError(
                    "Mutex poisoned: another thread panicked while holding the lock".into(),
                )
            })?;
            map.insert(id, tx);
        }

        let cmd = WsCommand {
            id,
            method: method.to_string(),
            params: Some(params),
            session_id,
        };

        let json_payload = serde_json::to_string(&cmd)?;
        self.command_tx.send(json_payload).map_err(|e| {
            self.is_alive.store(false, Ordering::SeqCst);
            CdpError::InternalError(format!("Failed to send command to writer task: {}", e))
        })?;

        match timeout(self.default_timeout, rx).await {
            Ok(Ok(response)) => {
                if let Some(error_obj) = response.error {
                    return Err(CdpError::ProtocolError {
                        code: error_obj["code"].as_i64().unwrap_or(-1),
                        message: error_obj["message"]
                            .as_str()
                            .unwrap_or("Unknown CDP error")
                            .to_string(),
                    });
                }
                Ok(response)
            }
            Ok(Err(_)) => Err(CdpError::Disconnected),
            Err(_) => {
                if let Ok(mut map) = self.pending_requests.lock() {
                    map.remove(&id);
                }

                Err(CdpError::Timeout {
                    method: method.to_string(),
                    timeout: self.default_timeout,
                })
            }
        }
    }
}

/// Writer task: drains the command queue onto the socket.
///
/// Owning the sink here is what lets [`CdpClient`] stay `Clone`: every clone
/// pushes into the same queue instead of sharing a locked sink.
async fn write_commands(
    mut ws_sink: SplitSink<CdpWebSocket, Message>,
    mut command_rx: mpsc::UnboundedReceiver<String>,
) {
    let result: CdpResult<()> = async {
        while let Some(json_str) = command_rx.recv().await {
            debug!("About to send command {}", json_str);
            ws_sink.send(Message::Text(json_str.into())).await?;
        }
        Ok(())
    }
    .await;

    if let Err(e) = result {
        error!("Fatal error in CDP Writer task: {}", e);
    }
}

/// Reader task: demultiplexes everything the browser sends back.
async fn read_messages(
    mut ws_source: SplitStream<CdpWebSocket>,
    pending_requests: PendingRequests,
    event_tx: broadcast::Sender<WsResponse>,
    is_alive: Arc<AtomicBool>,
) {
    let reader_result: CdpResult<()> = async {
        while let Some(result_stream) = ws_source.next().await {
            let msg = result_stream.map_err(CdpError::from)?;

            if let Message::Text(received_text) = msg {
                let response: WsResponse = serde_json::from_str(&received_text)?;
                route_message(response, &pending_requests, &event_tx, &received_text)?;
            }
        }
        Ok(())
    }
    .await;

    is_alive.store(false, Ordering::SeqCst);

    match reader_result {
        Err(e) => debug!("CDP Connection lost due to error: {}", e),
        Ok(_) => debug!("CDP Connection closed by server/gracefully"),
    }

    fail_pending_requests(&pending_requests);
}

/// Hands a message to whoever is waiting for it.
///
/// Anything carrying an `id` answers a specific command; anything carrying a
/// `method` is an event and goes to every subscriber. On a browser-level
/// connection both kinds may also carry a `sessionId`, which subscribers use
/// to tell tabs apart.
fn route_message(
    response: WsResponse,
    pending_requests: &PendingRequests,
    event_tx: &broadcast::Sender<WsResponse>,
    raw_message: &str,
) -> CdpResult<()> {
    if let Some(id) = response.id {
        let mut map = pending_requests.lock().map_err(|_| {
            CdpError::InternalError(
                "Could not acquire mutex lock on pending requests cloned map".to_string(),
            )
        })?;

        if let Some(responder) = map.remove(&id) {
            trace!("Received expected id {} from CDP", id);
            let _ = responder.send(response);
        } else {
            trace!("Discarded message with id {} (no listener found)", id);
        }
    } else if let Some(ref method) = response.method {
        trace!("CDP Event: {}", method);
        let _ = event_tx.send(response.clone());
    } else {
        debug!(
            "Received unexpected message from CDP (no id, no method): {}",
            raw_message
        );
    }

    Ok(())
}

/// Wakes every caller still waiting for a response once the socket is gone.
fn fail_pending_requests(pending_requests: &PendingRequests) {
    if let Ok(mut map) = pending_requests.lock() {
        debug!(
            "Closing {} pending requests due to disconnection",
            map.len()
        );
        map.clear(); // Dropping the 'oneshot' senders sends an Err to the receivers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::time::Duration;

    /// Builds a client whose writer end is under the test's control, so no
    /// real browser or socket is involved.
    fn test_client(
        command_tx: mpsc::UnboundedSender<String>,
        event_tx: broadcast::Sender<WsResponse>,
        default_timeout: Duration,
    ) -> CdpClient {
        CdpClient {
            command_tx,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            default_timeout,
            is_alive: Arc::new(AtomicBool::new(true)),
        }
    }

    #[tokio::test]
    async fn test_command_timeout_and_cleanup() {
        let (command_tx, _command_rx) = mpsc::unbounded_channel();

        let client = test_client(
            command_tx,
            broadcast::channel(1).0,
            Duration::from_millis(100),
        );

        let result = client
            .send_raw_command("Page.navigate", serde_json::json!({"url": "about:blank"}))
            .await;

        match result {
            Err(CdpError::Timeout { method, .. }) => {
                assert_eq!(method, "Page.navigate");
                println!("✅ Timeout detected successfully");
            }
            other => panic!("❌ Expected Timeout, but got: {:?}", other),
        }

        let pending = client.pending_requests.lock().unwrap();
        assert_eq!(pending.len(), 0, "❌ ID was not removed from HashMap");
        println!("✅ HashMap is clean, no memory leaks");
    }

    #[tokio::test]
    async fn test_protocol_error_mapping() {
        let (command_tx, _command_rx) = mpsc::unbounded_channel();
        let client = test_client(command_tx, broadcast::channel(1).0, Duration::from_secs(1));

        let client_clone = client.clone();

        let handle = tokio::spawn(async move {
            client_clone
                .send_raw_command("Page.navigate", serde_json::json!({"url": "invalid-url"}))
                .await
        });

        // Simulate server time response
        tokio::time::sleep(Duration::from_millis(10)).await;

        let responder = {
            let mut map = client.pending_requests.lock().unwrap();
            map.remove(&1)
                .expect("The client should have registered request with ID 1")
        };

        // Simulate response with error in Browser
        let error_response = WsResponse {
            id: Some(1),
            error: Some(serde_json::json!({
                "code": -32000,
                "message": "Cannot navigate to invalid URL"
            })),
            ..Default::default()
        };

        responder.send(error_response).unwrap();

        // Verify the client mapped the JSON to the CdpError enum
        let result = handle.await.unwrap();
        match result {
            Err(CdpError::ProtocolError { code, message }) => {
                assert_eq!(code, -32000);
                assert_eq!(message, "Cannot navigate to invalid URL");
                println!("✅ Protocol Error mapped correctly");
            }
            other => panic!("❌ Expected ProtocolError, but got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_event_broadcasting_and_filtering() {
        let (command_tx, _command_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(32);

        let client = test_client(command_tx, event_tx.clone(), Duration::from_secs(1));

        let mut page_events = client.on_domain("Page");

        // Mock a CDP Event
        let mock_event = WsResponse {
            method: Some("Page.loadEventFired".to_string()),
            params: Some(serde_json::json!({
                "timestamp": 12345.678
            })),
            ..Default::default()
        };

        // Simulate the reader task sending the event to the broadcast channel
        let event_to_send = mock_event.clone();
        tokio::spawn(async move {
            // Wait a bit to ensure the subscriber is ready
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = event_tx.send(event_to_send);
        });

        // Await the event in our filter
        let received_event = timeout(Duration::from_secs(1), page_events.next())
            .await
            .expect("Timeout waiting for Page event")
            .expect("Stream closed unexpectedly")
            .expect("Failed to receive event from broadcast channel");

        assert_eq!(
            received_event.method.as_deref(),
            Some("Page.loadEventFired")
        );

        let timestamp = received_event
            .params
            .as_ref()
            .and_then(|p| p.get("timestamp"))
            .and_then(|t| t.as_f64());

        assert_eq!(timestamp, Some(12345.678));
        println!("✅ Event correctly filtered and received by domain subscriber");
    }

    #[tokio::test]
    async fn test_broadcast_lagged_error() {
        // Setup with a very small broadcast buffer (only 4 messages allowed)
        let (command_tx, _command_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(4);

        let client = test_client(command_tx, event_tx.clone(), Duration::from_secs(1));

        let mut page_events = client.on_domain("Page");

        // Flood the channel with 10 events without reading them
        // This will exceed the buffer capacity of 4
        for i in 0..10 {
            let mock_event = WsResponse {
                method: Some("Page.event".to_string()),
                params: Some(serde_json::json!({ "index": i })),
                ..Default::default()
            };
            let _ = event_tx.send(mock_event);
        }

        // Try to read from the subscriber
        // Since we sent 10 messages but the buffer is 4, we are "lagged"
        let result = page_events.next().await;

        // The first call to next() after a lag should return an Error
        // specifically a CdpError that wraps broadcast::error::RecvError::Lagged
        match result {
            Some(Err(CdpError::InternalError(msg))) => {
                // Your CdpError::from implementation for RecvError likely formats it as a string
                assert!(msg.contains("channel overflow") || msg.contains("lagged"));
                println!("✅ Correctly detected lagged subscriber (buffer overflow)");
            }
            Some(Ok(event)) => {
                panic!(
                    "❌ Expected a Lagged error, but received a successful event: {:?}",
                    event
                );
            }
            None => panic!("❌ Stream closed unexpectedly"),
            _ => {}
        }

        println!("✅ Buffer overflow test passed: Oldest messages were dropped as expected");
    }

    #[tokio::test]
    async fn test_connection_drop_during_request() {
        let (command_tx, _command_rx) = mpsc::unbounded_channel();

        let client = test_client(command_tx, broadcast::channel(1).0, Duration::from_secs(5));
        let pending_requests = Arc::clone(&client.pending_requests);

        let client_clone = client.clone();
        let request_handle = tokio::spawn(async move {
            client_clone
                .send_raw_command("Debugger.enable", serde_json::json!({}))
                .await
        });

        // Simulate a sudden disconnection
        tokio::time::sleep(Duration::from_millis(10)).await;
        {
            let mut map = pending_requests.lock().unwrap();
            // Dropping the 'oneshot::Sender' is what happens when we clear the map
            // or the task dies. This sends a 'RecvError' to the 'Receiver'.
            map.clear();
        }
        client.is_alive.store(false, Ordering::SeqCst);

        let result = request_handle.await.unwrap();
        match result {
            Err(CdpError::Disconnected) => {
                println!("✅ Correctly detected disconnection during pending request");
            }
            other => panic!("❌ Expected CdpError::Disconnected, but got: {:?}", other),
        }
    }

    /// Captures the JSON a command puts on the wire, letting the request time
    /// out afterwards (no server is answering).
    async fn capture_sent_payload(
        send: impl FnOnce(CdpClient) -> tokio::task::JoinHandle<CdpResult<WsResponse>>,
    ) -> Value {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let client = test_client(
            command_tx,
            broadcast::channel(1).0,
            Duration::from_millis(50),
        );

        let handle = send(client);

        let payload = command_rx
            .recv()
            .await
            .expect("the client should have written a command");
        let _ = handle.await;

        serde_json::from_str(&payload).expect("the command must be valid JSON")
    }

    #[tokio::test]
    async fn legacy_command_puts_no_session_id_on_the_wire() {
        let sent = capture_sent_payload(|client| {
            tokio::spawn(async move {
                client
                    .send_raw_command("Page.navigate", serde_json::json!({"url": "about:blank"}))
                    .await
            })
        })
        .await;

        assert_eq!(sent["method"], "Page.navigate");
        assert!(
            sent.get("sessionId").is_none(),
            "connections opened with CdpClient::new must keep the pre-existing wire format, got {sent}",
        );
    }

    #[tokio::test]
    async fn session_command_puts_session_id_on_the_wire() {
        let sent = capture_sent_payload(|client| {
            tokio::spawn(async move {
                client
                    .send_raw_command_to_session(
                        "SESSION-42",
                        "Page.navigate",
                        serde_json::json!({"url": "about:blank"}),
                    )
                    .await
            })
        })
        .await;

        assert_eq!(sent["method"], "Page.navigate");
        assert_eq!(sent["sessionId"], "SESSION-42");
    }

    #[tokio::test]
    async fn ids_stay_unique_across_sessions() {
        // A single pending-request map is shared by every tab, so the id
        // counter must not restart per session or responses would be
        // delivered to the wrong caller.
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let client = test_client(
            command_tx,
            broadcast::channel(1).0,
            Duration::from_millis(50),
        );

        for session in ["SESSION-A", "SESSION-B", "SESSION-A"] {
            let client = client.clone();
            let session = session.to_string();
            tokio::spawn(async move {
                client
                    .send_raw_command_to_session(&session, "Runtime.enable", serde_json::json!({}))
                    .await
            });
        }

        let mut ids = Vec::new();
        for _ in 0..3 {
            let payload = command_rx.recv().await.expect("command should be written");
            let sent: Value = serde_json::from_str(&payload).unwrap();
            ids.push(sent["id"].as_u64().expect("id must be a number"));
        }

        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "each command must get a distinct id");
    }
}
