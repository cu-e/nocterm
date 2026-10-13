//! Prepared documents and injected writers; installation reuses the shared driver.
use crate::connections::MAX_PENDING_WRITES;
use crate::publication::{
    InstalledPublication, ProfileSnapshot, hook::ProfileCommitHook, projection::ProfilePublications,
};
use gpui::{App, AppContext as _};
use nocterm_connections_model::profiles::{Profiles, Recents};
use nocterm_store::{Persisted, Publication, SnapshotWriter};
use std::{cell::Cell, rc::Rc, sync::Arc};
pub(crate) struct Prepared {
    profiles: Profiles,
    recents: Recents,
    profiles_writer: SnapshotWriter<Profiles>,
    recents_writer: SnapshotWriter<Recents>,
    memory: bool,
}
impl Prepared {
    pub(crate) fn new(
        profiles: Profiles,
        recents: Recents,
        profiles_writer: SnapshotWriter<Profiles>,
        recents_writer: SnapshotWriter<Recents>,
        memory: bool,
    ) -> Self {
        Self {
            profiles,
            recents,
            profiles_writer,
            recents_writer,
            memory,
        }
    }
    pub(crate) fn profiles(&self) -> &Profiles {
        &self.profiles
    }
    pub(crate) fn recents(&self) -> &Recents {
        &self.recents
    }
    pub(crate) fn install(&self, cx: &mut App) -> InstalledPublication {
        let recents = cx.new(|_| {
            Persisted::new(
                self.recents.clone(),
                self.recents_writer.clone(),
                Publication::Optimistic,
                MAX_PENDING_WRITES,
            )
        });
        let consumed = Rc::new(Cell::new(0));
        let initial = ProfileSnapshot::new(self.profiles.clone());
        let publications = cx.new(|_| ProfilePublications::new(initial.publication()));
        let hook = ProfileCommitHook::new(recents.clone(), consumed.clone(), publications.clone());
        let writer = self.profiles_writer.clone();
        let writer: SnapshotWriter<ProfileSnapshot> =
            Arc::new(move |snapshot| writer(snapshot.into_profiles()));
        let profiles = cx.new(|_| {
            Persisted::new(initial, writer, Publication::Durable, MAX_PENDING_WRITES)
                .with_publication_hook(move |_, snapshot, cx| hook.publish(snapshot, cx))
        });
        InstalledPublication::new(profiles, recents, consumed, self.memory, publications)
    }
}
