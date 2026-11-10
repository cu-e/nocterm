//! Last native stream boundary: check original source on every socket/relay poll.
use futures::{
    FutureExt as _,
    future::{BoxFuture, Shared},
};
use nocterm_session::{ConnectionSource, SessionDriver};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
pub(crate) struct SourceStream<S> {
    stream: S,
    source: ConnectionSource,
    closed: Shared<BoxFuture<'static, ()>>,
}
impl<S> SourceStream<S> {
    pub(crate) fn new(stream: S, driver: SessionDriver) -> Self {
        let source = driver.source().clone();
        let closed = async move { driver.io_closed().await }.boxed().shared();
        Self {
            stream,
            source,
            closed,
        }
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for SourceStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.closed.poll_unpin(cx).is_ready() {
            return Poll::Ready(Err(std::io::ErrorKind::ConnectionAborted.into()));
        }
        if let Err(error) = this.source.check() {
            return Poll::Ready(Err(std::io::Error::other(error)));
        }
        let result = Pin::new(&mut this.stream).poll_read(cx, buffer);
        if this.closed.poll_unpin(cx).is_ready() {
            return Poll::Ready(Err(std::io::ErrorKind::ConnectionAborted.into()));
        }
        if let Err(error) = this.source.check() {
            return Poll::Ready(Err(std::io::Error::other(error)));
        }
        result
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for SourceStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.closed.poll_unpin(cx).is_ready() {
            return Poll::Ready(Err(std::io::ErrorKind::ConnectionAborted.into()));
        }
        if let Err(error) = this.source.check() {
            return Poll::Ready(Err(std::io::Error::other(error)));
        }
        Pin::new(&mut this.stream).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.closed.poll_unpin(cx).is_ready() {
            return Poll::Ready(Err(std::io::ErrorKind::ConnectionAborted.into()));
        }
        if let Err(error) = this.source.check() {
            return Poll::Ready(Err(std::io::Error::other(error)));
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

/// A producer owns closure even when a handshake/auth/serve future is dropped.
/// Its native stream retains the receiver, never this cleanup owner.
pub(crate) struct CloseOnDrop(SessionDriver);
impl CloseOnDrop {
    pub(crate) fn new(driver: SessionDriver) -> Self {
        Self(driver)
    }
}
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt as _;
    #[tokio::test]
    async fn producer_drop_wakes_a_separately_spawned_native_handshake_read() {
        let (session, driver) = nocterm_session::channel(None);
        let cleanup = CloseOnDrop::new(driver.clone());
        let (stream, _peer) = tokio::io::duplex(128);
        let mut stream = SourceStream::new(stream, driver);
        let (started, waiting) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let _ = started.send(());
            stream.read_u8().await
        });
        waiting.await.unwrap();
        drop(cleanup);
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        assert!(!session.input("closed producer"));
    }
}
