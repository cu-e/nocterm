//! General consumers refuse scoped documents before ownership or process admission.
use super::*;
use nocterm_ai::{
    history::{ContextId, SavedChat},
    storage::AgentStorageError,
};

pub(super) fn project_chat() -> SavedChat {
    let mut chat = snapshot().metadata.clone();
    chat.version = 2;
    chat.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    chat
}

#[test]
fn prepared_general_history_refuses_a_project_instead_of_reassigning_it() {
    assert!(matches!(
        crate::PreparedHistory::new(vec![project_chat()]),
        Err(AgentStorageError::ScopeMismatch {
            expected: ContextId::General,
            ..
        })
    ));
}

#[gpui::test]
fn general_registration_and_direct_start_refuse_project_documents_without_native_work(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, repository) = fixture_with_connector(cx, connector.clone());
    let chat = Arc::new(SharedChat {
        metadata: project_chat(),
        queue: Vec::new(),
    });
    let client = ProbeClient::new(cx, chat.clone());
    runtime.update(cx, |runtime, cx| {
        assert!(matches!(
            ownership::register(runtime, client.clone(), cx),
            Err(crate::ChatOpenError::Storage(
                AgentStorageError::ScopeMismatch { .. }
            ))
        ));
        assert!(!runtime.document_is_current(&chat.header.id, client.id()));
        let launch = nocterm_ai::AgentLaunch {
            id: "codex".into(),
            name: "test".into(),
            command: "unused".into(),
            args: Vec::new(),
            env: Default::default(),
            inherit_env: Vec::new(),
        };
        assert!(matches!(
            runtime.connect(client.clone(), chat.header.id.clone(), launch, cx),
            Err(crate::AgentActivationError::Storage(
                AgentStorageError::ScopeMismatch { .. }
            ))
        ));
        assert!(matches!(
            runtime.save_chat(chat.clone(), client.id(), 1, cx),
            Err(AgentPersistenceError::Storage(
                AgentStorageError::ScopeMismatch { .. }
            ))
        ));
        runtime.request_activation(client.clone(), cx);
    });
    cx.run_until_parked();
    assert!(connector.requests().is_empty());
    assert!(repository.encoded(&chat.header.id).is_none());
}

#[gpui::test]
fn general_hydration_and_catalog_preserve_other_contexts_in_storage(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let project = project_chat();
    let mut general = snapshot().metadata.clone();
    general.header.id = "00000000-0000-0000-0000-000000000002".into();
    repository.seed_chat(&project).unwrap();
    repository.seed_chat(&general).unwrap();
    runtime.update(cx, |runtime, cx| runtime.reload_history(cx));
    cx.run_until_parked();
    let rows = runtime.update(cx, |runtime, _| {
        runtime.catalog_snapshot(ContextId::General).unwrap()
    });
    assert_eq!(rows.rows().len(), 1);
    assert_eq!(rows.rows()[0].id, general.header.id);
    let reader = cx.update(|cx| runtime.read(cx).history_reader());
    let (loaded, permit) = futures::executor::block_on(reader.load(project.header.id.clone()));
    assert!(matches!(loaded, Err(AgentStorageError::NotFound(_))));
    drop(permit);
    assert_eq!(
        repository
            .transcript_in_scope(project.header.scope, &project.header.id)
            .unwrap()
            .header
            .scope,
        project.header.scope
    );
}

