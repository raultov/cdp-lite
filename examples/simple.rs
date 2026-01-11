use cdp_lite::error::CdpResult;
use cdp_lite::protocol::NoParams;
use serde::Serialize;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

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
    info!("CDP Client connected");

    cdp_client.send_raw_command("Page.enable", NoParams).await?;

    let params = NavigateParams {
        url: "https://www.rust-lang.org".to_string()
    };
    let response = cdp_client.send_raw_command("Page.navigate", params).await?;

    info!("Chrome replied: {:?}", response);

    Ok(())
}
