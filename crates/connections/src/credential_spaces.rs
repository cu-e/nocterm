//! The late-bound link between a saved password and the space of the connection it was typed
//! for.
//!
//! A password remembered for a connection of one space (the user's own or an organization's)
//! must not be attached to a connection of another space that merely has the same address:
//! attaching it would change what the other space's members see. The replication that knows
//! the spaces is created after the retained installation, so this handle is bound later and is
//! cheap to clone into the credential provider.
use gpui_kit::{App, Entity};
use nocterm_connections_model::{credential_scope::CredentialScope, profiles::ProfileId};
use nocterm_connections_store::ProfileReplication;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Default)]
pub struct CredentialSpaces(Arc<OnceLock<Entity<ProfileReplication>>>);

impl CredentialSpaces {
    /// Only the first call has an effect.
    pub(crate) fn bind(&self, replication: Entity<ProfileReplication>) {
        let _ = self.0.set(replication);
    }

    pub(crate) fn replication(&self) -> Option<&Entity<ProfileReplication>> {
        self.0.get()
    }

    /// `scope` limited to the connections that live in the space of `profile` (the personal
    /// one for none). Without an account every connection is personal, so nothing is limited.
    pub fn within_space_of(
        &self,
        scope: CredentialScope,
        profile: Option<ProfileId>,
        cx: &App,
    ) -> CredentialScope {
        match self.replication() {
            Some(replication) => scope.within(replication.read(cx).in_space_of(profile, cx)),
            None => scope,
        }
    }
}

#[cfg(test)]
mod tests;
