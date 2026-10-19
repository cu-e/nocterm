//! Public document ownership and repository effects across foreground cancellation.
use super::*;
use gpui::{Entity, TestAppContext};
use nocterm_ai::{
    history::{ChatWrite, SavedChat, SharedChat},
    model::retention::RetentionAuthority,
    testing::{InMemoryAgentRepository, ScriptedConnector, StorageOperation},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
mod closing;
mod events;
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
    chat: RefCell<Arc<ChatWrite>>,
    document: RefCell<nocterm_ai::thread::ThreadState>,
    revision: Cell<u64>,
    events: RefCell<Vec<SessionEvent>>,
    on_saved: Option<SavedCallback>,
    on_admitted: RefCell<Option<SavedCallback>>,
}
impl ProbeClient {
    fn new(cx: &mut TestAppContext, chat: Arc<ChatWrite>) -> Rc<Self> {
        Self::create(cx, chat, None)
    }
    fn create(
        cx: &mut TestAppContext,
        chat: Arc<ChatWrite>,
        on_saved: Option<SavedCallback>,
    ) -> Rc<Self> {
        let saved = shared(&chat).snapshot();
        let mut document = nocterm_ai::thread::ThreadState::new(
            &authority(),
            &saved.metadata().agent_id,
            saved.id(),
            saved.metadata().updated,
        )
        .unwrap();
        import_fixture(&mut document, &chat);
        Rc::new(Self {
            marker: cx.new(|_| ()),
            chat: RefCell::new(chat),
            document: RefCell::new(document),
            revision: Cell::new(1),
            events: RefCell::new(Vec::new()),
            on_saved,
            on_admitted: RefCell::new(None),
        })
    }
    fn save(
        &self,
        writer: &DocumentWriter,
        chat: &ChatWrite,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        let mut document = self.document.borrow_mut();
        import_fixture(&mut document, chat);
        document.prepare_candidate().unwrap();
        writer.save(
            document
                .candidate(nocterm_ai::history::writing::ManagedWriteIntent::Snapshot)
                .unwrap(),
            self.id(),
            revision,
            cx,
        )
    }
}

