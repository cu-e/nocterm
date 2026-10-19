//! Original raw bytes are prepaid before native line growth or UTF-8 publication.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{ManagedError, Source};
use futures::{
    AsyncReadExt,
    io::{AsyncRead, AsyncWrite},
};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use std::{fmt, io};

#[derive(Debug, thiserror::Error)]
enum Cause {
    #[error("native protocol IO failed: {0}")]
    Io(#[source] io::Error),
    #[error(transparent)]
    Allocation(#[from] ManagedError<JsonDecodeError>),
    #[error("protocol line exceeds its limit")]
    Limit,
    #[error("protocol line missing newline")]
    MissingNewline,
    #[error("invalid outgoing protocol line")]
    Outgoing,
}
pub(super) struct TransportError {
    cause: Cause,
    _source: SourceBudget,
}
impl TransportError {
    pub(super) fn admission(
        &self,
    ) -> Option<nocterm_ai::model::json::deferred::NativeAllocationError> {
        match &self.cause {
            Cause::Allocation(error) => super::native_admission::sdk(error),
            _ => None,
        }
    }
}
impl fmt::Debug for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.cause, f)
    }
}
impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}
impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
/// This inline reader performs no allocation at construction. Its containing
/// native task/owner must prepay the concrete layout before boxing or sharing it.
pub(super) struct Reader<R> {
    input: R,
    scratch: [u8; 4096],
    available: std::ops::Range<usize>,
    budget: SourceBudget,
    pending: agent_client_protocol::managed::Buffer<u8, SourceBudget>,
}
impl<R: AsyncRead + Unpin> Reader<R> {
    pub(super) fn new(input: R, budget: SourceBudget) -> Self {
        Self {
            input,
            scratch: [0; 4096],
            available: 0..0,
            pending: Source::buffer(budget.clone()),
            budget,
        }
    }
    fn error(&self, cause: Cause) -> TransportError {
        TransportError {
            cause,
            _source: self.budget.clone(),
        }
    }
    pub(super) async fn line(&mut self) -> Result<Option<Source<SourceBudget>>, TransportError> {
        loop {
            if self.available.is_empty() {
                let count = self
                    .input
                    .read(&mut self.scratch)
                    .await
                    .map_err(|error| self.error(Cause::Io(error)))?;
                if count == 0 {
                    return if self.pending.is_empty() {
                        Ok(None)
                    } else {
                        Err(self.error(Cause::MissingNewline))
                    };
                }
                self.available = 0..count;
            }
            let pending = &self.scratch[self.available.clone()];
            let newline = pending.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(pending.len(), |end| end + 1);
            if self.pending.len().saturating_add(count) > crate::lines::ACP_LINE_LIMIT + 1 {
                return Err(self.error(Cause::Limit));
            }
            self.pending
                .extend_bytes(&pending[..count])
                .map_err(|error| self.error(Cause::Allocation(error)))?;
            self.available.start += count;
            if newline.is_some() {
                self.pending.pop();
                if self.pending.as_slice().last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let bytes =
                    std::mem::replace(&mut self.pending, Source::buffer(self.budget.clone()));
                return Source::from_buffer(bytes)
                    .map(Some)
                    .map_err(|error| self.error(Cause::Allocation(error)));
            }
        }
    }
}
/// The exact paid source and write offset outlive a cancelled borrowing future.
pub(super) struct Writer<W> {
    output: W,
    budget: SourceBudget,
    pending: Option<Source<SourceBudget>>,
    written: usize,
    newline: bool,
}
impl<W: AsyncWrite + Unpin> Writer<W> {
    pub(super) fn new(output: W, budget: SourceBudget) -> Self {
        Self {
            output,
            budget,
            pending: None,
            written: 0,
            newline: false,
        }
    }
    fn error(&self, cause: Cause) -> TransportError {
        TransportError {
            cause,
            _source: self.budget.clone(),
        }
    }
    pub(super) fn begin(&mut self, source: Source<SourceBudget>) -> Result<(), TransportError> {
        if self.pending.is_some() {
            return Err(self.error(Cause::Outgoing));
        }
        if !source.belongs_to(&self.budget) {
            return Err(self.error(Cause::Allocation(ManagedError::InvalidEnvelope)));
        }
        if source.text().len() > crate::lines::ACP_LINE_LIMIT || source.text().contains('\n') {
            return Err(self.error(Cause::Outgoing));
        }
        self.pending = Some(source);
        self.written = 0;
        self.newline = false;
        Ok(())
    }
    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn poll_progress(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), TransportError>> {
        use std::{pin::Pin, task::Poll};
        let Some(source) = &self.pending else {
            return Poll::Ready(Ok(()));
        };
        while self.written < source.text().len() {
            let count = match Pin::new(&mut self.output)
                .poll_write(cx, &source.text().as_bytes()[self.written..])
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(self.error(Cause::Io(error)))),
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(
                        Err(self.error(Cause::Io(io::ErrorKind::WriteZero.into()))),
                    );
                }
                Poll::Ready(Ok(count)) => count,
            };
            self.written += count;
        }
        if !self.newline {
            match Pin::new(&mut self.output).poll_write(cx, b"\n") {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(self.error(Cause::Io(error)))),
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(
                        Err(self.error(Cause::Io(io::ErrorKind::WriteZero.into()))),
                    );
                }
                Poll::Ready(Ok(_)) => self.newline = true,
            }
        }
        match Pin::new(&mut self.output).poll_flush(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(self.error(Cause::Io(error)))),
            Poll::Ready(Ok(())) => {
                self.pending = None;
                Poll::Ready(Ok(()))
            }
        }
    }
    #[cfg(test)]
    pub(super) async fn line(
        &mut self,
        source: &Source<SourceBudget>,
    ) -> Result<(), TransportError> {
        if let Some(pending) = &self.pending {
            if !source.belongs_to(&self.budget) || source.text() != pending.text() {
                return Err(self.error(Cause::Outgoing));
            }
        } else {
            self.begin(source.clone())?;
        }
        futures::future::poll_fn(|cx| self.poll_progress(cx)).await
    }
}
