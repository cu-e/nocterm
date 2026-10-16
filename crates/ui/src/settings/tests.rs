use super::*;
use crate::{AppearanceSettings, TerminalSettings};
use futures::{FutureExt as _, channel::oneshot};
use gpui_kit::TestAppContext;
use nocterm_store::{SETTINGS_CAPACITY as MAX_PENDING, ShutdownCoordinator, SnapshotWriter};
use std::sync::{Arc, Mutex};

fn install(store: SettingsStore, cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(store);
        let shutdown = ShutdownCoordinator::default();
        init(cx).register_shutdown(&shutdown).unwrap();
        crate::LayoutMemory::init(None, &shutdown, cx);
        register_setting::<TerminalSettings>(cx);
        register_setting::<AppearanceSettings>(cx);
    });
}

fn on_disk(file: &SettingsFile) -> TerminalSettings {
    let mut document = file.load().unwrap();
    document.register::<TerminalSettings>();
    document.get::<TerminalSettings>().clone()
}

fn copy_on_select(cx: &mut TestAppContext) -> Task<Result<u64, StoreError>> {
    cx.update(|cx| cx.update_setting::<TerminalSettings>(|terminal| terminal.copy_on_select = true))
}

/// Puts a writer in front of the disk that waits for `release` before its
/// first write and records every value it is given.
fn gated_writer(
    cx: &mut TestAppContext,
) -> (
    oneshot::Sender<Result<(), StoreError>>,
    Arc<Mutex<Vec<SettingsDocument>>>,
) {
    let document = cx.update(|cx| cx.global::<SettingsStore>().document(cx).clone());
    let (writer, release, writes) = nocterm_store::test_support::gated_writer().into_parts();
    install(
        SettingsStore::from_config(SettingsConfig::with_writer(document, writer)),
        cx,
    );
    (release, writes)
}

#[gpui_kit::test]
async fn installed_service_handle_keeps_its_identity_through_ui_access(cx: &mut TestAppContext) {
    let handle = cx.update(|cx| {
        SettingsConfig::in_memory(SettingsDocument::default())
            .with(TerminalSettings::default())
            .install(cx)
    });
    install(SettingsStore::from_handle(handle.clone()), cx);
    assert_eq!(copy_on_select(cx).await.unwrap(), 1);
    cx.update(|cx| {
        assert!(handle.setting::<TerminalSettings>(cx).copy_on_select);
        assert_eq!(handle.revision(cx), 1);
    });
}

#[gpui_kit::test]
async fn failed_save_leaves_active_settings_and_revision_intact(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    install(
        SettingsStore::new(
            SettingsDocument::default(),
            SettingsFile::new(directory.path()),
        ),
        cx,
    );
    assert!(copy_on_select(cx).await.is_err());
    cx.update(|cx| {
        assert_eq!(
            cx.setting::<TerminalSettings>(),
            &TerminalSettings::default()
        );
        assert_eq!(cx.global::<SettingsStore>().revision(cx), 0);
    });
}

#[gpui_kit::test]
async fn saved_settings_survive_reload(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    install(
        SettingsStore::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    let task = cx.update(|cx| {
        cx.update_setting::<TerminalSettings>(|terminal| terminal.font_size = Some(18.))
    });
    assert_eq!(task.await.unwrap(), 1);
    assert_eq!(on_disk(&file).font_size, Some(18.));
    assert!(!on_disk(&file).copy_on_select);
}

#[gpui_kit::test]
async fn ordered_background_writes_do_not_notify_before_commit(cx: &mut TestAppContext) {
    install(
        SettingsStore::new(
            SettingsDocument::default(),
            SettingsFile::new("/unused-test-writer"),
        ),
        cx,
    );
    let (release, writes) = gated_writer(cx);
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let count = notifications.clone();
        cx.observe_global::<SettingsStore>(move |_| count.set(count.get() + 1))
            .detach();
    });
    let first = cx.update(|cx| {
        cx.update_setting::<TerminalSettings>(|terminal| terminal.font_size = Some(18.))
    });
    let second = copy_on_select(cx);
    cx.run_until_parked();
    assert_eq!(writes.lock().unwrap().len(), 1);
    assert_eq!(notifications.get(), 0);
    cx.update(|cx| {
        assert_eq!(
            cx.setting::<TerminalSettings>(),
            &TerminalSettings::default()
        )
    });
    release.send(Ok(())).unwrap();
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(second.await.unwrap(), 2);
    cx.run_until_parked();
    assert_eq!(notifications.get(), 2);
    let writes = writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    let last = writes[1].get::<TerminalSettings>();
    assert_eq!((last.font_size, last.copy_on_select), (Some(18.), true));
}

