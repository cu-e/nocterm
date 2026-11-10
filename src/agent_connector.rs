//! The connector every agent chat starts through: local agents run as ACP processes, hub agents
//! run on their hub and are reached through the hub adapter (ADR 0148).
use std::sync::Arc;

use futures::future::BoxFuture;
use nocterm_ai::{AgentConnection, AgentConnector, ConnectError, ConnectRequest};

pub(crate) struct AgentConnectors {
    local: Arc<dyn AgentConnector>,
    hub: Option<Arc<dyn AgentConnector>>,
}

impl AgentConnectors {
    pub(crate) fn new(
        local: Arc<dyn AgentConnector>,
        hub: Option<Arc<dyn AgentConnector>>,
    ) -> Self {
        Self { local, hub }
    }
}

impl AgentConnector for AgentConnectors {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, ConnectError>> {
        match (&request.launch.hub, &self.hub) {
            (Some(_), Some(hub)) => hub.connect(request),
            (Some(_), None) => Box::pin(async {
                Err(nocterm_ai::AgentError::Io(
                    "This build reaches no hubs, so a hub agent cannot start.".into(),
                ))
            }),
            (None, _) => self.local.connect(request),
        }
    }
}
