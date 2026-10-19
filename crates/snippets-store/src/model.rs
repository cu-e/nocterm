//! Viewless checked transitions over the common ordered persistence owner.
use crate::{SnippetsError, SnippetsLoadError};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task};
use nocterm_snippets::{Library, LibraryError, Snippet};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};
use std::sync::Arc;
const MAX_PENDING: usize = 32;
pub(crate) struct Snippets {
    state: Entity<Persisted<Library>>,
    load_error: Option<Arc<SnippetsLoadError>>,
    persistent: bool,
    _subscription: Subscription,
}

impl Snippets {
    pub(crate) fn with_writer(
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
