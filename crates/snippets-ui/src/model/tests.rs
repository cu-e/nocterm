use super::*;
fn snippet() -> Snippet {
    Snippet {
        name: "Deploy 日本語".into(),
        description: "Описание".into(),
        content: "#!/bin/bash\necho 'привет'\n\n".into(),
        ..Snippet::new(uuid::Uuid::from_u128(1))
    }
}
#[gpui_kit::test]
async fn persistence_round_trip_and_queued_updates(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deep/snippets.toml");
    let model = cx.new(|cx| Snippets::new(Some(path.clone()), cx));
    let first = snippet();
    let second = Snippet {
        id: uuid::Uuid::from_u128(2),
        name: "Second".into(),
        ..snippet()
    };
    let save1 = model.update(cx, |model, cx| model.save(first.clone(), None, cx));
    let save2 = model.update(cx, |model, cx| model.save(second.clone(), None, cx));
    save1.await.unwrap();
    save2.await.unwrap();
    let restored = cx.new(|cx| Snippets::new(Some(path), cx));
    assert_eq!(
        restored.read_with(cx, |model, cx| model.library(cx).snippets.clone()),
        [first, second]
    );
}
#[gpui_kit::test]
async fn corrupt_file_is_read_only_and_never_overwritten(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.toml");
    std::fs::write(&path, "snippets = =").unwrap();
    let model = cx.new(|cx| Snippets::new(Some(path.clone()), cx));
    assert!(!model.read_with(cx, |model, cx| model.writable(cx)));
    use std::error::Error;
    let initial = model.read_with(cx, |model, cx| model.error(cx).unwrap());
    let task = model.update(cx, |model, cx| model.save(snippet(), None, cx));
    let error = task.await.unwrap_err();
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<SnippetsLoadError>()
        .unwrap();
    let first = initial
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<SnippetsLoadError>()
        .unwrap();
    assert!(std::ptr::eq(original, first));
    let SnippetsLoadError::File(persist::PersistError::Parse { source, .. }) = original else {
        panic!("malformed native library retains its original parse diagnostic")
    };
    // The transparent File variant delegates directly to the original codec source.
    let codec = original.source().unwrap().downcast_ref().unwrap();
    assert!(std::ptr::eq(codec, source.as_ref()));
    assert!(model.read_with(cx, |model, cx| model.library(cx).snippets.is_empty()));
    assert!(!model.read_with(cx, |model, cx| model.writable(cx)));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "snippets = =");
}
#[gpui_kit::test]
async fn failed_write_keeps_published_state(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("blocked");
    let model = cx.new(|cx| Snippets::new(Some(parent.join("snippets.toml")), cx));
    std::fs::write(&parent, "file").unwrap();
    let task = model.update(cx, |model, cx| model.save(snippet(), None, cx));
    assert!(
        matches!(task.await, Err(SnippetsError::Persistence(StoreError::Backend(ref source))) if matches!(source.downcast_ref::<persist::PersistError>(), Some(persist::PersistError::Io { .. })))
    );
    assert!(model.read_with(cx, |model, cx| model.library(cx).snippets.is_empty()));
}
#[gpui_kit::test]
async fn concurrent_stale_edits_rebase_on_persisted_state(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let model = cx.new(|cx| Snippets::new(Some(directory.path().join("snippets.toml")), cx));
    let original = snippet();
    model
        .update(cx, |model, cx| model.save(original.clone(), None, cx))
        .await
        .unwrap();
    let first = Snippet {
        content: "first".into(),
        ..original.clone()
    };
    let second = Snippet {
        content: "second".into(),
        ..original.clone()
    };
    let one = model.update(cx, |model, cx| {
        model.save(first.clone(), Some(original.clone()), cx)
    });
    let two = model.update(cx, |model, cx| model.save(second, Some(original), cx));
    one.await.unwrap();
    assert!(matches!(
        two.await,
        Err(SnippetsError::Library(LibraryError::SaveConflict(_)))
    ));
    assert_eq!(
        model.read_with(cx, |model, cx| model.library(cx).snippets.clone()),
        [first]
    );
}

#[gpui_kit::test]
fn memory_edits_publish_before_the_receipt_is_polled(cx: &mut gpui_kit::TestAppContext) {
    let model = cx.new(|cx| Snippets::new(None, cx));
    let original = snippet();
    let task = model.update(cx, |model, cx| model.save(original.clone(), None, cx));
    assert_eq!(
        model.read_with(cx, |model, cx| model.library(cx).snippets.clone()),
        [original]
    );
    task.detach();
}

