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

mod rest_client;
pub mod protocol;
pub mod client;
pub mod error;
pub mod event_filter;