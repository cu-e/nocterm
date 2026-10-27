//! The environments of the open windows, remembered across restarts.
//!
//! Owns: the live list of open windows ([`WindowSet`]), the save of that list through one
//! `Persisted` document, and the list as it was loaded at start ([`WindowSet::previous`]).
//! Does not own: the file format and the rules (`nocterm_hubs_model::windows`), reading and
//! writing the file (the application supplies a [`SnapshotWriter`] and the loaded document),
//! opening windows (the application), the hubs a window shows (`nocterm-hubs`), views.
//! Ports used: `SnapshotWriter<WindowDocument>`, the shared `ShutdownCoordinator`.
//! Ports provided: [`WindowSet`].
//! Settings section: none. Globals: none; one `on_window_closed` subscription, kept for the
//! life of the application, applies the user's closes.
//! Invariants: windows are remembered in the order they opened; a window the user closes is
//! forgotten except the last one, which the next start restores; quitting forgets nothing,
//! because the set is captured at shutdown before windows are dropped; a document that cannot
//! be read starts an empty list, never an error the user sees.

mod error;

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, WindowId};
use nocterm_hubs_model::{Environment, OpenWindows, WindowDocument, WindowDocumentError};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter};

pub use error::WindowSetError;

/// How many snapshots may wait for the disk; later ones replace the pending one.
const WRITE_CAPACITY: usize = 16;

/// The saved list, read before the first window opens.
pub struct PreparedWindows {
    document: WindowDocument,
}

impl PreparedWindows {
    /// What reading the file gave: no file, a document, or a failure. A document that cannot be
    /// read is logged and replaced by an empty one; the next save overwrites it.
    pub fn loaded(loaded: Result<Option<WindowDocument>, WindowDocumentError>) -> Self {
        let document = loaded.unwrap_or_else(|error| {
            tracing::warn!(?error, "the open windows were not restored");
            None
        });
        Self {
            document: document.unwrap_or_default(),
        }
    }

    /// Nothing remembered, for a run that keeps no files.
    pub fn memory() -> Self {
        Self::loaded(Ok(None))
    }
}

/// The open windows of the application. Clones share one list.
#[derive(Clone)]
pub struct WindowSet {
    state: Entity<Persisted<OpenWindows<WindowId>>>,
    previous: Arc<[Environment]>,
}

impl WindowSet {
    /// Starts an empty live list that saves through `writer` and at shutdown.
    pub fn install(
        prepared: PreparedWindows,
        writer: SnapshotWriter<WindowDocument>,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, WindowSetError> {
        let writer: SnapshotWriter<OpenWindows<WindowId>> =
            Arc::new(move |windows| writer(windows.document()));
        let state = cx.new(|_| {
            Persisted::new(
                OpenWindows::default(),
                writer,
                Publication::Optimistic,
                WRITE_CAPACITY,
            )
        });
        shutdown
            .register(&state)
            .map_err(WindowSetError::Registration)?;
        let set = Self {
            state,
            previous: prepared.document.environments().cloned().collect(),
        };
        let closed = set.clone();
        cx.on_window_closed(move |cx, window| closed.edit(cx, |windows| windows.closed(window)))
            .detach();
        Ok(set)
    }

    /// The environments of the windows the previous run left open, in order.
    pub fn previous(&self) -> &[Environment] {
        &self.previous
    }

    /// The environments of the windows that are open now, in the order they opened.
    pub fn environments(&self, cx: &App) -> Vec<Environment> {
        self.state
            .read(cx)
            .visible()
            .document()
            .environments()
            .cloned()
            .collect()
    }

    /// A window opened showing `environment`.
    pub fn opened(&self, window: WindowId, environment: Environment, cx: &mut App) {
        self.edit(cx, |windows| windows.opened(window, environment));
    }

    /// An open window now shows `environment`.
    pub fn switched(&self, window: WindowId, environment: Environment, cx: &mut App) {
        self.edit(cx, |windows| windows.switched(window, environment));
    }

    fn edit(&self, cx: &mut App, edit: impl FnOnce(&mut OpenWindows<WindowId>)) {
        let result = self
            .state
            .update(cx, |state, cx| state.update_latest(edit, cx));
        // Only a set that is being captured refuses an edit, and then it has been saved.
        if let Err(error) = result {
            tracing::debug!(%error, "the open windows changed while shutting down");
        }
    }
}

#[cfg(test)]
mod tests;
