//! # cdp-lite
//!
//! A lightweight and low-overhead Rust client for the Chrome DevTools Protocol.
//!
//! ## Examples
//! ### 1. Basic Navigation
//! This example shows how to connect to a browser and navigate to a specific URL.
//!
//! ```rust
//! use cdp_lite::client::CdpClient;
//! use cdp_lite::protocol::NoParams;
//! use cdp_lite::error::CdpResult;
//! use std::time::Duration;
//! use serde_json::json;
//!
//! # async fn doc_example() -> CdpResult<()> {
//! let client = CdpClient::new("127.0.0.1:9222", Duration::from_secs(5)).await?;
//! client.send_raw_command("Page.enable", NoParams).await?;
//! client.send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"})).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### 2. Listening to Events
//! This example demonstrates how to subscribe to specific domains (like "Network" and "Page") and process incoming events.
//!
//! ```rust
//! use cdp_lite::client::CdpClient;
//! use cdp_lite::protocol::NoParams;
//! use cdp_lite::error::CdpResult;
//! use std::time::Duration;
//! use serde_json::json;
//! use tokio_stream::StreamExt;
//!
//! # async fn doc_example() -> CdpResult<()>{
//! let client = CdpClient::new("127.0.0.1:9222", Duration::from_secs(5)).await?;
//! let network = client.on_domain("Network");
//! let page = client.on_domain("Page");
//! let mut activity = StreamExt::merge(network, page);
//!     tokio::spawn(async move {
//!         while let Some(Ok(event)) = activity.next().await {
//!             println!("📢 Activity: {}", event.method.unwrap());
//!         }
//!     });
//! client.send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"})).await?;
//! # Ok(())
//! # }
//! ```

//! ### 3. Driving several tabs at once
//! [`client::CdpClient`] is bound to the single page it connected to. To
//! control more than one tab, connect to the *browser* endpoint with
//! [`browser::BrowserClient`]: it multiplexes every tab over one WebSocket
//! using CDP flat sessions, and hands out a [`tab::Tab`] per tab.
//!
//! ```rust
//! use cdp_lite::browser::BrowserClient;
//! use cdp_lite::error::CdpResult;
//! use cdp_lite::protocol::NoParams;
//! use std::time::Duration;
//! use serde_json::json;
//! use tokio_stream::StreamExt;
//!
//! # async fn doc_example() -> CdpResult<()> {
//! let browser = BrowserClient::connect("127.0.0.1:9222", Duration::from_secs(5)).await?;
//!
//! // Attach to what is already open, and open one more tab.
//! let mut tabs = browser.attach_to_all_tabs().await?;
//! tabs.push(browser.new_tab("https://docs.rs").await?);
//!
//! for tab in &tabs {
//!     // Events are scoped to the tab they came from.
//!     let mut loads = tab.on_domain("Page");
//!     tokio::spawn(async move {
//!         while let Some(Ok(event)) = loads.next().await {
//!             println!("📢 {}", event.method.unwrap());
//!         }
//!     });
//!
//!     tab.send_raw_command("Page.enable", NoParams).await?;
//! }
//!
//! // Commands act on one tab only.
//! tabs[0].send_raw_command("Page.navigate", json!({"url": "https://crates.io"})).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Adding tab support changed nothing for existing code: [`client::CdpClient`]
//! still attaches directly to a page target and never sends a `sessionId`.

pub mod browser;
pub mod client;
pub mod error;
pub mod event_filter;
pub mod protocol;
mod rest_client;
pub mod tab;
