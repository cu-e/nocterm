//! Serialize mutations and publish only after atomic persistence succeeds.
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, Global, Task, WeakEntity};
use nocterm_core::{
    Paths,
    persist::{self, Rejected, WriteQueue},
};
use nocterm_snippets::{Library, Snippet};
use std::path::PathBuf;

const MAX_PENDING: usize = 32;

enum Mutation {
    Save(Snippet, Option<Snippet>),
    Delete(Snippet),
}
struct Pending {
    mutation: Mutation,
    done: oneshot::Sender<Result<(), String>>,
}
struct GlobalLibrary(Entity<Snippets>);
impl Global for GlobalLibrary {}

pub(crate) struct Snippets {
    pub(crate) library: Library,
    path: Option<PathBuf>,
    load_error: Option<String>,
    error: Option<String>,
    queue: WriteQueue<Pending>,
    writer: Option<Task<()>>,
}
impl Snippets {
    fn new(path: Option<PathBuf>) -> Self {
        let loaded = path.as_ref().map_or(Ok(Library::default()), |path| {
            let library: Library = persist::load(path)
                .map_err(|error| error.to_string())?
                .unwrap_or_default();
            library.validate()?;
            Ok(library)
        });
        let (library, load_error) = match loaded {
            Ok(library) => (library, None),
            Err(error) => (Library::default(), Some(error)),
        };
        Self {
            library,
            path,
            load_error,
            error: None,
            queue: WriteQueue::new(MAX_PENDING),
            writer: None,
        }
    }
    pub(crate) fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalLibrary>().0.clone()
    }
    pub(crate) fn error(&self) -> Option<&str> {
        self.load_error.as_deref().or(self.error.as_deref())
    }
    pub(crate) fn writable(&self) -> bool {
        self.load_error.is_none() && !self.queue.is_closing()
    }
    pub(crate) fn save(
        &mut self,
        snippet: Snippet,
        expected: Option<Snippet>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::Save(snippet, expected), cx)
    }
    pub(crate) fn delete(
        &mut self,
        expected: Snippet,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::Delete(expected), cx)
    }
    fn candidate(&self, mutation: &Mutation) -> Result<Library, String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "Snippet library is read-only until its file is fixed: {error}"
            ));
        }
        let mut candidate = self.library.clone();
        match mutation {
            Mutation::Save(snippet, expected) => {
                candidate.save(snippet.clone(), expected.as_ref())?
            }
            Mutation::Delete(expected) => candidate.delete(expected)?,
        }
        Ok(candidate)
    }
    fn enqueue(&mut self, mutation: Mutation, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        if self.queue.is_closing() {
            return Task::ready(Err(rejection(Rejected::Closing)));
        }
        if let Mutation::Save(snippet, _) = &mutation
            && let Err(error) = snippet.validate()
        {
            return Task::ready(Err(error.to_string()));
        }
        if self.path.is_none() {
            let result = self.candidate(&mutation).map(|library| {
                self.library = library;
                self.error = None;
                cx.notify();
            });
            return Task::ready(result);
        }
        let (done, result) = oneshot::channel();
        match self.queue.push(Pending { mutation, done }) {
            Err(rejected) => return Task::ready(Err(rejection(rejected))),
            Ok(false) => {}
            Ok(true) => {
                self.writer = Some(cx.spawn(async move |this, cx| {
                    while let Some(pending) = next(&this, cx) {
                        let result = write(&pending.mutation, &this, cx).await;
                        let _ = pending.done.send(result);
                    }
                }))
            }
        }
        cx.spawn(async move |_, _| {
            result
                .await
                .unwrap_or_else(|_| Err("Snippet save was interrupted.".into()))
        })
    }
}

fn rejection(rejected: Rejected) -> String {
    match rejected {
        Rejected::Closing => "Snippet service is shutting down.",
        Rejected::Full => "Snippet save queue is full. Wait for pending saves and retry.",
    }
    .into()
}

fn next(this: &WeakEntity<Snippets>, cx: &mut AsyncApp) -> Option<Pending> {
    this.update(cx, |this, _| {
        let pending = this.queue.take();
        if pending.is_none() {
            this.writer = None;
        }
        pending
    })
    .ok()
    .flatten()
}

/// Applies `mutation` to the saved library, writes it off the main thread
/// and publishes it once saved.
async fn write(
    mutation: &Mutation,
    this: &WeakEntity<Snippets>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let closed = || "Snippet service closed before saving completed.".to_string();
    let (candidate, path, writing) = this
        .update(cx, |this, _| {
            (
                this.candidate(mutation),
                this.path.clone(),
                this.queue.begin_write(),
            )
        })
        .map_err(|_| closed())?;
    let persisted = match candidate {
        Err(error) => Err(error),
        Ok(library) => {
            cx.background_executor()
                .spawn(async move {
                    let _writing = writing;
                    if let Some(path) = path {
                        persist::save(&path, &library).map_err(|error| error.to_string())?;
                    }
                    Ok(library)
                })
                .await
        }
    };
    this.update(cx, |this, cx| {
        cx.notify();
        match persisted {
            Ok(library) => {
                this.library = library;
                this.error = None;
                Ok(())
            }
            Err(error) => {
                this.error = Some(error.clone());
                Err(error)
            }
        }
    })
    .unwrap_or_else(|_| Err(closed()))
}

pub(crate) fn init(paths: Option<&Paths>, cx: &mut App) {
    let model = cx.new(|_| Snippets::new(paths.map(Paths::snippets_file)));
    let shutdown = model.clone();
    cx.on_app_quit(move |cx| {
        let (in_flight, queued) =
            shutdown.update(cx, |this, _| (this.queue.in_flight(), this.queue.close()));
        let executor = cx.background_executor().clone();
        async move {
            if queued > 0 {
                tracing::warn!(queued, "snippet changes queued at shutdown were not saved");
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(180);
            while in_flight.count() > 0 {
                if std::time::Instant::now() >= deadline {
                    tracing::warn!("timed out saving snippets at shutdown");
                    break;
                }
                executor.timer(std::time::Duration::from_millis(10)).await;
            }
        }
    })
    .detach();
    cx.set_global(GlobalLibrary(model));
}
#[cfg(test)]
mod tests;
