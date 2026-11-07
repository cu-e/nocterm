//! Agent access of the saved servers of a hub window, over the hub agents service (ADR 0148):
//! the connection editor's switch and the state on the servers' rows read and change the hub's
//! agent servers through it.
use gpui_kit::{App, Entity, Subscription, Task};
use nocterm_connections::{AgentAccessRefused, AgentAccessState, AgentServer, ServerAgentAccess};
use nocterm_hub_agents::{AgentAccessError, AgentTarget, HubAgents, HubAgentsEvent};
use nocterm_hub_agents_model::ServerStatus;
use nocterm_hubs_model::HubId;
use nocterm_ssh_keys::authorized_keys::HostAccessError;

pub(crate) struct HubServerAccess {
    hub: HubId,
    agents: Entity<HubAgents>,
}

impl HubServerAccess {
    pub(crate) fn new(hub: HubId, agents: Entity<HubAgents>) -> Self {
        Self { hub, agents }
    }
}

impl ServerAgentAccess for HubServerAccess {
    fn state(&self, server: &AgentServer, cx: &App) -> Option<AgentAccessState> {
        let record = self.agents.read(cx).server_of(&self.hub, &target(server))?;
        Some(state(record.status))
    }

    fn session_open(&self, server: &AgentServer, cx: &App) -> bool {
        self.agents.read(cx).session_open(&server.connection)
    }

    fn set(
        &self,
        server: AgentServer,
        granted: bool,
        cx: &mut App,
    ) -> Task<Result<(), AgentAccessRefused>> {
        let (hub, target) = (self.hub.clone(), target(&server));
        let change = self.agents.update(cx, |agents, cx| {
            if granted {
                agents.grant_access(&hub, target, cx)
            } else {
                let withdrawn = agents.withdraw_access(&hub, &target, cx);
                cx.background_executor()
                    .spawn(async move { withdrawn.await.map(drop) })
            }
        });
        cx.background_executor()
            .spawn(async move { change.await.map_err(|error| refusal(&error)) })
    }

    fn observe(&self, changed: Box<dyn Fn(&mut App)>, cx: &mut App) -> Subscription {
        let hub = self.hub.clone();
        cx.subscribe(&self.agents, move |_, event: &HubAgentsEvent, cx| {
            if matches!(event, HubAgentsEvent::Changed(changed_hub) if *changed_hub == hub) {
                changed(cx);
            }
        })
    }
}

/// What the row of a server says about what the agents last found there.
fn state(status: ServerStatus) -> AgentAccessState {
    match status {
        ServerStatus::Unchecked => AgentAccessState::Unchecked,
        ServerStatus::Ready => AgentAccessState::Ready,
        ServerStatus::NoAccess => AgentAccessState::NoAccess,
        ServerStatus::HostKeyChanged => AgentAccessState::HostKeyChanged,
        ServerStatus::Unreachable => AgentAccessState::Unreachable,
        ServerStatus::KeyRotated => AgentAccessState::Reconnect,
    }
}

fn target(server: &AgentServer) -> AgentTarget {
    AgentTarget {
        connection: server.connection.clone(),
        name: server.name.clone(),
        host: server.host.clone(),
        port: server.port,
        user: server.user.clone(),
    }
}

/// What the editor says when agent access did not change.
fn refusal(error: &AgentAccessError) -> AgentAccessRefused {
    AgentAccessRefused::new(match error {
        AgentAccessError::NotConnected => {
            "The session to the server closed before the key was written. Try again.".to_owned()
        }
        AgentAccessError::UnknownHostKey => {
            "This device has no recorded host key for the server. Connect to it once, then try again."
                .to_owned()
        }
        AgentAccessError::Host(HostAccessError::Changed) => {
            "The server's authorized_keys changed while it was edited. Nothing was written; try again."
                .to_owned()
        }
        AgentAccessError::Host(HostAccessError::EditLocked { path }) => format!(
            "Another Nocterm edit may be running. Retry when it finishes; if an edit was interrupted, remove the lock file {path}."
        ),
        AgentAccessError::Host(_) => "The server's authorized_keys could not be edited.".to_owned(),
        AgentAccessError::Hub(error) => format!("The hub did not accept the change: {error}."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_names_what_to_do_next() {
        let missing = refusal(&AgentAccessError::NotConnected);
        assert!(missing.reason().contains("Try again"));
        let locked = refusal(&AgentAccessError::Host(HostAccessError::EditLocked {
            path: "/home/u/.ssh/.lock".into(),
        }));
        assert!(locked.reason().contains("/home/u/.ssh/.lock"));
    }
}
