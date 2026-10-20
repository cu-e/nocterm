//! Public document ownership and repository effects across foreground cancellation.
use super::*;
use gpui::{Entity, TestAppContext};
use nocterm_ai::{
    history::SharedChat,
    testing::{InMemoryAgentRepository, ScriptedConnector, StorageOperation},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
mod closing;
mod preferences;
mod usage;
mod workspace;
struct UnusedBridge;
impl nocterm_ai::ToolBridge for UnusedBridge {
    fn register(
        &self,
    ) -> Result<nocterm_ai::BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError>
    {
        Ok(nocterm_ai::BridgeRegistration::new(
            1,
            nocterm_ai::BridgeLaunch {
                program: "unused".into(),
                args: Vec::new(),
                env: Vec::new(),
            },
            Arc::new(|_| {}),
        ))
    }
    fn calls(&self) -> async_channel::Receiver<nocterm_ai::BridgeCall> {
        async_channel::unbounded().1
    }
    fn stop(&self) {}
}
type SavedCallback = Box<dyn Fn(EntityId, &mut App)>;
struct ProbeClient {
    marker: Entity<()>,
    chat: RefCell<Arc<SharedChat>>,
    revision: Cell<u64>,
    events: RefCell<Vec<SessionEvent>>,
    on_saved: Option<SavedCallback>,
    on_admitted: RefCell<Option<SavedCallback>>,
}
impl ProbeClient {
    fn new(cx: &mut TestAppContext, chat: Arc<SharedChat>) -> Rc<Self> {
        Self::create(cx, chat, None)
    }
    fn create(
        cx: &mut TestAppContext,
        chat: Arc<SharedChat>,
        on_saved: Option<SavedCallback>,
    ) -> Rc<Self> {
        Rc::new(Self {
            marker: cx.new(|_| ()),
            chat: RefCell::new(chat),
            revision: Cell::new(1),
            events: RefCell::new(Vec::new()),
            on_saved,
            on_admitted: RefCell::new(None),
        })
    }
}
impl SessionClient for ProbeClient {
    fn id(&self) -> EntityId {
        self.marker.entity_id()
    }
    fn alive(&self) -> bool {
        true
    }
    fn emit(&self, event: SessionEvent, cx: &mut App) {
        let saved = matches!(&event, SessionEvent::Saved { .. });
        let admitted = matches!(&event, SessionEvent::SaveAdmitted { .. });
        self.events.borrow_mut().push(event);
        if saved && let Some(callback) = &self.on_saved {
            callback(self.id(), cx);
        }
        if admitted && let Some(callback) = self.on_admitted.borrow().as_ref() {
            callback(self.id(), cx);
        }
    }
    fn state(&self, _: &App) -> Option<ClientState> {
        Some(ClientState {
            chat_id: self.chat.borrow().id.clone(),
            workdir: None,
            agent_id: "codex".into(),
            session: None,
            busy: false,
            leased: false,
            activation_pending: false,
            closing: false,
            shown: false,
        })
    }
    fn snapshot(&self, _: &App) -> Option<Arc<SharedChat>> {
        Some(self.chat.borrow().clone())
    }
    fn capture(&self, _: &mut App) -> Option<(Arc<SharedChat>, u64)> {
        Some((self.chat.borrow().clone(), self.revision.get()))
    }
}
fn fixture(
    cx: &mut TestAppContext,
) -> (
    Entity<Runtime>,
    tempfile::TempDir,
    Arc<InMemoryAgentRepository>,
) {
    fixture_with_connector(cx, Arc::new(ScriptedConnector::default()))
}
fn fixture_with_connector(
    cx: &mut TestAppContext,
    connector: Arc<ScriptedConnector>,
) -> (
    Entity<Runtime>,
    tempfile::TempDir,
    Arc<InMemoryAgentRepository>,
) {
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(InMemoryAgentRepository::default());
    let storage = AgentStorage::prepared(
        repository.clone(),
        repository.clone(),
        Ok(Default::default()),
    );
    let runtime = runtime_from_storage(cx, connector, storage, directory.path());
    (runtime, directory, repository)
}
fn runtime_from_storage(
    cx: &mut TestAppContext,
    connector: Arc<ScriptedConnector>,
    storage: AgentStorage,
    directory: &std::path::Path,
) -> Entity<Runtime> {
    runtime_with_usage_source(
        cx,
        connector,
        storage,
        directory,
        Arc::new(nocterm_ai::testing::ScriptedUsageSource::default()),
        Arc::new(nocterm_ai::testing::ScriptedWorkdirResolver::accept_requests()),
    )
}
fn runtime_with_usage_source(
    cx: &mut TestAppContext,
    connector: Arc<ScriptedConnector>,
    storage: AgentStorage,
    directory: &std::path::Path,
    usage_source: Arc<dyn nocterm_ai::usage::AccountUsageSource>,
    workdir_resolver: Arc<dyn nocterm_ai::workspace::WorkdirResolver>,
) -> Entity<Runtime> {
    cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        let agents = nocterm_agent_directory::AgentsHandle::new(
            nocterm_agent_providers::catalog(),
            settings.clone(),
            cx,
        );
        cx.new(|cx| {
            Runtime::new(
                RuntimeServices {
                    settings,
                    agents,
                    connector,
                    bridge: Arc::new(UnusedBridge),
                    storage,
                    usage_source,
                    workdir_resolver,
                    workdir: directory.join("work"),
                    terminal_auth: false,
                    private_dirs: Vec::new(),
                    shared_dirs: Vec::new(),
                },
                cx,
            )
        })
    })
}

