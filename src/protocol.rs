use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Clone)]
pub struct WsResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub method: Option<String>,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct WsCommand<P: Serialize> {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<P>,
}

#[derive(Debug, Serialize)]
pub struct NoParams;
