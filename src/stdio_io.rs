//! Opt-in stdio progress tracing; payloads never enter these events.
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::AsyncWrite;

pub(super) struct ObservedWriter<W>(pub(super) W);

impl<W: AsyncWrite + Unpin> AsyncWrite for ObservedWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        tracing::trace!(target: "codesight_mcp::stdio_io", bytes = bytes.len(), "poll_write entered");
        let result = Pin::new(&mut self.get_mut().0).poll_write(cx, bytes);
        tracing::trace!(target: "codesight_mcp::stdio_io", ?result, "poll_write returned");
        result
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        tracing::trace!(target: "codesight_mcp::stdio_io", "poll_flush entered");
        let result = Pin::new(&mut self.get_mut().0).poll_flush(cx);
        tracing::trace!(target: "codesight_mcp::stdio_io", ?result, "poll_flush returned");
        result
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}
