//! Exercise final disk ownership at the boundary between dequeue and acknowledgement.
use super::*;
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_ai::history::{SavedChat, SharedChat};

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

#[gpui_kit::test]
fn shutdown_persists_a_dequeued_snapshot_after_its_thread_has_gone(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let chat = snapshot();
    runtime.update(cx, |runtime, cx| {
        // No live model can supply a replacement. The writer dequeued this
        // snapshot, but its disk operation has not been acknowledged yet.
        assert!(runtime.documents.is_empty());
        runtime.chat_in_flight = Some(PendingChatWrite {
            id: chat.id.clone(),
            chat: Some(chat.clone()),
            revision: None,
        });
        runtime.flush_chats(cx);
    });
    let saved = nocterm_ai::history::load_all(&directory.path().join("chats"));
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, chat.id);
    assert_eq!(saved[0].draft, chat.draft);
}

#[gpui_kit::test]
fn the_latest_queued_snapshot_wins_over_an_unacknowledged_disk_write(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let old = snapshot();
    let mut metadata = old.metadata.clone();
    metadata.draft = Some("newer text".into());
    let latest = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    runtime.update(cx, |runtime, cx| {
        runtime.chat_in_flight = Some(PendingChatWrite {
            id: old.id.clone(),
            chat: Some(old.clone()),
            revision: None,
        });
        let owner = cx.entity_id();
        runtime.document_owners.insert(latest.id.clone(), owner);
        runtime.save_chat(latest.clone(), owner, 1, cx);
        runtime.flush_chats(cx);
    });
    let saved = nocterm_ai::history::load_all(&directory.path().join("chats"));
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, old.id);
    assert_eq!(saved[0].draft.as_deref(), Some("newer text"));
}

#[gpui_kit::test]
fn deleting_a_chat_overrides_its_unacknowledged_disk_write_at_shutdown(cx: &mut TestAppContext) {
    let (runtime, directory) = fixture(cx);
    let chat = snapshot();
    let path = directory.path().join("chats");
    nocterm_ai::history::save_shared(&path, &chat).unwrap();
    runtime.update(cx, |runtime, cx| {
        runtime.chat_in_flight = Some(PendingChatWrite {
            id: chat.id.clone(),
            chat: Some(chat.clone()),
            revision: None,
        });
        runtime.delete_chat(chat.id.clone(), cx);
        runtime.flush_chats(cx);
    });
    assert!(!path.join(format!("{}.json", chat.id)).exists());
    assert!(nocterm_ai::history::load_all(&path).is_empty());
}
