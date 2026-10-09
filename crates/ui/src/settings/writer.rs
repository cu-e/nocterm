//! Writes settings changes to disk one at a time, publishing each only once
//! it is saved.
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, Global, Task, WeakEntity};
use nocterm_core::persist::{Rejected, WriteQueue, Writing};
use nocterm_settings::{SettingsDocument, SettingsFile};
#[cfg(test)]
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{SettingsStore, publish};

pub(super) const MAX_PENDING: usize = 16;

type Edit = Box<dyn FnOnce(&mut SettingsDocument)>;
#[cfg(test)]
pub(super) type TestWriter = Arc<
    dyn Fn(SettingsDocument) -> futures::future::BoxFuture<'static, Result<(), String>>
        + Send
        + Sync,
>;

pub(super) struct Pending {
    edit: Edit,
    done: oneshot::Sender<Result<u64, String>>,
}
pub(super) struct Writer {
    pub(super) queue: WriteQueue<Pending>,
    task: Option<Task<()>>,
    #[cfg(test)]
    pub(super) test_writer: Option<TestWriter>,
}
pub(super) struct SettingsWriter(pub(super) Entity<Writer>);
impl Global for SettingsWriter {}

/// One queued change, ready to be written.
struct Job {
    document: SettingsDocument,
    /// The revision to report when the change alters nothing.
    unchanged: Option<u64>,
    file: Option<SettingsFile>,
    writing: Writing,
    #[cfg(test)]
    test_writer: Option<TestWriter>,
}

pub(crate) fn init(cx: &mut App) {
    let writer = cx.new(|_| Writer {
        queue: WriteQueue::new(MAX_PENDING),
        task: None,
        #[cfg(test)]
        test_writer: None,
    });
    cx.set_global(SettingsWriter(writer.clone()));
    // The app stays borrowed while quit futures run, so only the disk write
    // already in flight can finish; queued changes are reported as lost.
    cx.on_app_quit(move |cx| {
        let (in_flight, queued) =
            writer.update(cx, |writer, _| (writer.queue.in_flight(), writer.queue.close()));
        let executor = cx.background_executor().clone();
        async move {
            if queued > 0 {
                tracing::warn!(queued, "settings changes queued at shutdown were not saved");
            }
            let deadline = Instant::now() + Duration::from_millis(180);
            while in_flight.count() > 0 {
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
        if self.queue.is_closing() {
            return Task::ready(Err(rejection(Rejected::Closing)));
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
        let (done, receive) = oneshot::channel();
        match self.queue.push(Pending { edit, done }) {
            Err(rejected) => return Task::ready(Err(rejection(rejected))),
            Ok(false) => {}
            Ok(true) => {
                self.task = Some(cx.spawn(async move |writer, cx| {
                    while let Some((done, job)) = next_job(&writer, cx) {
                        let _ = done.send(write(job, cx).await);
                    }
                }))
            }
        }
        cx.spawn(async move |_, _| {
            receive
                .await
                .unwrap_or_else(|_| Err("Settings service closed before saving completed.".into()))
        })
    }
}

type Done = oneshot::Sender<Result<u64, String>>;

/// Applies the oldest queued change to the current settings.
fn next_job(writer: &WeakEntity<Writer>, cx: &mut AsyncApp) -> Option<(Done, Job)> {
    writer
        .update(cx, |writer, cx| {
            let Some(pending) = writer.queue.take() else {
                writer.task = None;
                return None;
            };
            let store = cx.global::<SettingsStore>();
            let mut document = store.document.clone();
            (pending.edit)(&mut document);
            let job = Job {
                unchanged: (document == store.document).then_some(store.revision),
                document,
                file: store.file.clone(),
                writing: writer.queue.begin_write(),
                #[cfg(test)]
                test_writer: writer.test_writer.clone(),
            };
            Some((pending.done, job))
        })
        .ok()
        .flatten()
}

/// Saves `document` off the main thread, then publishes it.
async fn write(job: Job, cx: &mut AsyncApp) -> Result<u64, String> {
    if let Some(revision) = job.unchanged {
        return Ok(revision);
    }
    let value = job.document.clone();
    let file = job.file;
    let writing = job.writing;
    #[cfg(test)]
    let test_writer = job.test_writer;
    cx.background_executor()
        .spawn(async move {
            let _writing = writing;
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
    Ok(cx.update(|cx| publish(job.document, cx)))
}

fn rejection(rejected: Rejected) -> String {
    match rejected {
        Rejected::Closing => "Settings are closing. New changes cannot be saved.",
        Rejected::Full => {
            "Settings save queue is full. Wait for pending saves to finish and retry."
        }
    }
    .into()
}
