//! Basics of driving several tabs of the same browser over one connection.
//!
//! Run Chrome with `--remote-debugging-port=9222`, then:
//! `cargo run --example multi_tab [host:port]`

use cdp_lite::browser::BrowserClient;
use cdp_lite::error::CdpResult;
use cdp_lite::protocol::NoParams;
use cdp_lite::tab::Tab;
use serde_json::json;
use std::time::Duration;
use tokio_stream::StreamExt;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> CdpResult<()> {
    fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Connect to the browser itself, not to a single page.
    let browser = BrowserClient::connect(&host(), Duration::from_secs(5)).await?;
    info!("Browser connected");

    for info in browser.list_tabs().await? {
        info!("Already open: {} ({})", info.title, info.url);
    }

    // Open two tabs; each gets its own CDP session over the same socket.
    let rust = browser.new_tab("https://www.rust-lang.org").await?;
    let docs = browser.new_tab("https://docs.rs").await?;
    info!("Opened tabs {} and {}", rust.target_id(), docs.target_id());

    watch_page_events(&rust, "rust-lang");
    watch_page_events(&docs, "docs.rs");

    rust.send_raw_command("Page.enable", NoParams).await?;
    docs.send_raw_command("Page.enable", NoParams).await?;

    // Commands are routed per tab: only this one navigates.
    docs.send_raw_command("Page.navigate", json!({ "url": "https://crates.io" }))
        .await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Each tab has its own JavaScript context.
    for (name, tab) in [("rust-lang", &rust), ("docs.rs", &docs)] {
        info!("{name} is at {}", current_url(tab).await?);
    }

    rust.activate().await?;

    // Closing one tab leaves the other, and the connection, untouched.
    docs.close().await?;
    info!("Closed the docs.rs tab, {} still works", rust.target_id());
    info!("Remaining tabs: {}", browser.list_tabs().await?.len());

    Ok(())
}

fn host() -> String {
    std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9222".to_string())
}

/// Reads `document.location.href` inside one tab.
async fn current_url(tab: &Tab) -> CdpResult<String> {
    let response = tab
        .send_raw_command(
            "Runtime.evaluate",
            json!({ "expression": "document.location.href", "returnByValue": true }),
        )
        .await?;

    Ok(response
        .result
        .as_ref()
        .and_then(|result| result.pointer("/result/value"))
        .and_then(|value| value.as_str())
        .unwrap_or("<unknown>")
        .to_string())
}

/// Spawns a listener for one tab's `Page` events.
///
/// The stream is already scoped to this tab, so no other tab's events show up.
fn watch_page_events(tab: &Tab, name: &'static str) {
    let mut events = tab.on_domain("Page");

    tokio::spawn(async move {
        while let Some(Ok(event)) = events.next().await {
            if let Some(method) = event.method {
                info!("[{name}] {method}");
            }
        }
    });
}
