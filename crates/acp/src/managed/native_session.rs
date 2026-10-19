//! Startup readiness and stop share the driver's actual paid queues.
use super::{
    channel::{Receiver, Sender},
    rpc::Rpc,
};
use nocterm_ai::{
    connection::{
        ManagedAgentConnection, ManagedCommandsHandle, ManagedEventReceiver, ManagedStartupHandle,
    },
    history::source::{ManagedCommandError, ManagedSourceFuture},
    model::json::deferred::NativeAllocationError,
};
pub(super) type Ready = Result<ManagedAgentConnection, ManagedCommandError>;
pub(super) struct Session {
    initialization: ManagedSourceFuture<
        Result<nocterm_ai::history::source::ManagedAgentInfo, ManagedCommandError>,
    >,
    commands: ManagedCommandsHandle,
    events: ManagedEventReceiver,
    stop: Receiver<()>,
    ready: Sender<Ready>,
}
impl Session {
    pub(super) fn new(
        rpc: &Rpc,
        startup: ManagedStartupHandle,
        commands: ManagedCommandsHandle,
        events: ManagedEventReceiver,
        stop: Receiver<()>,
        ready: Sender<Ready>,
    ) -> Result<Self, NativeAllocationError> {
        Ok(Self {
            initialization: super::initialization_command::initialize(rpc, startup)?,
            commands,
            events,
            stop,
            ready,
        })
    }
    pub(super) async fn run(mut self) {
        let result = self
            .initialization
            .await
            .map(|info| ManagedAgentConnection::new(info, self.commands, self.events));
        let accepted = result.is_ok();
        if self.ready.try_send(result).is_err() || !accepted {
            return;
        }
        let _ = futures::future::poll_fn(|cx| self.stop.poll_recv(cx)).await;
    }
}
