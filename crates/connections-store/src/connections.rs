use gpui::{App, AppContext as _, Context, Entity, Task};
use nocterm_session::{Auth, CredentialId, Target};

use nocterm_connections_model::profiles::{Profile, ProfileId, Profiles, Recent, Recents};

use crate::{error::ConnectionsError, publication};
use nocterm_connections_model::recents::RecentUse;

use nocterm_connections_model::profiles::{ProfileExpectation, ProfileMutation as Mutation};
use nocterm_connections_model::{
    repository::ConnectionStorageError,
    value_ports::{ConnectionClock, SuppliedValues},
};
use std::sync::Arc;

pub(super) const MAX_PENDING_WRITES: usize = 32;

/// Saved connections and recent ones, shared by every view that lists them.
pub struct ConnectionStore {
    state: publication::PublicationDriver,
    load_error: Option<Arc<ConnectionStorageError>>,
    clock: Arc<dyn ConnectionClock>,
}
impl ConnectionStore {
    pub fn prepared(
        profiles: Profiles,
        recents: Recents,
        profiles_writer: nocterm_store::SnapshotWriter<Profiles>,
        recents_writer: nocterm_store::SnapshotWriter<Recents>,
        load_error: Option<Arc<ConnectionStorageError>>,
        clock: Arc<dyn ConnectionClock>,
    ) -> Self {
        Self {
            state: publication::PublicationDriver::backends(
                profiles,
                recents,
                profiles_writer,
                recents_writer,
            ),
            load_error,
            clock,
        }
    }
    pub fn memory_with_clock(
        profiles: Profiles,
        recents: Recents,
        clock: Arc<dyn ConnectionClock>,
    ) -> Self {
        Self {
            state: publication::PublicationDriver::memory(profiles, recents),
            load_error: None,
            clock,
        }
    }
    pub fn in_memory() -> Self {
        Self::with_documents(Profiles::default(), Recents::default())
    }
    /// A synchronous, explicitly memory-only owner, with no filesystem writes.
    pub fn with_documents(profiles: Profiles, recents: Recents) -> Self {
        Self::memory_with_clock(profiles, recents, Arc::new(SuppliedValues::new(1, 0)))
    }
    /// Alternate-backend fixture convenience; clock is the explicitly inert value source.
    pub fn with_backends(
        profiles: Profiles,
        recents: Recents,
        profiles_writer: nocterm_store::SnapshotWriter<Profiles>,
        recents_writer: nocterm_store::SnapshotWriter<Recents>,
    ) -> Self {
        Self::prepared(
            profiles,
            recents,
            profiles_writer,
            recents_writer,
            None,
            Arc::new(SuppliedValues::new(1, 0)),
        )
    }
    /// Construct an owner with explicit shutdown lifetime, without global lookup.
    pub fn into_entity(
        mut self,
        shutdown: &nocterm_store::ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Entity<Self>, nocterm_store::StoreError> {
        self.state.install(cx).register(shutdown)?;
        Ok(cx.new(|cx| {
            self.observe(cx);
            self
        }))
    }
    fn observe(&self, cx: &mut Context<Self>) {
        if let Some(installed) = self.state.installed() {
            installed.observe(cx);
        }
    }
    pub fn profiles<'a>(&'a self, cx: &'a App) -> &'a Profiles {
        self.state.profiles(cx)
    }
    pub fn recents<'a>(&'a self, cx: &'a App) -> &'a Recents {
        self.state.recents(cx)
    }
    pub fn load_error(&self) -> Option<&ConnectionStorageError> {
        self.load_error.as_deref()
    }
    /// Failures remain visible without rolling back a successfully written profile.
    pub fn persistence_error(&self, cx: &App) -> Option<nocterm_store::StoreError> {
        self.state.error(cx)
    }
    fn enqueue(
        &mut self,
        mutation: Mutation,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        if let Err(error) = mutation.validate() {
            return Task::ready(Err(error.into()));
        }
        let newly_installed = self.state.installed().is_none();
        self.state.install(cx);
        if newly_installed {
            self.observe(cx);
        }
        let installed = self.state.install(cx);
        let consumed = installed.consumed();
        let load_error = self.load_error.clone();
        let edit = move |snapshot: &mut publication::ProfileSnapshot| {
            snapshot
                .apply(&mutation, consumed.get(), load_error.as_ref())
                .map_err(nocterm_store::StoreError::edit)
        };
        if installed.memory_only() {
            let result = installed
                .profiles()
                .update(cx, |state, cx| state.update_immediate_checked(edit, cx));
            return Task::ready(result.map_err(Into::into));
        }
        match installed
            .profiles()
            .update(cx, |state, cx| state.update_checked(edit, cx))
        {
            Ok(receipt) => {
                cx.spawn(async move |_, _| receipt.wait().await.map(|_| ()).map_err(Into::into))
            }
            Err(error) => Task::ready(Err(error.into())),
        }
    }

    /// Adds or updates a profile; completion means the write succeeded.
    pub fn save_profile(
        &mut self,
        profile: Profile,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(
            Mutation::Save {
                profile: Box::new(profile),
                expected: ProfileExpectation::Any,
            },
            cx,
        )
    }
    /// An editor may create an absent profile or replace exactly its original snapshot.
    pub fn save_profile_checked(
        &mut self,
        profile: Profile,
        expected: Option<Profile>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(
            Mutation::Save {
                profile: Box::new(profile),
                expected: expected.map_or(ProfileExpectation::Absent, |profile| {
                    ProfileExpectation::Snapshot(Box::new(profile))
                }),
            },
            cx,
        )
    }
    pub fn move_profile(
        &mut self,
        id: ProfileId,
        group: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.place_profile(id, group, None, cx)
    }
    /// Files a connection under `group`, just before `before` (else last).
    pub fn place_profile(
        &mut self,
        id: ProfileId,
        group: Option<String>,
        before: Option<ProfileId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(Mutation::Move { id, group, before }, cx)
    }
    pub fn delete_profile(
        &mut self,
        id: ProfileId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(Mutation::Delete(id), cx)
    }
    /// Renames a group; an existing group with the new name absorbs it.
    pub fn rename_group(
        &mut self,
        from: String,
        to: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(Mutation::RenameGroup { from, to }, cx)
    }
    /// Moves a group's connections to the top level and removes the group.
    pub fn ungroup(
        &mut self,
        group: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(Mutation::Ungroup(group), cx)
    }
    /// Deletes a group and its connections, provided its members are still
    /// exactly `expected` (the ones the user was shown) when the write runs.
    pub fn delete_group(
        &mut self,
        group: String,
        expected: Vec<ProfileId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(Mutation::DeleteGroup { group, expected }, cx)
    }
    pub fn associate_credential(
        &mut self,
        target: &Target,
        auth: &Auth,
        id: CredentialId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConnectionsError>> {
        self.enqueue(
            Mutation::Credential {
                target: target.clone(),
                auth: auth.clone(),
                id,
            },
            cx,
        )
    }
    pub fn record_use(&mut self, request: RecentUse, cx: &mut Context<Self>) {
        let recent = Recent {
            options: request.options,
            target: request.target,
            auth: request.auth,
            profile: request.profile,
            last_used: self.clock.unix_seconds(),
            credential: request.credential,
            launch: request.launch,
        };
        let newly_installed = self.state.installed().is_none();
        self.state.install(cx);
        if newly_installed {
            self.observe(cx);
        }
        self.state.install(cx).record(recent, cx);
    }
    /// Distinct immutable carrier; callers never need to reread leased profile storage.
    pub fn publications(&mut self, cx: &mut Context<Self>) -> Entity<crate::ProfilePublications> {
        let newly_installed = self.state.installed().is_none();
        self.state.install(cx);
        if newly_installed {
            self.observe(cx);
        }
        self.state.install(cx).publications().clone()
    }
}

#[cfg(test)]
mod tests;
