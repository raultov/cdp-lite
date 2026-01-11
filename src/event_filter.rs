use std::pin::Pin;
use std::task::{Context, Poll};
use crate::protocol::WsResponse;
use tokio_stream::{Stream, wrappers::BroadcastStream};
use crate::error::CdpResult;

pub struct EventFilter {
    inner: BroadcastStream<WsResponse>,
    prefix: String,
}

impl EventFilter {
    pub fn new(receiver: tokio::sync::broadcast::Receiver<WsResponse>, domain: &'static str) -> Self {
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
                    if let Some(ref method) = response.method {
                        if method.starts_with(&self.prefix) {
                            return Poll::Ready(Some(Ok(response)));
                        }
                    }
                    // Not the domain we want, loop again to poll next
                    continue;
                }
                Poll::Ready(Some(Err(_))) => {
                    // This 'Err' is specifically a BroadcastStreamRecvError::Lagged
                    // We skip it to continue receiving the next fresh message
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(None), // Channel closed
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
