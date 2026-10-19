//! The original fixture channel feeds source-paid copies without a second queue.
//! Fixture channels and their preexisting input DTOs are explicit test boundaries;
//! this adapter does not describe their internals as the native allocator proof.
use crate::{
    AgentEvent,
    connection::{
        ManagedAgentEvent, ManagedEventAcknowledgement, ManagedEventBarrier, ManagedEventReceiver,
        ManagedEventSource,
    },
    history::source::{
        ManagedCommandError, NativeAllocation, SourceContext, SourceLifetime, SourceSemanticRefusal,
    },
    model::{auth::ProviderPermission, json::deferred::JsonDecodeError, native::NativeCopies},
};
use futures::Stream;
use std::{
    alloc::Layout,
    pin::Pin,
    task::{Context, Poll},
};
struct OriginalReceiver {
    original: Pin<Box<async_channel::Receiver<AgentEvent>>>,
    _allocation: NativeAllocation,
}
struct Events {
    original: OriginalReceiver,
    context: SourceContext,
    ended: bool,
}
pub fn managed_events(
    context: &SourceContext,
    lifetime: SourceLifetime,
    original: async_channel::Receiver<AgentEvent>,
) -> Result<ManagedEventReceiver, JsonDecodeError> {
    let allocation = context.reserve_native(Layout::new::<async_channel::Receiver<AgentEvent>>())?;
    let original = OriginalReceiver {
        original: Box::pin(original),
        _allocation: allocation,
    };
    ManagedEventReceiver::with_lifetime(
        context,
        lifetime,
        Events {
            original,
            context: context.clone(),
            ended: false,
        },
    )
}
#[derive(Debug, thiserror::Error)]
enum CopyError {
    #[error(transparent)]
    Json(#[from] JsonDecodeError),
    #[error(transparent)]
    Notice(#[from] SourceSemanticRefusal),
    #[error(transparent)]
    Permission(#[from] SourceSemanticRefusal<ProviderPermission>),
}
/// Copies one fixture event into paid native owners under the connection's original Source.
/// An error is the exact failure an adapter would publish; the caller decides whether to end.
pub fn managed_event(
    context: &SourceContext,
    event: AgentEvent,
) -> Result<ManagedAgentEvent, ManagedCommandError> {
    let ticket = context
        .prepare_error::<CopyError>(0)
        .map_err(ManagedCommandError::Admission)?;
    copy(context, event).map_err(|error| ManagedCommandError::Native(ticket.capture(error)))
}
#[expect(
    clippy::result_large_err,
    reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
)]
fn copy(context: &SourceContext, event: AgentEvent) -> Result<ManagedAgentEvent, CopyError> {
    let c = NativeCopies::new(context.ledger());
    Ok(match event {
        AgentEvent::Session(value) => {
            ManagedAgentEvent::Session(context.certify_notice(c.notice(&value)?)?)
        }
        AgentEvent::Permission { request, respond } => ManagedAgentEvent::Permission {
            request: context.certify_permission(c.permission(&request)?)?,
            respond,
        },
        AgentEvent::Exited { code, stderr_tail } => ManagedAgentEvent::Exited {
            code,
            stderr_tail: c.text(&stderr_tail)?,
        },
        AgentEvent::Barrier(original) => ManagedAgentEvent::Barrier(ManagedEventBarrier::new(
            context,
            Acknowledgement(Some(original)),
        )?),
    })
}
impl ManagedEventSource for Events {
    fn poll_event(&mut self, cx: &mut Context<'_>) -> Poll<Option<ManagedAgentEvent>> {
        if self.ended {
            return Poll::Ready(None);
        }
        let event = match self.original.original.as_mut().poll_next(cx) {
            Poll::Ready(Some(event)) => event,
            Poll::Ready(None) => {
                self.ended = true;
                return Poll::Ready(None);
            }
            Poll::Pending => return Poll::Pending,
        };
        Poll::Ready(Some(match managed_event(&self.context, event) {
            Ok(event) => event,
            Err(error) => {
                self.ended = true;
                self.original.original.close();
                ManagedAgentEvent::Failed(error)
            }
        }))
    }
}
struct Acknowledgement(Option<futures::channel::oneshot::Sender<()>>);
impl ManagedEventAcknowledgement for Acknowledgement {
    fn acknowledge(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
