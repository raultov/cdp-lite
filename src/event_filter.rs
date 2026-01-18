use crate::error::CdpResult;
use crate::protocol::WsResponse;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_stream::{Stream, wrappers::BroadcastStream};

pub struct EventFilter {
    inner: BroadcastStream<WsResponse>,
    prefix: String,
}

impl EventFilter {
    pub fn new(
        receiver: tokio::sync::broadcast::Receiver<WsResponse>,
        domain: &'static str,
    ) -> Self {
        Self {
            inner: BroadcastStream::new(receiver),
            prefix: format!("{}.", domain),
        }
    }
}

impl Stream for EventFilter {
    type Item = CdpResult<WsResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(response))) => {
                    if let Some(ref method) = response.method
                        && method.starts_with(&self.prefix)
                    {
                        return Poll::Ready(Some(Ok(response)));
                    }
                    // Not the domain we want, loop again to poll next
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
