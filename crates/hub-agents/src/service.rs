//! The entity every window reads the agents and agent servers of a hub from.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, EventEmitter, Task};
use nocterm_ai::agents::AgentLaunch;
use nocterm_hub_agents_model::{
    HubAgent, HubAgentDirectory, HubAgentsError, HubChatChanges, HubServer, HubServers, ServerId,
    ServerRemoval,
};
use nocterm_hubs_model::HubId;

use crate::access::{self, AgentAccessError, AgentTarget};
use crate::installed::Deps;

const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(60);

/// What changed for the views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HubAgentsEvent {
    /// The agents or agent servers of the hub changed.
    Changed(HubId),
    /// The hub's chats changed: a history that lists them reads them again.
    ChatsChanged(HubId),
}

/// The lists of one tracked hub.
#[derive(Default)]
struct Entry {
    agents: Vec<HubAgent>,
    servers: Vec<HubServer>,
    /// Why the last listing failed, while it does.
    problem: Option<HubAgentsError>,
    _listing: Option<Task<()>>,
    _follow: Option<Task<()>>,
}

/// The agents and agent servers of every tracked hub.
pub struct HubAgents {
    deps: Deps,
    hubs: HashMap<HubId, Entry>,
}

impl EventEmitter<HubAgentsEvent> for HubAgents {}

impl HubAgents {
    pub(crate) fn new(deps: Deps) -> Self {
        Self {
            deps,
            hubs: HashMap::new(),
        }
    }

    /// Starts keeping the lists of `hub`. Tracking twice is harmless.
    pub fn track(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hubs.contains_key(hub) {
            return;
        }
        let follow = follow_chats(self.deps.chats.clone(), hub.clone(), cx);
        self.hubs.insert(
            hub.clone(),
            Entry {
                _follow: Some(follow),
                ..Entry::default()
            },
        );
        self.refresh(hub, cx);
    }

    /// Forgets `hub`: its lists are dropped and its chats are no longer followed.
    pub fn untrack(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hubs.remove(hub).is_some() {
            cx.emit(HubAgentsEvent::Changed(hub.clone()));
            cx.notify();
        }
    }

    /// Lists the agents and agent servers of `hub` again.
    pub fn refresh(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        let Some(entry) = self.hubs.get_mut(hub) else {
            return;
        };
        let agents = self.deps.directory.agents(hub);
        let servers = self.deps.servers.servers(hub);
        let hub_id = hub.clone();
        entry._listing = Some(cx.spawn(async move |this, cx| {
            let (agents, servers) = futures::join!(agents, servers);
            this.update(cx, |this, cx| this.listed(&hub_id, agents, servers, cx))
                .ok();
        }));
    }

    fn listed(
        &mut self,
        hub: &HubId,
        agents: Result<Vec<HubAgent>, HubAgentsError>,
        servers: Result<Vec<HubServer>, HubAgentsError>,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.hubs.get_mut(hub) else {
            return;
        };
        let mut problem = None;
        match agents {
            Ok(agents) => entry.agents = agents,
            Err(error) => problem = Some(error),
        }
        match servers {
            Ok(servers) => entry.servers = servers,
            Err(error) => problem = problem.or(Some(error)),
        }
        if let Some(error) = &problem {
            tracing::debug!(%error, hub = %hub, "the hub's agents could not be listed");
        }
        entry.problem = problem;
        cx.emit(HubAgentsEvent::Changed(hub.clone()));
        cx.notify();
    }

    /// The agents of `hub`, as last listed.
    pub fn agents(&self, hub: &HubId) -> &[HubAgent] {
        self.hubs.get(hub).map_or(&[], |entry| &entry.agents)
    }

    /// The servers connected to the agents of `hub`, as last listed.
    pub fn servers(&self, hub: &HubId) -> &[HubServer] {
        self.hubs.get(hub).map_or(&[], |entry| &entry.servers)
    }

    /// Why the last listing of `hub` failed, while it does.
    pub fn problem(&self, hub: &HubId) -> Option<&HubAgentsError> {
        self.hubs.get(hub)?.problem.as_ref()
    }