fn snapshot() -> Arc<SharedChat> {
    let metadata=nocterm_ai::history::decode(br#"{"version":1,"id":"00000000-0000-0000-0000-000000000001","agent_id":"codex","updated":1,"draft":"draft from the released window","entries":[]}"#,"00000000-0000-0000-0000-000000000001").unwrap();
    Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    })
}
fn admit(runtime: &Entity<Runtime>, client: Rc<ProbeClient>, cx: &mut TestAppContext) {
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        runtime
            .save_chat(
                client.chat.borrow().clone(),
                client.id(),
                client.revision.get(),
                cx,
            )
            .unwrap();
    });
}
#[gpui::test]
async fn shutdown_persists_a_dequeued_snapshot_after_its_thread_has_gone(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    runtime.update(cx, |runtime, cx| {
        runtime.unregister_document(client.id(), cx)
    });
    drop(client);
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    release.send(Ok(())).unwrap();
    final_work.await;
    let saved = repository.chats();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, chat.id);
    assert_eq!(saved[0].draft, chat.draft);
}
#[gpui::test]
async fn the_latest_queued_snapshot_wins_over_an_unacknowledged_disk_write(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let old = snapshot();
    let client = ProbeClient::new(cx, old.clone());
    let release = repository.block_next(StorageOperation::Save(old.id.clone()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    let mut metadata = old.metadata.clone();
    metadata.draft = Some("newer text".into());
    let latest = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    *client.chat.borrow_mut() = latest.clone();
    client.revision.set(2);
    runtime.update(cx, |runtime, cx| {
        runtime.save_chat(latest, client.id(), 2, cx).unwrap()
    });
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    release.send(Ok(())).unwrap();
    final_work.await;
    assert_eq!(repository.chats()[0].draft.as_deref(), Some("newer text"));
}
#[gpui::test]
async fn deleting_a_chat_overrides_its_unacknowledged_disk_write_at_shutdown(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, client, cx);
    cx.run_until_parked();
    runtime.update(cx, |runtime, cx| runtime.delete_chat(chat.id.clone(), cx));
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    release.send(Ok(())).unwrap();
    final_work.await;
    assert!(repository.chats().is_empty());
}
#[gpui::test]
async fn final_tombstone_waits_for_active_disk_write_and_prevents_resurrection(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, client, cx);
    cx.run_until_parked();
    runtime.update(cx, |runtime, cx| runtime.delete_chat(chat.id.clone(), cx));
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    cx.run_until_parked();
    assert!(repository.chats().is_empty());
    assert!(
        !repository
            .operations()
            .contains(&StorageOperation::Delete(chat.id.clone()))
    );
    release.send(Ok(())).unwrap();
    final_work.await;
    assert!(repository.chats().is_empty());
}
#[gpui::test]
async fn shutdown_flushes_chat_and_pending_preferences_without_app_access(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let client = ProbeClient::new(cx, snapshot());
    admit(&runtime, client, cx);
    let final_work = runtime.update(cx, |runtime, cx| {
        runtime.set_last_agent("claude", cx);
        runtime.shutdown(cx)
    });
    drop(runtime);
    final_work.await;
    assert_eq!(repository.chats()[0].draft, snapshot().draft);
    assert_eq!(repository.state().last_agent.as_deref(), Some("claude"));
}
#[gpui::test]
async fn quit_does_not_write_unedited_preferences(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    final_work.await;
    assert!(
        !repository
            .operations()
            .contains(&StorageOperation::SaveState)
    );
}

