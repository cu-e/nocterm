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
