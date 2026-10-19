//! One original SDK router over injected byte IO and persistent write progress.
use super::{
    budget::SourceBudget,
    error::DecodeError,
    outgoing,
    transport::{Reader, TransportError, Writer},
    wire,
};
use agent_client_protocol::managed::{Dispatch, Driver, ManagedError, Reception};
use futures::io::{AsyncRead, AsyncWrite};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
#[derive(Debug, thiserror::Error)]
pub(crate) enum DriverError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Decode(#[from] DecodeError<JsonDecodeError>),
    #[error(transparent)]
    Routing(#[from] ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Notice(nocterm_ai::history::source::SourceSemanticRefusal),
    #[error(transparent)]
    Permission(
        nocterm_ai::history::source::SourceSemanticRefusal<
            nocterm_ai::model::auth::ProviderPermission,
        >,
    ),
}
impl DriverError {
    pub(super) fn admission(
        &self,
    ) -> Option<nocterm_ai::model::json::deferred::NativeAllocationError> {
        match self {
            Self::Routing(error) => super::native_admission::sdk(error),
            Self::Decode(error) => super::native_admission::decode(error),
            Self::Transport(error) => error.admission(),
            Self::Notice(error) => super::native_admission::semantic(error),
            Self::Permission(error) => super::native_admission::semantic(error),
        }
    }
    pub(super) fn source_failure(&self) -> bool {
        matches!(
            self,
            Self::Notice(_) | Self::Permission(_) | Self::Routing(_)
        ) || matches!(self,Self::Decode(error) if !super::native_admission::processing(error))
            || self.admission().is_some()
    }
}
/// The handler owns paid notification/cache/permission work. Its polls share
/// this driver's task, so no unaccounted executor task is created per request.
pub(super) trait Handler: Unpin {
    fn dispatch(
        &mut self,
        dispatch: Dispatch<SourceBudget>,
        driver: &Driver<SourceBudget>,
    ) -> Result<(), DriverError>;
    fn poll_work(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), DriverError>>;
    fn closed(&mut self);
}
pub(super) struct DriverTask<R, W, H: Handler> {
    driver: Driver<SourceBudget>,
    reader: Reader<R>,
    writer: Writer<W>,
    reception: Option<Reception<SourceBudget>>,
    handler: H,
    budget: SourceBudget,
    closed: bool,
    read_closed: bool,
}
impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, H: Handler> DriverTask<R, W, H> {
    pub(super) fn new(
        driver: Driver<SourceBudget>,
        input: R,
        output: W,
        handler: H,
        budget: SourceBudget,
    ) -> Self {
        Self {
            driver,
            reader: Reader::new(input, budget.clone()),
            writer: Writer::new(output, budget.clone()),
            reception: None,
            handler,
            budget,
            closed: false,
            read_closed: false,
        }
    }
    fn poll_step(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), DriverError>> {
        if let Poll::Ready(Err(error)) = self.handler.poll_work(cx) {
            return Poll::Ready(Err(error));
        }
        if self.writer.is_pending() {
            match self.writer.poll_progress(cx) {
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error.into())),
                Poll::Pending => {}
                Poll::Ready(Ok(())) => cx.waker().wake_by_ref(),
            }
        }
        if !self.writer.is_pending() {
            match self.driver.poll_outgoing(cx) {
                Poll::Ready(Some(output)) => {
                    let source = outgoing::encode(&self.budget, &output)?;
                    self.writer.begin(source)?;
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => {}
            }
        }
        if self.read_closed && !self.writer.is_pending() {
            return Poll::Ready(Ok(()));
        }
        if let Some(reception) = &mut self.reception {
            match reception.poll_dispatch(&mut self.driver, cx) {
                Poll::Ready(Ok(Some(dispatch))) => {
                    self.handler.dispatch(dispatch, &self.driver)?;
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(Ok(None)) => {
                    self.reception = None;
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error.into())),
                Poll::Pending => {}
            }
        } else {
            let line = self.reader.line();
            futures::pin_mut!(line);
            match line.poll(cx) {
                Poll::Ready(Ok(Some(source))) => {
                    let frame = wire::frame(&source, self.budget.clone())?;
                    self.reception = Some(self.driver.receive(frame)?);
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(Ok(None)) => {
                    self.read_closed = true;
                    cx.waker().wake_by_ref();
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error.into())),
                Poll::Pending => {}
            }
        }
        Poll::Pending
    }
}
impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, H: Handler> Future for DriverTask<R, W, H> {
    type Output = Result<(), DriverError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Pending;
        }
        let result = this.poll_step(cx);
        if result.is_ready() {
            this.driver.close();
            this.handler.closed();
            this.closed = true;
        }
        result
    }
}
impl<R, W, H: Handler> Drop for DriverTask<R, W, H> {
    fn drop(&mut self) {
        self.driver.close();
        if !self.closed {
            self.handler.closed();
        }
    }
}