#[gpui::test]
fn one_chat_failure_does_not_block_another_owner_and_new_intent_retries(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let first = snapshot();
    let mut second = first.metadata.clone();
    second.id = "00000000-0000-0000-0000-000000000002".into();
    second.draft = Some("independent successful document".into());
    let second = Arc::new(SharedChat {
        metadata: second,
        queue: Vec::new(),
    });
    let first_client = ProbeClient::new(cx, first.clone());
    let second_client = ProbeClient::new(cx, second.clone());
    repository.fail_next(
        StorageOperation::Save(first.id.clone()),
        AgentStorageError::Interrupted,
    );
    admit(&runtime, first_client.clone(), cx);
    admit(&runtime, second_client.clone(), cx);
    cx.run_until_parked();
    assert!(repository.transcript(&first.id).is_err());
    assert_eq!(
        repository.transcript(&second.id).unwrap().draft,
        second.draft
    );
    assert!(
        first_client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::SaveFailed { .. }))
    );
    assert!(
        second_client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Saved { revision: 1 }))
    );
    let mut retry = first.metadata.clone();
    retry.draft = Some("new accepted intent".into());
    *first_client.chat.borrow_mut() = Arc::new(SharedChat {
        metadata: retry,
        queue: Vec::new(),
    });
    first_client.revision.set(2);
    admit(&runtime, first_client.clone(), cx);
    cx.run_until_parked();
    assert_eq!(
        repository.transcript(&first.id).unwrap().draft.as_deref(),
        Some("new accepted intent")
    );
    assert!(
        first_client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Saved { revision: 2 }))
    );
}

#[gpui::test]
fn an_old_owner_failure_cannot_poison_its_replacement(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let old = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, old.clone(), cx);
    cx.run_until_parked();
    let mut latest = chat.metadata.clone();
    latest.draft = Some("replacement text".into());
    let latest = Arc::new(SharedChat {
        metadata: latest,
        queue: Vec::new(),
    });
    let replacement = ProbeClient::new(cx, latest);
    replacement.revision.set(2);
    admit(&runtime, replacement.clone(), cx);
    release.send(Err(AgentStorageError::Interrupted)).unwrap();
    cx.run_until_parked();
    assert!(
        old.events
            .borrow()
            .iter()
            .all(|event| matches!(event, SessionEvent::SaveAdmitted { revision: 1 }))
    );
    assert!(
        !replacement
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::SaveFailed { .. }))
    );
    assert!(
        replacement
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Saved { revision: 2 }))
    );
    assert_eq!(
        repository.transcript(&chat.id).unwrap().draft.as_deref(),
        Some("replacement text")
    );
}

#[gpui::test]
fn publication_client_can_synchronously_retire_itself_without_reentering_worker_borrow(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let owner = runtime.clone();
    let client = ProbeClient::create(
        cx,
        snapshot(),
        Some(Box::new(move |id, cx| {
            owner.update(cx, |runtime, cx| runtime.unregister_document(id, cx));
        })),
    );
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Saved { revision: 1 }))
    );
    assert!(repository.transcript(&snapshot().id).is_ok());
    cx.update(|cx| {
        assert!(
            !runtime
                .read(cx)
                .document_is_current(&snapshot().id, client.id())
        )
    });
}

#[gpui::test]
fn an_older_failed_revision_cannot_pause_a_newer_accepted_document(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    let mut metadata = chat.metadata.clone();
    metadata.draft = Some("newer accepted text".into());
    *client.chat.borrow_mut() = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    client.revision.set(2);
    admit(&runtime, client.clone(), cx);
    release.send(Err(AgentStorageError::Interrupted)).unwrap();
    cx.run_until_parked();
    assert!(
        !client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::SaveFailed { .. }))
    );
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::Saved { revision: 2 }))
    );
    assert_eq!(
        repository.transcript(&chat.id).unwrap().draft.as_deref(),
        Some("newer accepted text")
    );
}

#[gpui::test]
fn an_active_success_acknowledges_only_its_revision_while_the_newer_write_waits(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let first = repository.block_next(StorageOperation::Save(chat.id.clone()));
    let second = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    let mut metadata = chat.metadata.clone();
    metadata.draft = Some("newer desired document".into());
    *client.chat.borrow_mut() = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    client.revision.set(2);
    admit(&runtime, client.clone(), cx);
    first.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(repository.transcript(&chat.id).unwrap().draft, chat.draft);
    assert_eq!(
        client
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                SessionEvent::Saved { revision } => Some(*revision),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [1],
        "the active prefix cannot acknowledge the still-blocked newer document",
    );
    second.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        repository.transcript(&chat.id).unwrap().draft.as_deref(),
        Some("newer desired document"),
    );
    assert_eq!(
        client
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                SessionEvent::Saved { revision } => Some(*revision),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [1, 2],
    );
}

