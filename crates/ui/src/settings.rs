//! Revision-checked settings publication with a bounded, ordered disk writer.
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, BorrowAppContext as _, Context, Entity, Global, Task};
use nocterm_settings::{Settings, SettingsFile};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_PENDING: usize = 16;
const CONFLICT: &str =
    "Settings changed in another view. Reload saved settings before applying this draft.";

/// Published settings only. Queue changes do not notify theme/settings observers.
pub struct SettingsStore {
    settings: Settings,
    file: Option<SettingsFile>,
    revision: u64,
}
impl SettingsStore {
    pub fn new(settings: Settings, file: SettingsFile) -> Self {
        Self {
            settings: settings.sanitized(),
            file: Some(file),
            revision: 0,
        }
    }
    pub fn in_memory(settings: Settings) -> Self {
        Self {
            settings: settings.sanitized(),
            file: None,
            revision: 0,
        }
    }
    pub fn is_persistent(&self) -> bool {
        self.file.is_some()
    }
    /// Optimistic revision for drafts. Advances only after successful publication.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
impl Global for SettingsStore {}

pub trait ActiveSettings {
    fn settings(&self) -> &Settings;
}
impl ActiveSettings for App {
    fn settings(&self) -> &Settings {
        &self.global::<SettingsStore>().settings
    }
}
struct Pending {
    expected: u64,
    settings: Settings,
    done: oneshot::Sender<Result<u64, String>>,
}
struct Writer {
    pending: VecDeque<Pending>,
    task: Option<Task<()>>,
    closing: bool,
    #[cfg(test)]
    test_writer: Option<
        std::sync::Arc<
            dyn Fn(Settings) -> futures::future::BoxFuture<'static, Result<(), String>>
                + Send
                + Sync,
        >,
    >,
}
struct SettingsWriter(Entity<Writer>);
impl Global for SettingsWriter {}

pub(crate) fn init(cx: &mut App) {
    let writer = cx.new(|_| Writer {
        pending: VecDeque::new(),
        task: None,
        closing: false,
        #[cfg(test)]
        test_writer: None,
    });
    cx.set_global(SettingsWriter(writer.clone()));
    cx.on_app_quit(move |cx| {
        writer.update(cx, |writer, _| writer.closing = true);
        let writer = writer.clone();
        let app = cx.to_async();
        let executor = cx.background_executor().clone();
        async move {
            let deadline = Instant::now() + Duration::from_millis(180);
            loop {
                let busy = writer.read_with(&app, |writer, _| writer.task.is_some());
                if !busy { break; }
                if Instant::now() >= deadline {
                    tracing::warn!("timed out saving settings during shutdown; pending changes may not be saved");
                    break;
                }
                executor.timer(Duration::from_millis(10)).await;
            }
        }
    }).detach();
}

/// Persists a draft before publishing it. Stale drafts never reach disk.
/// The application owns accepted jobs even when their caller closes a view.
pub fn save_settings(cx: &mut App, expected: u64, settings: Settings) -> Task<Result<u64, String>> {
    let settings = settings.sanitized();
    let writer = cx.global::<SettingsWriter>().0.clone();
    writer.update(cx, |writer, cx| writer.enqueue(expected, settings, cx))
}

/// Edits the latest published settings using its current revision.
pub fn update_settings(
    cx: &mut App,
    edit: impl FnOnce(&mut Settings),
) -> Task<Result<u64, String>> {
    let expected = cx.global::<SettingsStore>().revision();
    let mut settings = cx.settings().clone();
    edit(&mut settings);
    save_settings(cx, expected, settings)
}
fn publish(settings: Settings, cx: &mut App) -> u64 {
    cx.update_global::<SettingsStore, _>(|store, _| {
        store.settings = settings;
        store.revision += 1;
        store.revision
    })
}
impl Writer {
    fn enqueue(
        &mut self,
        expected: u64,
        settings: Settings,
        cx: &mut Context<Self>,
    ) -> Task<Result<u64, String>> {
        if self.closing {
            return Task::ready(Err(
                "Settings are closing. New changes cannot be saved.".into()
            ));
        }
        let store = cx.global::<SettingsStore>();
        if expected != store.revision {
            return Task::ready(Err(CONFLICT.into()));
        }
        if store.file.is_none() {
            return Task::ready(Ok(if settings == store.settings {
                store.revision
            } else {
                publish(settings, cx)
            }));
        }
        if self.pending.len() >= MAX_PENDING {
            return Task::ready(Err(
                "Settings save queue is full. Wait for pending saves to finish and retry.".into(),
            ));
        }
        let (done, receive) = oneshot::channel();
        self.pending.push_back(Pending {
            expected,
            settings,
            done,
        });
        if self.task.is_none() {
            self.task = Some(cx.spawn(async move |writer, cx| {
                loop {
                    let next = writer.update(cx, |writer, cx| {
                        let Some(pending) = writer.pending.pop_front() else {
                            writer.task = None;
                            return None;
                        };
                        let store = cx.global::<SettingsStore>();
                        let candidate = if pending.expected != store.revision {
                            Err(CONFLICT.to_owned())
                        } else {
                            Ok((
                                store.file.clone(),
                                pending.settings == store.settings,
                                store.revision,
                            ))
                        };
                        #[cfg(test)]
                        let test_writer = writer.test_writer.clone();
                        #[cfg(not(test))]
                        let test_writer = ();
                        Some((pending, candidate, test_writer))
                    });
                    let Ok(Some((pending, candidate, _test_writer))) = next else {
                        break;
                    };
                    let result = match candidate {
                        Err(error) => Err(error),
                        Ok((_, true, revision)) => Ok(revision),
                        Ok((file, false, _)) => {
                            let value = pending.settings.clone();
                            let result = cx
                                .background_executor()
                                .spawn(async move {
                                    #[cfg(test)]
                                    if let Some(test_writer) = _test_writer {
                                        return test_writer(value).await;
                                    }
                                    if let Some(file) = file {
                                        file.save(&value).map_err(|error| error.to_string())?;
                                    }
                                    Ok(())
                                })
                                .await;
                            match result {
                                Ok(()) => Ok(cx.update(|cx| publish(pending.settings, cx))),
                                Err(error) => Err(error),
                            }
                        }
                    };
                    let _ = pending.done.send(result);
                }
            }));
        }
        cx.spawn(async move |_, _| {
            receive
                .await
                .unwrap_or_else(|_| Err("Settings service closed before saving completed.".into()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    fn install(store: SettingsStore, cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(store);
            init(cx);
        });
    }
    #[gpui_kit::test]
    async fn failed_save_leaves_active_settings_and_revision_intact(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        install(
            SettingsStore::new(Settings::default(), SettingsFile::new(directory.path())),
            cx,
        );
        let task =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true));
        assert!(task.await.is_err());
        cx.update(|cx| {
            assert_eq!(cx.settings(), &Settings::default());
            assert_eq!(cx.global::<SettingsStore>().revision(), 0);
        });
    }
    #[gpui_kit::test]
    async fn saved_settings_survive_reload_and_stale_draft_cannot_overwrite(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        install(SettingsStore::new(Settings::default(), file.clone()), cx);
        let task =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.font_size = Some(18.)));
        assert_eq!(task.await.unwrap(), 1);
        let mut stale = Settings::default();
        stale.terminal.copy_on_select = true;
        assert!(
            cx.update(|cx| save_settings(cx, 0, stale))
                .await
                .unwrap_err()
                .contains("Reload saved settings")
        );
        assert_eq!(file.load().unwrap().terminal.font_size, Some(18.));
        assert!(!file.load().unwrap().terminal.copy_on_select);
    }
    #[gpui_kit::test]
    async fn ordered_background_writes_reject_queued_stale_drafts_and_do_not_notify_before_commit(
        cx: &mut TestAppContext,
    ) {
        use futures::FutureExt as _;
        use std::sync::{Arc, Mutex};
        install(
            SettingsStore::new(
                Settings::default(),
                SettingsFile::new("/unused-test-writer"),
            ),
            cx,
        );
        let writer = cx.update(|cx| cx.global::<SettingsWriter>().0.clone());
        let (release, receive) = oneshot::channel::<Result<(), String>>();
        let gate = Arc::new(Mutex::new(Some(receive)));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let observed = writes.clone();
        writer.update(cx, |writer, _| {
            writer.test_writer = Some(Arc::new(move |value| {
                observed.lock().unwrap().push(value);
                let gate = gate
                    .lock()
                    .unwrap()
                    .take()
                    .expect("only the current draft is persisted");
                async { gate.await.unwrap() }.boxed()
            }))
        });
        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        cx.update(|cx| {
            let count = notifications.clone();
            cx.observe_global::<SettingsStore>(move |_| count.set(count.get() + 1))
                .detach();
        });
        let first =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.font_size = Some(18.)));
        let stale =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true));
        cx.run_until_parked();
        assert_eq!(writes.lock().unwrap().len(), 1);
        assert_eq!(notifications.get(), 0);
        cx.update(|cx| assert_eq!(cx.settings(), &Settings::default()));
        release.send(Ok(())).unwrap();
        assert_eq!(first.await.unwrap(), 1);
        assert!(stale.await.unwrap_err().contains("Reload saved settings"));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 1);
        assert_eq!(writes.lock().unwrap().len(), 1);
    }
    #[gpui_kit::test]
    async fn failed_write_does_not_invalidate_following_draft_or_wedge_writer(
        cx: &mut TestAppContext,
    ) {
        use futures::FutureExt as _;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        install(
            SettingsStore::new(
                Settings::default(),
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
        let failed =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.font_size = Some(18.)));
        let next =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true));
        assert_eq!(failed.await.unwrap_err(), "disk full");
        assert_eq!(next.await.unwrap(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        cx.update(|cx| {
            assert_eq!(cx.settings().terminal.font_size, None);
            assert!(cx.settings().terminal.copy_on_select);
        });
    }
    #[gpui_kit::test]
    fn accepted_save_survives_dropped_caller_and_reaches_disk(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        install(SettingsStore::new(Settings::default(), file.clone()), cx);
        let task =
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true));
        drop(task);
        cx.run_until_parked();
        assert!(file.load().unwrap().terminal.copy_on_select);
        cx.update(|cx| assert_eq!(cx.global::<SettingsStore>().revision(), 1));
    }

    #[gpui_kit::test]
    async fn bounded_queue_rejects_overflow_and_shutdown_without_blocking(cx: &mut TestAppContext) {
        use futures::FutureExt as _;
        use std::sync::{Arc, Mutex};
        install(
            SettingsStore::new(
                Settings::default(),
                SettingsFile::new("/unused-test-writer"),
            ),
            cx,
        );
        let writer = cx.update(|cx| cx.global::<SettingsWriter>().0.clone());
        let (release, receive) = oneshot::channel::<Result<(), String>>();
        let gate = Arc::new(Mutex::new(Some(receive)));
        writer.update(cx, |writer, _| {
            writer.test_writer = Some(Arc::new(move |_| {
                let gate = gate.lock().unwrap().take().unwrap();
                async { gate.await.unwrap() }.boxed()
            }))
        });
        let mut tasks = Vec::new();
        for _ in 0..MAX_PENDING {
            tasks.push(cx.update(|cx| {
                update_settings(cx, |settings| settings.terminal.copy_on_select = true)
            }));
        }
        assert!(
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true))
                .await
                .unwrap_err()
                .contains("queue is full")
        );
        cx.run_until_parked();
        writer.update(cx, |writer, _| writer.closing = true);
        assert!(
            cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true))
                .await
                .unwrap_err()
                .contains("closing")
        );
        release.send(Ok(())).unwrap();
        assert_eq!(tasks.remove(0).await.unwrap(), 1);
        for task in tasks {
            assert!(task.await.is_err());
        }
    }
}
