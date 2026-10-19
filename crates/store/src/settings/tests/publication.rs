use crate::{SETTINGS_CAPACITY as MAX_PENDING, SnapshotWriter};
use crate::{SettingsConfig, SettingsHandle, StoreError};
use futures::{FutureExt as _, channel::oneshot};
use gpui::Task;
use gpui::TestAppContext;
use nocterm_monitor::MonitorSettings;
use nocterm_settings::{SettingsDocument, SettingsFile};
use nocterm_terminal_model::TerminalSettings;
use std::sync::{Arc, Mutex};

fn install(mut config: SettingsConfig, cx: &mut TestAppContext) -> SettingsHandle {
    cx.update(|cx| {
        config.register::<TerminalSettings>();
        config.register::<MonitorSettings>();
        config.install(cx)
    })
}

fn on_disk(file: &SettingsFile) -> TerminalSettings {
    let mut document = file.load().unwrap();
    document.register::<TerminalSettings>();
    document.get::<TerminalSettings>().clone()
}

fn copy_on_select(
    settings: &SettingsHandle,
    cx: &mut TestAppContext,
) -> Task<Result<u64, StoreError>> {
    cx.update(|cx| {
        settings.update::<TerminalSettings>(cx, |terminal| terminal.copy_on_select = true)
    })
}

/// Puts a writer in front of the disk that waits for `release` before its
/// first write and records every value it is given.
fn gated_writer(
    settings: &SettingsHandle,
    cx: &mut TestAppContext,
) -> (
    SettingsHandle,
    oneshot::Sender<Result<(), StoreError>>,
    Arc<Mutex<Vec<SettingsDocument>>>,
) {
    let document = cx.update(|cx| settings.document(cx).clone());
    let (writer, release, writes) = crate::test_support::gated_writer().into_parts();
    let settings = install(SettingsConfig::with_writer(document, writer), cx);
    (settings, release, writes)
}

#[gpui::test]
async fn failed_save_leaves_active_settings_and_revision_intact(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let settings = install(
        SettingsConfig::new(
            SettingsDocument::default(),
            SettingsFile::new(directory.path()),
        ),
        cx,
    );
    assert!(copy_on_select(&settings, cx).await.is_err());
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<TerminalSettings>(cx),
            &TerminalSettings::default()
        );
        assert_eq!(settings.revision(cx), 0);
    });
}

