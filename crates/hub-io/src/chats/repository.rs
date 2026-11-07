//! The history of a hub window: the local chats of its account and the hub's chats.
//!
//! Local chats stay in the local repository. Chats of hub agents live on the hub: the list
//! comes from `ListChats` and is kept in a private file per hub, so it shows at once after a
//! restart and while the hub cannot be reached; a hub chat's transcript comes from the hub
//! when the chat opens. Renaming, pinning and deleting a hub chat goes to the hub. A hub chat
//! started on this device keeps the id the chat had here; the file remembers which hub chat
//! that is.

mod list;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nocterm_ai::agents::AgentLaunch;
use nocterm_ai::history::{ChatSummary, ContextId, SavedChat, SharedChat};
use nocterm_ai::local_environment::{ChatScope, LocalEnvironment};
use nocterm_ai::storage::{
    AgentStorageError, ChatRepository, PreparedChatBytes, PreparedChatLoad, StorageFuture,
};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubId;
use nocterm_hubs_model::source_identity::LocalAccountPartition;
use tokio::runtime::Handle;

use self::list::{ChatList, Row};
use super::ChatHints;
use super::connector::call;
use crate::devices::Captured;
use crate::links::HubLinks;
use crate::pool::Pool;

/// The most chats of a hub the history lists.
const MAX_CHATS: u32 = 500;

/// Puts the chats of hub agents beside the local history of hub windows.
pub struct HubChatRepository {
    local: Arc<dyn ChatRepository>,
    handle: Handle,
    pool: Arc<Pool>,
    root: PathBuf,
    hints: Arc<ChatHints>,
    lists: Mutex<HashMap<HubId, Arc<Mutex<ChatList>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hub whose account `scope` belongs to.
fn hub_of(scope: &ChatScope) -> Option<HubId> {
    match scope.environment() {
        LocalEnvironment::HubAccount(LocalAccountPartition::LegacyConfigured(account)) => {
            Some(account.hub().clone())
        }
        _ => None,
    }
}

impl HubChatRepository {
    /// Hub chats beside `local`; the lists are kept in private files under `root`.
    pub fn new(links: &HubLinks, local: Arc<dyn ChatRepository>, root: PathBuf) -> Self {
        let (handle, pool) = links.agent_parts();
        Self {
            local,
            handle,
            pool,
            root,
            hints: links.chat_hints(),
            lists: Mutex::default(),
        }
    }

    fn list(&self, hub: &HubId) -> Arc<Mutex<ChatList>> {
        lock(&self.lists)
            .entry(hub.clone())
            .or_insert_with(|| Arc::new(Mutex::new(ChatList::load(&self.root, hub))))
            .clone()
    }

    /// Reads the hub's chats in the background; a changed list is kept and hinted, so the
    /// history is read again.
    fn refresh(&self, hub: &HubId) {
        let Ok(captured) = Captured::new(self.pool.clone(), hub) else {
            return;
        };
        let captured = Arc::new(captured);
        let list = self.list(hub);
        let hints = self.hints.clone();
        let root = self.root.clone();
        let hub = hub.clone();
        let read = call(&self.handle, &captured, |client| async move {
            let request = v2::ListChatsRequest {
                cursor: String::new(),
                limit: MAX_CHATS,
            };
            Ok(client.chats().list_chats(request).await?.into_inner().chats)
        });
        self.handle.spawn(async move {
            let Ok(chats) = read.await else {
                return;
            };
            let changed = lock(&list).replace(chats.iter().map(Row::of).collect());
            if changed {
                lock(&list).save(&root, &hub);
                hints.raise();
            }
        });
    }

    /// A hub chat as a saved chat without transcript: the session reopens on the hub.
    fn hub_chat(&self, scope: &ChatScope, hub: &HubId, id: &str) -> Option<SavedChat> {
        let list = self.list(hub);
        let list = lock(&list);
        let row = list.row_for(id)?;
        let agent =
            AgentLaunch::hub_agent(hub.clone(), row.agent_id.clone(), row.agent_name.clone());
        let mut chat =
            SavedChat::for_environment(agent.id, id.to_owned(), row.updated(), scope.clone());
        chat.header.title = Some(row.title.clone()).filter(|title| !title.is_empty());
        chat.header.pinned = row.pinned;
        chat.session_id = Some(row.id.clone());
        Some(chat)
    }

