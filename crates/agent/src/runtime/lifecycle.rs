//! One admission queue and resource policy for every window.
use super::lease::ReleasedLease;
use super::*;
use std::time::Duration;
impl Runtime {
    pub(crate) fn register_document(
        &mut self,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) {
        let owner = thread.entity_id();
        let chat_id = thread.read(cx).chat_id.clone();
        // A replacement may have been tracked under its temporary new-chat id.
        self.document_owners.retain(|_, current| *current != owner);
        self.document_owners.insert(chat_id, owner);
        self.documents.insert(owner, thread.downgrade());
        if self._lifecycle.is_none() {
            self._lifecycle = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this
                        .update(cx, |this, cx| this.maintain_sessions(cx))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }
    pub(crate) fn unregister_document(&mut self, id: EntityId) {
        self.documents.remove(&id);
        self.document_owners.retain(|_, owner| *owner != id);
        self.pending_activation
            .retain(|thread| thread.entity_id() != id);
    }
    pub(crate) fn request_activation(
        &mut self,
        thread: WeakEntity<AgentThread>,
        cx: &mut Context<Self>,
    ) {
        if self.shutting_down || !cx.ai_enabled() {
            return;
        }
        if !self
            .pending_activation
            .iter()
            .any(|pending| pending.entity_id() == thread.entity_id())
        {
            self.pending_activation.push_back(thread);
        }
        self.maintain_sessions(cx);
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn maintain_sessions(&mut self, cx: &mut Context<Self>) {
        if self.shutting_down || !cx.ai_enabled() {
            self.pending_activation.clear();
            return;
        }
        let policy = cx
            .setting::<nocterm_settings::AiSettings>()
            .sessions
            .clone();
        let now = cx.background_executor().now();
        let mut idle = Vec::new();
        for (key, connection) in &self.connections {
            let busy = connection
                .users
                .values()
                .filter_map(WeakEntity::upgrade)
                .any(|thread| thread.read(cx).session_busy());
            if busy {
                self.idle_since.remove(key);
            } else {
                let since = *self.idle_since.entry(*key).or_insert(now);
                idle.push((*key, since));
            }
        }
        idle.sort_by_key(|(_, since)| *since);
        let excess = idle.len().saturating_sub(policy.max_idle);
        let needs_slot = !self.pending_activation.is_empty()
            && self.connections.len() + self.closing.len() >= policy.max_live;
        for (index, (key, since)) in idle.into_iter().enumerate() {
            if index < excess
                || now.duration_since(since) >= Duration::from_secs(policy.idle_timeout_secs)
                || (needs_slot && index == 0)
            {
                let threads: Vec<_> = self
                    .connections
                    .get(&key)
                    .into_iter()
                    .flat_map(|connection| connection.users.values())
                    .filter_map(WeakEntity::upgrade)
                    .collect();
                for thread in threads {
                    thread.update(cx, |thread, cx| {
                        thread.detach_session(cx);
                        thread.status = "Agent idle; conversation retained.".into();
                        cx.notify();
                    });
                }
            }
        }
        let mut pending = self.pending_activation.len();
        while pending > 0 && self.connections.len() + self.closing.len() < policy.max_live {
            pending -= 1;
            let Some(owner) = self.pending_activation.pop_front() else {
                break;
            };
            let Some(thread) = owner.upgrade() else {
                continue;
            };
            if self.document_owners.get(&thread.read(cx).chat_id) != Some(&thread.entity_id())
                || thread.read(cx).lease.is_some()
                || !thread.read(cx).activation_pending
            {
                continue;
            }
            if thread.read(cx).closing_session
                || self
                    .connections
                    .values()
                    .any(|connection| connection.chat_id == thread.read(cx).chat_id)
                || self
                    .closing
                    .values()
                    .any(|chat| chat == &thread.read(cx).chat_id)
            {
                self.pending_activation.push_back(owner);
                continue;
            }
            let agent = thread.read(cx).agent_id.clone();
            if let Some(launch) =
                nocterm_ai::AgentRegistry::new(cx.setting::<nocterm_settings::AiSettings>())
                    .get(&agent)
                    .cloned()
            {
                self.connect(thread, launch, true, cx);
            } else {
                thread.update(cx, |thread, cx| {
                    thread.activation_pending = false;
                    thread.fail("The agent is no longer configured.", cx);
                });
            }
        }
    }
    fn cleanup_failed(&mut self, chat_id: &str, error: &str, cx: &mut Context<Self>) {
        tracing::warn!(%error, "agent process cleanup could not be confirmed");
        for thread in self.documents.values().filter_map(WeakEntity::upgrade) {
            if thread.read(cx).chat_id != chat_id {
                continue;
            }
            thread.update(cx, |thread, cx| {
                thread.status = format!("Agent process cleanup could not be confirmed: {error}");
                thread.status_error = true;
                thread.queue_paused = true;
                cx.notify();
            });
        }
    }
    pub(crate) fn release_lease(&mut self, mut lease: ReleasedLease, cx: &mut Context<Self>) {
        let id = lease.owner;
        let key = lease.connection_key;
        if let Some(registration) = lease.registration.take() {
            self.registrations.remove(&registration.id);
            drop(registration);
        }
        self.idle_since.remove(&key);
        let connection = self.connections.remove(&key);
        if connection.is_none() {
            return;
        }
        // Keep the event pump alive until close's FIFO barrier is acknowledged.
        let mut connection = connection.expect("owned connection");
        let chat_id = connection.chat_id.clone();
        self.closing.insert(key, chat_id.clone());
        connection.cancellation.cancel();
        connection.users.remove(&id);
        let commands = lease.commands.take().or_else(|| connection.commands.take());
        let connecting = connection._connecting.take();
        let startup_completion = connection.startup_completion.take();
        cx.spawn(async move |this, cx| {
            if commands.is_none()
                && let Some(connecting) = connecting
            {
                connecting.await;
                if let Some(completion) = startup_completion {
                    let result = completion
                        .await
                        .unwrap_or_else(|_| Err("Startup cleanup acknowledgement was lost".into()));
                    if let Err(error) = result {
                        let _ =
                            this.update(cx, |this, cx| this.cleanup_failed(&chat_id, &error, cx));
                        return;
                    }
                }
            }
            if let Some(commands) = commands {
                if let Some(session) = lease.session.take() {
                    commands.cancel(session.clone());
                    let close = commands.close_session(session);
                    let bounded = futures::future::select(
                        close,
                        Box::pin(cx.background_executor().timer(Duration::from_secs(6))),
                    )
                    .await;
                    if let futures::future::Either::Left((Err(error), _)) = bounded {
                        tracing::warn!(%error, "agent session close failed");
                    }
                }
                if let Err(error) = commands.shutdown_gracefully().await {
                    let _ = this.update(cx, |this, cx| {
                        this.cleanup_failed(&chat_id, &error.to_string(), cx)
                    });
                    return; // Keep Closing counted: the process may still be alive.
                }
            }
            drop(connection);
            let _ = this.update(cx, |this, cx| {
                this.closing.remove(&key);
                if let Some(thread) = this.documents.get(&id).and_then(WeakEntity::upgrade) {
                    thread.update(cx, |thread, cx| {
                        thread.closing_session = false;
                        cx.notify();
                    });
                }
                this.maintain_sessions(cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
impl Runtime {
    /// The fixture injects one event stream; production has a separate stream per connection.
    pub(crate) fn inject_events(
        &mut self,
        events: async_channel::Receiver<AgentEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let _ = this.update(cx, |this, cx| {
                    let session = match &event {
                        AgentEvent::Session(notification) => Some(&notification.session_id),
                        AgentEvent::Permission { request, .. } => Some(&request.session_id),
                        _ => None,
                    };
                    let owner = this
                        .connections
                        .iter()
                        .find(|(_, connection)| {
                            session.is_none_or(|session| {
                                connection
                                    .users
                                    .values()
                                    .filter_map(WeakEntity::upgrade)
                                    .any(|thread| {
                                        thread.read(cx).session().as_ref() == Some(session)
                                    })
                            })
                        })
                        .map(|(key, connection)| (*key, connection.serial))
                        .or_else(|| {
                            this.connections
                                .iter()
                                .next()
                                .map(|(key, connection)| (*key, connection.serial))
                        });
                    if let Some((key, serial)) = owner {
                        this.event(key, serial, event, cx);
                    }
                });
            }
        })
        .detach();
    }
}
