use std::time::Duration;
use serde::Serialize;
use tokio_stream::StreamExt;
use tracing::{debug, info};
use tracing_subscriber::{fmt, EnvFilter};
use cdp_lite::error::CdpResult;
use cdp_lite::protocol::NoParams;

#[derive(Serialize)]
struct NavigateParams {
    url: String,
}

#[tokio::main]
async fn main() -> CdpResult<()> {
    fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cdp_client = cdp_lite::client::CdpClient::new("127.0.0.1:9222", Duration::from_secs(1)).await?;

    let network = cdp_client.on_domain("Network");
    let page = cdp_client.on_domain("Page");
    let mut activity = StreamExt::merge(network, page);

    tokio::spawn(async move {
        while let Some(Ok(event)) = activity.next().await {
            info!("📢 Activity: {}", event.method.unwrap());
        }
    });

    let mut events = cdp_client.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            debug!("📢 Event received: {:?}", event.method.unwrap());
        }
    });

    let mut network_events = cdp_client.on_domain("Network");
    
    tokio::spawn(async move {
        while let Some(Ok(event)) = network_events.next().await {
            debug!("🌐 Network Activity: {}", event.method.unwrap());
        }
    });

    cdp_client.send_raw_command("Page.enable", NoParams).await?;

    let params = NavigateParams {
        url: "https://www.rust-lang.org".to_string()
    };
    let _ = cdp_client.send_raw_command("Page.navigate", params).await?;

    Ok(())
}