//! Writes settings changes to disk one at a time, publishing each only once
//! it is saved.
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, Global, Task, WeakEntity};
use nocterm_settings::{SettingsDocument, SettingsFile};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use super::{SettingsStore, publish};

pub(super) const MAX_PENDING: usize = 16;

type Edit = Box<dyn FnOnce(&mut SettingsDocument)>;
#[cfg(test)]
pub(super) type TestWriter = Arc<
    dyn Fn(SettingsDocument) -> futures::future::BoxFuture<'static, Result<(), String>>
        + Send
        + Sync,
>;

struct Pending {
    edit: Edit,
    done: oneshot::Sender<Result<u64, String>>,
}
pub(super) struct Writer {
    pending: VecDeque<Pending>,
    task: Option<Task<()>>,
    /// Cloned into every disk write; a count above one means a write is in
    /// flight. Shutdown polls it because the app cannot be read while quitting.
    saving: Arc<()>,
    pub(super) closing: bool,
    #[cfg(test)]
    pub(super) test_writer: Option<TestWriter>,
}
pub(super) struct SettingsWriter(pub(super) Entity<Writer>);
impl Global for SettingsWriter {}

/// One queued change, ready to be written.
struct Job {
    done: oneshot::Sender<Result<u64, String>>,
    document: SettingsDocument,
    /// The revision to report when the change alters nothing.
    unchanged: Option<u64>,
    file: Option<SettingsFile>,
    saving: Arc<()>,
    #[cfg(test)]
    test_writer: Option<TestWriter>,
}

pub(crate) fn init(cx: &mut App) {
    let writer = cx.new(|_| Writer {
        pending: VecDeque::new(),
        task: None,
        saving: Arc::new(()),
        closing: false,
        #[cfg(test)]
        test_writer: None,
    });
    cx.set_global(SettingsWriter(writer.clone()));
    // The app stays borrowed while quit futures run, so only the disk write
    // already in flight can finish; queued changes are reported as lost.
    cx.on_app_quit(move |cx| {
        let (saving, queued) = writer.update(cx, |writer, _| {
            writer.closing = true;
            (writer.saving.clone(), writer.pending.len())
        });
        let executor = cx.background_executor().clone();
        async move {
            if queued > 0 {
                tracing::warn!(queued, "settings changes queued at shutdown were not saved");
            }
            let deadline = Instant::now() + Duration::from_millis(180);
            // One reference is the writer's, one is ours; the rest are writes.
            while Arc::strong_count(&saving) > 2 {
                if Instant::now() >= deadline {
                    tracing::warn!("timed out saving settings during shutdown; pending changes may not be saved");
                    break;
                }
                executor.timer(Duration::from_millis(10)).await;
            }
        }
    }).detach();
}

/// Applies `edit` to the settings current when the write runs, so edits made
/// in quick succession (autosave) never conflict with one another. Use
/// [`SettingsExt::update_setting`](super::SettingsExt::update_setting) to
/// change a single section.
pub fn edit_settings(
    cx: &mut App,
    edit: impl FnOnce(&mut SettingsDocument) + 'static,
) -> Task<Result<u64, String>> {
    let writer = cx.global::<SettingsWriter>().0.clone();
    writer.update(cx, |writer, cx| writer.push(Box::new(edit), cx))
}

impl Writer {
    fn push(&mut self, edit: Edit, cx: &mut Context<Self>) -> Task<Result<u64, String>> {
        if self.closing {
            return Task::ready(Err(
                "Settings are closing. New changes cannot be saved.".into()
            ));
        }
        let store = cx.global::<SettingsStore>();
        if store.file.is_none() {
            let mut document = store.document.clone();
            edit(&mut document);
            let store = cx.global::<SettingsStore>();
            return Task::ready(Ok(if document == store.document {
                store.revision
            } else {
                publish(document, cx)
            }));
        }
        if self.pending.len() >= MAX_PENDING {
            return Task::ready(Err(
                "Settings save queue is full. Wait for pending saves to finish and retry.".into(),
            ));
        }
        let (done, receive) = oneshot::channel();
        self.pending.push_back(Pending { edit, done });
        if self.task.is_none() {
            self.task = Some(cx.spawn(async move |writer, cx| {
                while let Some(job) = next_job(&writer, cx) {
                    let result = write(&job, cx).await;
                    let _ = job.done.send(result);
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

/// Applies the oldest queued change to the current settings.
fn next_job(writer: &WeakEntity<Writer>, cx: &mut AsyncApp) -> Option<Job> {
    writer
        .update(cx, |writer, cx| {
            let Some(pending) = writer.pending.pop_front() else {
                writer.task = None;
                return None;
            };
            let store = cx.global::<SettingsStore>();
            let mut document = store.document.clone();
            (pending.edit)(&mut document);
            Some(Job {
                done: pending.done,
                unchanged: (document == store.document).then_some(store.revision),
                document,
                file: store.file.clone(),
                saving: writer.saving.clone(),
                #[cfg(test)]
                test_writer: writer.test_writer.clone(),
            })
        })
        .ok()
        .flatten()
}

/// Saves `document` off the main thread, then publishes it.
async fn write(job: &Job, cx: &mut AsyncApp) -> Result<u64, String> {
    if let Some(revision) = job.unchanged {
        return Ok(revision);
    }
    let value = job.document.clone();
    let file = job.file.clone();
    let saving = job.saving.clone();
    #[cfg(test)]
    let test_writer = job.test_writer.clone();
    cx.background_executor()
        .spawn(async move {
            let _saving = saving;
            #[cfg(test)]
            if let Some(test_writer) = test_writer {
                return test_writer(value).await;
            }
            if let Some(file) = file {
                file.save(&value).map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        })
        .await?;
    let document = job.document.clone();
    Ok(cx.update(|cx| publish(document, cx)))
}
