//! A configuration request retains its original ticket and operation lifetime.
use super::super::session_driver::OperationGuard;
use futures::FutureExt as _;
use nocterm_ai::{
    connection::ManagedCommandsHandle,
    history::source::ManagedCommandError,
    model::{
        controls::ConfigOption,
        ids::{ConfigId, ModeId, SessionId},
        native::NativeOwned,
        session::{ConfigurationSelection, ModeSelection},
    },
    session::Ticket,
    session_config::ConfigValue,
};
pub(in crate::thread) enum ConfigurationRequest {
    Option(ConfigId, ConfigValue),
    Mode(ModeId),
}
pub(in crate::thread) enum ConfigurationResult {
    Option(Result<NativeOwned<Vec<ConfigOption>>, ManagedCommandError>),
    Mode(ModeId, Result<(), ManagedCommandError>),
}
pub(in crate::thread) struct ConfigurationOperation {
    ticket: Ticket,
    reconciling: bool,
    guard: OperationGuard,
    future: futures::future::BoxFuture<'static, ConfigurationResult>,
}
impl ConfigurationOperation {
    pub(in crate::thread) fn new(
        ticket: Ticket,
        reconciling: bool,
        guard: OperationGuard,
        commands: ManagedCommandsHandle,
        session: SessionId,
        request: ConfigurationRequest,
    ) -> Self {
        let future = Box::pin(match request {
            ConfigurationRequest::Option(id, value) => {
                let work = commands.set_config_option(&ConfigurationSelection {
                    session,
                    option: id,
                    value,
                });
                async move {
                    ConfigurationResult::Option(super::super::session_driver::settle(work).await)
                }
                .boxed()
            }
            ConfigurationRequest::Mode(id) => {
                let work = commands.set_mode(&ModeSelection {
                    session,
                    mode: id.clone(),
                });
                async move {
                    ConfigurationResult::Mode(id, super::super::session_driver::settle(work).await)
                }
                .boxed()
            }
        });
        Self {
            ticket,
            reconciling,
            guard,
            future,
        }
    }
    pub(in crate::thread) async fn run(self) -> (Ticket, bool, ConfigurationResult) {
        let _guard = self.guard;
        (self.ticket, self.reconciling, self.future.await)
    }
}

pub(in crate::thread) fn rejection_description(option: bool) -> &'static str {
    if option {
        "agent rejected a chosen configuration option"
    } else {
        "agent rejected the chat's mode"
    }
}