    /// Sends what changed of a hub chat's name or pin to the hub.
    fn publish(&self, hub: &HubId, chat: &SharedChat) {
        let header = &chat.metadata.header;
        let list = self.list(hub);
        let (id, rename, pin) = {
            let mut list = lock(&list);
            if let Some(session) = &chat.metadata.session_id {
                list.remember(&header.id, session);
            }
            let Some(row) = list.row_for(&header.id).cloned() else {
                return;
            };
            let rename = header
                .name
                .clone()
                .filter(|name| !name.is_empty() && *name != row.title);
            let pin = (header.pinned != row.pinned).then_some(header.pinned);
            list.apply(&row.id, rename.as_deref(), pin);
            list.save(&self.root, hub);
            (row.id, rename, pin)
        };
        if rename.is_none() && pin.is_none() {
            return;
        }
        let Ok(captured) = Captured::new(self.pool.clone(), hub) else {
            return;
        };
        let captured = Arc::new(captured);
        let sent = call(&self.handle, &captured, move |client| async move {
            if let Some(title) = rename {
                let request = v2::RenameChatRequest {
                    id: id.clone(),
                    title,
                };
                client.chats().rename_chat(request).await?;
            }
            if let Some(pinned) = pin {
                client
                    .chats()
                    .pin_chat(v2::PinChatRequest { id, pinned })
                    .await?;
            }
            Ok(())
        });
        self.handle.spawn(async move {
            if let Err(error) = sent.await {
                tracing::warn!(%error, "the hub did not take the chat's new name or pin");
            }
        });
    }
}

impl ChatRepository for HubChatRepository {
    fn prepare_load_in(
        &self,
        scope: ChatScope,
        id: String,
    ) -> StorageFuture<Box<dyn PreparedChatLoad>> {
        let Some(hub) = hub_of(&scope) else {
            return self.local.prepare_load_in(scope, id);
        };
        let Some(chat) = self.hub_chat(&scope, &hub, &id) else {
            return self.local.prepare_load_in(scope, id);
        };
        Box::pin(async move {
            let bytes = nocterm_ai::history::encode(&chat)?;
            let prepared = PreparedChatBytes::new(bytes, scope, id, Some(chat.version))?;
            Ok(Box::new(prepared) as Box<dyn PreparedChatLoad>)
        })
    }

    fn catalog_in(&self, scope: ChatScope) -> StorageFuture<Vec<ChatSummary>> {
        let local = self.local.catalog_in(scope.clone());
        let Some(hub) = hub_of(&scope) else {
            return local;
        };
        self.refresh(&hub);
        let hub_chats: Vec<ChatSummary> = lock(&self.list(&hub))
            .local_ids()
            .filter_map(|id| self.hub_chat(&scope, &hub, &id))
            .map(|chat| ChatSummary::from_chat(&chat))
            .collect();
        Box::pin(async move {
            let mut rows = local.await?;
            rows.retain(|row| !hub_chats.iter().any(|hub_row| hub_row.id == row.id));
            rows.extend(hub_chats);
            Ok(rows)
        })
    }

    fn load_in(&self, scope: ChatScope, id: String) -> StorageFuture<SavedChat> {
        match hub_of(&scope).and_then(|hub| self.hub_chat(&scope, &hub, &id)) {
            Some(chat) => Box::pin(async move { Ok(chat) }),
            None => self.local.load_in(scope, id),
        }
    }

    fn delete_in(&self, scope: ChatScope, id: String) -> StorageFuture<()> {
        let local = self.local.delete_in(scope.clone(), id.clone());
        let Some(hub) = hub_of(&scope) else {
            return local;
        };
        let list = self.list(&hub);
        let Some(chat) = lock(&list).forget(&id) else {
            return local;
        };
        lock(&list).save(&self.root, &hub);
        let deleted = Captured::new(self.pool.clone(), &hub).map(|captured| {
            call(&self.handle, &Arc::new(captured), |client| async move {
                client
                    .chats()
                    .delete_chat(v2::IdRequest { id: chat })
                    .await?;
                Ok(())
            })
        });
        Box::pin(async move {
            if let Ok(deleted) = deleted
                && let Err(error) = deleted.await
            {
                tracing::warn!(%error, "the hub did not delete the chat");
            }
            match local.await {
                Ok(()) | Err(AgentStorageError::NotFound(_)) => Ok(()),
                Err(error) => Err(error),
            }
        })
    }

    fn catalog(&self, scope: ContextId) -> StorageFuture<Vec<ChatSummary>> {
        self.local.catalog(scope)
    }

    fn load(&self, scope: ContextId, id: String) -> StorageFuture<SavedChat> {
        self.local.load(scope, id)
    }

    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()> {
        let scope = chat.metadata.header.chat_scope();
        match hub_of(&scope) {
            Some(hub) if is_hub_agent(&chat.metadata.header.agent_id) => {
                self.publish(&hub, &chat);
                Box::pin(async { Ok(()) })
            }
            _ => self.local.save(chat),
        }
    }

    fn delete(&self, scope: ContextId, id: String) -> StorageFuture<()> {
        self.local.delete(scope, id)
    }
}

/// Whether a chat's agent is a hub agent: its transcript lives on the hub, not here.
fn is_hub_agent(agent_id: &str) -> bool {
    agent_id.starts_with(nocterm_ai::agents::HUB_AGENT_PREFIX)
}

#[cfg(test)]
mod tests;
