//! Reading and writing the connection files.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use nocterm_core::persist;

use crate::store::{Profiles, Recents};

pub(super) fn write_profiles(path: Option<PathBuf>, profiles: &Profiles) -> Result<(), String> {
    path.map_or(Ok(()), |path| {
        persist::save_preserving(&path, profiles).map_err(|error| error.to_string())
    })
}

pub(super) fn load_profiles(path: &Path) -> (Profiles, Option<String>) {
    match persist::load::<Profiles>(path) {
        Ok(profiles) => (profiles.unwrap_or_default(), None),
        Err(error) => {
            tracing::error!(%error, "could not read saved connections");
            (Profiles::default(), Some(error.to_string()))
        }
    }
}

pub(super) fn load_recents(path: &Path) -> Recents {
    persist::load(path)
        .unwrap_or_else(|error| {
            // Only bookkeeping: start afresh.
            tracing::warn!(%error, "could not read recent connections");
            None
        })
        .unwrap_or_default()
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
