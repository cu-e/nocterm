use crate::ProfilePublications;
use gpui::{App, Entity};
use nocterm_connections_model::{profile_authority::ProfilePublication, profiles::ProfileId};
pub(super) struct ProfileAuthority {
    carrier: Entity<ProfilePublications>,
    last: ProfilePublication,
}
impl ProfileAuthority {
    pub(super) fn new(carrier: Entity<ProfilePublications>, cx: &App) -> Self {
        let last = carrier.read(cx).current().clone();
        Self { carrier, last }
    }
    /// Reads the ACTUAL distinct carrier on every call, even before observers execute.
    pub(super) fn synchronize(&mut self, cx: &App) -> (ProfilePublication, Vec<ProfileId>, bool) {
        let current = self.carrier.read(cx).current().clone();
        let removed = self
            .last
            .iter()
            .filter_map(|(id, old)| {
                (!current.get(id).is_some_and(|row| {
                    row.stamp.incarnation == old.stamp.incarnation
                        && row.stamp.endpoint == old.stamp.endpoint
                }))
                .then_some(id)
            })
            .collect();
        let changed = current != self.last;
        self.last = current.clone();
        (current, removed, changed)
    }
}