type ScopedFixture = (
    RuntimeHandle,
    tempfile::TempDir,
    Arc<InMemoryAgentRepository>,
    Arc<ScriptedConnector>,
    Arc<nocterm_ai::testing::ScriptedWorkdirResolver>,
);
pub(super) fn scoped_fixture(cx: &mut TestAppContext) -> ScopedFixture {
    scoped_fixture_with_history(cx, None)
}
pub(super) fn scoped_fixture_with_history(
    cx: &mut TestAppContext,
    history: Option<crate::PreparedHistory>,
) -> ScopedFixture {
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(InMemoryAgentRepository::default());
    let connector = Arc::new(ScriptedConnector::default());
    let resolver = Arc::new(nocterm_ai::testing::ScriptedWorkdirResolver::accept_requests());
    let storage = AgentStorage::prepared(
        repository.clone(),
        repository.clone(),
        Ok(Default::default()),
    );
    let storage = match history {
        Some(history) => storage.with_history(history),
        None => storage,
    };
    let runtime = cx.update(|cx| {
        RuntimeHandle::new(
            runtime_services(
                connector.clone(),
                storage,
                directory.path(),
                Arc::new(nocterm_ai::testing::ScriptedUsageSource::default()),
                resolver.clone(),
                cx,
            ),
            cx,
        )
    });
    (runtime, directory, repository, connector, resolver)
}
#[gpui::test]
fn scoped_catalog_snapshots_are_repeatable_and_cannot_consume_another_context(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let mut first = snapshot().metadata.clone();
    first.header.title = Some("General caption".into());
    let mut second = project_chat();
    second.header.title = Some("Project caption".into());
    repository.seed_chat(&first).unwrap();
    repository.seed_chat(&second).unwrap();
    cx.update(|cx| {
        general.reload_history(cx);
        project.reload_history(cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let rows = general.catalog_snapshot(cx).unwrap();
        assert_eq!(rows.scope(), ContextId::General);
        assert_eq!(rows.rows().len(), 1);
        assert_eq!(rows.rows()[0].title(), "General caption");
        let other = project.catalog_snapshot(cx).unwrap();
        assert_eq!(other.scope(), second.header.scope);
        assert_eq!(other.rows()[0].title(), "Project caption");
        assert_eq!(
            general.catalog_snapshot(cx).unwrap().generation(),
            rows.generation()
        );
        assert_eq!(
            project.catalog_snapshot(cx).unwrap().generation(),
            other.generation()
        );
        assert_eq!(
            general.catalog_snapshot(cx).unwrap().rows()[0].title(),
            "General caption"
        );
    });
}
#[gpui::test]
async fn readers_keep_their_original_context_after_another_scoped_handle_is_created(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let mut first = snapshot().metadata.clone();
    first.header.title = Some("General body".into());
    let mut second = project_chat();
    second.header.title = Some("Project body".into());
    repository.seed_chat(&first).unwrap();
    repository.seed_chat(&second).unwrap();
    let reader = cx.update(|cx| project.history_reader(cx));
    let _selected_elsewhere = project.for_scope(ContextId::General);
    let (loaded, permit) = reader.load(second.header.id.clone()).await;
    let loaded = loaded.unwrap();
    assert_eq!(loaded.header.scope, second.header.scope);
    assert_eq!(loaded.header.title, second.header.title);
    drop(permit);
    let (loaded, permit) = cx
        .update(|cx| general.history_reader(cx))
        .load(first.header.id)
        .await;
    assert_eq!(loaded.unwrap().header.title, first.header.title);
    drop(permit);
}
#[gpui::test]
fn scoped_writers_and_deletion_do_not_share_a_key_for_identical_chat_ids(cx: &mut TestAppContext) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let first = snapshot();
    let second = Arc::new(SharedChat {
        metadata: project_chat(),
        queue: Vec::new(),
    });
    assert_eq!(first.header.id, second.header.id);
    let general_client = ProbeClient::new(cx, first.clone());
    let project_client = ProbeClient::new(cx, second.clone());
    let release = repository.block_next(StorageOperation::Save(first.header.id.clone()));
    cx.update(|cx| {
        ownership::register_bound(&general, general_client.clone(), cx).unwrap();
        ownership::register_bound(&project, project_client.clone(), cx).unwrap();
        assert!(
            general
                .save_chat(first.clone(), general_client.id(), 1, cx)
                .unwrap()
        );
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            project
                .save_chat(second.clone(), project_client.id(), 1, cx)
                .unwrap()
        )
    });
    cx.run_until_parked();
    assert!(repository.encoded(&first.header.id).is_none());
    // The shared Store intentionally publishes one operation at a time. The
    // second context must retain its separate queued key while the first waits.
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert!(repository.encoded(&first.header.id).is_some());
    let before = repository
        .encoded_in_scope(second.header.scope, &second.header.id)
        .unwrap();
    cx.update(|cx| general.delete_chat(first.header.id.clone(), cx));
    cx.run_until_parked();
    assert!(repository.encoded(&first.header.id).is_none());
    assert_eq!(
        repository
            .encoded_in_scope(second.header.scope, &second.header.id)
            .unwrap(),
        before
    );
}
#[gpui::test]
fn a_scoped_document_uses_the_ordinary_local_connector_without_changing_its_membership(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, connector, resolver) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let chat = Arc::new(SharedChat {
        metadata: project_chat(),
        queue: Vec::new(),
    });
    let client = ProbeClient::new(cx, chat.clone());
    cx.update(|cx| {
        ownership::register_bound(&project, client.clone(), cx).unwrap();
        project.request_activation(client.clone(), cx).unwrap();
        assert!(matches!(
            general.request_activation(client.clone(), cx),
            Err(crate::AgentActivationError::Origin(
                nocterm_ai::execution_origin::ExecutionOriginError::ScopeMismatch { .. }
            ))
        ));
        let launch = nocterm_ai::AgentLaunch {
            id: "codex".into(),
            name: "test".into(),
            command: "unused".into(),
            args: Vec::new(),
            env: Default::default(),
            inherit_env: Vec::new(),
        };
        let registration = project
            .register_bridge(&(client.clone() as Client), cx)
            .unwrap();
        assert_eq!(
            registration.launch().program,
            std::path::Path::new("unused")
        );
        project
            .connect(client.clone(), chat.header.id.clone(), launch, cx)
            .unwrap();
    });
    cx.run_until_parked();
    assert_eq!(resolver.requests().len(), 1);
    assert_eq!(connector.requests().len(), 1);
    assert_eq!(connector.requests()[0].launch.command, "unused");
    assert_eq!(
        connector.requests()[0].working_directory,
        _directory.path().join("work")
    );
    cx.update(|cx| {
        let state = client.state(cx).unwrap();
        assert_eq!(state.origin().scope(), chat.header.scope);
        assert_eq!(state.origin().key().id(), chat.header.id);
        assert!(project.document_is_current(&chat.header.id, client.id(), cx));
    });
}

