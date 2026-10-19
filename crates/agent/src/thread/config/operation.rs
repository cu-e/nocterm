//! A configuration request retains its original ticket and operation lifetime.
use super::super::session_driver::OperationGuard;
use nocterm_ai::model::controls::{ConfigOption, ConfigSetting};
use nocterm_ai::model::ids::{ConfigId, ModeId, SessionId};
use nocterm_ai::model::session::{SetConfigOption, SetMode};
use nocterm_ai::{AgentCommands, AgentError, session::Ticket};
use std::sync::Arc;
pub(in crate::thread) enum ConfigurationRequest {
    Option(ConfigId, ConfigSetting),
    Mode(ModeId),
}
pub(in crate::thread) enum ConfigurationResult {
    Option(Result<Vec<ConfigOption>, AgentError>),
    Mode(ModeId, Result<(), AgentError>),
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
        commands: Arc<dyn AgentCommands>,
        session: SessionId,
        request: ConfigurationRequest,
    ) -> Self {
        let future = Box::pin(async move {
            match request {
                ConfigurationRequest::Option(id, value) => ConfigurationResult::Option(
                    commands
                        .set_config_option(SetConfigOption {
                            session,
                            config: id,
                            setting: value,
                        })
                        .await,
                ),
                ConfigurationRequest::Mode(id) => {
                    let result = commands
                        .set_mode(SetMode {
                            session,
                            mode: id.clone(),
                        })
                        .await;
                    ConfigurationResult::Mode(id, result)
                }
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
