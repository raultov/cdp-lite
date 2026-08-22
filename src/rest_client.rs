use crate::error::{CdpError, CdpResult};
use serde::Deserialize;
use tracing::debug;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChromeTarget {
    pub title: String,
    pub r#type: String,
    pub url: String,
    pub web_socket_debugger_url: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct BrowserVersion {
    #[serde(default)]
    web_socket_debugger_url: String,
}

pub async fn get_websocket_url(host: &str) -> CdpResult<String> {
    let url = format!("http://{}/json/list", host);

    let targets: Vec<ChromeTarget> = reqwest::get(&url).await?.json().await?;

    let target = select_target(&targets).ok_or(CdpError::NoPageTargetFound(host.to_string()))?;

    debug!("Found target: {} - {}", target.title, target.url);

    Ok(target.web_socket_debugger_url.clone())
}

/// Resolves the browser-level WebSocket endpoint from `GET /json/version`.
///
/// Unlike [`get_websocket_url`], this endpoint is not bound to any page. It is
/// the connection used to drive the `Target` domain, which is what makes it
/// possible to attach to several tabs over a single socket.
///
/// Chrome omits `webSocketDebuggerUrl` when remote debugging is exposed
/// through certain proxies, so an empty value is treated as "no usable
/// target at this host".
pub async fn get_browser_websocket_url(host: &str) -> CdpResult<String> {
    let url = format!("http://{}/json/version", host);

    let version: BrowserVersion = reqwest::get(&url).await?.json().await?;

    if version.web_socket_debugger_url.is_empty() {
        return Err(CdpError::NoPageTargetFound(host.to_string()));
    }

    debug!(
        "Found browser endpoint: {}",
        version.web_socket_debugger_url
    );

    Ok(version.web_socket_debugger_url)
}

/// Picks the best target to attach to from a `GET /json/list` response.
///
/// Chrome reports the DevTools front-end (`devtools://devtools/...`) as a
/// `type == "page"` target when DevTools is open, and it can appear before
/// the real page in `/json/list`. Attaching to it makes every subsequent
/// CDP command (e.g. `Page.navigate`, `Runtime.evaluate`) operate on the
/// DevTools window instead of the page the user is looking at. See
/// <https://github.com/raultov/cdp-lite/issues/1>.
///
/// Strategy:
/// 1. Prefer an attachable target whose URL is a real web page
///    (`http`/`https`/`file`/`about`).
/// 2. Fall back to any other attachable page (e.g. `chrome://newtab/`) so
///    the caller gets a working page rather than an error.
/// 3. The DevTools front-end is **never** selected, even as a fallback.
fn select_target(targets: &[ChromeTarget]) -> Option<&ChromeTarget> {
    let is_attachable = |t: &ChromeTarget| {
        t.r#type == "page"
            && !t.web_socket_debugger_url.is_empty()
            && !t.url.starts_with("devtools://")
    };
    let is_web_page = |t: &ChromeTarget| {
        t.url.starts_with("http://")
            || t.url.starts_with("https://")
            || t.url.starts_with("file://")
            || t.url.starts_with("about:")
    };

    targets
        .iter()
        .find(|t| is_attachable(t) && is_web_page(t))
        .or_else(|| targets.iter().find(|t| is_attachable(t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ty: &str, url: &str, ws: &str) -> ChromeTarget {
        ChromeTarget {
            title: url.to_string(),
            r#type: ty.to_string(),
            url: url.to_string(),
            web_socket_debugger_url: ws.to_string(),
        }
    }

    #[test]
    fn skips_devtools_even_when_listed_first() {
        let targets = vec![
            t(
                "page",
                "devtools://devtools/bundled/devtools_app.html",
                "ws://x/devtools",
            ),
            t("page", "https://m.baidu.com/", "ws://x/baidu"),
        ];
        assert_eq!(
            select_target(&targets).map(|t| t.url.as_str()),
            Some("https://m.baidu.com/"),
        );
    }

    #[test]
    fn prefers_real_web_page_over_chrome_internal() {
        let targets = vec![
            t("page", "chrome://newtab/", "ws://x/newtab"),
            t(
                "page",
                "devtools://devtools/bundled/devtools_app.html",
                "ws://x/devtools",
            ),
            t("page", "https://example.com/", "ws://x/example"),
        ];
        assert_eq!(
            select_target(&targets).map(|t| t.url.as_str()),
            Some("https://example.com/"),
        );
    }

    #[test]
    fn accepts_http_https_file_and_about_schemes() {
        for url in [
            "http://example.com/",
            "https://example.com/",
            "file:///tmp/index.html",
            "about:blank",
        ] {
            let targets = vec![t("page", url, "ws://x/page")];
            assert_eq!(
                select_target(&targets).map(|t| t.url.as_str()),
                Some(url),
                "scheme {url} should be accepted as a web page",
            );
        }
    }

    #[test]
    fn falls_back_to_chrome_internal_when_no_web_page() {
        let targets = vec![
            t(
                "page",
                "devtools://devtools/bundled/devtools_app.html",
                "ws://x/devtools",
            ),
            t("page", "chrome://newtab/", "ws://x/newtab"),
        ];
        assert_eq!(
            select_target(&targets).map(|t| t.url.as_str()),
            Some("chrome://newtab/"),
        );
    }

    #[test]
    fn returns_none_when_only_devtools() {
        let targets = vec![t(
            "page",
            "devtools://devtools/bundled/devtools_app.html",
            "ws://x/devtools",
        )];
        assert!(select_target(&targets).is_none());
    }

    #[test]
    fn ignores_non_page_and_empty_ws() {
        let targets = vec![
            t(
                "service_worker",
                "chrome-extension://abc/sw.js",
                "ws://x/sw",
            ),
            t("iframe", "https://ads.example/", "ws://x/iframe"),
            t("page", "https://real.example/", ""),
            t("page", "https://good.example/", "ws://x/good"),
        ];
        assert_eq!(
            select_target(&targets).map(|t| t.url.as_str()),
            Some("https://good.example/"),
        );
    }

    #[test]
    fn returns_none_on_empty_list() {
        let targets: Vec<ChromeTarget> = vec![];
        assert!(select_target(&targets).is_none());
    }

    #[test]
    fn single_web_page_still_selected() {
        let targets = vec![t("page", "https://example.com/", "ws://x/example")];
        assert_eq!(
            select_target(&targets).map(|t| t.url.as_str()),
            Some("https://example.com/"),
        );
    }

    mod http_tests {
        use super::*;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn chrome_target_json(ty: &str, url: &str, ws: &str) -> String {
            format!(
                r#"{{"title":"{url}","type":"{ty}","url":"{url}","webSocketDebuggerUrl":"{ws}"}}"#
            )
        }

        fn json_list_body(targets: &[(&str, &str, &str)]) -> String {
            let entries: Vec<String> = targets
                .iter()
                .map(|(ty, url, ws)| chrome_target_json(ty, url, ws))
                .collect();
            format!("[{}]", entries.join(","))
        }

        async fn json_list_endpoint(server: &MockServer, body: &str) -> String {
            Mock::given(method("GET"))
                .and(path("/json/list"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(server)
                .await;
            server.address().to_string()
        }

        #[tokio::test]
        async fn get_websocket_url_skips_devtools_first_in_json_list() {
            let server = MockServer::start().await;
            let body = json_list_body(&[
                (
                    "page",
                    "devtools://devtools/bundled/devtools_app.html",
                    "ws://devtools",
                ),
                ("page", "https://m.baidu.com/", "ws://baidu"),
            ]);
            let host = json_list_endpoint(&server, &body).await;

            let ws = get_websocket_url(&host)
                .await
                .expect("should pick a target");

            assert_eq!(ws, "ws://baidu");
        }

        #[tokio::test]
        async fn get_websocket_url_errors_when_only_devtools() {
            let server = MockServer::start().await;
            let body = json_list_body(&[(
                "page",
                "devtools://devtools/bundled/devtools_app.html",
                "ws://devtools",
            )]);
            let host = json_list_endpoint(&server, &body).await;

            let err = get_websocket_url(&host)
                .await
                .expect_err("devtools must not be selected");
            assert!(
                matches!(err, CdpError::NoPageTargetFound(_)),
                "expected NoPageTargetFound, got {err:?}",
            );
        }

        #[tokio::test]
        async fn get_websocket_url_happy_path_unchanged() {
            let server = MockServer::start().await;
            let body = json_list_body(&[("page", "https://example.com/", "ws://example")]);
            let host = json_list_endpoint(&server, &body).await;

            let ws = get_websocket_url(&host)
                .await
                .expect("happy path should still work");
            assert_eq!(ws, "ws://example");
        }

        async fn json_version_endpoint(server: &MockServer, body: &str) -> String {
            Mock::given(method("GET"))
                .and(path("/json/version"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(server)
                .await;
            server.address().to_string()
        }

        #[tokio::test]
        async fn get_browser_websocket_url_returns_browser_endpoint() {
            let server = MockServer::start().await;
            let body = r#"{"Browser":"Chrome/120.0","webSocketDebuggerUrl":"ws://host/devtools/browser/abc"}"#;
            let host = json_version_endpoint(&server, body).await;

            let ws = get_browser_websocket_url(&host)
                .await
                .expect("browser endpoint should be resolved");

            assert_eq!(ws, "ws://host/devtools/browser/abc");
        }

        #[tokio::test]
        async fn get_browser_websocket_url_errors_when_endpoint_missing() {
            let server = MockServer::start().await;
            let body = r#"{"Browser":"Chrome/120.0"}"#;
            let host = json_version_endpoint(&server, body).await;

            let err = get_browser_websocket_url(&host)
                .await
                .expect_err("a missing endpoint must not yield an empty URL");

            assert!(
                matches!(err, CdpError::NoPageTargetFound(_)),
                "expected NoPageTargetFound, got {err:?}",
            );
        }
    }
}
