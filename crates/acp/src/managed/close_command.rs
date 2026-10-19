//! Close removes legacy models on every outcome and fences the real event FIFO.
use super::{
    channel::Sender,
    events, outgoing, results,
    rpc::{Cause, Rpc},
};
use agent_client_protocol::managed::Source;
use nocterm_ai::{
    connection::{CloseSessionOutcome, ManagedAgentEvent},
    history::source::{ManagedCommandError, ManagedSourceFuture},
    model::{ids::SessionId, json::deferred::NativeAllocationError},
};
use std::{error::Error, time::Duration};
pub(super) struct Close {
    rpc: Rpc,
    events: Sender<ManagedAgentEvent>,
}
impl Close {
    pub(super) fn new(rpc: Rpc, events: Sender<ManagedAgentEvent>) -> Self {
        Self { rpc, events }
    }
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn execute(
        self,
        session: &SessionId,
    ) -> Result<
        ManagedSourceFuture<Result<CloseSessionOutcome, ManagedCommandError>>,
        NativeAllocationError,
    > {
        let lifetime = self.rpc.state().lifetime()?;
        let ticket = self.rpc.ticket()?;
        let key = Source::copy(session.as_str(), self.rpc.budget().clone());
        let (_, _, supported) = self.rpc.state().capabilities();
        let request = if supported {
            let envelope = self.rpc.id().map_err(Cause::from).and_then(|id| {
                Ok(outgoing::session(
                    self.rpc.budget(),
                    Some(&id),
                    "session/close",
                    session.as_str(),
                )?)
            });
            Some(self.rpc.operation_with_timeout(
                envelope,
                false,
                Some(Duration::from_secs(5)),
                |value, _| Ok(results::empty(value)?),
            )?)
        } else {
            None
        };
        ManagedSourceFuture::new(lifetime, async move {
            let key = key.map_err(|error| super::rpc::failure(Cause::Routing(error), ticket))?;
            let result = match request {
                None => Ok(CloseSessionOutcome::Unsupported),
                Some(request) => match request.await {
                    Ok(()) => Ok(CloseSessionOutcome::Closed),
                    Err(ManagedCommandError::Native(error))
                        if error
                            .source()
                            .and_then(|cause| cause.downcast_ref::<Cause>())
                            .is_some_and(|cause| {
                                matches!(cause, Cause::Timeout("Agent operation timed out"))
                            }) =>
                    {
                        Ok(CloseSessionOutcome::TimedOut)
                    }
                    Err(error) => Err(error),
                },
            };
            self.rpc
                .models()
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(key.text());
            if matches!(result, Ok(CloseSessionOutcome::Closed)) {
                return self.fence().await;
            }
            result
        })
    }
    async fn fence(self) -> Result<CloseSessionOutcome, ManagedCommandError> {
        let ticket = self.rpc.ticket().map_err(ManagedCommandError::Admission)?;
        let (barrier, mut receiver) = events::fence(self.rpc.budget())
            .map_err(|error| super::rpc::failure(Cause::Decode(error), ticket))?;
        let send_ticket = self.rpc.ticket().map_err(ManagedCommandError::Admission)?;
        let work = async {
            match self.events.send(ManagedAgentEvent::Barrier(barrier)).await {
                Ok(()) => Ok(futures::future::poll_fn(|cx| receiver.poll_recv(cx)).await),
                Err(super::channel::SendError::Closed(_)) => Ok(None),
                Err(super::channel::SendError::Allocation(error, _)) => {
                    Err(super::rpc::failure(Cause::Routing(error), send_ticket))
                }
            }
        };
        let deadline = async_io::Timer::after(Duration::from_secs(5));
        futures::pin_mut!(work, deadline);
        Ok(match futures::future::select(work, deadline).await {
            futures::future::Either::Left((Ok(Some(())), _)) => CloseSessionOutcome::Closed,
            futures::future::Either::Left((Err(error), _)) => return Err(error),
            _ => CloseSessionOutcome::TimedOut,
        })
    }
}
