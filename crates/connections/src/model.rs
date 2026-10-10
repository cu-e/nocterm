use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task, WeakEntity, Window};
use nocterm_core::Paths;
use nocterm_session::{Auth, CredentialId, Target};
use nocterm_workspace::{SessionSpec, Workspace};

use crate::store::{Profile, ProfileId, Profiles, Recent, Recents};

mod error;
mod files;
mod persistence;
pub use error::ConnectionsError;

use crate::store::{ProfileExpectation, ProfileMutation as Mutation};
use files::{load_profiles, load_recents, now};

const MAX_PENDING_WRITES: usize = 32;

/// Saved connections and recent ones, shared by every view that lists them.
pub struct Connections {
    state: persistence::ConnectionStore,
    load_error: Option<std::sync::Arc<nocterm_core::persist::PersistError>>,
}
struct GlobalConnections(Entity<Connections>);
impl Global for GlobalConnections {}

impl Connections {
    pub fn load(paths: &Paths) -> Self {
        let (profiles, load_error) = load_profiles(&paths.connections_file());
        Self {
            state: persistence::ConnectionStore::files(
                profiles,
                load_recents(&paths.recents_file()),
                paths,
            ),
            load_error,
        }
    }
    pub fn in_memory() -> Self {
        Self::with_documents(Profiles::default(), Recents::default())
    }
    /// A synchronous, explicitly memory-only owner, with no filesystem writes.
    pub fn with_documents(profiles: Profiles, recents: Recents) -> Self {
        Self {
            state: persistence::ConnectionStore::memory(profiles, recents),
            load_error: None,
        }
    }
    /// Explicit persistence ports; useful for alternate backends and behavior tests.
    pub fn with_backends(
        profiles: Profiles,
        recents: Recents,
        profiles_writer: nocterm_store::SnapshotWriter<Profiles>,
        recents_writer: nocterm_store::SnapshotWriter<Recents>,
    ) -> Self {
        Self {
            state: persistence::ConnectionStore::backends(
                profiles,
                recents,
                profiles_writer,
                recents_writer,
            ),
            load_error: None,
        }
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
    pub(crate) fn install(
        self,
        shutdown: &nocterm_store::ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<(), nocterm_store::StoreError> {
        let entity = self.into_entity(shutdown, cx)?;
        cx.set_global(GlobalConnections(entity));
        Ok(())
    }
    fn observe(&self, cx: &mut Context<Self>) {
        if let Some(installed) = self.state.installed() {
            installed.observe(cx);
        }
    }
    pub fn global(cx: &App) -> Entity<Connections> {
        cx.global::<GlobalConnections>().0.clone()
    }
    pub fn profiles<'a>(&'a self, cx: &'a App) -> &'a Profiles {
        self.state.profiles(cx)
    }
    pub fn recents<'a>(&'a self, cx: &'a App) -> &'a Recents {
        self.state.recents(cx)
    }
    pub fn load_error(&self) -> Option<&nocterm_core::persist::PersistError> {
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
        let installed = self.state.installed().unwrap();
        let consumed = installed.consumed();
        let load_error = self.load_error.clone();
        let edit = move |snapshot: &mut persistence::ProfileSnapshot| {
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
    pub fn record_use(
        &mut self,
        spec: &SessionSpec,
        profile: Option<ProfileId>,
        cx: &mut Context<Self>,
    ) {
        let recent = Recent {
            options: spec.options.clone(),
            target: spec.target.clone(),
            auth: spec.auth.clone(),
            profile,
            last_used: now(),
            credential: spec.credential,
            launch: spec.launch.clone(),
        };
        let newly_installed = self.state.installed().is_none();
        self.state.install(cx);
        if newly_installed {
            self.observe(cx);
        }
        self.state.installed().unwrap().record(recent, cx);
    }
    pub fn spec_for_recent(&self, recent: &Recent, cx: &App) -> SessionSpec {
        match recent.profile.and_then(|id| self.profiles(cx).get(id)) {
            Some(profile) => spec_for_profile(profile),
            None => SessionSpec {
                profile: None,
                options: recent.options.clone(),
                title: recent.target.to_string().into(),
                target: recent.target.clone(),
                auth: recent.auth.clone(),
                credential: recent.credential,
                launch: recent.launch.clone(),
            },
        }
    }
}

pub fn spec_for_profile(profile: &Profile) -> SessionSpec {
    SessionSpec {
        profile: Some(profile.id.to_string().into()),
        options: profile.options.clone(),
        title: profile.name.clone().into(),
        target: profile.target.clone(),
        auth: profile.auth.clone(),
        credential: profile.credential,
        launch: profile.launch.clone(),
    }
}

/// Opens `spec` in a new tab and remembers it among the recent connections.
pub fn connect(
    workspace: &WeakEntity<Workspace>,
    spec: SessionSpec,
    profile: Option<ProfileId>,
    window: &mut Window,
    cx: &mut App,
) {
    Connections::global(cx).update(cx, |connections, cx| {
        connections.record_use(&spec, profile, cx);
    });
    let _ = workspace.update(cx, |workspace, cx| workspace.open_session(spec, window, cx));
}

#[cfg(test)]
mod tests;
