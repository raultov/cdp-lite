use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio::sync::broadcast;
use tracing::{debug, error, trace};
use crate::rest_client::get_websocket_url;
use crate::error::{ CdpError, CdpResult };
use crate::event_filter::EventFilter;
use crate::protocol::{WsCommand, WsResponse};


#[derive(Clone)]
pub struct CdpClient {
    command_tx: mpsc::UnboundedSender<String>,
    pending_requests: Arc<Mutex<HashMap<u64, oneshot::Sender<WsResponse>>>>,
    event_tx: broadcast::Sender<WsResponse>,
    next_id: Arc<AtomicU64>,
    default_timeout: Duration,
    is_alive: Arc<AtomicBool>,
}

impl CdpClient {
    pub async fn new(host: &str, default_timeout: Duration) -> CdpResult<Self> {
        let ws_url = get_websocket_url(host).await?;

        let (ws_stream, _) = connect_async(ws_url).await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        let pending_requests = Arc::new(Mutex::new(HashMap::<u64, oneshot::Sender<WsResponse>>::new()));
        let pending_requests_clone = Arc::clone(&pending_requests);

        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<String>();

        let (event_tx, _) = broadcast::channel(128);
        let event_tx_clone = event_tx.clone();

        let is_alive = Arc::new(AtomicBool::new(true));
        let is_alive_reader = is_alive.clone();

        tokio::spawn(async move {
            let result: CdpResult<()> = async {
                while let Some(json_str) = command_rx.recv().await {
                    debug!("About to send command {}", json_str);
                    ws_sink.send(Message::Text(json_str.into())).await?;
                }
                Ok(())
            }.await;

            if let Err(e) = result {
                error!("Fatal error in CDP Writer task: {}", e);
            }
        });

        tokio::spawn(async move {
            let reader_result: CdpResult<()> = async {
                while let Some(result_stream) = ws_stream.next().await {
                    let msg = result_stream.map_err(CdpError::from)?;

                    if let Message::Text(received_text) = msg {
                        let response: WsResponse = serde_json::from_str(&received_text)?;

                        if let Some(id) = response.id {
                            let mut map = pending_requests_clone.lock()
                                .map_err(|_| CdpError::InternalError("Could not acquire mutex lock on pending requests cloned map".to_string()))?;

                            if let Some(responder) = map.remove(&id) {
                                trace!("Received expected id {} from CDP", id);
                                let _ = responder.send(response);
                            } else {
                                trace!("Discarded message with id {} (no listener found)", id);
                            }
                        } else if let Some(ref method) = response.method {
                            trace!("CDP Event: {}", method);
                            let _ = event_tx_clone.send(response.clone());
                        } else {
                            debug!("Received unexpected message from CDP (no id, no method): {}", received_text);
                        }
                    }
                }
                Ok(())
            }.await;

            is_alive_reader.store(false, Ordering::SeqCst);

            match reader_result {
                Err(e) => debug!("CDP Connection lost due to error: {}", e),
                Ok(_) => debug!("CDP Connection closed by server/gracefully"),
            }

            if let Ok(mut map) = pending_requests_clone.lock() {
                debug!("Closing {} pending requests due to disconnection", map.len());
                map.clear(); // Dropping the 'oneshot' senders sends an Err to the receivers
            }
        });

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

    pub async fn send_raw_command<P: Serialize>(&self, method: &str, params: P) -> CdpResult<WsResponse> {
        if !self.is_alive.load(Ordering::SeqCst) {
            return Err(CdpError::Disconnected);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut map = self.pending_requests.lock()
                .map_err(|_| CdpError::InternalError("Mutex poisoned: another thread panicked while holding the lock".into()))?;
            map.insert(id, tx);
        }

        let cmd = WsCommand {
          id,
          method: method.to_string(),
          params: Some(params),
        };

        let json_payload = serde_json::to_string(&cmd)?;
        self.command_tx.send(json_payload)
            .map_err(|e| {
                self.is_alive.store(false, Ordering::SeqCst);
                CdpError::InternalError(format!("Failed to send command to writer task: {}", e))
            })?;

        match timeout(self.default_timeout, rx).await {
            Ok(Ok(response)) => {
                if let Some(error_obj) = response.error {
                    return Err(CdpError::ProtocolError {
                        code: error_obj["code"].as_i64().unwrap_or(-1),
                        message: error_obj["message"].as_str().unwrap_or("Unknown CDP error").to_string(),
                    })
                }
                Ok(response)
            },
            Ok(Err(_)) => {
                Err(CdpError::Disconnected)
            },
            Err(_) => {
                if let Ok (mut map) = self.pending_requests.lock() {
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