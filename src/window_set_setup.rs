//! Application wiring for the saved list of open windows: where it lives and how it is
//! read and written.

use std::{io, path::Path, sync::Arc};

use nocterm_core::{Paths, persist};
use nocterm_hubs_model::WindowDocument;
use nocterm_store::{SnapshotWriter, StoreError};
use nocterm_window_set::PreparedWindows;

/// The file the open windows are saved in.
fn file_of(paths: &Paths) -> std::path::PathBuf {
    paths.state_dir().join("windows.json")
}

/// The list the previous run left in the state directory, and the writer that replaces it.
pub(crate) fn native(paths: &Paths) -> (PreparedWindows, SnapshotWriter<WindowDocument>) {
    let file = file_of(paths);
    let loaded = match read(&file) {
        Ok(text) => text.map(|text| WindowDocument::decode(&text)).transpose(),
        Err(error) => {
            tracing::warn!(%error, file = %file.display(), "the open windows could not be read");
            Ok(None)
        }
    };
    (PreparedWindows::loaded(loaded), writer(file))
}

/// Nothing remembered and nothing written, for runs that keep no files.
pub(crate) fn memory() -> (PreparedWindows, SnapshotWriter<WindowDocument>) {
    (
        PreparedWindows::memory(),
        Arc::new(|_| Box::pin(async { Ok(()) })),
    )
}

/// The text of the file, or none when there is no file yet.
fn read(file: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn writer(file: std::path::PathBuf) -> SnapshotWriter<WindowDocument> {
    Arc::new(move |document| {
        let file = file.clone();
        Box::pin(async move {
            let text = document.encode().map_err(StoreError::backend)?;
            persist::save_text(&file, &text).map_err(|error| {
                tracing::warn!(%error, file = %file.display(), "the open windows were not saved");
                StoreError::backend(error)
            })
        })
    })
}

#[cfg(test)]
mod tests;
