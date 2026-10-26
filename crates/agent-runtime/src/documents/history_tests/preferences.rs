//! Preference hydration and read failure are observable through Runtime APIs.
use super::*;

#[gpui::test]
async fn delayed_preferences_are_loaded_once_and_unedited_quit_never_rewrites_them(
    cx: &mut TestAppContext,
) {
    let repository = Arc::new(InMemoryAgentRepository::default());
    let saved = nocterm_ai::favorites::AgentStateFile {
        last_agent: Some("claude".into()),
        ..Default::default()
    };
    repository.seed_state(saved.clone());
    let release = repository.block_next(StorageOperation::LoadState);
    let loading = cx.background_executor.spawn(AgentStorage::prepare(
        repository.clone(),
        repository.clone(),
    ));
    cx.run_until_parked();
    assert_eq!(repository.operations(), vec![StorageOperation::LoadState]);
    release.send(Ok(())).unwrap();
    let storage = loading.await;
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime_from_storage(
        cx,
        Arc::new(ScriptedConnector::default()),
        storage,
        directory.path(),
    );
    cx.update(|cx| {
        assert_eq!(runtime.read(cx).favorites(cx), &saved);
        assert!(runtime.read(cx).favorites_error(cx).is_none());
    });
    let drain = runtime.update(cx, |runtime, cx| runtime.capture_shutdown(cx));
    drop(runtime);
    drain.await.unwrap();
    assert_eq!(repository.state(), saved);
    assert_eq!(
        repository
            .operations()
            .iter()
            .filter(|operation| **operation == StorageOperation::LoadState)
            .count(),
        1,
    );
    assert!(
        !repository
            .operations()
            .contains(&StorageOperation::SaveState)
    );
}

#[gpui::test]
async fn failed_preference_hydration_reports_its_typed_error_and_quit_preserves_prior_data(
    cx: &mut TestAppContext,
) {
    let repository = Arc::new(InMemoryAgentRepository::default());
    let saved = nocterm_ai::favorites::AgentStateFile {
        last_agent: Some("previously-saved-agent".into()),
        ..Default::default()
    };
    repository.seed_state(saved.clone());
    repository.fail_next(StorageOperation::LoadState, AgentStorageError::Interrupted);
    let storage = AgentStorage::prepare(repository.clone(), repository.clone()).await;
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime_from_storage(
        cx,
        Arc::new(ScriptedConnector::default()),
        storage,
        directory.path(),
    );
    cx.update(|cx| {
        assert_eq!(
            runtime.read(cx).favorites(cx),
            &nocterm_ai::favorites::AgentStateFile::default(),
        );
        assert!(matches!(
            runtime.read(cx).favorites_error(cx),
            Some(crate::AgentPersistenceError::Storage(
                AgentStorageError::Interrupted
            )),
        ));
    });
    let drain = runtime.update(cx, |runtime, cx| runtime.capture_shutdown(cx));
    drop(runtime);
    drain.await.unwrap();
    assert_eq!(repository.state(), saved);
    assert!(
        !repository
            .operations()
            .contains(&StorageOperation::SaveState)
    );
}

#[gpui::test]
async fn captured_chat_failures_preserve_original_primary_and_independent_diagnostics(
    cx: &mut TestAppContext,
) {
    use std::error::Error as _;
    let (runtime, _directory, repository) = fixture(cx);
    let first = snapshot();
    let mut metadata = first.metadata.clone();
    metadata.header.id = "00000000-0000-0000-0000-000000000002".into();
    let second = Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    });
    for chat in [&first, &second] {
        repository.reject(
            StorageOperation::Save(chat.header.id.clone()),
            AgentStorageError::Interrupted,
        );
        let client = ProbeClient::new(cx, chat.clone());
        admit(&runtime, client, cx);
    }
    let work = runtime.update(cx, |runtime, cx| runtime.capture_shutdown(cx));
    let failure = work.await.unwrap_err();
    let primary = failure
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<nocterm_store::StoreError>()
        .unwrap();
    assert!(matches!(primary, nocterm_store::StoreError::Backend(_)));
    assert!(matches!(
        primary
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<AgentStorageError>(),
        Some(AgentStorageError::Interrupted)
    ));
    let cloned = failure.clone();
    assert!(std::ptr::eq(
        primary,
        cloned
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<nocterm_store::StoreError>()
            .unwrap()
    ));
    let nocterm_store::StoreError::Joined {
        second: diagnostic, ..
    } = &failure
    else {
        panic!("expected aggregate")
    };
    let nocterm_store::StoreError::Joined {
        second: cloned_diagnostic,
        ..
    } = &cloned
    else {
        panic!("expected aggregate clone")
    };
    assert!(Arc::ptr_eq(diagnostic, cloned_diagnostic));
    assert!(repository.encoded(&first.header.id).is_none());
    assert!(repository.encoded(&second.header.id).is_none());
}
