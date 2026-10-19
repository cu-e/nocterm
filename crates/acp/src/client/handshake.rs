//! The ACP `initialize` exchange, bounded in time and in accepted protocol version.
use std::{sync::Arc, time::Duration};

use agent_client_protocol::{Agent, ConnectionTo, schema::ProtocolVersion};
use futures::future::{Either, select};
use nocterm_ai::{AgentInfo, acp};

use super::{error::StartupError, remote_errors::map_error_with};

const TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct Handshake {
    fallback_name: String,
    terminal_auth: bool,
    secrets: Arc<Vec<String>>,
}
impl Handshake {
    pub(super) fn new(
        fallback_name: String,
        terminal_auth: bool,
        secrets: Arc<Vec<String>>,
    ) -> Self {
        Self {
            fallback_name,
            terminal_auth,
            secrets,
        }
    }
    pub(super) async fn run(
        self,
        connection: &ConnectionTo<Agent>,
    ) -> Result<AgentInfo, StartupError> {
        let capabilities = acp::ClientCapabilities::default()
            .auth(acp::AuthCapabilities::new().terminal(self.terminal_auth))
            .session(
                acp::ClientSessionCapabilities::default().config_options(
                    acp::SessionConfigOptionsCapabilities::default()
                        .boolean(acp::BooleanConfigOptionCapabilities::default()),
                ),
            );
        let initialize = connection
            .send_request(
                acp::InitializeRequest::new(ProtocolVersion::V1)
                    .client_capabilities(capabilities)
                    .client_info(acp::Implementation::new("nocterm", "0")),
            )
            .block_task();
        let response = match select(
            Box::pin(initialize),
            Box::pin(async_io::Timer::after(TIMEOUT)),
        )
        .await
        {
            Either::Left((response, _)) => {
                response.map_err(|error| map_error_with(error, &self.secrets))?
            }
            Either::Right(_) => return Err(StartupError::InitializationTimeout),
        };
        if response.protocol_version != ProtocolVersion::V1 {
            return Err(StartupError::UnsupportedProtocol);
        }
        let implementation = response.agent_info;
        Ok(AgentInfo {
            name: implementation
                .as_ref()
                .map_or(self.fallback_name, |v| v.name.clone()),
            version: implementation.map_or_else(String::new, |v| v.version),
            capabilities: response.agent_capabilities,
            auth_methods: response.auth_methods,
        })
    }
}