#[gpui::test]
fn an_old_owner_success_cannot_acknowledge_its_replacements_blocked_document(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let old = ProbeClient::new(cx, chat.clone());
    let first = repository.block_next(StorageOperation::Save(chat.id.clone()));
    let second = repository.block_next(StorageOperation::Save(chat.id.clone()));
    admit(&runtime, old.clone(), cx);
    cx.run_until_parked();
    let mut metadata = chat.metadata.clone();
    metadata.draft = Some("replacement desired document".into());
    let replacement = ProbeClient::new(
        cx,
        Arc::new(SharedChat {
            metadata,
            queue: Vec::new(),
        }),
    );
    replacement.revision.set(2);
    admit(&runtime, replacement.clone(), cx);
    first.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(repository.transcript(&chat.id).unwrap().draft, chat.draft);
    assert!(
        old.events
            .borrow()
            .iter()
            .all(|event| matches!(event, SessionEvent::SaveAdmitted { revision: 1 }))
    );
    assert!(
        replacement
            .events
            .borrow()
            .iter()
            .all(|event| matches!(event, SessionEvent::SaveAdmitted { revision: 2 }))
    );
    second.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert!(
        old.events
            .borrow()
            .iter()
            .all(|event| matches!(event, SessionEvent::SaveAdmitted { revision: 1 }))
    );
    assert_eq!(
        replacement
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                SessionEvent::Saved { revision } => Some(*revision),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [2],
    );
    assert_eq!(
        repository.transcript(&chat.id).unwrap().draft.as_deref(),
        Some("replacement desired document"),
    );
}

#[gpui::test]
fn a_delayed_offer_cannot_replace_an_already_admitted_newer_revision(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let old = snapshot();
    let client = ProbeClient::new(cx, old.clone());
    let mut metadata = old.metadata.clone();
    metadata.draft = Some("newer admitted draft".into());
    let latest = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    *client.chat.borrow_mut() = latest.clone();
    client.revision.set(2);
    admit(&runtime, client.clone(), cx);
    let accepted = runtime.update(cx, |runtime, cx| {
        runtime.save_chat(old.clone(), client.id(), 1, cx).unwrap()
    });
    assert!(
        !accepted,
        "revision one cannot revoke an already admitted revision two"
    );
    cx.run_until_parked();
    assert_eq!(repository.transcript(&old.id).unwrap().draft, latest.draft);
    assert!(!client.events.borrow().iter().any(|event| matches!(
        event,
        SessionEvent::SaveAdmitted { revision: 1 } | SessionEvent::Saved { revision: 1 }
    )));
}

#[gpui::test]
async fn stale_closing_capture_preserves_the_newer_writer_owned_candidate(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let old = snapshot();
    let client = ProbeClient::new(cx, old.clone());
    let mut metadata = old.metadata.clone();
    metadata.draft = Some("writer owned newest".into());
    let latest = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        assert!(
            runtime
                .save_chat(latest.clone(), client.id(), 2, cx)
                .unwrap()
        );
    });
    let final_work = runtime.update(cx, |runtime, cx| runtime.capture_shutdown(cx));
    final_work.await.unwrap();
    assert_eq!(repository.transcript(&old.id).unwrap().draft, latest.draft);
}

#[gpui::test]
fn a_detached_capture_acknowledges_its_original_client_outside_the_runtime_borrow(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let client = ProbeClient::new(cx, snapshot());
    let callback_runtime = runtime.clone();
    let reentered = Rc::new(Cell::new(false));
    let observed = reentered.clone();
    *client.on_admitted.borrow_mut() = Some(Box::new(move |id, cx| {
        callback_runtime.update(cx, |runtime, cx| runtime.unregister_document(id, cx));
        observed.set(true);
    }));
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        assert!(
            runtime
                .capture_and_detach_document(client.id(), cx)
                .unwrap()
        );
        assert!(
            !reentered.get(),
            "admission delivery is deferred until the Runtime borrow ends"
        );
    });
    cx.run_until_parked();
    assert!(reentered.get());
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::SaveAdmitted { revision: 1 }))
    );
    assert_eq!(
        repository.transcript(&snapshot().id).unwrap().draft,
        snapshot().draft
    );
}
