# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-08-22

### Added

- **Multi-tab support** — drive several tabs of the same browser over a single
  WebSocket connection using CDP flat sessions:
  - `browser::BrowserClient` connects to the browser-level endpoint
    (`/json/version`) and can `list_tabs`, `list_targets`, `attach`,
    `attach_to_all_tabs`, `new_tab`, `close_tab` and `set_discover_targets`.
  - `tab::Tab` is the per-tab handle: scoped commands (`send_raw_command`),
    scoped events (`on_domain`, `events`), plus `activate`, `close` and
    `detach`.
  - `CdpClient::send_raw_command_to_session` sends a command routed by
    `sessionId`.
  - `CdpClient::connect` opens a connection to an already-known WebSocket
    debugger URL.
  - `rest_client::get_browser_websocket_url` resolves the browser endpoint.
  - `WsCommand`/`WsResponse` gained an optional `sessionId`, omitted from the
    wire when unused.
  - `EventFilter::for_session` / `CdpClient::on_domain_for_session` filter
    events by tab.
- New examples: `multi_tab.rs` (tab lifecycle) and `multi_tab_parallel.rs`
  (concurrent page loads over one socket).
- `clippy.toml` with lowered quality-gate thresholds (max 50-line functions,
  max complexity 15, max 4 arguments).
- `CHANGELOG.md`.

### Changed

- Refactored `CdpClient::connect` into smaller tasks (`write_commands`,
  `read_messages`, `route_message`, `fail_pending_requests`) to satisfy the
  new thresholds; the reader/writer task behaviour is unchanged.
- Refactored `proxy_usage.rs` and `breakpoints_usage.rs` examples to satisfy
  the new thresholds, preserving their behaviour.

## [0.1.3] - 2026-07-19

### Fixed

- Target selection no longer attaches to the DevTools front-end
  (`devtools://devtools/...`), which Chrome reports as a `page` target when
  DevTools is open and could hijack every command. See
  [issue #1](https://github.com/raultov/cdp-lite/issues/1).

### Added

- Target selection prefers real web pages (`http`, `https`, `file`, `about`)
  over Chrome-internal pages like `chrome://newtab/`.
- GitHub Actions CI workflow (`rust.yml`).

### Changed

- Test suite for target selection moved to `wiremock`, so HTTP behaviour is
  tested without a live browser.

## [0.1.1] - 2026-01-18

First published release.

### Added

- Asynchronous WebSocket client for the Chrome DevTools Protocol.
- Raw command dispatch (`send_raw_command`) with timeout handling and
  protocol-error mapping.
- Event subscription and per-domain filtering (`on_domain`).
- Automatic target discovery through `GET /json/list`.
- Usage examples: `simple`, `filter_domains`, `runtime_usage`,
  `breakpoints_usage`, `proxy_usage`.

[Unreleased]: https://github.com/raultov/cdp-lite/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/raultov/cdp-lite/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/raultov/cdp-lite/compare/v0.1.1...v0.1.3
[0.1.1]: https://github.com/raultov/cdp-lite/releases/tag/v0.1.1
