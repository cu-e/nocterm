//! The plugin records, one private document through the snapshot persistence service.
use std::{path::PathBuf, sync::Arc};

use gpui_kit::{App, AppContext as _, Entity, Task};
use nocterm_core::fs::{read_regular_bounded, write_private_atomic};
use nocterm_plugin_api::Cause;
use nocterm_plugin_host::{PluginRecords, RecordsDoc, RecordsError};
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};

use super::is_missing;

/// Largest records file read.
const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Replacements that may wait for the worker at once.
const PENDING_REPLACEMENTS: usize = 8;

/// The records document of the installation.
pub(crate) struct RecordsFile {
    state: Entity<Persisted<RecordsDoc>>,
}

impl RecordsFile {
    /// Reads the file once. A missing or broken file starts empty; the next change replaces it.
    pub(crate) fn open(file: PathBuf, cx: &mut App) -> Self {
        let initial = match read_regular_bounded(&file, MAX_FILE_BYTES) {
            Ok(bytes) => RecordsDoc::decode(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, file = %file.display(), "plugin records are unreadable");
                RecordsDoc::default()
            }),
            Err(error) => {
                if !is_missing(&error) {
                    tracing::warn!(%error, "plugin records are unreadable");
                }
                RecordsDoc::default()
            }
        };
        let state = cx.new(|_| {
            Persisted::new(
                initial,
                writer(file),
                Publication::Optimistic,
                PENDING_REPLACEMENTS,
            )
        });
        Self { state }
    }

    pub(crate) fn register_shutdown(
        &self,
        shutdown: &ShutdownCoordinator,
    ) -> Result<(), StoreError> {
        shutdown.register(&self.state)
    }
}

fn writer(file: PathBuf) -> SnapshotWriter<RecordsDoc> {
    Arc::new(move |doc| {
        let file = file.clone();
        Box::pin(async move {
            let bytes = doc.encode().map_err(StoreError::backend)?;
            write_private_atomic(&file, &bytes).map_err(StoreError::backend)
        })
    })
}

impl PluginRecords for RecordsFile {
    fn load(&self, cx: &App) -> RecordsDoc {
        self.state.read(cx).visible().clone()
    }

    fn replace(&self, doc: RecordsDoc, cx: &mut App) -> Task<Result<(), RecordsError>> {
        let receipt = self.state.update(cx, |state, cx| {
            state.update_checked(
                move |current| {
                    *current = doc;
                    Ok(())
                },
                cx,
            )
        });
        match receipt {
            Ok(receipt) => {
                cx.spawn(async move |_| receipt.wait().await.map(|_| ()).map_err(failed))
            }
            Err(error) => Task::ready(Err(failed(error))),
        }
    }
}

/// A refused or failed save, keeping the persistence error as the cause. A closing service is
/// its own case: nothing more will be written and the user can do nothing about it.
fn failed(error: StoreError) -> RecordsError {
    match error {
        StoreError::Closing => RecordsError::Closing,
        error => RecordsError::Write(Cause::new(error)),
    }
}
