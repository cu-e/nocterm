//! Serialize mutations and publish only after atomic persistence succeeds.
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use nocterm_core::{Paths, persist};
use nocterm_snippets::{Library, Snippet};
use std::{collections::VecDeque, path::PathBuf, sync::Arc};

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
    queue: VecDeque<Pending>,
    writer: Option<Task<()>>,
    saving: Arc<()>,
    closing: bool,
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
            queue: VecDeque::new(),
            writer: None,
            saving: Arc::new(()),
            closing: false,
        }
    }
    pub(crate) fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalLibrary>().0.clone()
    }
    pub(crate) fn error(&self) -> Option<&str> {
        self.load_error.as_deref().or(self.error.as_deref())
    }
    pub(crate) fn writable(&self) -> bool {
        self.load_error.is_none() && !self.closing
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
        if self.closing {
            return Task::ready(Err("Snippet service is shutting down.".into()));
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
        if self.queue.len() >= 32 {
            return Task::ready(Err(
                "Snippet save queue is full. Wait for pending saves and retry.".into(),
            ));
        }
        let (done, result) = oneshot::channel();
        self.queue.push_back(Pending { mutation, done });
        if self.writer.is_none() {
            self.writer = Some(cx.spawn(async move |this, cx| {
                loop {
                    let next = this.update(cx, |this, _| {
                        let Some(pending) = this.queue.pop_front() else {
                            this.writer = None;
                            return None;
                        };
                        let candidate = this.candidate(&pending.mutation);
                        Some((pending, candidate, this.path.clone(), this.saving.clone()))
                    });
                    let Ok(Some((pending, candidate, path, saving))) = next else {
                        break;
                    };
                    let persisted = match candidate {
                        Err(error) => Err(error),
                        Ok(library) => {
                            cx.background_executor()
                                .spawn(async move {
                                    let _saving = saving;
                                    if let Some(path) = path {
                                        persist::save(&path, &library)
                                            .map_err(|error| error.to_string())?;
                                    }
                                    Ok(library)
                                })
                                .await
                        }
                    };
                    let result = this
                        .update(cx, |this, cx| match persisted {
                            Ok(library) => {
                                this.library = library;
                                this.error = None;
                                cx.notify();
                                Ok(())
                            }
                            Err(error) => {
                                this.error = Some(error.clone());
                                cx.notify();
                                Err(error)
                            }
                        })
                        .unwrap_or_else(|_| {
                            Err("Snippet service closed before saving completed.".into())
                        });
                    let _ = pending.done.send(result);
                }
            }));
        }
        cx.spawn(async move |_, _| {
            result
                .await
                .unwrap_or_else(|_| Err("Snippet save was interrupted.".into()))
        })
    }
}

pub(crate) fn init(paths: Option<&Paths>, cx: &mut App) {
    let model = cx.new(|_| Snippets::new(paths.map(Paths::snippets_file)));
    let shutdown = model.clone();
    cx.on_app_quit(move |cx| {
        let (saving, queued) = shutdown.update(cx, |this, _| {
            this.closing = true;
            (this.saving.clone(), this.queue.len())
        });
        let executor = cx.background_executor().clone();
        async move {
            if queued > 0 {
                tracing::warn!(queued, "snippet changes queued at shutdown were not saved");
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(180);
            while Arc::strong_count(&saving) > 2 {
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
