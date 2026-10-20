//! ConnectionStore persistence owners and immutable projections of committed effects.
use crate::connections::ConnectionStore;
use crate::error::ConnectionsError;
use futures::future::BoxFuture;
use gpui::{App, Context, Entity};
use nocterm_connections_model::credential_scope::CredentialScope;
use nocterm_connections_model::profiles::{ProfileId, ProfileMutation, Profiles, Recent, Recents};
use nocterm_connections_model::repository::ConnectionStorageError;
use nocterm_session::CredentialId;
use nocterm_store::{
    CommitPhase, Committed, Persisted, ShutdownCoordinator, SnapshotWriter, StoreError,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

pub(super) enum PublicationDriver {
    Prepared(Prepared),
    Installed(InstalledPublication),
}

pub(super) struct InstalledPublication {
    profiles: Entity<Persisted<ProfileSnapshot>>,
    recents: Entity<Persisted<Recents>>,
    consumed: Rc<Cell<u64>>,
    memory: bool,
    publications: Entity<projection::ProfilePublications>,
}

impl PublicationDriver {
    pub(super) fn memory(profiles: Profiles, recents: Recents) -> Self {
        Self::Prepared(Prepared::new(
            profiles,
            recents,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Arc::new(|_| Box::pin(async { Ok(()) })),
            true,
        ))
    }
    pub(super) fn backends(
        profiles: Profiles,
        recents: Recents,
        profiles_writer: SnapshotWriter<Profiles>,
        recents_writer: SnapshotWriter<Recents>,
    ) -> Self {
        Self::Prepared(Prepared::new(
            profiles,
            recents,
            profiles_writer,
            recents_writer,
            false,
        ))
    }
    pub(super) fn install(&mut self, cx: &mut App) -> &InstalledPublication {
        match self {
            Self::Installed(installed) => installed,
            Self::Prepared(prepared) => {
                let installed = prepared.install(cx);
                *self = Self::Installed(installed);
                self.install(cx)
            }
        }
    }
    pub(super) fn installed(&self) -> Option<&InstalledPublication> {
        match self {
            Self::Installed(installed) => Some(installed),
            Self::Prepared(_) => None,
        }
    }
    pub(super) fn profiles<'a>(&'a self, cx: &'a App) -> &'a Profiles {
        match self {
            Self::Prepared(prepared) => prepared.profiles(),
            Self::Installed(installed) => installed.profiles.read(cx).visible().profiles(),
        }
    }
    pub(super) fn recents<'a>(&'a self, cx: &'a App) -> &'a Recents {
        match self {
            Self::Prepared(prepared) => prepared.recents(),
            Self::Installed(installed) => installed.recents.read(cx).visible(),
        }
    }
    pub(super) fn error(&self, cx: &App) -> Option<StoreError> {
        self.installed().and_then(|installed| {
            installed
                .profiles
                .read(cx)
                .error()
                .or_else(|| installed.recents.read(cx).error())
                .cloned()
        })
    }
}

impl InstalledPublication {
    pub(super) fn new(
        profiles: Entity<Persisted<ProfileSnapshot>>,
        recents: Entity<Persisted<Recents>>,
        consumed: Rc<Cell<u64>>,
        memory: bool,
        publications: Entity<projection::ProfilePublications>,
    ) -> Self {
        Self {
            profiles,
            recents,
            consumed,
            memory,
            publications,
        }
    }
    pub(super) fn publications(&self) -> &Entity<projection::ProfilePublications> {
        &self.publications
    }
    pub(super) fn register(&self, shutdown: &ShutdownCoordinator) -> Result<(), StoreError> {
        capture::register(
            self.profiles.clone(),
            self.recents.clone(),
            self.consumed.clone(),
            shutdown,
        )
    }

    pub(super) fn profiles(&self) -> &Entity<Persisted<ProfileSnapshot>> {
        &self.profiles
    }
    pub(super) fn consumed(&self) -> Rc<Cell<u64>> {
        self.consumed.clone()
    }
    pub(super) fn memory_only(&self) -> bool {
        self.memory
    }
    pub(super) fn record(&self, recent: Recent, cx: &mut App) {
        if let Err(error) = self.recents.update(cx, |state, cx| {
            state.update_latest(move |recents| recents.record(recent), cx)
        }) {
            tracing::error!(%error, "could not admit recent connection");
        }
    }
    pub(super) fn observe(&self, cx: &mut Context<ConnectionStore>) {
        cx.observe(&self.profiles, |_, _, cx| cx.notify()).detach();
        cx.observe(&self.recents, |_, _, cx| cx.notify()).detach();
    }
}

mod snapshot;
pub(crate) use snapshot::ProfileSnapshot;
mod capture;
pub(crate) mod hook;
pub(crate) mod projection;
use crate::prepared::Prepared;
