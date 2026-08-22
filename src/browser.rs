use crate::client::CdpClient;
use crate::error::{CdpError, CdpResult};
use crate::protocol::{NoParams, WsResponse};
use crate::rest_client::get_browser_websocket_url;
use crate::tab::Tab;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tracing::debug;

/// Description of a CDP target as reported by `Target.getTargets`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub target_id: String,
    pub r#type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub attached: bool,
}

impl TargetInfo {
    /// Whether this target is a tab worth attaching to.
    ///
    /// Chrome reports the DevTools front-end as a `page` target as well, and
    /// attaching to it would drive the DevTools window instead of a real tab.
    pub fn is_tab(&self) -> bool {
        self.r#type == "page" && !self.url.starts_with("devtools://")
    }
}

/// A connection to the browser itself rather than to a single page.
///
/// This is the entry point for driving several tabs at once: one WebSocket
/// carries the traffic of every attached tab, each distinguished by its
/// `sessionId` (CDP "flat" sessions).
///
/// ```no_run
/// use cdp_lite::browser::BrowserClient;
/// use cdp_lite::error::CdpResult;
/// use cdp_lite::protocol::NoParams;
/// use serde_json::json;
/// use std::time::Duration;
///
/// # async fn doc_example() -> CdpResult<()> {
/// let browser = BrowserClient::connect("127.0.0.1:9222", Duration::from_secs(5)).await?;
///
/// let docs = browser.new_tab("https://docs.rs").await?;
/// let crates = browser.new_tab("https://crates.io").await?;
///
/// // Each command only affects its own tab.
/// docs.send_raw_command("Page.enable", NoParams).await?;
/// crates.send_raw_command("Page.reload", json!({})).await?;
/// # Ok(())
/// # }
/// ```
///
/// [`CdpClient::new`] keeps connecting straight to a single page, so existing
/// code is unaffected by this type.
#[derive(Clone)]
pub struct BrowserClient {
    client: CdpClient,
}

impl BrowserClient {
    /// Connects to the browser-level endpoint advertised at
    /// `GET http://{host}/json/version`.
    ///
    /// `default_timeout` applies to every command sent through this
    /// connection, including the ones issued by attached [`Tab`]s.
    pub async fn connect(host: &str, default_timeout: Duration) -> CdpResult<Self> {
        let ws_url = get_browser_websocket_url(host).await?;
        let client = CdpClient::connect(&ws_url, default_timeout).await?;

        Ok(Self { client })
    }

    /// Wraps an existing browser-level connection.
    ///
    /// The client must already be connected to a browser endpoint; a
    /// page-level client built with [`CdpClient::new`] will reject the
    /// `Target` commands this type relies on.
    pub fn from_client(client: CdpClient) -> Self {
        Self { client }
    }

    /// The underlying connection, for browser-level commands this type does
    /// not wrap.
    pub fn client(&self) -> &CdpClient {
        &self.client
    }

    /// Lists every target the browser knows about, tabs included.
    pub async fn list_targets(&self) -> CdpResult<Vec<TargetInfo>> {
        let response = self
            .client
            .send_raw_command("Target.getTargets", NoParams)
            .await?;

        parse_target_infos(&response)
    }

    /// Lists the tabs that can be attached to, skipping service workers,
    /// iframes and the DevTools front-end.
    pub async fn list_tabs(&self) -> CdpResult<Vec<TargetInfo>> {
        Ok(self
            .list_targets()
            .await?
            .into_iter()
            .filter(TargetInfo::is_tab)
            .collect())
    }