#[gpui_kit::test]
async fn failed_predecessor_is_not_used_by_expected_record_checks(
    cx: &mut gpui_kit::TestAppContext,
) {
    let original = snippet();
    let attempts = Arc::new(std::sync::Mutex::new(0));
    let calls = attempts.clone();
    let writer: SnapshotWriter<Library> = Arc::new(move |_| {
        let call = {
            let mut calls = calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        Box::pin(async move {
            if call == 1 {
                Err(StoreError::backend(std::io::Error::from(
                    std::io::ErrorKind::StorageFull,
                )))
            } else {
                Ok(())
            }
        })
    });
    let library = Library {
        snippets: vec![original.clone()],
    };
    let model = cx.new(|cx| Snippets::with_writer(library, None, true, writer, cx));
    let mut first = original.clone();
    first.content = "will fail".into();
    let mut second = original.clone();
    second.content = "saved".into();
    let one = model.update(cx, |model, cx| {
        model.save(first, Some(original.clone()), cx)
    });
    let two = model.update(cx, |model, cx| {
        model.save(second.clone(), Some(original), cx)
    });
    let error = one.await.unwrap_err();
    assert!(
        matches!(error, SnippetsError::Persistence(StoreError::Backend(ref source)) if source.downcast_ref::<std::io::Error>().unwrap().kind() == std::io::ErrorKind::StorageFull)
    );
    two.await.unwrap();
    assert_eq!(
        model.read_with(cx, |model, cx| model.library(cx).snippets.clone()),
        [second]
    );
    assert_eq!(*attempts.lock().unwrap(), 2);
}

#[gpui_kit::test]
async fn shutdown_captures_accepted_records_and_rejects_a_stale_edit(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.toml");
    let original = snippet();
    let library = Library {
        snippets: vec![original.clone()],
    };
    persist::save(&path, &library).unwrap();
    let coordinator = ShutdownCoordinator::default();
    let model = cx.new(|cx| Snippets::new(Some(path.clone()), cx));
    model
        .read_with(cx, |model, _| model.register_shutdown(&coordinator))
        .unwrap();
    let mut first = original.clone();
    first.content = "saved at quit".into();
    let mut stale = original.clone();
    stale.content = "stale".into();
    let mut added = snippet();
    added.id = uuid::Uuid::from_u128(2);
    let one = model.update(cx, |model, cx| {
        model.save(first.clone(), Some(original.clone()), cx)
    });
    let two = model.update(cx, |model, cx| model.save(stale, Some(original), cx));
    let three = model.update(cx, |model, cx| model.save(added.clone(), None, cx));
    let work = cx.update(|cx| coordinator.capture_all(cx));
    assert!(!model.read_with(cx, |model, cx| model.writable(cx)));
    let rejected = model.update(cx, |model, cx| model.delete(first.clone(), cx));
    assert!(matches!(rejected.await, Err(SnippetsError::Closing)));
    drop(model);
    let report = cx.background_executor.spawn(work).await;
    assert!(report.failures().is_empty());
    one.await.unwrap();
    assert!(matches!(
        two.await,
        Err(SnippetsError::Library(LibraryError::SaveConflict(_)))
    ));
    three.await.unwrap();
    let restored: Library = persist::load(&path).unwrap().unwrap();
    assert_eq!(restored.snippets, [first, added]);
}

#[gpui_kit::test]
async fn bounded_queue_reports_full_without_losing_accepted_records(
    cx: &mut gpui_kit::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.toml");
    let coordinator = ShutdownCoordinator::default();
    let model = cx.new(|cx| Snippets::new(Some(path.clone()), cx));
    model
        .read_with(cx, |model, _| model.register_shutdown(&coordinator))
        .unwrap();
    let mut tasks = Vec::new();
    for index in 0..MAX_PENDING {
        let mut record = snippet();
        record.id = uuid::Uuid::from_u128(index as u128 + 1);
        tasks.push(model.update(cx, |model, cx| model.save(record, None, cx)));
    }
    let mut rejected = snippet();
    rejected.id = uuid::Uuid::from_u128(999);
    let rejected = model.update(cx, |model, cx| model.save(rejected, None, cx));
    assert!(matches!(rejected.await, Err(SnippetsError::Full)));
    let work = cx.update(|cx| coordinator.capture_all(cx));
    let report = cx.background_executor.spawn(work).await;
    assert!(report.failures().is_empty());
    for task in tasks {
        task.await.unwrap();
    }
    let library: Library = persist::load(&path).unwrap().unwrap();
    assert_eq!(library.snippets.len(), MAX_PENDING);
    for (index, record) in library.snippets.iter().enumerate() {
        assert_eq!(record.id, uuid::Uuid::from_u128(index as u128 + 1));
    }
}

#[test]
fn readonly_constructor_preserves_supplied_nonclone_diagnostic_allocation() {
    use std::error::Error;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.toml");
    std::fs::write(&path, "snippets = =").unwrap();
    let diagnostic = Arc::new(SnippetsLoadError::File(
        persist::load::<Library>(&path).unwrap_err(),
    ));
    let first = SnippetsError::read_only(diagnostic.clone());
    let second = SnippetsError::read_only(diagnostic.clone());
    let original = first
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<SnippetsLoadError>()
        .unwrap();
    assert!(std::ptr::eq(original, diagnostic.as_ref()));
    assert!(std::ptr::eq(
        original,
        second
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<SnippetsLoadError>()
            .unwrap()
    ));
    assert_eq!(first.to_string(), second.to_string());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "snippets = =");
}
