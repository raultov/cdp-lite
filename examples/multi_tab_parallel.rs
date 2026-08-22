//! Loads several pages *concurrently*, one tab each, over a single WebSocket.
//!
//! This is what multiplexing buys you: the tabs load in parallel and the total
//! time is roughly that of the slowest page, not the sum of all of them.
//!
//! Run Chrome with `--remote-debugging-port=9222`, then:
//! `cargo run --example multi_tab_parallel [host:port]`

use cdp_lite::browser::BrowserClient;
use cdp_lite::error::{CdpError, CdpResult};
use cdp_lite::event_filter::EventFilter;
use cdp_lite::protocol::NoParams;
use cdp_lite::tab::Tab;
use futures_util::future::join_all;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_stream::StreamExt;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};

const URLS: [&str; 4] = [
    "https://www.rust-lang.org",
    "https://docs.rs",
    "https://crates.io",
    "https://blog.rust-lang.org",
];

const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(20);

#[tokio::main]
async fn main() -> CdpResult<()> {
    fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let browser = BrowserClient::connect(&host(), Duration::from_secs(10)).await?;
    let started = Instant::now();

    // One task per tab. They share the connection but never block each other.
    let jobs = URLS.map(|url| {
        let browser = browser.clone();
        async move { (url, title_of(&browser, url).await) }
    });

    for (url, result) in join_all(jobs).await {
        match result {
            Ok(title) => info!("✅ {url} => {title}"),
            Err(e) => warn!("❌ {url} => {e}"),
        }
    }

    info!("Loaded {} pages in {:?}", URLS.len(), started.elapsed());
    Ok(())
}

fn host() -> String {
    std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9222".to_string())
}

/// Opens a tab, loads `url`, reads its title, then closes the tab.
async fn title_of(browser: &BrowserClient, url: &str) -> CdpResult<String> {
    let tab = browser.new_tab("about:blank").await?;
    let title = load_and_read_title(&tab, url).await;

    // Close the tab even if the load failed, so a slow page leaks nothing.
    tab.close().await?;

    title
}

async fn load_and_read_title(tab: &Tab, url: &str) -> CdpResult<String> {
    tab.send_raw_command("Page.enable", NoParams).await?;

    // Subscribe *before* navigating, otherwise the load event can be missed.
    let mut page_events = tab.on_domain("Page");
    tab.send_raw_command("Page.navigate", json!({ "url": url }))
        .await?;
    wait_for(&mut page_events, "Page.loadEventFired").await?;

    let response = tab
        .send_raw_command(
            "Runtime.evaluate",
            json!({ "expression": "document.title", "returnByValue": true }),
        )
        .await?;

    Ok(response
        .result
        .as_ref()
        .and_then(|result| result.pointer("/result/value"))
        .and_then(|value| value.as_str())
        .unwrap_or("<no title>")
        .to_string())
}

/// Waits for one specific event on an already-scoped stream.
async fn wait_for(events: &mut EventFilter, method: &str) -> CdpResult<()> {
    let waiting = async {
        while let Some(event) = events.next().await {
            if event?.method.as_deref() == Some(method) {
                return Ok(());
            }
        }
        Err(CdpError::Disconnected)
    };

    tokio::time::timeout(PAGE_LOAD_TIMEOUT, waiting)
        .await
        .unwrap_or_else(|_| {
            Err(CdpError::Timeout {
                method: method.to_string(),
                timeout: PAGE_LOAD_TIMEOUT,
            })
        })
}
