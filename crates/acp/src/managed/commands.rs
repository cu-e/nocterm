//! Managed commands stage actual wire ownership before native request work runs.
use super::{
    channel::Sender,
    initialize::Restore,
    outgoing, results,
    rpc::{Cause, Rpc},
};
use agent_client_protocol::managed::Source;
use nocterm_ai::{
    connection::{
        CloseSessionOutcome, ManagedAgentCommands, ManagedAgentEvent, ManagedPreparedPrompt,
    },
    history::source::{ManagedCommandError, ManagedOpened, ManagedSourceFuture},
    model::{
        controls::ConfigOption,
        ids::{AuthMethodId, SessionId},
        json::deferred::NativeAllocationError,
        native::NativeOwned,
        session::{ConfigurationSelection, ModeSelection, RestoreSession, SessionOpen},
    },
};
use std::sync::Arc;
type Operation<T> =
    Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError>;
pub(super) struct Commands {
    rpc: Rpc,
    events: Sender<ManagedAgentEvent>,
    stop: Sender<()>,
    group: Arc<crate::process::ProcessGroup>,
    cleanup_lifetime: nocterm_ai::history::source::SourceLifetime,
}
impl Commands {
    pub(super) fn new(
        rpc: Rpc,
        events: Sender<ManagedAgentEvent>,
        stop: Sender<()>,
        group: Arc<crate::process::ProcessGroup>,
        cleanup_lifetime: nocterm_ai::history::source::SourceLifetime,
    ) -> Self {
        Self {
            rpc,
            events,
            stop,
            group,
            cleanup_lifetime,
        }
    }
    fn cleanup(&self, final_attempt: bool) -> Operation<()> {
        let lifetime = self.cleanup_lifetime.clone();
        let group = self.group.clone();
        let stop = self.stop.clone();
        let ticket = self.rpc.ticket()?;
        ManagedSourceFuture::new(lifetime, async move {
            let result = if final_attempt {
                group.shutdown_final_native().await
            } else {
                group.shutdown_native().await
            };
            let _ = stop.try_send(());
            result.map_err(|error| {
                ManagedCommandError::CleanupUnconfirmed(ticket.capture(Cause::Cleanup(error)))
            })
        })
    }
}
impl ManagedAgentCommands for Commands {
    fn new_session(&self, request: &SessionOpen) -> Operation<ManagedOpened> {
        let envelope = self.rpc.id().map_err(Cause::from).and_then(|id| {
            Ok(outgoing::open(
                self.rpc.budget(),
                &id,
                "session/new",
                &request.working_directory,
                &request.endpoints,
                None,
            )?)
        });
        self.rpc.opened(envelope, false, None)
    }
    fn restore_session(&self, request: &RestoreSession) -> Operation<ManagedOpened> {
        let (restore, fork, _) = self.rpc.state().capabilities();
        let method = if request.fork && fork {
            "session/fork"
        } else if request.fork {
            return self.rpc.unsupported(true);
        } else {
            match restore {
                Restore::Resume => "session/resume",
                Restore::Load => "session/load",
                Restore::Unsupported => return self.rpc.unsupported(false),
            }
        };
        let envelope = self.rpc.id().map_err(Cause::from).and_then(|id| {
            Ok(outgoing::open(
                self.rpc.budget(),
                &id,
                method,
                &request.working_directory,
                &request.endpoints,
                Some(request.session.as_str()),
            )?)
        });
        let restored = if request.fork {
            None
        } else {
            match Source::copy(request.session.as_str(), self.rpc.budget().clone()) {
                Ok(source) => Some(source),
                Err(error) => return self.rpc.opened(Err(Cause::Routing(error)), true, None),
            }
        };
        self.rpc.opened(envelope, true, restored)
    }
    fn prepare_prompt(
        &self,
        payload: &nocterm_ai::history::physical::DispatchPayload,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError> {
        self.rpc.prepare_prompt(payload)
    }

    fn cancel(&self, session: &SessionId) -> Result<(), ManagedCommandError> {
        let _lifetime = self
            .rpc
            .state()
            .lifetime()
            .map_err(ManagedCommandError::Admission)?;
        let ticket = self.rpc.ticket().map_err(ManagedCommandError::Admission)?;
        let result = outgoing::session(self.rpc.budget(), None, "session/cancel", session.as_str())
            .map_err(Cause::from)
            .and_then(|envelope| Ok(self.rpc.notify(envelope)?));
        result.map_err(|error| super::rpc::failure(error, ticket))
    }
    fn set_mode(&self, request: &ModeSelection) -> Operation<()> {
        let envelope = self.rpc.id().map_err(Cause::from).and_then(|id| {
            Ok(outgoing::mode(
                self.rpc.budget(),
                &id,
                request.session.as_str(),
                request.mode.as_str(),
            )?)
        });
        self.rpc
            .operation(envelope, false, |value, _| Ok(results::empty(value)?))
    }
    fn set_config_option(
        &self,
        request: &ConfigurationSelection,
    ) -> Operation<NativeOwned<Vec<ConfigOption>>> {
        super::configuration_command::Configuration::new(self.rpc.clone()).execute(request)
    }
    fn authenticate(&self, method: &AuthMethodId) -> Operation<()> {
        let envelope = self.rpc.id().map_err(Cause::from).and_then(|id| {
            Ok(outgoing::authenticate(
                self.rpc.budget(),
                &id,
                method.as_str(),
            )?)
        });
        self.rpc
            .operation(envelope, false, |value, _| Ok(results::empty(value)?))
    }
    fn close_session(&self, session: &SessionId) -> Operation<CloseSessionOutcome> {
        super::close_command::Close::new(self.rpc.clone(), self.events.clone()).execute(session)
    }
    fn shutdown_gracefully(&self) -> Operation<()> {
        self.cleanup(false)
    }
    fn shutdown_finally(&self) -> Operation<()> {
        self.cleanup(true)
    }
    fn shutdown(&self) {
        self.group.kill();
        let _ = self.stop.try_send(());
    }
}