#[gpui_kit::test]
async fn failed_write_does_not_wedge_the_writer(cx: &mut TestAppContext) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    install(
        SettingsStore::new(
            SettingsDocument::default(),
            SettingsFile::new("/unused-test-writer"),
        ),
        cx,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let writer: SnapshotWriter<SettingsDocument> = Arc::new(move |_| {
        let first = observed.fetch_add(1, Ordering::SeqCst) == 0;
        async move {
            if first {
                Err(StoreError::backend(std::io::Error::from(
                    std::io::ErrorKind::StorageFull,
                )))
            } else {
                Ok(())
            }
        }
        .boxed()
    });
    install(
        SettingsStore::from_config(SettingsConfig::with_writer(Default::default(), writer)),
        cx,
    );
    let failed = cx.update(|cx| {
        cx.update_setting::<TerminalSettings>(|terminal| terminal.font_size = Some(18.))
    });
    let next = copy_on_select(cx);
    assert!(matches!(failed.await.unwrap_err(), StoreError::Backend(_)));
    assert_eq!(next.await.unwrap(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        assert_eq!(cx.setting::<TerminalSettings>().font_size, None);
        assert!(cx.setting::<TerminalSettings>().copy_on_select);
    });
}

#[gpui_kit::test]
async fn queued_edits_apply_to_the_latest_settings_without_conflicts(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    install(
        SettingsStore::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    let first = cx.update(|cx| {
        cx.update_setting::<TerminalSettings>(|terminal| terminal.font_size = Some(18.))
    });
    let second = copy_on_select(cx);
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(second.await.unwrap(), 2);
    let saved = on_disk(&file);
    assert_eq!(saved.font_size, Some(18.));
    assert!(saved.copy_on_select);
}

#[gpui_kit::test]
fn accepted_save_survives_dropped_caller_and_reaches_disk(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    install(
        SettingsStore::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    drop(copy_on_select(cx));
    cx.run_until_parked();
    assert!(on_disk(&file).copy_on_select);
    cx.update(|cx| assert_eq!(cx.global::<SettingsStore>().revision(cx), 1));
}

#[gpui_kit::test]
async fn bounded_queue_rejects_overflow_and_shutdown_without_blocking(cx: &mut TestAppContext) {
    install(
        SettingsStore::new(
            SettingsDocument::default(),
            SettingsFile::new("/unused-test-writer"),
        ),
        cx,
    );
    let (release, _writes) = gated_writer(cx);
    let handle = cx.update(|cx| cx.global::<SettingsStore>().handle().unwrap().clone());
    // The first change leaves the queue for the gated writer.
    let mut tasks = vec![copy_on_select(cx)];
    cx.run_until_parked();
    for _ in 0..MAX_PENDING {
        tasks.push(copy_on_select(cx));
    }
    assert!(matches!(
        copy_on_select(cx).await.unwrap_err(),
        StoreError::Full
    ));
    let work = cx.update(|cx| handle.capture_shutdown(cx));
    let final_work = cx.background_executor.spawn(work);
    assert!(matches!(
        copy_on_select(cx).await.unwrap_err(),
        StoreError::Closing
    ));
    release.send(Ok(())).unwrap();
    final_work.await.unwrap();
    assert_eq!(tasks.remove(0).await.unwrap(), 1);
    for task in tasks {
        assert_eq!(task.await.unwrap(), 1, "the rest change nothing");
    }
}

#[gpui_kit::test]
async fn changing_a_broken_section_clears_its_error(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    std::fs::write(
        file.path(),
        "[terminal]\nfont_szie = 1\n[appearance]\nbogus = 1\n",
    )
    .unwrap();
    install(SettingsStore::new(file.load().unwrap(), file.clone()), cx);
    cx.update(|cx| assert_eq!(cx.global::<SettingsStore>().section_errors(cx).len(), 2));
    assert_eq!(copy_on_select(cx).await.unwrap(), 1);
    cx.update(|cx| {
        let errors = cx.global::<SettingsStore>().section_errors(cx);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].key, "appearance");
    });
    assert!(
        std::fs::read_to_string(file.path())
            .unwrap()
            .contains("bogus = 1")
    );
}

