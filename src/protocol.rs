use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WsResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub method: Option<String>,
    pub params: Option<Value>,
    /// Flat-session identifier telling which tab this message belongs to.
    ///
    /// Chrome only sets it once the connection has attached to a target with
    /// `flatten: true`. It stays `None` for browser-level traffic and for
    /// connections opened straight against a single page endpoint, which is
    /// what [`crate::client::CdpClient::new`] does.
    #[serde(rename = "sessionId", default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct WsCommand<P: Serialize> {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
    /// Routes the command to a single tab. Skipped entirely when `None`, so
    /// commands sent over a page-level connection keep the exact same wire
    /// format they had before flat sessions were supported.
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NoParams;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_without_session_keeps_legacy_wire_format() {
        let cmd = WsCommand {
            id: 7,
            method: "Page.navigate".to_string(),
            params: Some(json!({ "url": "about:blank" })),
            session_id: None,
        };

        let encoded: Value = serde_json::to_value(&cmd).expect("command should serialize");

        assert_eq!(
            encoded,
            json!({
                "id": 7,
                "method": "Page.navigate",
                "params": { "url": "about:blank" },
            }),
            "a session-less command must not gain any new field",
        );
    }

    #[test]
    fn command_with_session_carries_session_id() {
        let cmd = WsCommand {
            id: 8,
            method: "Page.navigate".to_string(),
            params: Some(json!({ "url": "about:blank" })),
            session_id: Some("SESSION-42".to_string()),
        };

        let encoded: Value = serde_json::to_value(&cmd).expect("command should serialize");

        assert_eq!(encoded["sessionId"], "SESSION-42");
    }

    #[test]
    fn command_without_params_still_omits_params() {
        let cmd = WsCommand::<NoParams> {
            id: 1,
            method: "Target.getTargets".to_string(),
            params: None,
            session_id: None,
        };

        let encoded: Value = serde_json::to_value(&cmd).expect("command should serialize");

        assert_eq!(encoded, json!({ "id": 1, "method": "Target.getTargets" }));
    }

    #[test]
    fn response_parses_session_id_when_present() {
        let raw = r#"{"method":"Page.loadEventFired","params":{},"sessionId":"SESSION-42"}"#;

        let response: WsResponse = serde_json::from_str(raw).expect("event should deserialize");

        assert_eq!(response.session_id.as_deref(), Some("SESSION-42"));
        assert_eq!(response.method.as_deref(), Some("Page.loadEventFired"));
    }

    #[test]
    fn response_without_session_id_still_parses() {
        let raw = r#"{"id":3,"result":{"frameId":"F1"}}"#;

        let response: WsResponse = serde_json::from_str(raw).expect("response should deserialize");

        assert_eq!(response.id, Some(3));
        assert!(
            response.session_id.is_none(),
            "page-level responses carry no sessionId",
        );
    }
}
