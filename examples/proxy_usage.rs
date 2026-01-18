use cdp_lite::error::CdpResult;
use cdp_lite::protocol::{NoParams, WsResponse};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> CdpResult<()> {
    fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cdp_client =
        cdp_lite::client::CdpClient::new("127.0.0.1:9222", Duration::from_secs(2)).await?;
    cdp_client.send_raw_command("Page.enable", NoParams).await?;

    let fetch_params = json!({
        "patterns": [
            {
                "urlPattern": "*",
                "requestStage": "Request"
            }
        ],
        "handleAuthRequests": true
    });
    cdp_client
        .send_raw_command("Fetch.enable", fetch_params)
        .await?;

    let mut fetch_events = cdp_client.on_domain("Fetch");
    let cdp_client_clone = cdp_client.clone();

    let proxy_auth_signal = Arc::new(Notify::new());
    let proxy_auth_signal_clone = proxy_auth_signal.clone();

    tokio::spawn(async move {
        let result: CdpResult<()> = async {
            while let Some(Ok(event)) = fetch_events.next().await {
                match event.method.as_ref().unwrap().as_str() {
                    "Fetch.requestPaused" => {
                        if let Some(request_id) = get_request_id(&event) {
                            let params = json!({"requestId": request_id});
                            cdp_client_clone
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
                            cdp_client_clone
                                .send_raw_command("Fetch.continueWithAuth", params)
                                .await?;
                            cdp_client_clone
                                .send_raw_command("Fetch.disable", NoParams)
                                .await?;
                            proxy_auth_signal_clone.notify_one();
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
        .await;

        if let Err(e) = result {
            error!("Fatal error in Fetch task: {}", e);
        }
    });

    cdp_client
        .send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"}))
        .await?;

    info!("Waiting for proxy authentication...");
    proxy_auth_signal.notified().await;
    info!("Proxy is authenticated! Continuing main execution...");

    Ok(())
}

fn get_request_id(response: &WsResponse) -> Option<&str> {
    response
        .params
        .as_ref()
        .and_then(|p| p.get("requestId"))
        .and_then(|v| v.as_str())
}
