//! One admission queue and resource policy for every window.
use super::admission::ConnectionObservation;
use super::lease::ReleasedLease;
use super::*;
use std::time::Duration;
impl Runtime {
    fn start_session_maintenance(&mut self, cx: &mut Context<Self>) {
        if !self.admission.running() {
            self.admission.install(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if !this
                        .update(cx, |this, cx| {
                            this.maintain_sessions(cx);
                            let active =
                                this.connections.active() || this.admission.pending_len() > 0;
                            if !active {
                                this.admission.idle_task();
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
    pub fn unregister_document(&mut self, id: EntityId, cx: &mut Context<Self>) {
        self.chats.retire(id, cx);
        self.admission.withdraw(id);
        cx.notify();
    }
    pub(crate) fn unregister_scoped_document(
        &mut self,
        scope: nocterm_ai::history::ContextId,
        id: EntityId,
        cx: &mut Context<Self>,
    ) -> Result<(), nocterm_ai::storage::AgentStorageError> {
        self.chats.require_owner_scope(scope, id, cx)?;
        self.unregister_document(id, cx);
        Ok(())
    }
    pub(crate) fn request_scoped_activation(
        &mut self,
        scope: nocterm_ai::history::ContextId,
        client: Client,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::AgentActivationError> {
        let state = client
            .state(cx)
            .ok_or(crate::AgentActivationError::ClientClosed)?;
        state.origin().require_scope(scope)?;
        documents::ChatStore::require_client_scope(scope, &client, cx)?;
        self.request_activation(client, cx);
        Ok(())
    }
    pub fn request_activation(&mut self, client: Client, cx: &mut Context<Self>) {
        if self.shutting_down || !self.ai_enabled(cx) {
            return;
        }
        if !client
            .state(cx)
            .is_some_and(|state| self.chats.current(state.origin().key(), client.id()))
        {
            return;
        }
        self.admission.request(client);
        self.start_session_maintenance(cx);
        self.maintain_sessions(cx);
    }
    fn maintain_sessions(&mut self, cx: &mut Context<Self>) {
        if self.shutting_down || !self.ai_enabled(cx) {
            self.admission.clear();
            return;
        }
        let policy = self.ai(cx).sessions.clone();
        self.release_idle(&policy, cx);
        self.admit_pending(&policy, cx);
    }
    /// Tells idle connections past the policy, or needed by a waiting chat,
    /// to yield.
    fn release_idle(&mut self, policy: &nocterm_ai::AgentSessionSettings, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let observations = self
            .connections
            .iter()
            .map(|(key, connection)| {
                let states = connection
                    .users
                    .values()
                    .filter_map(|client| client.state(cx))
                    .collect::<Vec<_>>();
                ConnectionObservation::new(
                    *key,
                    states.iter().any(|state| state.busy()),
                    states.iter().any(|state| state.shown()),
                )
            })
            .collect::<Vec<_>>();
        let closing = self.connections.closing_keys().copied().collect::<Vec<_>>();
        let waiting = self
            .admission
            .pending()
            .filter(|client| {
                client
                    .state(cx)
                    .is_some_and(|state| state.activation_pending() && !state.leased())
            })
            .count();
        for key in self
            .admission
            .decisions(&observations, &closing, policy, now, waiting)
        {
            let clients: Vec<_> = self
                .connections
                .get(key)
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
        let mut pending = self.admission.pending_len();
        while pending > 0 && self.connections.occupied() < policy.max_live {
            pending -= 1;
            let Some(client) = self.admission.pop() else {
                break;
            };
            let Some(state) = client.state(cx) else {
                continue;
            };
            if !self.chats.current(state.origin().key(), client.id())
                || state.leased()
                || !state.activation_pending()
            {
                continue;
            }
            if state.closing() || self.connections.same_chat(state.origin().key()) {
                self.admission.defer(client);
                continue;
            }
            if let Some(agent) = self.services.agents.get(state.agent_id(), cx) {
                if let Err(error) = self.connect_scoped(
                    state.origin().scope(),
                    client.clone(),
                    state.origin().key().id().to_owned(),
                    agent.launch().clone(),
                    cx,
                ) {
                    client.emit(SessionEvent::Failed(error.to_string()), cx);
                }
            } else {
                client.emit(SessionEvent::AgentRemoved, cx);
            }
        }
    }
    fn cleanup_failed(
        &mut self,
        chat_id: &nocterm_ai::execution_origin::ChatKey,
        error: &str,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(%error, "agent process cleanup could not be confirmed");
        let clients: Vec<_> = self
            .chats
            .clients()
            .into_iter()
            .filter(|client| {
                client
                    .state(cx)
                    .is_some_and(|state| state.origin().key() == chat_id)
            })
            .collect();
        for client in clients {
            client.emit(SessionEvent::CleanupUnconfirmed(error.to_owned()), cx);
        }
    }
    pub fn release_lease(&mut self, mut lease: ReleasedLease, cx: &mut Context<Self>) {
        let id = lease.owner;
        let key = lease.connection_key;
        if let Some(registration) = lease.registration.take() {
            self.bridge.remove(registration.id);
            drop(registration);
        }
        self.admission.released(key);
        let Some((mut connection, commands)) =
            self.connections.release(key, id, lease.commands.take())
        else {
            return;
        };
        let chat_id = connection.identity.key().clone();
        let connecting = connection._connecting.take();
        let startup_completion = connection.startup_completion.take();
        cx.spawn(async move |this, cx| {
            if commands.is_none()
                && let Some(connecting) = connecting
            {
                connecting.await;
                if let Some(completion) = startup_completion {
                    let result = completion.await;
                    if let Err(error) = result {
                        let _ = this.update(cx, |this, cx| {
                            this.admission.failed_close(key);
                            this.cleanup_failed(&chat_id, &error.to_string(), cx)
                        });
                        return;
                    }
                }
            }
            if let Some(cleanup) = commands {
                let commands = cleanup.commands();
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
                if let Err(error) = cleanup.join().await {
                    let _ = this.update(cx, |this, cx| {
                        this.admission.failed_close(key);
                        this.cleanup_failed(&chat_id, &error.to_string(), cx)
                    });
                    return; // Keep Closing counted: the process may still be alive.
                }
            }
            drop(connection);
            let _ = this.update(cx, |this, cx| {
                this.connections.closed(key);
                if let Some(client) = this.chats.client(id) {
                    client.emit(SessionEvent::SessionClosed, cx);
                }
                this.maintain_sessions(cx);
            });
        })
        .detach();
    }
}

impl Runtime {
    /// Capture every UI-dependent transition now; returned work is App-free.
    pub fn capture_shutdown(
        &mut self,
        cx: &mut Context<Self>,
    ) -> BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
        if self.shutting_down {
            return Box::pin(async { Ok(()) });
        }
        self.shutting_down = true;
        self.admission.stop();
        self.chats.begin_shutdown();
        for client in self.chats.clients() {
            client.emit(SessionEvent::Shutdown, cx)
        }
        for released in self.releases.drain() {
            self.release_lease(released, cx)
        }
        self.releases.stop();
        let chats = self.chats.capture(cx);
        let state = self.state.capture(cx);
        let cleanup = self.connections.take_cleanup();
        self.bridge.stop();
        self.services.bridge.stop();
        self.usage.stop();
        let executor = cx.background_executor().clone();
        Box::pin(async move {
            // Independent process cleanup cannot be serialized behind synchronous file I/O.
            let chats = executor.spawn(chats);
            let state = executor.spawn(state);
            let cleanup = executor.spawn(cleanup.run());
            let (chats, state, cleanup) = futures::future::join3(chats, state, cleanup).await;
            let mut failure = None;
            for result in [chats, state, cleanup] {
                if let Err(error) = result {
                    failure = Some(match failure {
                        None => error,
                        Some(first) => {
                            nocterm_store::StoreError::joined(Arc::new(first), Arc::new(error))
                        }
                    })
                }
            }
            failure.map_or(Ok(()), Err)
        })
    }
    /// Immediate host/test drain; application quit supplies its shared coordinator budget.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let work = self.capture_shutdown(cx);
        cx.background_executor().spawn(async move {
            if let Err(error) = work.await {
                tracing::warn!(%error,"agent final drain failed")
            }
        })
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
        let task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let _ = this.update(cx, |this, cx| {
                    let session = match &event {
                        AgentEvent::Session(notification) => Some(&notification.session),
                        AgentEvent::Permission { request, .. } => Some(&request.session),
                        _ => None,
                    };
                    let owner = this
                        .connections
                        .iter()
                        .find(|(_, connection)| {
                            session.is_none_or(|session| {
                                connection.users.values().any(|client| {
                                    client.state(cx).is_some_and(|state| {
                                        state.session().as_ref() == Some(session)
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
        });
        self.bridge.install_test_events(task);
    }
}