    /// The record of `target` among the agent servers of `hub`, as last listed.
    pub fn server_of(&self, hub: &HubId, target: &AgentTarget) -> Option<&HubServer> {
        self.servers(hub)
            .iter()
            .find(|server| server.is(&target.host, target.port, &target.user))
    }

    /// Lets the agents of `hub` work on `target`: the agent key's line is written through the
    /// session the user opened and the server is recorded on the hub, then in the list.
    pub fn grant_access(
        &mut self,
        hub: &HubId,
        target: AgentTarget,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), AgentAccessError>> {
        let work = cx.background_executor().spawn(access::grant(
            self.deps.servers.clone(),
            self.deps.hosts.clone(),
            hub.clone(),
            target,
        ));
        let hub = hub.clone();
        cx.spawn(async move |this, cx| {
            let server = work.await?;
            this.update(cx, |this, cx| this.recorded(&hub, Some(server), None, cx))
                .ok();
            Ok(())
        })
    }

    /// Stops the agents of `hub` working on `target`. `None` when the hub has no record of it.
    pub fn withdraw_access(
        &mut self,
        hub: &HubId,
        target: &AgentTarget,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<ServerRemoval>, AgentAccessError>> {
        let Some(server) = self.server_of(hub, target) else {
            return Task::ready(Ok(None));
        };
        let work = cx.background_executor().spawn(access::withdraw(
            self.deps.servers.clone(),
            self.deps.hosts.clone(),
            hub.clone(),
            target.connection.clone(),
            server.id.clone(),
        ));
        let (hub, id) = (hub.clone(), server.id.clone());
        cx.spawn(async move |this, cx| {
            let removal = work.await?;
            this.update(cx, |this, cx| this.recorded(&hub, None, Some(&id), cx))
                .ok();
            Ok(Some(removal))
        })
    }

    /// Applies what the hub answered to a change of its servers: `added` replaces a record of
    /// the same id, `removed` leaves the list.
    fn recorded(
        &mut self,
        hub: &HubId,
        added: Option<HubServer>,
        removed: Option<&ServerId>,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.hubs.get_mut(hub) else {
            return;
        };
        let gone = |server: &HubServer| {
            added.as_ref().is_some_and(|added| added.id == server.id) || removed == Some(&server.id)
        };
        entry.servers.retain(|server| !gone(server));
        entry.servers.extend(added);
        cx.emit(HubAgentsEvent::Changed(hub.clone()));
        cx.notify();
    }

    /// The agents of every tracked hub as agents a chat can start.
    pub fn launches(&self) -> Vec<AgentLaunch> {
        self.hubs
            .iter()
            .flat_map(|(hub, entry)| {
                entry.agents.iter().map(|agent| {
                    AgentLaunch::hub_agent(hub.clone(), agent.id.to_string(), agent.name.clone())
                })
            })
            .collect()
    }

    /// The agents port, for the forms that edit agents and providers.
    pub fn directory(&self) -> &Arc<dyn HubAgentDirectory> {
        &self.deps.directory
    }

    /// The servers port, for the agent key and the server rows.
    pub fn server_port(&self) -> &Arc<dyn HubServers> {
        &self.deps.servers
    }
}

/// Follows the chat list of `hub`, re-subscribing with growing delays, until it is untracked.
fn follow_chats(
    chats: Arc<dyn HubChatChanges>,
    hub: HubId,
    cx: &mut Context<HubAgents>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut delay = FIRST_RETRY;
        loop {
            if let Ok(mut hints) = chats.watch(&hub).await {
                delay = FIRST_RETRY;
                while let Ok(Some(())) = hints.next().await {
                    let alive = this.update(cx, |_, cx| {
                        cx.emit(HubAgentsEvent::ChatsChanged(hub.clone()));
                    });
                    if alive.is_err() {
                        return;
                    }
                }
            }
            cx.background_executor().timer(delay).await;
            delay = (delay * 2).min(MAX_RETRY);
            if this
                .read_with(cx, |this, _| !this.hubs.contains_key(&hub))
                .unwrap_or(true)
            {
                return;
            }
        }
    })
}
