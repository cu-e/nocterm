//! Checked snippet mutations delegate ordered durable publication to the store service.
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Subscription, Task};
use nocterm_core::{Paths, SharedError, persist};
use nocterm_snippets::{Library, LibraryError, Snippet};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};
use std::{path::PathBuf, sync::Arc};

const MAX_PENDING: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum SnippetsLoadError {
    #[error(transparent)]
    File(#[from] persist::PersistError),
    #[error(transparent)]
    Library(#[from] LibraryError),
}

#[derive(Debug, thiserror::Error)]
pub enum SnippetsError {
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error("Snippet library is read-only until its file is fixed: {0}")]
    ReadOnly(#[source] SharedError<SnippetsLoadError>),
    #[error("Snippet service is shutting down.")]
    Closing,
    #[error("Snippet save queue is full. Wait for pending saves and retry.")]
    Full,
    #[error("Snippet save was interrupted.")]
    Cancelled,
    #[error("Could not persist the snippet library: {0}")]
    Persistence(#[source] StoreError),
}

impl SnippetsError {
    pub fn read_only(source: Arc<SnippetsLoadError>) -> Self {
        Self::ReadOnly(SharedError::from_arc(source))
    }
}

impl From<StoreError> for SnippetsError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Closing => Self::Closing,
            StoreError::Full => Self::Full,
            StoreError::CompletionDropped => Self::Cancelled,
            StoreError::Edit(ref source) => source
                .downcast_ref::<LibraryError>()
                .cloned()
                .map_or(Self::Persistence(error), Self::Library),
            error => Self::Persistence(error),
        }
    }
}

struct GlobalLibrary(Entity<Snippets>);
impl Global for GlobalLibrary {}

pub(crate) struct Snippets {
    state: Entity<Persisted<Library>>,
    load_error: Option<Arc<SnippetsLoadError>>,
    persistent: bool,
    _subscription: Subscription,
}

impl Snippets {
    fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let loaded = path.as_ref().map_or(Ok(Library::default()), |path| {
            let library: Library = persist::load(path)?.unwrap_or_default();
            library.validate()?;
            Ok::<_, SnippetsLoadError>(library)
        });
        let (library, load_error) = match loaded {
            Ok(library) => (library, None),
            Err(error) => (Library::default(), Some(Arc::new(error))),
        };
        let persistent = path.is_some();
        let writer: SnapshotWriter<Library> = Arc::new(move |library| {
            let path = path.clone();
            Box::pin(async move {
                if let Some(path) = path {
                    persist::save(&path, &library).map_err(StoreError::backend)?;
                }
                Ok(())
            })
        });
        Self::with_writer(library, load_error, persistent, writer, cx)
    }

    fn with_writer(
        library: Library,
        load_error: Option<Arc<SnippetsLoadError>>,
        persistent: bool,
        writer: SnapshotWriter<Library>,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|_| Persisted::new(library, writer, Publication::Durable, MAX_PENDING));
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            load_error,
            persistent,
            _subscription: subscription,
        }
    }

    pub(crate) fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalLibrary>().0.clone()
    }
    pub(crate) fn library<'a>(&self, cx: &'a App) -> &'a Library {
        self.state.read(cx).visible()
    }
    pub(crate) fn error(&self, cx: &App) -> Option<SnippetsError> {
        self.load_error
            .as_ref()
            .map(|error| SnippetsError::read_only(error.clone()))
            .or_else(|| {
                self.state
                    .read(cx)
                    .error()
                    .cloned()
                    .map(SnippetsError::from)
            })
    }
    pub(crate) fn writable(&self, cx: &App) -> bool {
        self.load_error.is_none() && !self.state.read(cx).is_closing()
    }
    pub(crate) fn register_shutdown(
        &self,
        shutdown: &ShutdownCoordinator,
    ) -> Result<(), StoreError> {
        shutdown.register(&self.state)
    }
    pub(crate) fn save(
        &mut self,
        snippet: Snippet,
        expected: Option<Snippet>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SnippetsError>> {
        if self.state.read(cx).is_closing() {
            return Task::ready(Err(SnippetsError::Closing));
        }
        if let Err(error) = snippet.validate() {
            return Task::ready(Err(LibraryError::from(error).into()));
        }
        self.edit(move |library| library.save(snippet, expected.as_ref()), cx)
    }
    pub(crate) fn delete(
        &mut self,
        expected: Snippet,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SnippetsError>> {
        self.edit(move |library| library.delete(&expected), cx)
    }
    fn edit(
        &mut self,
        edit: impl FnOnce(&mut Library) -> Result<(), LibraryError> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SnippetsError>> {
        if self.state.read(cx).is_closing() {
            return Task::ready(Err(SnippetsError::Closing));
        }
        if let Some(error) = &self.load_error {
            return Task::ready(Err(SnippetsError::read_only(error.clone())));
        }
        let edit = move |library: &mut Library| edit(library).map_err(StoreError::edit);
        if !self.persistent {
            let result = self
                .state
                .update(cx, |state, cx| state.update_immediate_checked(edit, cx));
            cx.notify();
            return Task::ready(result.map_err(SnippetsError::from));
        }
        let receipt = self
            .state
            .update(cx, |state, cx| state.update_checked(edit, cx));
        match receipt {
            Err(error) => Task::ready(Err(error.into())),
            Ok(receipt) => cx.spawn(async move |_, _| {
                receipt
                    .wait()
                    .await
                    .map(|_| ())
                    .map_err(SnippetsError::from)
            }),
        }
    }
}

pub(crate) fn init(
    paths: Option<&Paths>,
    shutdown: &ShutdownCoordinator,
    cx: &mut App,
) -> Result<(), StoreError> {
    let model = cx.new(|cx| Snippets::new(paths.map(Paths::snippets_file), cx));
    model.read(cx).register_shutdown(shutdown)?;
    cx.set_global(GlobalLibrary(model));
    Ok(())
}
#[cfg(test)]
mod tests;
