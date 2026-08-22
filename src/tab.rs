use crate::client::CdpClient;
use crate::error::CdpResult;
use crate::event_filter::EventFilter;
use crate::protocol::WsResponse;
use serde::Serialize;
use serde_json::json;
use std::fmt;

/// A handle to one browser tab, multiplexed over the shared browser-level
/// connection owned by [`crate::browser::BrowserClient`].
///
/// Commands sent through a `Tab` carry its `sessionId`, so they act on that
/// tab only. Cloning a `Tab` is cheap and every clone drives the same tab.
///
/// Dropping a `Tab` neither closes nor detaches it; call [`Tab::close`] or
/// [`Tab::detach`] explicitly.
#[derive(Clone)]
pub struct Tab {
    client: CdpClient,
    target_id: String,
    session_id: String,
}

impl Tab {
    pub(crate) fn new(client: CdpClient, target_id: String, session_id: String) -> Self {
        Self {
            client,
            target_id,
            session_id,
        }
    }

    /// The `Target.TargetID` of this tab, stable for the tab's lifetime.
    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    /// The flat-session id used to route commands and events to this tab.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The underlying browser-level connection, for commands that are not
    /// scoped to a tab.
    pub fn client(&self) -> &CdpClient {
        &self.client
    }

    /// Sends a CDP command to this tab.
    ///
    /// Mirrors [`CdpClient::send_raw_command`], with the `sessionId` filled in.
    pub async fn send_raw_command<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> CdpResult<WsResponse> {
        self.client
            .send_raw_command_to_session(&self.session_id, method, params)
            .await
    }

    /// Streams this tab's events for a single domain, e.g. `"Page"`.
    pub fn on_domain(&self, domain: &'static str) -> EventFilter {
        self.client.on_domain_for_session(domain, &self.session_id)
    }

    /// Streams every event this tab emits, across all domains.
    pub fn events(&self) -> EventFilter {
        self.client.on_domain_for_session("", &self.session_id)
    }

    /// Brings this tab to the foreground.
    pub async fn activate(&self) -> CdpResult<()> {
        self.client
            .send_raw_command(
                "Target.activateTarget",
                json!({ "targetId": self.target_id }),
            )
            .await?;
        Ok(())
    }

    /// Stops receiving this tab's events without closing the tab itself.
    pub async fn detach(self) -> CdpResult<()> {
        self.client
            .send_raw_command(
                "Target.detachFromTarget",
                json!({ "sessionId": self.session_id }),
            )
            .await?;
        Ok(())
    }

    /// Closes the tab in the browser.
    pub async fn close(self) -> CdpResult<()> {
        self.client
            .send_raw_command("Target.closeTarget", json!({ "targetId": self.target_id }))
            .await?;
        Ok(())
    }
}

impl fmt::Debug for Tab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tab")
            .field("target_id", &self.target_id)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}
