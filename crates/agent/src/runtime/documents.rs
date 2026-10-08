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
    pub(crate) fn take_saved_chats(&mut self) -> Option<Vec<nocterm_ai::history::SavedChat>> {
        self.saved_chats.as_mut().map(std::mem::take)
    }
    /// Queues `chat` to be written, replacing an older queued snapshot.
    pub(crate) fn save_chat(
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
    pub(crate) fn delete_chat(&mut self, id: String, cx: &mut Context<Self>) {
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
        for thread in self.documents.values().filter_map(WeakEntity::upgrade) {
            if self.document_owners.get(&thread.read(cx).chat_id) == Some(&thread.entity_id())
                && let Some(chat) = thread.read(cx).shared_snapshot(cx)
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
        if let Some(thread) = self.documents.get(&owner).and_then(WeakEntity::upgrade) {
            thread.update(cx, |thread, cx| {
                if thread.chat_id == id {
                    thread.persisted_revision = thread.persisted_revision.max(revision);
                    thread.persistence_error = None;
                    thread.dispatch_next(cx);
                    cx.notify();
                }
            });
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
                        },
                        None => {
                            this.chat_writer = None;
                            None
                        }
                    }
                });
                let Ok(Some(PendingChatWrite {id, chat, revision})) = next else {
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
                        if gate.closing { return Ok(()); }
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
                        for thread in this.documents.values().filter_map(WeakEntity::upgrade) {
                            thread.update(cx, |thread, cx| {
                                if thread.chat_id == write_id {
                                    thread.persistence_error = Some(error.clone());
                                    thread.queue_paused = true;
                                    thread.status = format!("Could not save chat: {error}. Queued messages are retained; check disk space before continuing.");
                                    thread.status_error = true;
                                    cx.notify();
                                }
                            });
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
