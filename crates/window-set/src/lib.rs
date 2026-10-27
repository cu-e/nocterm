//! The environments of the open windows, remembered across restarts.
//!
//! Owns: the live list of open windows ([`WindowSet`]) with what each window of a hub keeps
//! (its organization and the sessions of its tabs), the save of that list through one
//! `Persisted` document, and the list as it was loaded at start ([`WindowSet::previous`]).
//! Does not own: the file format and the rules (`nocterm_hubs_model::windows`), reading and
//! writing the file (the application supplies a [`SnapshotWriter`] and the loaded document),
//! opening windows (the application), the hubs a window shows (`nocterm-hubs`), views.
//! Ports used: `SnapshotWriter<WindowDocument>`, the shared `ShutdownCoordinator`.
//! Ports provided: none; [`WindowSet`] is a concrete handle the application holds.
//! Settings section: none. Globals: none; one `on_window_closed` subscription, kept for the
//! life of the application, applies the user's closes.
//! Invariants: windows are remembered in the order they opened; a window the user closes is
//! forgotten except the last one, which the next start restores; quitting forgets nothing,
//! because the set is captured at shutdown before windows are dropped; an environment that
//! could not open in this run can be kept in the document; a document that cannot be read
//! starts an empty list, never an error the user sees, and one written by a newer version is
//! left untouched for the whole run.

mod error;

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, WindowId};
use nocterm_hubs_model::{
    Environment, HubId, HubWindowMemory, OpenWindows, WindowDocument, WindowDocumentError,
    WindowRecord,
};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter};

pub use error::WindowSetError;

/// How many snapshots may wait for the disk; later ones replace the pending one.
const WRITE_CAPACITY: usize = 16;

/// The saved list, read before the first window opens.
pub struct PreparedWindows {
    document: WindowDocument,
    /// The file belongs to a newer version of the application: this run does not write it.
    newer: bool,
}

impl PreparedWindows {
    /// What reading the file gave: no file, a document, or a failure. A document that cannot be
    /// read is logged and replaced by an empty one; the next save overwrites it. A document
    /// written by a newer version is logged, replaced by an empty one and never overwritten, so
    /// going back to the newer build finds it as it was.
    pub fn loaded(loaded: Result<Option<WindowDocument>, WindowDocumentError>) -> Self {
        let newer = matches!(loaded, Err(WindowDocumentError::UnsupportedVersion { .. }));
        let document = loaded.unwrap_or_else(|error| {
            tracing::warn!(?error, "the open windows were not restored");
            None
        });
        if let Some(document) = document.as_ref().filter(|document| document.skipped() > 0) {
            tracing::warn!(
                skipped = document.skipped(),
                "some remembered windows could not be read"
            );
        }
        Self {
            document: document.unwrap_or_default(),
            newer,
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
    previous: Arc<[WindowRecord]>,
}

impl WindowSet {
    /// Starts an empty live list that saves through `writer` and at shutdown.
    pub fn install(
        prepared: PreparedWindows,
        writer: SnapshotWriter<WindowDocument>,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, WindowSetError> {
        let writer: SnapshotWriter<OpenWindows<WindowId>> = if prepared.newer {
            Arc::new(|_| Box::pin(async { Ok(()) }))
        } else {
            Arc::new(move |windows| writer(windows.document()))
        };
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
            previous: prepared.document.records().into(),
        };
        let closed = set.clone();
        cx.on_window_closed(move |cx, window| closed.edit(cx, |windows| windows.closed(window)))
            .detach();
        Ok(set)
    }

    /// The windows the previous run left open, in order, with what each kept of its hub.
    pub fn previous(&self) -> &[WindowRecord] {
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

    /// The windows that are open now, in the order they opened, with what each keeps.
    pub fn records(&self, cx: &App) -> Vec<WindowRecord> {
        self.state.read(cx).visible().document().records().to_vec()
    }

    /// A window opened showing `environment`.
    pub fn opened(&self, window: WindowId, environment: Environment, cx: &mut App) {
        self.edit(cx, |windows| windows.opened(window, environment));
    }

    /// A window opened to bring `record` back, with what it kept of its hub, so the document
    /// still holds it if the hub cannot be reached in this run.
    pub fn restored(&self, window: WindowId, record: WindowRecord, cx: &mut App) {
        self.edit(cx, |windows| windows.restored(window, record));
    }

    /// A remembered window could not open in this run; the document keeps it for a later
    /// run instead of forgetting it with the next save.
    pub fn kept(&self, record: impl Into<WindowRecord>, cx: &mut App) {
        let record = record.into();
        self.edit(cx, |windows| windows.kept(record));
    }

    /// The window shows `hub`, and now keeps `memory` of it: its organization and the sessions
    /// of its tabs. Ignored when the window shows another server by now.
    pub fn remembered(&self, window: WindowId, hub: &HubId, memory: HubWindowMemory, cx: &mut App) {
        self.edit(cx, |windows| windows.remembered(window, hub, memory));
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