/// Exercise the existing repository fake and the document's original Reader;
/// fixtures remain ordinary test inputs rather than a production DTO save port.
fn import_fixture(document: &mut nocterm_ai::thread::ThreadState, chat: &ChatWrite) {
    use nocterm_ai::storage::ChatRepository as _;
    let repository = repository();
    repository.seed_chat(shared(chat).snapshot()).unwrap();
    let id = document.live_mut().prepare_reader_id().unwrap();
    let operation = repository
        .load_managed(id, document.reader().capture().unwrap())
        .unwrap();
    let delivery = futures::executor::block_on(operation);
    assert!(delivery.error().is_none());
    document.live_mut().install_delivery(delivery).unwrap();
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
            chat_id: self.chat.borrow().id().to_owned(),
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
    fn prepare_startup(
        &self,
        request: nocterm_ai::connection::ManagedStartupRequest<'_>,
        _: &mut App,
    ) -> Result<nocterm_ai::connection::ManagedStartup, nocterm_ai::history::source::SourceError>
    {
        let source = self.document.borrow().source().capture()?;
        let facts = source.context()?.prepare_startup(request)?;
        nocterm_ai::connection::ManagedStartup::new(facts, source)
    }
    fn capture_into(
        &self,
        admission: &CaptureAdmission,
        cx: &mut App,
    ) -> Result<Option<u64>, crate::CaptureError> {
        let revision = self.revision.get();
        let mut document = self.document.borrow_mut();
        import_fixture(&mut document, &self.chat.borrow());
        document.prepare_candidate().unwrap();
        admission
            .save(
                document
                    .candidate(nocterm_ai::history::writing::ManagedWriteIntent::Snapshot)
                    .unwrap(),
                revision,
                cx,
            )
            .map(|accepted| accepted.then_some(revision))
            .map_err(|error| crate::CaptureError::persistence(revision, error))
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
    let repository = Arc::new(repository());
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
        cx.new(|cx| {
            Runtime::new(
                RuntimeServices {
                    retention: authority(),
                    settings,
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

thread_local! {
    /// One host authority per test thread, shared by runtime, repository and fixtures.
    static AUTHORITY: RetentionAuthority = RetentionAuthority::new(usize::MAX).unwrap();
}
fn authority() -> RetentionAuthority {
    AUTHORITY.with(Clone::clone)
}
fn repository() -> InMemoryAgentRepository {
    InMemoryAgentRepository::new(authority())
}
fn snapshot() -> Arc<ChatWrite> {
    let base = SavedChat::for_identity(
        &authority(),
        "codex".into(),
        "00000000-0000-0000-0000-000000000001".into(),
        1,
    );
    let chat = Arc::new(ChatWrite::Snapshot(Box::new(
        SharedChat::from_saved(base).unwrap(),
    )));
    redraft(&chat, "draft from the released window")
}
/// The shared document carried by a snapshot write.
fn shared(chat: &ChatWrite) -> &SharedChat {
    chat.snapshot().expect("snapshot write")
}
fn draft(chat: &ChatWrite) -> Option<String> {
    shared(chat).snapshot().composer().draft.clone()
}
/// A new immutable candidate with the same content except identity/draft.
fn revise(chat: &ChatWrite, id: Option<&str>, text: &str) -> Arc<ChatWrite> {
    let source = shared(chat);
    let (old_id, metadata, resume, transcript, mut composer, _) =
        source.snapshot().clone().into_parts();
    composer.draft = Some(text.into());
    let saved = SavedChat::from_parts(
        &authority(),
        id.map_or(old_id, str::to_owned),
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    Arc::new(ChatWrite::Snapshot(Box::new(
        SharedChat::new(saved, source.prompts().to_vec()).unwrap(),
    )))
}
fn redraft(chat: &ChatWrite, text: &str) -> Arc<ChatWrite> {
    revise(chat, None, text)
}
fn admit(runtime: &Entity<Runtime>, client: Rc<ProbeClient>, cx: &mut TestAppContext) {
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        client
            .save(
                &runtime.chats.publication(),
                &client.chat.borrow(),
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
    let release = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
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
    assert_eq!(saved[0].id(), chat.id());
    assert_eq!(saved[0].composer().draft, draft(&chat));
}
#[gpui::test]
async fn the_latest_queued_snapshot_wins_over_an_unacknowledged_disk_write(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let old = snapshot();
    let client = ProbeClient::new(cx, old.clone());
    let release = repository.block_next(StorageOperation::Save(old.id().to_owned()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    let latest = redraft(&old, "newer text");
    *client.chat.borrow_mut() = latest.clone();
    client.revision.set(2);
    runtime.update(cx, |runtime, cx| {
        client
            .save(&runtime.chats.publication(), &latest, 2, cx)
            .unwrap()
    });
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    release.send(Ok(())).unwrap();
    final_work.await;
    assert_eq!(
        repository.chats()[0].composer().draft.as_deref(),
        Some("newer text")
    );
}
#[gpui::test]
async fn deleting_a_chat_overrides_its_unacknowledged_disk_write_at_shutdown(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    admit(&runtime, client, cx);
    cx.run_until_parked();
    runtime.update(cx, |runtime, cx| {
        runtime.delete_chat(chat.id().to_owned(), cx)
    });
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
    let release = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    admit(&runtime, client, cx);
    cx.run_until_parked();
    runtime.update(cx, |runtime, cx| {
        runtime.delete_chat(chat.id().to_owned(), cx)
    });
    let final_work = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    cx.run_until_parked();
    assert!(repository.chats().is_empty());
    assert!(
        !repository
            .operations()
            .contains(&StorageOperation::Delete(chat.id().to_owned()))
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
    assert_eq!(repository.chats()[0].composer().draft, draft(&snapshot()));
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
    let second = revise(
        &first,
        Some("00000000-0000-0000-0000-000000000002"),
        "independent successful document",
    );
    let first_client = ProbeClient::new(cx, first.clone());
    let second_client = ProbeClient::new(cx, second.clone());
    repository.fail_next(
        StorageOperation::Save(first.id().to_owned()),
        AgentStorageError::Interrupted,
    );
    admit(&runtime, first_client.clone(), cx);
    admit(&runtime, second_client.clone(), cx);
    cx.run_until_parked();
    assert!(repository.transcript(first.id()).is_err());
    assert_eq!(
        repository.transcript(second.id()).unwrap().composer().draft,
        draft(&second)
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
    *first_client.chat.borrow_mut() = redraft(&first, "new accepted intent");
    first_client.revision.set(2);
    admit(&runtime, first_client.clone(), cx);
    cx.run_until_parked();
    assert_eq!(
        repository
            .transcript(first.id())
            .unwrap()
            .composer()
            .draft
            .as_deref(),
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
    let release = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    admit(&runtime, old.clone(), cx);
    cx.run_until_parked();
    let latest = redraft(&chat, "replacement text");
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
        repository
            .transcript(chat.id())
            .unwrap()
            .composer()
            .draft
            .as_deref(),
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
    assert!(repository.transcript(snapshot().id()).is_ok());
    cx.update(|cx| {
        assert!(
            !runtime
                .read(cx)
                .document_is_current(snapshot().id(), client.id())
        )
    });
}

#[gpui::test]
fn an_older_failed_revision_cannot_pause_a_newer_accepted_document(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let release = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    *client.chat.borrow_mut() = redraft(&chat, "newer accepted text");
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
        repository
            .transcript(chat.id())
            .unwrap()
            .composer()
            .draft
            .as_deref(),
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
    let first = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    let second = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    admit(&runtime, client.clone(), cx);
    cx.run_until_parked();
    *client.chat.borrow_mut() = redraft(&chat, "newer desired document");
    client.revision.set(2);
    admit(&runtime, client.clone(), cx);
    first.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        repository.transcript(chat.id()).unwrap().composer().draft,
        draft(&chat)
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
        [1],
        "the active prefix cannot acknowledge the still-blocked newer document",
    );
    second.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        repository
            .transcript(chat.id())
            .unwrap()
            .composer()
            .draft
            .as_deref(),
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
    let first = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    let second = repository.block_next(StorageOperation::Save(chat.id().to_owned()));
    admit(&runtime, old.clone(), cx);
    cx.run_until_parked();
    let replacement = ProbeClient::new(cx, redraft(&chat, "replacement desired document"));
    replacement.revision.set(2);
    admit(&runtime, replacement.clone(), cx);
    first.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        repository.transcript(chat.id()).unwrap().composer().draft,
        draft(&chat)
    );
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
        repository
            .transcript(chat.id())
            .unwrap()
            .composer()
            .draft
            .as_deref(),
        Some("replacement desired document"),
    );
}

#[gpui::test]
fn a_delayed_offer_cannot_replace_an_already_admitted_newer_revision(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let old = snapshot();
    let client = ProbeClient::new(cx, old.clone());
    let latest = redraft(&old, "newer admitted draft");
    *client.chat.borrow_mut() = latest.clone();
    client.revision.set(2);
    admit(&runtime, client.clone(), cx);
    let accepted = runtime.update(cx, |runtime, cx| {
        client
            .save(&runtime.chats.publication(), &old, 1, cx)
            .unwrap()
    });
    assert!(
        !accepted,
        "revision one cannot revoke an already admitted revision two"
    );
    cx.run_until_parked();
    assert_eq!(
        repository.transcript(old.id()).unwrap().composer().draft,
        draft(&latest)
    );
    assert!(!client.events.borrow().iter().any(|event| matches!(
        event,
        SessionEvent::SaveAdmitted { revision: 1 } | SessionEvent::Saved { revision: 1 }
    )));
}