fn project_scope(index: u8) -> ContextId {
    serde_json::from_value(
        serde_json::json!({"kind": "project", "id": format!("{index:02x}").repeat(16)}),
    )
    .unwrap()
}
#[gpui::test]
fn retained_catalog_generations_consume_the_installation_limit_until_dropped(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let mut retained = Vec::new();
    for _ in 0..32 {
        retained.push(cx.update(|cx| general.catalog_snapshot(cx).unwrap()));
        if retained.len() < 32 {
            cx.update(|cx| general.try_reload_history(cx).unwrap());
            cx.run_until_parked();
        }
    }
    let generation = retained.last().unwrap().generation();
    let attempts = repository.operations().len();
    cx.update(|cx| {
        assert!(matches!(
            general.try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ));
        assert_eq!(
            general.catalog_snapshot(cx).unwrap().generation(),
            generation
        );
        assert!(matches!(
            general.for_scope(project_scope(1)).try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ));
    });
    cx.run_until_parked();
    assert_eq!(repository.operations().len(), attempts);
    drop(retained.remove(0));
    cx.update(|cx| general.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    assert!(cx.update(|cx| general.catalog_snapshot(cx).unwrap().generation()) > generation);
}
#[gpui::test]
fn pending_catalog_requests_share_the_same_installation_capacity(cx: &mut TestAppContext) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    // General's initial catalog is already reserved, even before its work runs.
    cx.update(|cx| {
        for index in 1..32 {
            general
                .for_scope(project_scope(index))
                .try_reload_history(cx)
                .unwrap();
        }
        assert!(matches!(
            general.for_scope(project_scope(32)).try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ));
    });
    cx.run_until_parked();
    assert_eq!(
        repository
            .operations()
            .iter()
            .filter(|operation| **operation == StorageOperation::Catalog)
            .count(),
        32
    );
    // Completed unretained LRU entries can make room without multiplying cache size.
    cx.update(|cx| {
        general
            .for_scope(project_scope(32))
            .try_reload_history(cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert!(
        cx.update(|cx| general.for_scope(project_scope(32)).catalog_snapshot(cx))
            .is_some()
    );
}
#[gpui::test]
fn an_old_catalog_load_cannot_replace_a_newer_scope_generation(cx: &mut TestAppContext) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let project = general.for_scope(project_chat().header.scope);
    let release = repository.block_next(StorageOperation::Catalog);
    cx.update(|cx| project.reload_history(cx));
    cx.run_until_parked();
    let mut chat = project_chat();
    chat.header.title = Some("Current generation".into());
    repository.seed_chat(&chat).unwrap();
    cx.update(|cx| project.reload_history(cx));
    cx.run_until_parked();
    let latest = cx.update(|cx| project.catalog_snapshot(cx).unwrap());
    chat.header.title = Some("Late stale generation".into());
    repository.seed_chat(&chat).unwrap();
    let _ = release.send(Ok(()));
    cx.run_until_parked();
    cx.update(|cx| {
        let actual = project.catalog_snapshot(cx).unwrap();
        assert_eq!(actual.generation(), latest.generation());
        assert_eq!(actual.rows()[0].title(), "Current generation");
    });
}
#[gpui::test]
fn closed_clients_and_mismatched_chat_arguments_cannot_reach_native_ports(cx: &mut TestAppContext) {
    let (general, _directory, _, connector, resolver) = scoped_fixture(cx);
    let chat = snapshot();
    let client = ProbeClient::new(cx, chat.clone());
    let launch = nocterm_ai::AgentLaunch {
        id: "codex".into(),
        name: "test".into(),
        command: "unused".into(),
        args: Vec::new(),
        env: Default::default(),
        inherit_env: Vec::new(),
    };
    cx.update(|cx| {
        assert!(matches!(
            general.connect(client.clone(), "another-id".into(), launch.clone(), cx),
            Err(crate::AgentActivationError::Storage(
                AgentStorageError::Codec(nocterm_ai::history::HistoryCodecError::IdentityMismatch)
            ))
        ));
        client.close();
        assert!(matches!(
            general.connect(client.clone(), chat.header.id.clone(), launch, cx),
            Err(crate::AgentActivationError::ClientClosed)
        ));
        assert!(matches!(
            general.register_bridge(&(client.clone() as Client), cx),
            Err(nocterm_ai::connection::BridgeRegistrationError::ClientClosed)
        ));
    });
    cx.run_until_parked();
    assert!(connector.requests().is_empty());
    assert!(resolver.requests().is_empty());
}
#[gpui::test]
fn fresh_catalog_snapshots_hide_deleted_membership_only_in_its_original_scope(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let first = snapshot().metadata.clone();
    let second = project_chat();
    repository.seed_chat(&first).unwrap();
    repository.seed_chat(&second).unwrap();
    cx.update(|cx| project.reload_history(cx));
    cx.run_until_parked();
    let before = cx.update(|cx| general.catalog_snapshot(cx).unwrap());
    assert_eq!(before.rows().len(), 1);
    cx.update(|cx| general.delete_chat(first.header.id.clone(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(general.catalog_snapshot(cx).unwrap().rows().is_empty());
        assert_eq!(project.catalog_snapshot(cx).unwrap().rows().len(), 1);
    });
    assert_eq!(before.rows().len(), 1); // An already-issued snapshot stays immutable.
    assert!(
        repository
            .encoded_in_scope(second.header.scope, &second.header.id)
            .is_some()
    );
}
#[gpui::test]
fn a_bound_handle_cannot_retire_or_capture_another_contexts_live_document(cx: &mut TestAppContext) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let chat = Arc::new(SharedChat {
        metadata: project_chat(),
        queue: Vec::new(),
    });
    let client = ProbeClient::new(cx, chat.clone());
    cx.update(|cx| {
        ownership::register_bound(&project, client.clone(), cx).unwrap();
        assert!(matches!(
            general.capture_and_detach_document(client.id(), cx),
            Err(AgentPersistenceError::Storage(
                AgentStorageError::ScopeMismatch {
                    expected: ContextId::General,
                    ..
                }
            ))
        ));
        assert!(matches!(
            general.unregister_document(client.id(), cx),
            Err(AgentStorageError::ScopeMismatch { .. })
        ));
        assert!(project.document_is_current(&chat.header.id, client.id(), cx));
    });
    cx.run_until_parked();
    assert!(
        repository
            .operations()
            .iter()
            .all(|operation| !matches!(operation, StorageOperation::Save(_)))
    );
}
#[gpui::test]
fn retained_catalog_rows_hold_the_original_accounted_payload_without_cloning_summaries(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let mut chat = snapshot().metadata.clone();
    chat.header.title = Some("Original row".into());
    repository.seed_chat(&chat).unwrap();
    cx.run_until_parked();
    let first = cx.update(|cx| general.catalog_snapshot(cx).unwrap());
    assert!(first.row(1).is_none());
    let row = first.row(0).unwrap();
    assert_eq!(row.scope(), ContextId::General);
    let generation = row.generation();
    drop(first);
    let mut retained = Vec::new();
    for index in 1..32 {
        let context = general.for_scope(project_scope(index));
        cx.update(|cx| context.try_reload_history(cx).unwrap());
        cx.run_until_parked();
        retained.push(cx.update(|cx| context.catalog_snapshot(cx).unwrap()));
    }
    cx.update(|cx| {
        assert!(matches!(
            general.for_scope(project_scope(32)).try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ))
    });
    assert_eq!(row.generation(), generation);
    assert_eq!(row.summary().title(), "Original row");
    drop(row);
    cx.update(|cx| {
        general
            .for_scope(project_scope(32))
            .try_reload_history(cx)
            .unwrap()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn prepared_payload_handoff_requires_its_original_claim_and_leaves_other_records_available(
    cx: &mut TestAppContext,
) {
    let first = snapshot().metadata.clone();
    let mut second = first.clone();
    second.header.id = "00000000-0000-0000-0000-000000000002".into();
    let history = crate::PreparedHistory::new(vec![first.clone(), second.clone()]).unwrap();
    let (general, _directory, _, _, _) = scoped_fixture_with_history(cx, Some(history));
    let (other_runtime, _other_directory, _, _, _) = scoped_fixture(cx);
    let window = cx.new(|_| ());
    cx.update(|cx| {
        assert_eq!(general.catalog_snapshot(cx).unwrap().rows().len(), 2);
        let claim = ownership::owned_claim(
            general
                .claim_document(first.header.id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        assert!(matches!(
            other_runtime.take_prepared_chat(&claim, cx),
            Err(crate::ChatOpenError::StaleClaim(_))
        ));
        assert!(matches!(
            general
                .for_scope(project_scope(1))
                .take_prepared_chat(&claim, cx),
            Err(crate::ChatOpenError::Storage(
                AgentStorageError::ScopeMismatch { .. }
            ))
        ));
        assert_eq!(
            general
                .take_prepared_chat(&claim, cx)
                .unwrap()
                .unwrap()
                .draft,
            first.draft
        );
        assert!(general.take_prepared_chat(&claim, cx).unwrap().is_none());
        let other = ownership::owned_claim(
            general
                .claim_document(second.header.id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        assert_eq!(
            general
                .take_prepared_chat(&other, cx)
                .unwrap()
                .unwrap()
                .header
                .id,
            second.header.id
        );
    });
}

#[gpui::test]
fn catalog_reload_keeps_unclaimed_prepared_records_after_another_record_is_deleted(
    cx: &mut TestAppContext,
) {
    let first = snapshot().metadata.clone();
    let mut second = first.clone();
    second.header.id = "00000000-0000-0000-0000-000000000002".into();
    let history = crate::PreparedHistory::new(vec![first.clone(), second.clone()]).unwrap();
    let (general, _directory, _, _, _) = scoped_fixture_with_history(cx, Some(history));
    cx.update(|cx| general.delete_chat(first.header.id.clone(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let snapshot = general.catalog_snapshot(cx).unwrap();
        assert_eq!(snapshot.rows().len(), 1);
        assert_eq!(snapshot.rows()[0].id, second.header.id);
    });
}
