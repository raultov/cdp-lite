use std::time::Duration;
use thiserror::Error;

/// Everything that can go wrong while talking to Chrome.
///
/// Marked `#[non_exhaustive]` so adding variants stays non-breaking: match on
/// the variants you care about and keep a wildcard arm for the rest.
#[non_exhaustive]
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

    /// The event channel overran this subscriber's buffer and `skipped`
    /// events were dropped before it could read them.
    ///
    /// The subscriber keeps working — it simply never saw those events. A
    /// consumer that caches event-derived state should treat this as a signal
    /// to resynchronise that state from the source rather than trust the cache.
    #[error("Event stream lagged: {skipped} events were dropped")]
    Lagged { skipped: u64 },
}

pub type CdpResult<T> = Result<T, CdpError>;
