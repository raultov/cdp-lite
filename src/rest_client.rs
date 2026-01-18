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

pub async fn get_websocket_url(host: &str) -> CdpResult<String> {
    let url = format!("http://{}/json/list", host);

    let targets: Vec<ChromeTarget> = reqwest::get(&url).await?.json().await?;

    let target = targets
        .into_iter()
        .find(|t| t.r#type == "page" && !t.web_socket_debugger_url.is_empty())
        .ok_or(CdpError::NoPageTargetFound(host.to_string()))?;

    debug!("Found target: {} - {}", target.title, target.url);

    Ok(target.web_socket_debugger_url)
}
