use super::*;
use nocterm_snippets::{LibraryError, StorageOperation};
use nocterm_store::test_support::gated_writer;
use std::error::Error;
fn snippet(id: u128, content: &str) -> Snippet {
    Snippet {
        name: "Deploy 日本語".into(),
        content: content.into(),
        ..Snippet::new(uuid::Uuid::from_u128(id))
    }
}
fn install(
    prepared: PreparedLibrary,
    writer: SnapshotWriter<Library>,
    shutdown: &ShutdownCoordinator,
    cx: &mut gpui::TestAppContext,
) -> SnippetsHandle {
    cx.update(|cx| SnippetsHandle::install(prepared, writer, shutdown, cx).unwrap())
}
fn memory(cx: &mut gpui::TestAppContext) -> SnippetsHandle {
    install(
        PreparedLibrary::memory(),
        Arc::new(|_| Box::pin(async { Ok(()) })),
        &ShutdownCoordinator::default(),
        cx,
    )
}
#[gpui::test]
fn memory_edits_publish_before_the_receipt_is_polled(cx: &mut gpui::TestAppContext) {
    let model = memory(cx);
    let original = snippet(1, "echo 日本語\n\n");
    let task = cx.update(|cx| model.save(original.clone(), None, cx));
    cx.update(|cx| assert_eq!(model.library(cx).snippets, [original]));
    task.detach();
}
#[gpui::test]
async fn queued_updates_publish_only_after_their_original_writer_completes(
    cx: &mut gpui::TestAppContext,
) {
    let (writer, release, writes) = gated_writer().into_parts();
    let model = install(
        PreparedLibrary::loaded(Ok(None), true),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let first = snippet(1, "echo first\n");
    let second = snippet(2, "echo second\n");
    let one = cx.update(|cx| model.save(first.clone(), None, cx));
    let two = cx.update(|cx| model.save(second.clone(), None, cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(model.library(cx).snippets.is_empty()));
    assert_eq!(
        writes.lock().unwrap()[0].snippets.as_slice(),
        std::slice::from_ref(&first)
    );
    release.send(Ok(())).unwrap();
    one.await.unwrap();
    two.await.unwrap();
    cx.update(|cx| assert_eq!(model.library(cx).snippets, [first, second]));
    assert_eq!(writes.lock().unwrap().len(), 2);
}
#[gpui::test]
async fn failed_predecessor_is_not_used_by_expected_record_checks(cx: &mut gpui::TestAppContext) {
    let original = snippet(1, "original");
    let prepared = PreparedLibrary::loaded(
        Ok(Some(Library {
            snippets: vec![original.clone()],
        })),
        true,
    );
    let (writer, release, writes) = gated_writer().into_parts();
    let model = install(prepared, writer, &ShutdownCoordinator::default(), cx);
    let first = Snippet {
        content: "will fail".into(),
        ..original.clone()
    };
    let second = Snippet {
        content: "saved".into(),
        ..original.clone()
    };
    let one = cx.update(|cx| model.save(first, Some(original.clone()), cx));
    let two = cx.update(|cx| model.save(second.clone(), Some(original), cx));
    cx.run_until_parked();
    release
        .send(Err(StoreError::backend(std::io::Error::from(
            std::io::ErrorKind::StorageFull,
        ))))
        .unwrap();
    assert!(
        matches!(one.await, Err(SnippetsError::Persistence(StoreError::Backend(ref source))) if source.downcast_ref::<std::io::Error>().unwrap().kind() == std::io::ErrorKind::StorageFull)
    );
    two.await.unwrap();
    cx.update(|cx| assert_eq!(model.library(cx).snippets, [second]));
    assert_eq!(writes.lock().unwrap().len(), 2);
}
#[gpui::test]
async fn concurrent_stale_edits_check_the_whole_original_record(cx: &mut gpui::TestAppContext) {
    let original = snippet(1, "original");
    let (writer, release, _) = gated_writer().into_parts();
    let model = install(
        PreparedLibrary::loaded(
            Ok(Some(Library {
                snippets: vec![original.clone()],
            })),
            true,
        ),
        writer,
        &ShutdownCoordinator::default(),
        cx,
    );
    let first = Snippet {
        description: "first".into(),
        ..original.clone()
    };
    let second = Snippet {
        content: "second".into(),
        ..original.clone()
    };
    let one = cx.update(|cx| model.save(first.clone(), Some(original.clone()), cx));
    let two = cx.update(|cx| model.save(second, Some(original), cx));
    release.send(Ok(())).unwrap();
    one.await.unwrap();
    assert!(matches!(
        two.await,
        Err(SnippetsError::Library(LibraryError::SaveConflict(_)))
    ));
    cx.update(|cx| assert_eq!(model.library(cx).snippets, [first]));
}
#[gpui::test]
async fn capture_keeps_accepted_receipts_after_handle_drop_and_rejects_stale_records(
    cx: &mut gpui::TestAppContext,
) {
    let original = snippet(1, "original");
    let (writer, release, writes) = gated_writer().into_parts();
    let shutdown = ShutdownCoordinator::default();
    let model = install(
        PreparedLibrary::loaded(
            Ok(Some(Library {
                snippets: vec![original.clone()],
            })),
            true,
        ),
        writer,
        &shutdown,
        cx,
    );
    let first = Snippet {
        content: "saved at quit".into(),
        ..original.clone()
    };
    let one = cx.update(|cx| model.save(first.clone(), Some(original.clone()), cx));
    let stale = cx.update(|cx| model.save(snippet(1, "stale"), Some(original), cx));
    let added = snippet(2, "new");
    let three = cx.update(|cx| model.save(added.clone(), None, cx));
    cx.run_until_parked();
    let work = cx.update(|cx| shutdown.capture_all(cx));
    assert!(!cx.update(|cx| model.writable(cx)));
    assert!(matches!(
        cx.update(|cx| model.delete(first.clone(), cx)).await,
        Err(SnippetsError::Closing)
    ));
    drop(model);
    release.send(Ok(())).unwrap();
    let report = cx.background_executor.spawn(work).await;
    assert!(report.failures().is_empty());
    one.await.unwrap();
    assert!(matches!(
        stale.await,
        Err(SnippetsError::Library(LibraryError::SaveConflict(_)))
    ));
    three.await.unwrap();
    assert_eq!(
        writes.lock().unwrap().last().unwrap().snippets,
        [first, added]
    );
}
#[gpui::test]
async fn queue32_reports_full_and_capture_keeps_all_accepted_records(
    cx: &mut gpui::TestAppContext,
) {
    let (writer, release, writes) = gated_writer().into_parts();
    let shutdown = ShutdownCoordinator::default();
    let model = install(
        PreparedLibrary::loaded(Ok(None), true),
        writer,
        &shutdown,
        cx,
    );
    let tasks: Vec<_> = (1..=32)
        .map(|id| cx.update(|cx| model.save(snippet(id, "accepted"), None, cx)))
        .collect();
    let rejected = cx.update(|cx| model.save(snippet(999, "rejected"), None, cx));
    assert!(matches!(rejected.await, Err(SnippetsError::Full)));
    let work = cx.update(|cx| shutdown.capture_all(cx));
    release.send(Ok(())).unwrap();
    assert!(
        cx.background_executor
            .spawn(work)
            .await
            .failures()
            .is_empty()
    );
    for task in tasks {
        task.await.unwrap();
    }
    let log = writes.lock().unwrap();
    let saved = &log.last().unwrap().snippets;
    assert_eq!(saved.len(), 32);
    for (index, record) in saved.iter().enumerate() {
        assert_eq!(record.id, uuid::Uuid::from_u128(index as u128 + 1));
    }
}
#[gpui::test]
fn registration_closing_exposes_no_unregistered_service(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let shutdown = ShutdownCoordinator::default();
        drop(shutdown.capture_all(cx));
        assert!(matches!(
            SnippetsHandle::install(
                PreparedLibrary::memory(),
                Arc::new(|_| Box::pin(async { Ok(()) })),
                &shutdown,
                cx
            ),
            Err(StoreError::Closing)
        ));
    });
}
#[derive(Debug, thiserror::Error)]
#[error("original nonclone diagnostic")]
struct NonCloneDiagnostic;
#[gpui::test]
async fn readonly_reuses_the_original_load_diagnostic_and_validation_still_precedes_readonly(
    cx: &mut gpui::TestAppContext,
) {
    let diagnostic = SnippetStorageError::new(StorageOperation::Read, NonCloneDiagnostic);
    let prepared = PreparedLibrary::loaded(Err(diagnostic), true);
    let model = install(
        prepared,
        Arc::new(|_| panic!("readonly must never write")),
        &ShutdownCoordinator::default(),
        cx,
    );
    let first = cx.update(|cx| model.error(cx).unwrap());
    let second = cx
        .update(|cx| model.save(snippet(1, "valid"), None, cx))
        .await
        .unwrap_err();
    let original = first
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<SnippetsLoadError>()
        .unwrap();
    let again = second
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<SnippetsLoadError>()
        .unwrap();
    assert!(std::ptr::eq(original, again));
    assert!(
        original
            .source()
            .unwrap()
            .downcast_ref::<NonCloneDiagnostic>()
            .is_some()
    );
    assert!(!cx.update(|cx| model.writable(cx)));
    let invalid = cx
        .update(|cx| model.save(snippet(2, ""), None, cx))
        .await
        .unwrap_err();
    assert!(matches!(
        invalid,
        SnippetsError::Library(LibraryError::Validation(_))
    ));
}
#[gpui::test]
async fn invalid_loaded_library_is_readonly_and_checked_delete_keeps_newer_record(
    cx: &mut gpui::TestAppContext,
) {
    let bad = install(
        PreparedLibrary::loaded(
            Ok(Some(Library {
                snippets: vec![snippet(1, "")],
            })),
            true,
        ),
        Arc::new(|_| panic!("readonly must never write")),
        &ShutdownCoordinator::default(),
        cx,
    );
    assert!(!cx.update(|cx| bad.writable(cx)));
    let model = memory(cx);
    let original = snippet(1, "original");
    cx.update(|cx| model.save(original.clone(), None, cx))
        .await
        .unwrap();
    let newer = snippet(1, "newer");
    cx.update(|cx| model.save(newer.clone(), Some(original.clone()), cx))
        .await
        .unwrap();
    assert!(matches!(
        cx.update(|cx| model.delete(original, cx)).await,
        Err(SnippetsError::Library(LibraryError::DeleteConflict(_)))
    ));
    cx.update(|cx| {
        assert_eq!(
            model.library(cx).snippets.as_slice(),
            std::slice::from_ref(&newer)
        )
    });
    cx.update(|cx| model.delete(newer, cx)).await.unwrap();
    assert!(cx.update(|cx| model.library(cx).snippets.is_empty()));
}