    /// Attaches to an existing target and returns a handle to drive it.
    ///
    /// Attaching twice to the same target yields two independent sessions,
    /// both valid.
    pub async fn attach(&self, target_id: &str) -> CdpResult<Tab> {
        let response = self
            .client
            .send_raw_command(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;

        let session_id = string_field(&response, "sessionId", "Target.attachToTarget")?;
        debug!("Attached to target {} as session {}", target_id, session_id);

        Ok(Tab::new(
            self.client.clone(),
            target_id.to_string(),
            session_id,
        ))
    }

    /// Attaches to every tab currently open.
    pub async fn attach_to_all_tabs(&self) -> CdpResult<Vec<Tab>> {
        let infos = self.list_tabs().await?;
        let mut tabs = Vec::with_capacity(infos.len());

        for info in infos {
            tabs.push(self.attach(&info.target_id).await?);
        }

        Ok(tabs)
    }

    /// Opens a new tab on `url` and attaches to it.
    ///
    /// Pass `"about:blank"` to open an empty tab.
    pub async fn new_tab(&self, url: &str) -> CdpResult<Tab> {
        let response = self
            .client
            .send_raw_command("Target.createTarget", json!({ "url": url }))
            .await?;

        let target_id = string_field(&response, "targetId", "Target.createTarget")?;

        self.attach(&target_id).await
    }

    /// Closes a tab by target id, whether or not it is attached.
    pub async fn close_tab(&self, target_id: &str) -> CdpResult<()> {
        self.client
            .send_raw_command("Target.closeTarget", json!({ "targetId": target_id }))
            .await?;

        Ok(())
    }

    /// Turns `Target.targetCreated` / `targetDestroyed` events on or off.
    ///
    /// Off by default because it makes the browser emit an event for every
    /// target it opens. Enable it to observe tabs the page itself opens, then
    /// read them from `client().on_domain("Target")`.
    pub async fn set_discover_targets(&self, discover: bool) -> CdpResult<()> {
        self.client
            .send_raw_command("Target.setDiscoverTargets", json!({ "discover": discover }))
            .await?;

        Ok(())
    }
}

fn string_field(response: &WsResponse, field: &str, method: &str) -> CdpResult<String> {
    response
        .result
        .as_ref()
        .and_then(|result| result.get(field))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CdpError::InternalError(format!("{method} returned no `{field}`")))
}

fn parse_target_infos(response: &WsResponse) -> CdpResult<Vec<TargetInfo>> {
    let infos = response
        .result
        .as_ref()
        .and_then(|result| result.get("targetInfos"))
        .ok_or_else(|| {
            CdpError::InternalError("Target.getTargets returned no `targetInfos`".to_string())
        })?;

    Ok(serde_json::from_value(infos.clone())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with_result(result: Value) -> WsResponse {
        WsResponse {
            id: Some(1),
            result: Some(result),
            ..Default::default()
        }
    }

    fn target(target_id: &str, ty: &str, url: &str) -> Value {
        json!({
            "targetId": target_id,
            "type": ty,
            "title": url,
            "url": url,
            "attached": false,
        })
    }

    #[test]
    fn extracts_session_id_from_attach_response() {
        let response = response_with_result(json!({ "sessionId": "SESSION-42" }));

        let session_id = string_field(&response, "sessionId", "Target.attachToTarget")
            .expect("sessionId should be extracted");

        assert_eq!(session_id, "SESSION-42");
    }

    #[test]
    fn missing_session_id_is_reported_with_the_method_name() {
        let response = response_with_result(json!({}));

        let err = string_field(&response, "sessionId", "Target.attachToTarget")
            .expect_err("a response without sessionId must not be accepted");

        let message = err.to_string();
        assert!(
            message.contains("Target.attachToTarget") && message.contains("sessionId"),
            "error should name the method and field, got: {message}",
        );
    }

    #[test]
    fn parses_target_infos() {
        let response = response_with_result(json!({
            "targetInfos": [
                target("T1", "page", "https://example.com/"),
                target("T2", "service_worker", "https://example.com/sw.js"),
            ],
        }));

        let infos = parse_target_infos(&response).expect("targetInfos should parse");

        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].target_id, "T1");
        assert_eq!(infos[0].url, "https://example.com/");
    }

    #[test]
    fn parse_target_infos_rejects_a_response_without_the_field() {
        let response = response_with_result(json!({}));

        assert!(parse_target_infos(&response).is_err());
    }

    #[test]
    fn only_real_pages_count_as_tabs() {
        let response = response_with_result(json!({
            "targetInfos": [
                target("T1", "service_worker", "https://example.com/sw.js"),
                target("T2", "page", "devtools://devtools/bundled/devtools_app.html"),
                target("T3", "page", "https://example.com/"),
                target("T4", "iframe", "https://ads.example/"),
                target("T5", "page", "about:blank"),
            ],
        }));

        let tabs: Vec<String> = parse_target_infos(&response)
            .expect("targetInfos should parse")
            .into_iter()
            .filter(TargetInfo::is_tab)
            .map(|info| info.target_id)
            .collect();

        assert_eq!(
            tabs,
            vec!["T3", "T5"],
            "service workers, iframes and the DevTools front-end are not tabs",
        );
    }

    #[test]
    fn target_info_tolerates_missing_optional_fields() {
        let response = response_with_result(json!({
            "targetInfos": [{ "targetId": "T1", "type": "page" }],
        }));

        let infos = parse_target_infos(&response).expect("targetInfos should parse");

        assert_eq!(infos[0].url, "");
        assert!(infos[0].is_tab());
    }
}
