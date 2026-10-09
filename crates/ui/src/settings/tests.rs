use super::writer::{MAX_PENDING, SettingsWriter};
use super::*;
use futures::{FutureExt as _, channel::oneshot};
use gpui_kit::TestAppContext;
use nocterm_settings::{MonitorSettings, TerminalSettings};
use std::sync::{Arc, Mutex};

fn install(store: SettingsStore, cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(store);
        init(cx);
        register_setting::<TerminalSettings>(cx);
        register_setting::<MonitorSettings>(cx);
    });
}

fn on_disk(file: &SettingsFile) -> TerminalSettings {
    let mut document = file.load().unwrap();
    document.register::<TerminalSettings>();
    document.get::<TerminalSettings>().clone()
}

fn copy_on_select(cx: &mut TestAppContext) -> Task<Result<u64, String>> {
    cx.update(|cx| cx.update_setting::<TerminalSettings>(|terminal| terminal.copy_on_select = true))
}

/// Puts a writer in front of the disk that waits for `release` before its
/// first write and records every value it is given.
fn gated_writer(
    cx: &mut TestAppContext,
) -> (
    oneshot::Sender<Result<(), String>>,
    Arc<Mutex<Vec<SettingsDocument>>>,
) {
    let writer = cx.update(|cx| cx.global::<SettingsWriter>().0.clone());
    let (release, receive) = oneshot::channel::<Result<(), String>>();
    let gate = Arc::new(Mutex::new(Some(receive)));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let observed = writes.clone();
    writer.update(cx, |writer, _| {
        writer.test_writer = Some(Arc::new(move |value| {
            observed.lock().unwrap().push(value);
            let gate = gate.lock().unwrap().take();
            async move {
                match gate {
                    Some(gate) => gate.await.unwrap(),
                    None => Ok(()),
                }
            }
            .boxed()
        }))
    });
    (release, writes)
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
        assert_eq!(cx.global::<SettingsStore>().revision(), 0);
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
    let writer = cx.update(|cx| cx.global::<SettingsWriter>().0.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    writer.update(cx, |writer, _| {
        writer.test_writer = Some(Arc::new(move |_| {
            let first = observed.fetch_add(1, Ordering::SeqCst) == 0;
            async move {
                if first {
                    Err("disk full".into())
                } else {
                    Ok(())
                }
            }
            .boxed()
        }))
    });
    let failed = cx.update(|cx| {
        cx.update_setting::<TerminalSettings>(|terminal| terminal.font_size = Some(18.))
    });
    let next = copy_on_select(cx);
    assert_eq!(failed.await.unwrap_err(), "disk full");
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
    cx.update(|cx| assert_eq!(cx.global::<SettingsStore>().revision(), 1));
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
    let writer = cx.update(|cx| cx.global::<SettingsWriter>().0.clone());
    let (release, _writes) = gated_writer(cx);
    // The first change leaves the queue for the gated writer.
    let mut tasks = vec![copy_on_select(cx)];
    cx.run_until_parked();
    for _ in 0..MAX_PENDING {
        tasks.push(copy_on_select(cx));
    }
    assert!(
        copy_on_select(cx)
            .await
            .unwrap_err()
            .contains("queue is full")
    );
    writer.update(cx, |writer, _| writer.closing = true);
    assert!(copy_on_select(cx).await.unwrap_err().contains("closing"));
    release.send(Ok(())).unwrap();
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
        "[terminal]\nfont_szie = 1\n[monitor]\nbogus = 1\n",
    )
    .unwrap();
    install(SettingsStore::new(file.load().unwrap(), file.clone()), cx);
    cx.update(|cx| assert_eq!(cx.global::<SettingsStore>().section_errors().len(), 2));
    assert_eq!(copy_on_select(cx).await.unwrap(), 1);
    cx.update(|cx| {
        let errors = cx.global::<SettingsStore>().section_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].key, "monitor");
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
    cx.update(|cx| cx.update_setting::<MonitorSettings>(|monitor| monitor.interval_secs = 9))
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
    install(SettingsStore::new(SettingsDocument::default(), file), cx);
    let _save = copy_on_select(cx);
    cx.quit();
}
