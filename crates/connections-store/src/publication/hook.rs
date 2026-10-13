//! Hook owns only recents and distinct projection carrier; it never rereads profile storage.
use super::ProfileSnapshot;
use super::projection::ProfilePublications;
use gpui::{App, Entity};
use nocterm_connections_model::profiles::Recents;
use nocterm_store::Persisted;
use std::{cell::Cell, rc::Rc};
pub(crate) struct ProfileCommitHook {
    recents: Entity<Persisted<Recents>>,
    consumed: Rc<Cell<u64>>,
    publications: Entity<ProfilePublications>,
}
impl ProfileCommitHook {
    pub(crate) fn new(
        recents: Entity<Persisted<Recents>>,
        consumed: Rc<Cell<u64>>,
        publications: Entity<ProfilePublications>,
    ) -> Self {
        Self {
            recents,
            consumed,
            publications,
        }
    }
    pub(crate) fn publish(&self, supplied: &ProfileSnapshot, cx: &mut App) {
        let consumed = self.consumed.get();
        let publication = supplied.publication();
        let generation = supplied.generation();
        if supplied.has_effects_since(consumed) {
            let effects = supplied.clone();
            if let Err(error) = self.recents.update(cx, |state, cx| {
                state.update_latest(move |recents| effects.apply_effects(recents, consumed), cx)
            }) {
                tracing::error!(%error, "could not admit recent connection projection");
            }
        }
        self.consumed.set(generation);
        self.publications
            .update(cx, |carrier, cx| carrier.publish(publication, cx));
    }
}
