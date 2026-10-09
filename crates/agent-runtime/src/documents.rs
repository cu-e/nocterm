//! Document persistence is independent of the live ACP connections.
use super::*;

#[derive(Default)]
pub(super) struct ChatIoGate {
    closing: bool,
}

#[derive(Clone)]
pub(super) struct PendingChatWrite {
    id: String,
    chat: Option<Arc<nocterm_ai::history::SharedChat>>,
    revision: Option<(EntityId, u64)>,
}

impl Runtime {
    /// Saved chats for the first panel that asks once they are read; later
    /// panels get none, so two windows never write the same chat.
    pub fn take_saved_chats(&mut self) -> Option<Vec<nocterm_ai::history::SavedChat>> {
        self.saved_chats.as_mut().map(std::mem::take)
    }
    /// Captures durable state before disabling AI or dropping a panel's documents.
    pub fn capture_and_detach_document(&mut self, id: EntityId, cx: &mut Context<Self>) {
        if let Some(client) = self.documents.get(&id).cloned()
            && let Some((chat, revision)) = client.capture(cx)
        {
            self.save_chat(chat, id, revision, cx);
        }
        self.unregister_document(id);
    }
    /// Queues `chat` to be written, replacing an older queued snapshot.
    pub fn save_chat(
        &mut self,
        chat: Arc<nocterm_ai::history::SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        if self.deleted_chats.contains(&chat.id)
            || self.document_owners.get(&chat.id) != Some(&owner)
        {
            return;
        }
        self.chat_revisions
            .insert(chat.id.clone(), (owner, revision));
        self.chat_writes.insert(chat.id.clone(), Some(chat));
        self.write_chats(cx);
    }
    /// Queues the removal of chat `id`.
    pub fn delete_chat(&mut self, id: String, cx: &mut Context<Self>) {
        self.deleted_chats.insert(id.clone());
        self.chat_revisions.remove(&id);
        self.chat_writes.insert(id, None);
        self.write_chats(cx);
    }
    /// Writes every open chat and queued change now, before the application
    /// exits.
    pub(super) fn flush_chats(&mut self, cx: &mut Context<Self>) {
        let mut writes = HashMap::new();
        if let Some(pending) = &self.chat_in_flight {
            writes.insert(pending.id.clone(), pending.chat.clone());
        }
        writes.extend(std::mem::take(&mut self.chat_writes));
        for client in self.documents.values() {
            if client
                .state(cx)
                .is_some_and(|state| self.document_owners.get(&state.chat_id) == Some(&client.id()))
                && let Some(chat) = client.snapshot(cx)
                && !self.deleted_chats.contains(&chat.id)
            {
                writes.insert(chat.id.clone(), Some(chat));
            }
        }
        for deleted in &self.deleted_chats {
            writes.insert(deleted.clone(), None);
        }
        let mut gate = self
            .chat_io
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Join an active rename and prevent queued background snapshots writing after this flush.
        gate.closing = true;
        for (id, chat) in writes {
            let result = match chat {
                Some(chat) => nocterm_ai::history::save_shared(&self.services.chats_dir, &chat),
                None => nocterm_ai::history::delete(&self.services.chats_dir, &id),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "could not save agent chat");
            }
        }
    }
    pub(super) fn acknowledge_chat(
        &mut self,
        id: &str,
        owner: EntityId,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        if self.deleted_chats.contains(id) || self.document_owners.get(id) != Some(&owner) {
            return;
        }
        if let Some(client) = self.documents.get(&owner).cloned()
            && client.state(cx).is_some_and(|state| state.chat_id == id)
        {
            client.emit(SessionEvent::Saved { revision }, cx);
        }
    }
    fn write_chats(&mut self, cx: &mut Context<Self>) {
        if self.shutting_down || self.chat_writer.is_some() {
            return;
        }
        let dir = self.services.chats_dir.clone();
        let chat_io = self.chat_io.clone();
        self.chat_writer = Some(cx.spawn(async move |this, cx| {
            loop {
                let next = this.update(cx, |this, _| {
                    let id = this.chat_writes.keys().next().cloned();
                    match id {
                        Some(id) => {
                            let revision = this.chat_revisions.get(&id).copied();
                            this.chat_writes.remove_entry(&id).map(|(id, chat)| {
                                let pending = PendingChatWrite { id, chat, revision };
                                this.chat_in_flight = Some(pending.clone());
                                pending
                            })
                        }
                        None => {
                            this.chat_writer = None;
                            None
                        }
                    }
                });
                let Ok(Some(PendingChatWrite { id, chat, revision })) = next else {
                    return;
                };
                let dir = dir.clone();
                let write_id = id.clone();
                let retry = chat.clone();
                let chat_io = chat_io.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let gate = chat_io.lock().unwrap_or_else(|error| error.into_inner());
                        if gate.closing {
                            return Ok(());
                        }
                        match chat {
                            Some(chat) => nocterm_ai::history::save_shared(&dir, &chat),
                            None => nocterm_ai::history::delete(&dir, &id),
                        }
                    })
                    .await;
                if let Err(error) = result {
                    tracing::warn!(%error, "could not save agent chat");
                    let _ = this.update(cx, |this, cx| {
                        this.chat_writes.entry(write_id.clone()).or_insert(retry);
                        this.chat_in_flight = None;
                        this.chat_writer = None;
                        let clients: Vec<_> = this
                            .documents
                            .values()
                            .filter(|client| {
                                client
                                    .state(cx)
                                    .is_some_and(|state| state.chat_id == write_id)
                            })
                            .cloned()
                            .collect();
                        for client in clients {
                            client.emit(SessionEvent::SaveFailed(error.clone()), cx);
                        }
                    });
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    this.chat_in_flight = None;
                    if let Some((owner, revision)) = revision {
                        this.acknowledge_chat(&write_id, owner, revision, cx);
                    }
                });
            }
        }));
    }
}

#[cfg(test)]
mod history_tests;
