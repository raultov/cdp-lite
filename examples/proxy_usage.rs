use cdp_lite::client::CdpClient;
use cdp_lite::error::CdpResult;
use cdp_lite::event_filter::EventFilter;
use cdp_lite::protocol::{NoParams, WsResponse};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> CdpResult<()> {
    fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cdp_client = CdpClient::new("127.0.0.1:9222", Duration::from_secs(2)).await?;
    enable_fetch(&cdp_client).await?;

    let proxy_auth_signal = Arc::new(Notify::new());
    spawn_fetch_handler(cdp_client.clone(), proxy_auth_signal.clone());

    cdp_client
        .send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"}))
        .await?;

    info!("Waiting for proxy authentication...");
    proxy_auth_signal.notified().await;
    info!("Proxy is authenticated! Continuing main execution...");

    Ok(())
}

/// Turns `Page` and `Fetch` on, asking Chrome to pause requests so the
/// background task below can answer the proxy's auth challenge.
async fn enable_fetch(client: &CdpClient) -> CdpResult<()> {
    client.send_raw_command("Page.enable", NoParams).await?;

    let fetch_params = json!({
        "patterns": [
            {
                "urlPattern": "*",
                "requestStage": "Request"
            }
        ],
        "handleAuthRequests": true
    });
    client
        .send_raw_command("Fetch.enable", fetch_params)
        .await?;

    Ok(())
}

/// Listens for `Fetch` events in the background until the proxy asks for
/// credentials, then answers it and notifies the main task.
fn spawn_fetch_handler(client: CdpClient, proxy_auth_signal: Arc<Notify>) -> JoinHandle<()> {
    let mut fetch_events = client.on_domain("Fetch");

    tokio::spawn(async move {
        let result: CdpResult<()> =
            handle_fetch_events(&client, &mut fetch_events, &proxy_auth_signal).await;

        if let Err(e) = result {
            error!("Fatal error in Fetch task: {}", e);
        }
    })
}

async fn handle_fetch_events(
    client: &CdpClient,
    fetch_events: &mut EventFilter,
    proxy_auth_signal: &Notify,
) -> CdpResult<()> {
    while let Some(Ok(event)) = fetch_events.next().await {
        match event.method.as_ref().unwrap().as_str() {
            "Fetch.requestPaused" => {
                if let Some(request_id) = get_request_id(&event) {
                    let params = json!({"requestId": request_id});
                    client
                        .send_raw_command("Fetch.continueRequest", params)
                        .await?;
                    info!("Request Fetch {} continued", request_id);
                }
            }
            "Fetch.authRequired" => {
                if let Some(request_id) = get_request_id(&event) {
                    let params = json!({
                        "requestId": request_id,
                        "authChallengeResponse": {
                            "response": "ProvideCredentials",
                            "username": "username",
                            "password": "password"
                        }
                    });
                    client
                        .send_raw_command("Fetch.continueWithAuth", params)
                        .await?;
                    client.send_raw_command("Fetch.disable", NoParams).await?;
                    proxy_auth_signal.notify_one();
                    info!("Browser authenticated against proxy");
                }

                break;
            }
            _ => {
                warn!("Unexpected Fetch event");
            }
        }
    }

    Ok(())
}

fn get_request_id(response: &WsResponse) -> Option<&str> {
    response
        .params
        .as_ref()
        .and_then(|p| p.get("requestId"))
        .and_then(|v| v.as_str())
}
