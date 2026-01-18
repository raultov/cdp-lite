use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CdpError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("WebSocket error: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Chrome target not found")]
    NotFound,

    #[error("Command {method} timed out after {timeout:?}")]
    Timeout { method: String, timeout: Duration },

    #[error("No suitable Chrome target found at host: {0}")]
    NoPageTargetFound(String),

    #[error("Internal communication error: {0}")]
    InternalError(String),

    #[error("Chrome returned an error (code {code}): {message}")]
    ProtocolError { code: i64, message: String },

    #[error("Connection lost")]
    Disconnected,
}

pub type CdpResult<T> = Result<T, CdpError>;
