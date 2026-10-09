//! Exercise final disk ownership at the boundary between dequeue and acknowledgement.
use super::*;
use gpui::{AppContext as _, TestAppContext};
use nocterm_ai::history::{SavedChat, SharedChat};

mod closing;

struct UnusedConnector;
impl nocterm_ai::AgentConnector for UnusedConnector {
    fn connect(
        &self,
        _: nocterm_ai::ConnectRequest,
    ) -> futures::future::BoxFuture<
        'static,
        Result<nocterm_ai::AgentConnection, nocterm_ai::AgentError>,
    > {
        Box::pin(async { Err(nocterm_ai::AgentError::Io("history-only fixture".into())) })
    }
}

struct UnusedBridge;
impl nocterm_ai::ToolBridge for UnusedBridge {
    fn register(&self) -> Result<nocterm_ai::BridgeRegistration, String> {
        Err("history-only fixture".into())
    }
    fn calls(&self) -> async_channel::Receiver<nocterm_ai::BridgeCall> {
        async_channel::unbounded().1
    }
    fn stop(&self) {}
}

fn fixture(cx: &mut TestAppContext) -> (Entity<Runtime>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = cx.update(|cx| {
        cx.set_global(AiSettingsSource(|_| {
            static AI: std::sync::LazyLock<nocterm_ai::AiSettings> =
                std::sync::LazyLock::new(Default::default);
            &AI
        }));
        cx.new(|cx| {
            Runtime::new(
                RuntimeServices {
                    connector: Arc::new(UnusedConnector),
                    bridge: Arc::new(UnusedBridge),
                    state_file: directory.path().join("agents.toml"),
                    chats_dir: directory.path().join("chats"),
                    codex_home: None,
                    workdir: directory.path().join("work"),
                    terminal_auth: false,
                    private_dirs: Vec::new(),
                    shared_dirs: Vec::new(),
                },
                cx,
            )
        })
    });
    (runtime, directory)
}

fn snapshot() -> Arc<SharedChat> {
    let mut metadata = SavedChat::new("codex".into());
    metadata.draft = Some("draft from the released window".into());
    Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    })
}

#[gpui::test]
fn shutdown_persists_a_dequeued_snapshot_after_its_thread_has_gone(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let chat = snapshot();
    let flush = runtime.update(cx, |runtime, cx| {
        // No live model can supply a replacement. The writer dequeued this
        // snapshot, but its disk operation has not been acknowledged yet.
        assert!(runtime.documents.is_empty());
        runtime.chat_in_flight = Some(PendingChatWrite {
            id: chat.id.clone(),
            chat: Some(chat.clone()),
            revision: None,
        });
        runtime.flush_chats(cx)
    });
    cx.foreground_executor().block_test(flush);
    let saved = nocterm_ai::history::load_all(&directory.path().join("chats"));
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, chat.id);
    assert_eq!(saved[0].draft, chat.draft);
}

#[gpui::test]
fn the_latest_queued_snapshot_wins_over_an_unacknowledged_disk_write(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let old = snapshot();
    let mut metadata = old.metadata.clone();
    metadata.draft = Some("newer text".into());
    let latest = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    let flush = runtime.update(cx, |runtime, cx| {
        runtime.chat_in_flight = Some(PendingChatWrite {
            id: old.id.clone(),
            chat: Some(old.clone()),
            revision: None,
        });
        let owner = cx.entity_id();
        runtime.document_owners.insert(latest.id.clone(), owner);
        runtime.save_chat(latest.clone(), owner, 1, cx);
        runtime.flush_chats(cx)
    });
    cx.foreground_executor().block_test(flush);
    let saved = nocterm_ai::history::load_all(&directory.path().join("chats"));
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, old.id);
    assert_eq!(saved[0].draft.as_deref(), Some("newer text"));
}

#[gpui::test]
fn deleting_a_chat_overrides_its_unacknowledged_disk_write_at_shutdown(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let chat = snapshot();
    let path = directory.path().join("chats");
    nocterm_ai::history::save_shared(&path, &chat).unwrap();
    let flush = runtime.update(cx, |runtime, cx| {
        runtime.chat_in_flight = Some(PendingChatWrite {
            id: chat.id.clone(),
            chat: Some(chat.clone()),
            revision: None,
        });
        runtime.delete_chat(chat.id.clone(), cx);
        runtime.flush_chats(cx)
    });
    cx.foreground_executor().block_test(flush);
    assert!(!path.join(format!("{}.json", chat.id)).exists());
    assert!(nocterm_ai::history::load_all(&path).is_empty());
}

#[gpui::test]
async fn final_tombstone_waits_for_active_disk_write_and_prevents_resurrection(
    cx: &mut TestAppContext,
) {
    let (runtime, directory) = fixture(cx);
    let chat = snapshot();
    let path = directory.path().join("chats");
    let gate = runtime.read_with(cx, |runtime, _| runtime.chat_io.clone());
    let (release, receive) = futures::channel::oneshot::channel();
    let old_chat = chat.clone();
    let old_path = path.clone();
    let active = cx.background_executor.spawn(async move {
        let _guard = gate.normal().await.unwrap();
        let _ = receive.await;
        nocterm_ai::history::save_shared(&old_path, &old_chat).unwrap();
    });
    cx.run_until_parked();
    let final_write = runtime.update(cx, |runtime, cx| {
        runtime.delete_chat(chat.id.clone(), cx);
        runtime.flush_chats(cx)
    });
    cx.run_until_parked();
    assert!(
        !path.exists(),
        "capturing quit must not serialize or write on the caller"
    );
    release.send(()).unwrap();
    active.await;
    final_write.await;
    cx.run_until_parked();
    assert!(!path.join(format!("{}.json", chat.id)).exists());
}

#[gpui::test]
fn real_quit_flushes_chat_and_pending_preferences_without_app_access(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let chat = snapshot();
    runtime.update(cx, |runtime, cx| {
        runtime
            .chat_writes
            .insert(chat.id.clone(), Some(chat.clone()));
        runtime.set_last_agent("claude", cx);
    });
    cx.update(|cx| {
        cx.on_app_quit(move |cx| runtime.update(cx, |runtime, cx| runtime.shutdown(cx)))
            .detach();
    });
    cx.quit();
    let saved = nocterm_ai::history::load_all(&directory.path().join("chats"));
    assert_eq!(saved[0].draft, chat.draft);
    let favorites =
        nocterm_ai::favorites::AgentStateFile::load(&directory.path().join("agents.toml")).unwrap();
    assert_eq!(favorites.last_agent.as_deref(), Some("claude"));
}

#[gpui::test]
fn quit_does_not_replace_unread_invalid_preferences_with_defaults(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let file = directory.path().join("agents.toml");
    std::fs::write(&file, "invalid = = =").unwrap();
    let task = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    cx.foreground_executor().block_test(task);
    assert_eq!(std::fs::read_to_string(file).unwrap(), "invalid = = =");
}
