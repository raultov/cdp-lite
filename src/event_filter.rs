use crate::error::CdpResult;
use crate::protocol::WsResponse;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_stream::{Stream, wrappers::BroadcastStream};

pub struct EventFilter {
    inner: BroadcastStream<WsResponse>,
    prefix: String,
    session_id: Option<String>,
}

impl EventFilter {
    pub fn new(
        receiver: tokio::sync::broadcast::Receiver<WsResponse>,
        domain: &'static str,
    ) -> Self {
        Self {
            inner: BroadcastStream::new(receiver),
            prefix: domain_prefix(domain),
            session_id: None,
        }
    }

    /// Like [`EventFilter::new`], but only yields events belonging to one
    /// attached tab.
    ///
    /// An empty `domain` matches every domain, which is how
    /// [`crate::tab::Tab::events`] streams everything a single tab emits.
    pub fn for_session(
        receiver: tokio::sync::broadcast::Receiver<WsResponse>,
        domain: &'static str,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            inner: BroadcastStream::new(receiver),
            prefix: domain_prefix(domain),
            session_id: Some(session_id.into()),
        }
    }

    fn matches(&self, response: &WsResponse) -> bool {
        let Some(method) = response.method.as_deref() else {
            return false;
        };

        if !method.starts_with(&self.prefix) {
            return false;
        }

        match (self.session_id.as_deref(), response.session_id.as_deref()) {
            // Not scoped to a tab: keep the pre-existing behaviour of
            // forwarding every event on the connection.
            (None, _) => true,
            (Some(wanted), Some(actual)) => wanted == actual,
            // Browser-level events belong to no tab.
            (Some(_), None) => false,
        }
    }
}

fn domain_prefix(domain: &str) -> String {
    if domain.is_empty() {
        String::new()
    } else {
        format!("{}.", domain)
    }
}

impl Stream for EventFilter {
    type Item = CdpResult<WsResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(response))) => {
                    if self.matches(&response) {
                        return Poll::Ready(Some(Ok(response)));
                    }
                    // Not what this subscriber wants, loop again to poll next
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    // This 'Err' is specifically a BroadcastStreamRecvError::Lagged
                    return Poll::Ready(Some(Err(crate::error::CdpError::InternalError(format!(
                        "Event stream lagged: {}",
                        e
                    )))));
                }
                Poll::Ready(None) => return Poll::Ready(None), // Channel closed
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;
    use tokio_stream::StreamExt;

    fn event(method: &str, session_id: Option<&str>) -> WsResponse {
        WsResponse {
            method: Some(method.to_string()),
            session_id: session_id.map(str::to_string),
            ..Default::default()
        }
    }

    async fn collect_methods(mut filter: EventFilter, expected: usize) -> Vec<String> {
        let mut methods = Vec::new();
        while methods.len() < expected {
            let received = filter
                .next()
                .await
                .expect("stream closed early")
                .expect("unexpected filter error");
            methods.push(received.method.expect("events always carry a method"));
        }
        methods
    }

    #[tokio::test]
    async fn domain_filter_ignores_session_id() {
        let (tx, _) = broadcast::channel(16);
        let filter = EventFilter::new(tx.subscribe(), "Page");

        let _ = tx.send(event("Network.requestWillBeSent", Some("A")));
        let _ = tx.send(event("Page.loadEventFired", Some("A")));
        let _ = tx.send(event("Page.frameNavigated", None));

        assert_eq!(
            collect_methods(filter, 2).await,
            vec!["Page.loadEventFired", "Page.frameNavigated"],
            "an unscoped filter must keep forwarding every tab's events",
        );
    }

    #[tokio::test]
    async fn session_filter_drops_other_tabs() {
        let (tx, _) = broadcast::channel(16);
        let filter = EventFilter::for_session(tx.subscribe(), "Page", "A");

        let _ = tx.send(event("Page.loadEventFired", Some("B")));
        let _ = tx.send(event("Page.loadEventFired", Some("A")));

        let received = collect_methods(filter, 1).await;
        assert_eq!(received, vec!["Page.loadEventFired"]);
    }

    #[tokio::test]
    async fn session_filter_drops_browser_level_events() {
        let (tx, _) = broadcast::channel(16);
        let filter = EventFilter::for_session(tx.subscribe(), "Target", "A");

        let _ = tx.send(event("Target.targetCreated", None));
        let _ = tx.send(event("Target.targetCreated", Some("A")));

        // The browser-level event was sent first: if it leaked through, it
        // would be the one returned here.
        let mut filter = filter;
        let first = filter.next().await.unwrap().unwrap();
        assert_eq!(first.session_id.as_deref(), Some("A"));
    }

    #[tokio::test]
    async fn empty_domain_matches_every_domain_of_one_tab() {
        let (tx, _) = broadcast::channel(16);
        let filter = EventFilter::for_session(tx.subscribe(), "", "A");

        let _ = tx.send(event("Page.loadEventFired", Some("B")));
        let _ = tx.send(event("Page.loadEventFired", Some("A")));
        let _ = tx.send(event("Runtime.consoleAPICalled", Some("A")));

        assert_eq!(
            collect_methods(filter, 2).await,
            vec!["Page.loadEventFired", "Runtime.consoleAPICalled"],
        );
    }
}
