//! The agent key and agent servers of the account, over `ServerService`.

use nocterm_hub_agents_model::{
    AgentKey, AgentsFuture, ChatChangeHints, HubAgentsError, HubChatChanges, HubServer, HubServers,
    NewServer, ServerId, ServerRemoval,
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

/// Hints from one `Watch` of the chat list (no open chats), where every chat event is a hint,
/// and from the lists this device refreshed.
struct ChatHints {
    hub: tonic::Streaming<v2::WatchEvent>,
    here: tokio::sync::watch::Receiver<u64>,
}

impl ChatChangeHints for ChatHints {
    fn next(&mut self) -> futures::future::BoxFuture<'_, Result<Option<()>, HubAgentsError>> {
        Box::pin(async move {
            loop {
                tokio::select! {
                    changed = self.here.changed() => {
                        if changed.is_ok() {
                            return Ok(Some(()));
                        }
                    }
                    message = self.hub.message() => match message {
                        Ok(Some(event)) => {
                            if matches!(
                                event.event,
                                Some(
                                    v2::watch_event::Event::Chat(_)
                                        | v2::watch_event::Event::ChatDeleted(_)
                                        | v2::watch_event::Event::Resync(_)
                                )
                            ) {
                                return Ok(Some(()));
                            }
                        }
                        Ok(None) | Err(_) => return Ok(None),
                    },
                }
            }
        })
    }
}

impl HubChatChanges for HubAgentsLink {
    fn watch(&self, hub: &HubId) -> AgentsFuture<Box<dyn ChatChangeHints>> {
        let here = self.hints.subscribe();
        self.call(hub, |client| async move {
            let request = v2::WatchRequest {
                chats_after_rev: 0,
                open: Vec::new(),
            };
            let hub = client.chats().watch(request).await?.into_inner();
            Ok(Box::new(ChatHints { hub, here }) as Box<dyn ChatChangeHints>)
        })
    }
}