#[gpui::test]
async fn saved_settings_survive_reload(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    let settings = install(
        SettingsConfig::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    let task = cx.update(|cx| {
        settings.update::<TerminalSettings>(cx, |terminal| terminal.font_size = Some(18.))
    });
    assert_eq!(task.await.unwrap(), 1);
    assert_eq!(on_disk(&file).font_size, Some(18.));
    assert!(!on_disk(&file).copy_on_select);
}

#[gpui::test]
async fn ordered_background_writes_do_not_notify_before_commit(cx: &mut TestAppContext) {
    let settings = install(
        SettingsConfig::new(
            SettingsDocument::default(),
            SettingsFile::new("/unused-test-writer"),
        ),
        cx,
    );
    let (settings, release, writes) = gated_writer(&settings, cx);
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    cx.update(|cx| {
        let count = notifications.clone();
        settings
            .observe(cx, move |_| count.set(count.get() + 1))
            .detach();
    });
    let first = cx.update(|cx| {
        settings.update::<TerminalSettings>(cx, |terminal| terminal.font_size = Some(18.))
    });
    let second = copy_on_select(&settings, cx);
    cx.run_until_parked();
    assert_eq!(writes.lock().unwrap().len(), 1);
    assert_eq!(notifications.get(), 0);
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<TerminalSettings>(cx),
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

#[gpui::test]
async fn failed_write_does_not_wedge_the_writer(cx: &mut TestAppContext) {
    use std::sync::atomic::{AtomicUsize, Ordering};
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
    let settings = install(SettingsConfig::with_writer(Default::default(), writer), cx);
    let failed = cx.update(|cx| {
        settings.update::<TerminalSettings>(cx, |terminal| terminal.font_size = Some(18.))
    });
    let next = copy_on_select(&settings, cx);
    assert!(matches!(failed.await.unwrap_err(), StoreError::Backend(_)));
    assert_eq!(next.await.unwrap(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        assert_eq!(settings.setting::<TerminalSettings>(cx).font_size, None);
        assert!(settings.setting::<TerminalSettings>(cx).copy_on_select);
    });
}

#[gpui::test]
async fn queued_edits_apply_to_the_latest_settings_without_conflicts(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    let settings = install(
        SettingsConfig::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    let first = cx.update(|cx| {
        settings.update::<TerminalSettings>(cx, |terminal| terminal.font_size = Some(18.))
    });
    let second = copy_on_select(&settings, cx);
    assert_eq!(first.await.unwrap(), 1);
    assert_eq!(second.await.unwrap(), 2);
    let saved = on_disk(&file);
    assert_eq!(saved.font_size, Some(18.));
    assert!(saved.copy_on_select);
}

#[gpui::test]
fn accepted_save_survives_dropped_caller_and_reaches_disk(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    let settings = install(
        SettingsConfig::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    drop(copy_on_select(&settings, cx));
    cx.run_until_parked();
    assert!(on_disk(&file).copy_on_select);
    cx.update(|cx| assert_eq!(settings.revision(cx), 1));
}

#[gpui::test]
async fn bounded_queue_rejects_overflow_and_shutdown_without_blocking(cx: &mut TestAppContext) {
    let settings = install(
        SettingsConfig::new(
            SettingsDocument::default(),
            SettingsFile::new("/unused-test-writer"),
        ),
        cx,
    );
    let (settings, release, _writes) = gated_writer(&settings, cx);
    let handle = settings.clone();
    // The first change leaves the queue for the gated writer.
    let mut tasks = vec![copy_on_select(&settings, cx)];
    cx.run_until_parked();
    for _ in 0..MAX_PENDING {
        tasks.push(copy_on_select(&settings, cx));
    }
    assert!(matches!(
        copy_on_select(&settings, cx).await.unwrap_err(),
        StoreError::Full
    ));
    let work = cx.update(|cx| handle.capture_shutdown(cx));
    let final_work = cx.background_executor.spawn(work);
    assert!(matches!(
        copy_on_select(&settings, cx).await.unwrap_err(),
        StoreError::Closing
    ));
    release.send(Ok(())).unwrap();
    final_work.await.unwrap();
    assert_eq!(tasks.remove(0).await.unwrap(), 1);
    for task in tasks {
        assert_eq!(task.await.unwrap(), 1, "the rest change nothing");
    }
}

#[gpui::test]
async fn changing_a_broken_section_clears_its_error(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    std::fs::write(
        file.path(),
        "[terminal]\nfont_szie = 1\n[monitor]\nbogus = 1\n",
    )
    .unwrap();
    let settings = install(SettingsConfig::new(file.load().unwrap(), file.clone()), cx);
    cx.update(|cx| assert_eq!(settings.section_errors(cx).len(), 2));
    assert_eq!(copy_on_select(&settings, cx).await.unwrap(), 1);
    cx.update(|cx| {
        let errors = settings.section_errors(cx);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].key, "monitor");
    });
    assert!(
        std::fs::read_to_string(file.path())
            .unwrap()
            .contains("bogus = 1")
    );
}

#[gpui::test]
async fn final_capture_keeps_in_flight_candidate_and_all_accepted_edits(cx: &mut TestAppContext) {
    let settings = install(
        SettingsConfig::new(Default::default(), SettingsFile::new("/test-settings")),
        cx,
    );
    let (settings, release, values) = gated_writer(&settings, cx);
    let handle = settings.clone();
    let first =
        cx.update(|cx| settings.update::<TerminalSettings>(cx, |s| s.font_size = Some(18.)));
    cx.run_until_parked();
    let second = copy_on_select(&settings, cx);
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

#[gpui::test]
async fn no_op_settings_edits_keep_published_revision_without_disk_writes(cx: &mut TestAppContext) {
    let settings = install(
        SettingsConfig::new(Default::default(), SettingsFile::new("/test-settings")),
        cx,
    );
    let (settings, _release, writes) = gated_writer(&settings, cx);
    let task = cx.update(|cx| settings.update::<TerminalSettings>(cx, |_| {}));
    assert_eq!(task.await.unwrap(), 0);
    assert!(writes.lock().unwrap().is_empty());
}

#[gpui::test]
async fn late_section_registration_survives_an_in_flight_publication(cx: &mut TestAppContext) {
    let settings = install(
        SettingsConfig::new(Default::default(), SettingsFile::new("/test-settings")),
        cx,
    );
    let (settings, release, writes) = gated_writer(&settings, cx);
    let task = copy_on_select(&settings, cx);
    cx.run_until_parked();
    cx.update(|cx| settings.register::<nocterm_session::LoggingOptions>(cx));
    release.send(Ok(())).unwrap();
    task.await.unwrap();
    cx.update(|cx| {
        assert!(
            settings
                .document(cx)
                .is_registered::<nocterm_session::LoggingOptions>()
        )
    });
    let work = cx.update(|cx| settings.capture_shutdown(cx));
    cx.background_executor.spawn(work).await.unwrap();
    assert_eq!(
        writes.lock().unwrap().len(),
        1,
        "schema hydration does not add a quit write"
    );
}
