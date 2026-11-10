//! The agent key and agent servers of the account, over `ServerService`.

use nocterm_hub_agents_model::{
    AgentKey, AgentsFuture, HubAgentsError, HubChange, HubChangeHints, HubChanges, HubServer,
    HubServers, NewServer, ServerId, ServerRemoval,
};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubId;

use super::{CallError, HubAgentsLink, convert};

impl HubServers for HubAgentsLink {
    fn agent_key(&self, hub: &HubId) -> AgentsFuture<AgentKey> {
        self.call(hub, |client| async move {
            let key = client.servers().get_agent_key(v2::Empty {}).await?;
            Ok(convert::agent_key(key.into_inner()))
        })
    }

    fn rotate_agent_key(&self, hub: &HubId) -> AgentsFuture<AgentKey> {
        self.call(hub, |client| async move {
            let key = client.servers().rotate_agent_key(v2::Empty {}).await?;
            Ok(convert::agent_key(key.into_inner()))
        })
    }

    fn servers(&self, hub: &HubId) -> AgentsFuture<Vec<HubServer>> {
        self.call(hub, |client| async move {
            let listed = client.servers().list_servers(v2::Empty {}).await?;
            listed
                .into_inner()
                .servers
                .into_iter()
                .map(convert::server)
                .collect::<Result<_, _>>()
                .map_err(CallError::from)
        })
    }

    fn add_server(&self, hub: &HubId, server: NewServer) -> AgentsFuture<HubServer> {
        self.call(hub, move |client| async move {
            let request = v2::AddServerRequest {
                name: server.name,
                host: server.host,
                port: server.port.into(),
                user: server.user,
                host_key: server.host_key,
            };
            let added = client.servers().add_server(request).await?;
            Ok(convert::server(added.into_inner())?)
        })
    }

    fn check_server(&self, hub: &HubId, server: &ServerId) -> AgentsFuture<HubServer> {
        let id = server.to_string();
        self.call(hub, |client| async move {
            let checked = client.servers().check_server(v2::IdRequest { id }).await?;
            Ok(convert::server(checked.into_inner())?)
        })
    }

    fn remove_server(&self, hub: &HubId, server: &ServerId) -> AgentsFuture<ServerRemoval> {
        let id = server.to_string();
        self.call(hub, |client| async move {
            let request = v2::RemoveServerRequest {
                id,
                credential: None,
            };
            let removed = client.servers().remove_server(request).await?.into_inner();
            Ok(ServerRemoval {
                line_removed: removed.line_removed,
                message: removed.message,
            })
        })
    }
}

/// Hints from one `Watch` of the account (no open chats) and from the chat lists this device
/// refreshed.
struct ChangeHints {
    hub: tonic::Streaming<v2::WatchEvent>,
    here: tokio::sync::watch::Receiver<u64>,
}

/// The hint of an event of the account's stream, if it is one.
fn hint_of(event: v2::watch_event::Event) -> Option<HubChange> {
    use v2::watch_event::Event;
    match event {
        Event::Chat(_) | Event::ChatDeleted(_) | Event::Resync(_) => Some(HubChange::Chats),
        Event::Account(changed) => match changed.list() {
            v2::AccountList::Agents | v2::AccountList::Providers => Some(HubChange::Agents),
            v2::AccountList::Servers => Some(HubChange::Servers),
            v2::AccountList::Unspecified => None,
        },
        Event::Item(_) | Event::Draft(_) | Event::Ready(_) => None,
    }
}

impl HubChangeHints for ChangeHints {
    fn next(
        &mut self,
    ) -> futures::future::BoxFuture<'_, Result<Option<HubChange>, HubAgentsError>> {
        Box::pin(async move {
            loop {
                tokio::select! {
                    changed = self.here.changed() => {
                        if changed.is_ok() {
                            return Ok(Some(HubChange::Chats));
                        }
                    }
                    message = self.hub.message() => match message {
                        Ok(Some(event)) => {
                            if let Some(hint) = event.event.and_then(hint_of) {
                                return Ok(Some(hint));
                            }
                        }
                        Ok(None) | Err(_) => return Ok(None),
                    },
                }
            }
        })
    }
}

impl HubChanges for HubAgentsLink {
    fn watch(&self, hub: &HubId) -> AgentsFuture<Box<dyn HubChangeHints>> {
        let here = self.hints.subscribe();
        self.call(hub, |client| async move {
            let request = v2::WatchRequest {
                chats_after_rev: 0,
                open: Vec::new(),
            };
            let hub = client.chats().watch(request).await?.into_inner();
            Ok(Box::new(ChangeHints { hub, here }) as Box<dyn HubChangeHints>)
        })
    }
}
