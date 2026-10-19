//! Runtime-neutral ACP connector: one protocol driver per live subprocess.
use futures::{FutureExt, future::BoxFuture};
use nocterm_ai::{AgentConnection, AgentConnector, AgentError, ConnectRequest};

mod commands;
mod driver;
mod error;
mod exit_report;
mod handlers;
mod handshake;
mod pipes;
mod remote_errors;
mod startup;
mod stderr_tail;

/// Names the behavior tests exercise through `use super::*`.
#[cfg(test)]
use {
    agent_client_protocol::{Agent, ConnectionTo},
    commands::close_session,
    futures::future::{Either, select},
    handlers::client_builder,
    nocterm_ai::{AgentEvent, acp},
    remote_errors::{map_error, map_error_with, restore_error},
    std::{sync::Arc, time::Duration},
};

/// Connects agents over a spawned ACP subprocess.
#[derive(Clone, Default)]
pub struct AcpConnector {
    helper: Option<std::path::PathBuf>,
}
impl AcpConnector {
    pub fn managed(helper: std::path::PathBuf) -> Self {
        Self {
            helper: Some(helper),
        }
    }
    pub fn unmanaged() -> Self {
        Self::default()
    }
}

impl AgentConnector for AcpConnector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        startup::connect(request, self.helper.clone()).boxed()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifecycle_tests;
#[cfg(all(test, unix))]
mod restore_classification;
#[cfg(all(test, unix))]
mod startup_failures;
