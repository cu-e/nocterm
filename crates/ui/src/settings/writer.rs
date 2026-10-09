//! Writes settings changes to disk one at a time, publishing each only once
//! it is saved.
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, Global, Task, WeakEntity};
use nocterm_core::persist::{Rejected, ShutdownDeadline, WriteGate, WriteQueue, Writing};
use nocterm_settings::{SettingsDocument, SettingsFile};
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

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
    gate: WriteGate,
    candidate: Option<(SettingsDocument, Done)>,
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
    gate: WriteGate,
    #[cfg(test)]
    test_writer: Option<TestWriter>,
}

pub(crate) fn init(cx: &mut App) {
    let writer = cx.new(|_| Writer {
        queue: WriteQueue::new(MAX_PENDING),
        task: None,
        gate: WriteGate::default(),
        candidate: None,
        #[cfg(test)]
        test_writer: None,
    });
    cx.set_global(SettingsWriter(writer.clone()));
    cx.on_app_quit(move |cx| {
        let captured = writer.update(cx, |writer, cx| writer.capture_shutdown(cx));
        let executor = cx.background_executor().clone();
        let deadline = ShutdownDeadline::new(Duration::from_millis(180));
        async move {
            let Some((job, acknowledgements)) = captured else {
                return;
            };
            let work = executor.spawn(async move {
                let gate = job.gate.clone();
                let _guard = gate.final_write().await;
                let result = save(job).await;
                for (done, revision) in acknowledgements {
                    let _ = done.send(result.clone().map(|()| revision));
                }
                if let Err(error) = result {
                    tracing::warn!(%error, "could not save final settings");
                }
            });
            if matches!(
                futures::future::select(work, executor.timer(deadline.remaining())).await,
                futures::future::Either::Right(_)
            ) {
                tracing::warn!(
                    "timed out saving settings during shutdown; accepted changes may not be saved"
                );
            }
        }
    })
    .detach();
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
    fn capture_shutdown(&mut self, cx: &mut Context<Self>) -> Option<(Job, Vec<(Done, u64)>)> {
        self.gate.freeze();
        self.queue.close();
        let store = cx.global::<SettingsStore>();
        let file = store.file.clone();
        let mut revision = store.revision;
        let mut document = store.document.clone();
        let mut acknowledgements = Vec::new();
        if let Some((candidate, done)) = self.candidate.take() {
            revision += u64::from(candidate != document);
            document = candidate;
            acknowledgements.push((done, revision));
        }
        while let Some(pending) = self.queue.take() {
            let before = document.clone();
            (pending.edit)(&mut document);
            revision += u64::from(document != before);
            acknowledgements.push((pending.done, revision));
        }
        if acknowledgements.is_empty() {
            return None;
        }
        Some((
            Job {
                document,
                unchanged: None,
                file,
                writing: self.queue.begin_write(),
                gate: self.gate.clone(),
                #[cfg(test)]
                test_writer: self.test_writer.clone(),
            },
            acknowledgements,
        ))
    }

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
                let gate = self.gate.clone();
                self.task = Some(cx.spawn(async move |writer, cx| {
                    while !gate.is_frozen() {
                        let Some(job) = next_job(&writer, cx) else {
                            break;
                        };
                        let document = job.document.clone();
                        let result = write(job, cx).await;
                        if gate.is_frozen() {
                            break;
                        }
                        let _ = writer.update(cx, |writer, cx| {
                            if let Some((_, done)) = writer.candidate.take() {
                                let result = result.map(|unchanged| {
                                    unchanged.unwrap_or_else(|| publish(document, cx))
                                });
                                let _ = done.send(result);
                            }
                        });
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
fn next_job(writer: &WeakEntity<Writer>, cx: &mut AsyncApp) -> Option<Job> {
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
                gate: writer.gate.clone(),
                #[cfg(test)]
                test_writer: writer.test_writer.clone(),
            };
            writer.candidate = Some((job.document.clone(), pending.done));
            Some(job)
        })
        .ok()
        .flatten()
}

/// Saves `document` off the main thread, then publishes it.
async fn write(job: Job, cx: &mut AsyncApp) -> Result<Option<u64>, String> {
    if let Some(revision) = job.unchanged {
        return Ok(Some(revision));
    }
    let (send, receive) = oneshot::channel();
    cx.background_executor()
        .spawn(async move {
            let gate = job.gate.clone();
            let Some(_guard) = gate.normal().await else {
                return;
            };
            let _ = send.send(save(job).await);
        })
        .detach();
    receive
        .await
        .map_err(|_| "Settings writer closed before saving completed.".to_owned())??;
    Ok(None)
}

async fn save(job: Job) -> Result<(), String> {
    let value = job.document;
    let file = job.file;
    let writing = job.writing;
    #[cfg(test)]
    let test_writer = job.test_writer;
    let _writing = writing;
    #[cfg(test)]
    if let Some(test_writer) = test_writer {
        return test_writer(value).await;
    }
    if let Some(file) = file {
        file.save(&value).map_err(|error| error.to_string())?;
    }
    Ok(())
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

#[cfg(test)]
mod shutdown_tests;
