use cdp_lite::error::CdpResult;
use cdp_lite::protocol::NoParams;
use log::info;
use serde_json::json;
use std::time::Duration;
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
    cdp_client
        .send_raw_command("Runtime.enable", NoParams)
        .await?;
    cdp_client
        .send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"}))
        .await?;

    let expression_result = cdp_client
        .send_raw_command(
            "Runtime.evaluate",
            json!({
                "expression": "({
          title: document.title,
          url: location.href,
          h1: document.querySelector(\"h1\")?.innerText
        })",
                "returnByValue": true
            }),
        )
        .await?;

    info!("Expression outcome: {:?}", expression_result);

    let expression_promise_result = cdp_client
        .send_raw_command(
            "Runtime.evaluate",
            json!({
                "expression": "  (async () => {
            const r = await fetch('/');
            return r.status;
        })()",
                "returnByValue": true,
                "awaitPromise": true
            }),
        )
        .await?;

    info!(
        "Expression promise outcome: {:?}",
        expression_promise_result
    );

    Ok(())
}
