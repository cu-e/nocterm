//! Capture on the application thread; durable work never reads the application.
use super::*;

impl Runtime {
    pub fn shutdown(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.shutting_down = true;
        self.chat_io.freeze();
        self.favorites_io.freeze();
        self.pending_activation.clear();
        let clients: Vec<_> = self.documents.values().cloned().collect();
        for client in clients {
            client.emit(SessionEvent::Shutdown, cx);
        }
        let chats = self.flush_chats(cx);
        let favorites = (self.favorites_revision > 0).then(|| self.favorites.clone());
        let favorites_path = self.services.state_file.clone();
        let favorites_gate = self.favorites_io.clone();
        let mut connections = std::mem::take(&mut self.connections);
        for connection in connections.values_mut() {
            connection.cancellation.cancel();
            connection._connecting.take();
            connection._events.take();
        }
        let mut commands: Vec<_> = connections
            .into_values()
            .filter_map(|mut connection| connection.commands.take())
            .collect();
        for closing in std::mem::take(&mut self.closing_commands).into_values() {
            if !commands
                .iter()
                .any(|command| Arc::ptr_eq(command, &closing))
            {
                commands.push(closing);
            }
        }
        self.registrations.clear();
        self.services.bridge.stop();
        let executor = cx.background_executor().clone();
        let deadline =
            nocterm_core::persist::ShutdownDeadline::new(std::time::Duration::from_millis(180));
        // Start process cleanup independently of serialization and slow disk I/O.
        let cleanup = executor.spawn(async move {
            futures::future::join_all(commands.into_iter().map(|commands| async move {
                if let Err(error) = commands.shutdown_gracefully().await {
                    tracing::warn!(%error, "agent shutdown could not be confirmed");
                }
            }))
            .await;
        });
        executor.clone().spawn(async move {
            let durable = async move {
                chats.await;
                let _guard = favorites_gate.final_write().await;
                if let Some(favorites) = favorites
                    && let Err(error) = favorites.save(&favorites_path)
                {
                    tracing::warn!(%error, "could not save final agent preferences");
                }
            };
            let work = Box::pin(futures::future::join(durable, cleanup));
            if matches!(
                futures::future::select(work, executor.timer(deadline.remaining())).await,
                futures::future::Either::Right(_)
            ) {
                tracing::warn!(
                    "agent persistence or cleanup did not finish before shutdown deadline"
                );
            }
        })
    }
}
