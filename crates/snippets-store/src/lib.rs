//! Shared viewless snippet library publication and checked record admission.
//!
//! Owns: prepared-load fallback, readonly diagnostics and one Persisted library owner.
//! Does not own: native I/O, identity generation, settings, controls or paste targets.
//! Ports used: supplied SnapshotWriter, shared ShutdownCoordinator. Ports provided: SnippetsHandle.
//! Settings section: none. Globals: none.
//! Invariants: durable queue32, immediate memory, original expected-record checks and readonly errors.
mod error;
mod model;
pub use error::{SnippetsError, SnippetsLoadError};
use gpui::{App, AppContext as _, Entity, Subscription, Task};
use nocterm_snippets::{Library, Snippet, SnippetStorageError};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter, StoreError};
use std::sync::Arc;
/// Acquisition is performed by composition before installing this viewless service.
pub struct PreparedLibrary {
    library: Library,
    load_error: Option<Arc<SnippetsLoadError>>,
    persistent: bool,
}
impl PreparedLibrary {
    pub fn loaded(loaded: Result<Option<Library>, SnippetStorageError>, persistent: bool) -> Self {
        let loaded = loaded.map_err(SnippetsLoadError::from).and_then(|library| {
            let library = library.unwrap_or_default();
            library.validate()?;
            Ok(library)
        });
        let (library, load_error) = match loaded {
            Ok(library) => (library, None),
            Err(error) => (Library::default(), Some(Arc::new(error))),
        };
        Self {
            library,
            load_error,
            persistent,
        }
    }
    pub fn memory() -> Self {
        Self::loaded(Ok(None), false)
    }
}
#[derive(Clone)]
pub struct SnippetsHandle(Entity<model::Snippets>);
impl SnippetsHandle {
    pub fn install(
        prepared: PreparedLibrary,
        writer: SnapshotWriter<Library>,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, StoreError> {
        let model = cx.new(|cx| {
            model::Snippets::with_writer(
                prepared.library,
                prepared.load_error,
                prepared.persistent,
                writer,
                cx,
            )
        });
        model.read(cx).register_shutdown(shutdown)?;
        Ok(Self(model))
    }
    pub fn library<'a>(&self, cx: &'a App) -> &'a Library {
        self.0.read(cx).library(cx)
    }
    pub fn writable(&self, cx: &App) -> bool {
        self.0.read(cx).writable(cx)
    }
    pub fn error(&self, cx: &App) -> Option<SnippetsError> {
        self.0.read(cx).error(cx)
    }
    pub fn save(
        &self,
        snippet: Snippet,
        expected: Option<Snippet>,
        cx: &mut App,
    ) -> Task<Result<(), SnippetsError>> {
        self.0
            .update(cx, |model, cx| model.save(snippet, expected, cx))
    }
    pub fn delete(&self, expected: Snippet, cx: &mut App) -> Task<Result<(), SnippetsError>> {
        self.0.update(cx, |model, cx| model.delete(expected, cx))
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.0, move |_, cx| changed(cx))
    }
}
#[cfg(test)]
mod tests;
