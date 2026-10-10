//! Informational snapshots coalesce through the shared service worker.
use std::{path::PathBuf, sync::Arc};

use gpui_kit::{App, AppContext as _, Entity};
use nocterm_core::persist;
use nocterm_store::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};

use super::Known;

pub(super) enum FactsStore {
    Prepared {
        known: Known,
        writer: SnapshotWriter<Known>,
    },
    Installed(Entity<Persisted<Known>>),
}

impl FactsStore {
    pub(super) fn new(known: Known, file: Option<PathBuf>) -> Self {
        let writer: SnapshotWriter<Known> = Arc::new(move |known| {
            let file = file.clone();
            Box::pin(async move {
                if let Some(file) = file {
                    persist::save(&file, &known).map_err(StoreError::backend)?;
                }
                Ok(())
            })
        });
        Self::Prepared { known, writer }
    }

    pub(super) fn read<'a>(&'a self, cx: &'a App) -> &'a Known {
        match self {
            Self::Prepared { known, .. } => known,
            Self::Installed(entity) => entity.read(cx).visible(),
        }
    }

    pub(super) fn install(&mut self, cx: &mut App) -> Entity<Persisted<Known>> {
        match self {
            Self::Installed(entity) => entity.clone(),
            Self::Prepared { known, writer } => {
                let known = known.clone();
                let writer = writer.clone();
                let entity = cx.new(|_| Persisted::new(known, writer, Publication::Optimistic, 1));
                *self = Self::Installed(entity.clone());
                entity
            }
        }
    }

    pub(super) fn update(&mut self, edit: impl FnOnce(&mut Known), cx: &mut App) {
        let result = self
            .install(cx)
            .update(cx, |state, cx| state.update_latest(edit, cx));
        if let Err(error) = result {
            tracing::debug!(%error, "detected server details no longer accept changes");
        }
    }

    pub(super) fn register(
        &mut self,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<(), StoreError> {
        shutdown.register(&self.install(cx))
    }
}
