# cdp-lite

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)

**cdp-lite** is a lightweight, asynchronous, and low-overhead Rust client for the **Chrome DevTools Protocol (CDP)**.

Unlike heavier frameworks, `cdp-lite` focuses on providing a minimal and fast interface to interact with Chromium-based browsers via WebSockets. It is designed for developers who need raw control, high performance, and a small binary footprint.

---

## ✨ Features

* **Zero Bloat:** Minimal dependencies and no unnecessary abstractions.
* **Fully Asynchronous:** Built on top of `tokio` and `tungstenite`.
* **Low Overhead:** Efficient command/event dispatching using `oneshot` and `broadcast` channels.
* **Direct Protocol Access:** Send any CDP command and receive raw JSON responses.
* **Event Filtering:** Subscribe to specific protocol domains (Page, Network, Debugger, etc.) with ease.
* **Multi-Tab Control:** Drive many tabs of the same browser over a single WebSocket using CDP flat sessions.

---

## 🚀 Quick Start

Add this to your `Cargo.toml`:

```toml
[dependencies]
cdp-lite = "0.1.0"
tokio = { version = "1", features = ["full"] }
serde_json = "1.0"
```

---

## 🕹️ Basic Usage
```rust
use cdp_lite::error::CdpResult;
use cdp_lite::protocol::NoParams;
use serde::Serialize;
use std::time::Duration;

#[tokio::main]
async fn main() -> CdpResult<()> {
    // 1. Connect to a running Chrome instance
    // Run chrome with: --remote-debugging-port=9222
    let cdp_client = cdp_lite::client::CdpClient::new("127.0.0.1:9222", Duration::from_secs(1)).await?;

    // 2. Enable necessary domains
    cdp_client.send_raw_command("Page.enable", NoParams).await?;

    // 3. Navigate to a URL
    let response = cdp_client.send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"})).await?;

    println!("Navigating to Rust homepage...");
    Ok(())
}
```
---

## 📂 Advanced examples

You can find complete, ready-to-run examples in the [examples/](https://github.com/raultov/cdp-lite/tree/master/examples) directory:
* `breakpoints_usage.rs`: Basic breakpoints in JavaScript upon execution (Debugger domain).
* `filter_domains.rs`: How to subscribe to different browser events.
* `runtime_usage.rs`: Using the Runtime domain.
* `proxy_usage.rs`: Authenticating the browser against a Proxy Server.
* `multi_tab.rs`: Controlling several tabs of the same browser at once.
* `multi_tab_parallel.rs`: Loading several pages concurrently, one tab each, over a single connection.

---

## 🗂️ Multiple Tabs

`CdpClient` attaches to one page and stays there. To drive several tabs, connect to the
**browser** endpoint with `BrowserClient`: it opens a single WebSocket and multiplexes every
tab over it as a CDP *flat session*, giving you one `Tab` handle per tab.

```rust
use cdp_lite::browser::BrowserClient;
use cdp_lite::protocol::NoParams;
use serde_json::json;
use std::time::Duration;

let browser = BrowserClient::connect("127.0.0.1:9222", Duration::from_secs(5)).await?;

// Inspect what is open, attach to it, or open new tabs.
for info in browser.list_tabs().await? {
    println!("{} -> {}", info.target_id, info.url);
}
let existing = browser.attach_to_all_tabs().await?;
let docs = browser.new_tab("https://docs.rs").await?;

// Commands and events are scoped to a single tab.
docs.send_raw_command("Page.enable", NoParams).await?;
docs.send_raw_command("Page.navigate", json!({ "url": "https://crates.io" })).await?;

let mut page_events = docs.on_domain("Page"); // only this tab's events

docs.activate().await?; // bring it to the foreground
docs.close().await?;    // or browser.close_tab(id)
```

| Method | Purpose |
| --- | --- |
| `BrowserClient::connect(host, timeout)` | Connect to the browser endpoint (`/json/version`) |
| `list_tabs()` / `list_targets()` | Enumerate tabs (or every target) |
| `attach(target_id)` / `attach_to_all_tabs()` | Get a `Tab` for existing tabs |
| `new_tab(url)` / `close_tab(target_id)` | Open and close tabs |
| `set_discover_targets(bool)` | Emit `Target.targetCreated` / `targetDestroyed` events |
| `Tab::send_raw_command(..)` | Send a command to that tab only |
| `Tab::on_domain(d)` / `Tab::events()` | Stream that tab's events |
| `Tab::activate()` / `close()` / `detach()` | Focus, close, or stop listening |

> **Backwards compatible:** this is purely additive. `CdpClient::new` still connects straight to a
> page target and never puts a `sessionId` on the wire, so existing code behaves exactly as before.

---

## 📡 Event Subscription

`cdp-lite` allows you to listen to specific protocol domains without overhead. You can spawn multiple listeners for different domains simultaneously:

```rust
// Create a stream for Network events
let mut network_events = client.on_domain("Network");

tokio::spawn(async move {
    while let Some(Ok(event)) = network_events.next().await {
        if event.method == "Network.requestWillBeSent" {
            println!("Request made to: {}", event.get_param_str("documentURL").unwrap_or("unknown"));
        }
    }
});
```

---

## 📖 Why cdp-lite?

Most Rust libraries for Chrome are high-level wrappers that abstract the protocol. While useful, they can be restrictive for:
* **Security Research:** Precise control over frame interception and debugger state.
* **High-Performance Scrapers:** Minimizing CPU and memory usage per browser instance.
* **Custom Tooling:** Building specialized debugging or profiling tools.

**cdp-lite** stays out of your way and gives you the "wire" directly.

---

## 📜 License

This project is licensed under the **MIT License**. It is free to use, modify, and distribute, even for commercial purposes. See the [LICENSE](LICENSE) file for more details.

---

## 🤝 Contributing

Contributions are welcome! Since this is a "lite" library, the focus is on maintaining a small footprint, high stability, and idiomatic asynchronous Rust. Feel free to open issues or submit pull requests.
