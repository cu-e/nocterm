//! Typed settings installation and durable publication through Persisted.
use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{App, AppContext as _, Entity, Subscription, Task};
use nocterm_settings::{SectionError, SettingsDocument, SettingsFile, SettingsSection};

use crate::{Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError};

pub const SETTINGS_CAPACITY: usize = 16;

/// Settings prepared before GPUI installation, with an explicit persistence backend.
pub struct SettingsConfig {
    document: SettingsDocument,
    backend: Backend,
}

enum Backend {
    Memory,
    File(SettingsFile),
    Writer(SnapshotWriter<SettingsDocument>),
}

impl SettingsConfig {
    pub fn new(document: SettingsDocument, file: SettingsFile) -> Self {
        Self {
            document,
            backend: Backend::File(file),
        }
    }
    pub fn in_memory(document: SettingsDocument) -> Self {
        Self {
            document,
            backend: Backend::Memory,
        }
    }
    pub fn with_writer(
        document: SettingsDocument,
        writer: SnapshotWriter<SettingsDocument>,
    ) -> Self {
        Self {
            document,
            backend: Backend::Writer(writer),
        }
    }
    pub fn with<S: SettingsSection>(mut self, section: S) -> Self {
        self.document.set(section);
        self
    }
    pub fn document(&self) -> &SettingsDocument {
        &self.document
    }
    pub fn register<S: SettingsSection>(&mut self) {
        self.document.register::<S>();
    }
    pub fn is_persistent(&self) -> bool {
        !matches!(self.backend, Backend::Memory)
    }

    pub fn install(self, cx: &mut App) -> SettingsHandle {
        let persistent = self.is_persistent();
        let writer: SnapshotWriter<SettingsSnapshot> = match self.backend {
            Backend::Memory => Arc::new(|_| Box::pin(async { Ok(()) })),
            Backend::File(file) => Arc::new(move |snapshot| {
                let file = file.clone();
                Box::pin(async move { file.save(&snapshot.document).map_err(StoreError::backend) })
            }),
            Backend::Writer(writer) => Arc::new(move |snapshot| writer(snapshot.document)),
        };
        let state = cx.new(|_| {
            Persisted::new(
                SettingsSnapshot {
                    document: self.document,
                    revision: 0,
                },
                writer,
                Publication::Durable,
                SETTINGS_CAPACITY,
            )
        });
        SettingsHandle { state, persistent }
    }
}

#[derive(Clone, PartialEq)]
struct SettingsSnapshot {
    document: SettingsDocument,
    revision: u64,
}

/// Explicit service dependency. It exposes immutable documents and ordered edits.
#[derive(Clone)]
pub struct SettingsHandle {
    state: Entity<Persisted<SettingsSnapshot>>,
    persistent: bool,
}

impl SettingsHandle {
    pub fn document<'a>(&self, cx: &'a App) -> &'a SettingsDocument {
        &self.state.read(cx).visible().document
    }
    pub fn setting<'a, S: SettingsSection>(&self, cx: &'a App) -> &'a S {
        self.document(cx).get::<S>()
    }
    pub fn revision(&self, cx: &App) -> u64 {
        self.state.read(cx).visible().revision
    }
    pub fn section_errors(&self, cx: &App) -> Vec<SectionError> {
        self.document(cx).errors()
    }
    pub fn is_persistent(&self) -> bool {
        self.persistent
    }

    pub fn register<S: SettingsSection>(&self, cx: &mut App) {
        self.state.update(cx, |state, _| {
            state.hydrate(std::any::TypeId::of::<S>(), |snapshot| {
                snapshot.document.register::<S>()
            });
        });
    }

    pub fn edit(
        &self,
        cx: &mut App,
        edit: impl FnOnce(&mut SettingsDocument) + 'static,
    ) -> Task<Result<u64, StoreError>> {
        let published_revision = Rc::new(Cell::new(0));
        let observed = published_revision.clone();
        let edit = move |snapshot: &mut SettingsSnapshot| {
            let before = snapshot.document.clone();
            edit(&mut snapshot.document);
            snapshot.revision += u64::from(snapshot.document != before);
            observed.set(snapshot.revision);
        };
        if !self.persistent {
            let result = self
                .state
                .update(cx, |state, cx| state.update_immediate(edit, cx));
            return Task::ready(result.map(|()| published_revision.get()));
        }
        let receipt = self.state.update(cx, |state, cx| state.update(edit, cx));
        match receipt {
            Err(error) => Task::ready(Err(error)),
            Ok(receipt) => {
                cx.spawn(async move |_| receipt.wait().await.map(|_| published_revision.get()))
            }
        }
    }

    pub fn update<S: SettingsSection>(
        &self,
        cx: &mut App,
        edit: impl FnOnce(&mut S) + 'static,
    ) -> Task<Result<u64, StoreError>> {
        self.edit(cx, move |document| {
            document.update::<S>(edit);
        })
    }

    /// Validate an edit at its actual ordered base before any publication or write.
    /// Mutations made by a rejected callback are discarded with their candidate.
    pub fn edit_checked(
        &self,
        cx: &mut App,
        edit: impl FnOnce(&mut SettingsDocument) -> Result<(), StoreError> + 'static,
    ) -> Task<Result<u64, StoreError>> {
        let published_revision = Rc::new(Cell::new(0));
        let observed = published_revision.clone();
        let edit = move |snapshot: &mut SettingsSnapshot| {
            let before = snapshot.document.clone();
            edit(&mut snapshot.document)?;
            snapshot.revision += u64::from(snapshot.document != before);
            observed.set(snapshot.revision);
            Ok(())
        };
        if !self.persistent {
            let result = self
                .state
                .update(cx, |state, cx| state.update_immediate_checked(edit, cx));
            return Task::ready(result.map(|()| published_revision.get()));
        }
        let receipt = self
            .state
            .update(cx, |state, cx| state.update_checked(edit, cx));
        match receipt {
            Err(error) => Task::ready(Err(error)),
            Ok(receipt) => {
                cx.spawn(async move |_| receipt.wait().await.map(|_| published_revision.get()))
            }
        }
    }

    /// Validate a typed section edit before inserting its sanitized result.
    pub fn update_checked<S: SettingsSection>(
        &self,
        cx: &mut App,
        edit: impl FnOnce(&mut S) -> Result<(), StoreError> + 'static,
    ) -> Task<Result<u64, StoreError>> {
        self.edit_checked(cx, move |document| {
            document.register::<S>();
            let mut section = document.get::<S>().clone();
            edit(&mut section)?;
            document.set(section);
            Ok(())
        })
    }

    /// Observe published revisions only, excluding queue admission and failures.
    pub fn observe(
        &self,
        cx: &mut App,
        mut on_change: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        let mut seen = self.revision(cx);
        cx.observe(&self.state, move |entity, cx| {
            let revision = entity.read(cx).visible().revision;
            if revision != seen {
                seen = revision;
                on_change(cx);
            }
        })
    }

    pub fn register_shutdown(&self, shutdown: &ShutdownCoordinator) -> Result<(), StoreError> {
        shutdown.register(&self.state)
    }
    pub fn capture_shutdown(
        &self,
        cx: &mut App,
    ) -> futures::future::BoxFuture<'static, Result<(), StoreError>> {
        self.state.update(cx, |state, _| state.capture_shutdown())
    }
}

#[cfg(test)]
mod tests;