#[gpui_kit::test]
async fn observers_of_a_section_ignore_changes_to_others(cx: &mut TestAppContext) {
    install(SettingsStore::in_memory(SettingsDocument::default()), cx);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let seen = seen.clone();
        cx.observe_setting::<TerminalSettings>(move |terminal, _| {
            seen.borrow_mut().push(terminal.copy_on_select)
        })
    });
    cx.update(|cx| cx.update_setting::<AppearanceSettings>(|appearance| appearance.card_gap = 9.0))
        .await
        .unwrap();
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());
    copy_on_select(cx).await.unwrap();
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [true]);
}

#[gpui_kit::test]
async fn quitting_with_queued_writes_does_not_panic(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    install(
        SettingsStore::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    let _save = copy_on_select(cx);
    cx.quit();
    assert!(
        on_disk(&file).copy_on_select,
        "accepted edits must reach disk during actual GPUI shutdown"
    );
}

#[gpui_kit::test]
fn quit_flushes_accepted_settings_before_foreground_writer_runs(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    install(SettingsStore::new(Default::default(), file.clone()), cx);
    let pending = cx.update(|cx| {
        let first = cx.update_setting::<TerminalSettings>(|s| s.font_size = Some(23.));
        let second = cx.update_setting::<TerminalSettings>(|s| s.copy_on_select = true);
        (first, second)
    });
    cx.quit();
    let saved = on_disk(&file);
    assert_eq!(saved.font_size, Some(23.));
    assert!(saved.copy_on_select);
    drop(pending);
}

#[gpui_kit::test]
async fn final_capture_keeps_in_flight_candidate_and_all_accepted_edits(cx: &mut TestAppContext) {
    install(
        SettingsStore::new(Default::default(), SettingsFile::new("/test-settings")),
        cx,
    );
    let (release, values) = gated_writer(cx);
    let handle = cx.update(|cx| cx.global::<SettingsStore>().handle().unwrap().clone());
    let first = cx.update(|cx| cx.update_setting::<TerminalSettings>(|s| s.font_size = Some(18.)));
    cx.run_until_parked();
    let second = copy_on_select(cx);
    let work = cx.update(|cx| handle.capture_shutdown(cx));
    let final_work = cx.background_executor.spawn(work);
    cx.run_until_parked();
    assert_eq!(
        values.lock().unwrap().len(),
        1,
        "final waits for in-flight disk work"
    );
    release
        .send(Err(StoreError::backend(std::io::Error::from(
            std::io::ErrorKind::StorageFull,
        ))))
        .unwrap();
    final_work.await.unwrap();
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(second.await.unwrap(), 2);
    let values = values.lock().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[1].get::<TerminalSettings>().font_size, Some(18.));
    assert!(values[1].get::<TerminalSettings>().copy_on_select);
    cx.update(|cx| {
        assert_eq!(
            handle.revision(cx),
            0,
            "final publication never reads or updates App"
        )
    });
}

#[gpui_kit::test]
async fn no_op_settings_edits_keep_published_revision_without_disk_writes(cx: &mut TestAppContext) {
    install(
        SettingsStore::new(Default::default(), SettingsFile::new("/test-settings")),
        cx,
    );
    let (_release, writes) = gated_writer(cx);
    let task = cx.update(|cx| cx.update_setting::<TerminalSettings>(|_| {}));
    assert_eq!(task.await.unwrap(), 0);
    assert!(writes.lock().unwrap().is_empty());
}

#[gpui_kit::test]
async fn late_section_registration_survives_an_in_flight_publication(cx: &mut TestAppContext) {
    install(
        SettingsStore::new(Default::default(), SettingsFile::new("/test-settings")),
        cx,
    );
    let (release, writes) = gated_writer(cx);
    let task = copy_on_select(cx);
    cx.run_until_parked();
    cx.update(register_setting::<nocterm_session::LoggingOptions>);
    release.send(Ok(())).unwrap();
    task.await.unwrap();
    cx.update(|cx| {
        assert!(
            cx.global::<SettingsStore>()
                .document(cx)
                .is_registered::<nocterm_session::LoggingOptions>()
        )
    });
    let work = cx.update(|cx| {
        cx.global::<SettingsStore>()
            .handle()
            .unwrap()
            .clone()
            .capture_shutdown(cx)
    });
    cx.background_executor.spawn(work).await.unwrap();
    assert_eq!(
        writes.lock().unwrap().len(),
        1,
        "schema hydration does not add a quit write"
    );
}
