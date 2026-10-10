//! One admission queue and resource policy for every window.
use super::admission::{IdleConnection, releases};
use super::lease::ReleasedLease;
use super::*;
use std::time::Duration;
impl Runtime {
    pub fn register_document(&mut self, client: Client, cx: &mut Context<Self>) {
        let owner = client.id();
        let Some(state) = client.state(cx) else {
            return;
        };
        let chat_id = state.chat_id;
        // A replacement may have been tracked under its temporary new-chat id.
        self.document_owners.retain(|_, current| *current != owner);
        self.document_owners.insert(chat_id, owner);
        self.documents.insert(owner, client);
    }
    fn start_session_maintenance(&mut self, cx: &mut Context<Self>) {
        if self._lifecycle.is_none() {
            self._lifecycle = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if !this
                        .update(cx, |this, cx| {
                            this.maintain_sessions(cx);
                            let active = !this.connections.is_empty()
                                || !this.closing.is_empty()
                                || !this.pending_activation.is_empty();
                            if !active {
                                this._lifecycle = None;
                            }
                            active
                        })
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
            }));
        }
    }
    pub fn unregister_document(&mut self, id: EntityId) {
        self.documents.remove(&id);
        self.document_owners.retain(|_, owner| *owner != id);
        self.pending_activation.retain(|client| client.id() != id);
    }
    pub fn request_activation(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.shutting_down || !cx.ai_enabled() {
            return;
        }
        if !self
            .pending_activation
            .iter()
            .any(|pending| pending.id() == client.id())
        {
            self.pending_activation.push_back(client);
        }
        self.start_session_maintenance(cx);
        self.maintain_sessions(cx);
    }
    fn maintain_sessions(&mut self, cx: &mut Context<Self>) {
        if self.shutting_down || !cx.ai_enabled() {
            self.pending_activation.clear();
            return;
        }
        let policy = cx.ai().sessions.clone();
        self.release_idle(&policy, cx);
        self.admit_pending(&policy, cx);
    }
    /// Tells idle connections past the policy, or needed by a waiting chat,
    /// to yield.
    fn release_idle(&mut self, policy: &nocterm_ai::AgentSessionSettings, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let mut idle = Vec::new();
        self.yielding
            .retain(|key| self.connections.contains_key(key));
        for (key, connection) in &self.connections {
            if self.yielding.contains(key) {
                continue;
            }
            let states = connection
                .users
                .values()
                .filter_map(|client| client.state(cx))
                .collect::<Vec<_>>();
            if states.iter().any(|state| state.busy) {
                self.idle_since.remove(key);
            } else {
                idle.push(IdleConnection {
                    key: *key,
                    since: *self.idle_since.entry(*key).or_insert(now),
                    shown: states.iter().any(|state| state.shown),
                });
            }
        }
        // Slots already on their way back: connections told to yield and
        // closes still in progress. Without counting them, every tick until
        // a close finished would evict another idle session.
        let freeing = self.yielding.len()
            + self
                .closing
                .keys()
                .filter(|key| !self.stalled_closes.contains(key))
                .count();
        let waiting = self
            .pending_activation
            .iter()
            .filter(|client| {
                client
                    .state(cx)
                    .is_some_and(|state| state.activation_pending && !state.leased)
            })
            .count();
        let needs_slot =
            waiting > freeing && self.connections.len() + self.closing.len() >= policy.max_live;
        for key in releases(&idle, policy, now, needs_slot) {
            self.yielding.insert(key);
            let clients: Vec<_> = self
                .connections
                .get(&key)
                .into_iter()
                .flat_map(|connection| connection.users.values().cloned())
                .collect();
            for client in clients {
                client.emit(SessionEvent::Idle { connection: key }, cx);
            }
        }
    }
    /// Connects waiting chats, oldest first, while slots are free.
    fn admit_pending(&mut self, policy: &nocterm_ai::AgentSessionSettings, cx: &mut Context<Self>) {
        let mut pending = self.pending_activation.len();
        while pending > 0 && self.connections.len() + self.closing.len() < policy.max_live {
            pending -= 1;
            let Some(client) = self.pending_activation.pop_front() else {
                break;
            };
            let Some(state) = client.state(cx) else {
                continue;
            };
            if self.document_owners.get(&state.chat_id) != Some(&client.id())
                || state.leased
                || !state.activation_pending
            {
                continue;
            }
            if state.closing
                || self
                    .connections
                    .values()
                    .any(|connection| connection.chat_id == state.chat_id)
                || self.closing.values().any(|chat| chat == &state.chat_id)
            {
                self.pending_activation.push_back(client);
                continue;
            }
            if let Some(launch) = nocterm_ai::AgentRegistry::new(cx.ai())
                .get(&state.agent_id)
                .cloned()
            {
                self.connect(client, state.chat_id, launch, cx);
            } else {
                client.emit(SessionEvent::AgentRemoved, cx);
            }
        }
    }
    fn cleanup_failed(&mut self, chat_id: &str, error: &str, cx: &mut Context<Self>) {
        tracing::warn!(%error, "agent process cleanup could not be confirmed");
        let clients: Vec<_> = self
            .documents
            .values()
            .filter(|client| {
                client
                    .state(cx)
                    .is_some_and(|state| state.chat_id == chat_id)
            })
            .cloned()
            .collect();
        for client in clients {
            client.emit(SessionEvent::CleanupUnconfirmed(error.to_owned()), cx);
        }
    }
    pub fn release_lease(&mut self, mut lease: ReleasedLease, cx: &mut Context<Self>) {
        let id = lease.owner;
        let key = lease.connection_key;
        if let Some(registration) = lease.registration.take() {
            self.registrations.remove(&registration.id);
            drop(registration);
        }
        self.idle_since.remove(&key);
        self.yielding.remove(&key);
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
        if let Some(commands) = &commands {
            self.closing_commands.insert(key, commands.clone());
        }
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
                        let _ = this.update(cx, |this, cx| {
                            this.stalled_closes.insert(key);
                            this.cleanup_failed(&chat_id, &error, cx)
                        });
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
                        this.stalled_closes.insert(key);
                        this.cleanup_failed(&chat_id, &error.to_string(), cx)
                    });
                    return; // Keep Closing counted: the process may still be alive.
                }
            }
            drop(connection);
            let _ = this.update(cx, |this, cx| {
                this.closing.remove(&key);
                this.closing_commands.remove(&key);
                if let Some(client) = this.documents.get(&id).cloned() {
                    client.emit(SessionEvent::SessionClosed, cx);
                }
                this.maintain_sessions(cx);
            });
        })
        .detach();
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Runtime {
    /// The fixture injects one event stream; production has a separate stream per connection.
    pub fn inject_events(
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
                                connection.users.values().any(|client| {
                                    client.state(cx).is_some_and(|state| {
                                        state.session.as_ref() == Some(session)
                                    })
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
