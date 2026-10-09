use std::path::PathBuf;

use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task, WeakEntity, Window};
use nocterm_core::{
    Paths,
    persist::{self, WriteQueue, Writing},
};
use nocterm_session::{Auth, CredentialId, Target};
use nocterm_workspace::{SessionSpec, Workspace};

use crate::store::{Profile, ProfileId, Profiles, Recent, Recents};

mod files;
mod recents;
mod shutdown;
mod writes;

use files::{load_profiles, load_recents, now, write_profiles};

const MAX_PENDING_WRITES: usize = 32;
const MAX_PROFILE_BYTES: usize = 64 * 1024;

enum ProfileExpectation {
    Any,
    Absent,
    Snapshot(Box<Profile>),
}
#[cfg(test)]
type ProfileWriter = std::sync::Arc<
    dyn Fn(Profiles) -> futures::future::BoxFuture<'static, Result<(), String>> + Send + Sync,
>;

/// A queued intent is rebased on the last successfully persisted state.
enum Mutation {
    Save {
        profile: Box<Profile>,
        expected: ProfileExpectation,
    },
    Move {
        id: ProfileId,
        group: Option<String>,
        before: Option<ProfileId>,
    },
    Delete(ProfileId),
    RenameGroup {
        from: String,
        to: String,
    },
    Ungroup(String),
    /// Deletes a group only while it still holds exactly the `expected` members.
    DeleteGroup {
        group: String,
        expected: Vec<ProfileId>,
    },
    Credential {
        target: Target,
        auth: Auth,
        id: CredentialId,
    },
}
struct PendingWrite {
    mutation: Mutation,
    done: oneshot::Sender<Result<(), String>>,
}
struct Write {
    pending: PendingWrite,
    candidate: Result<Profiles, String>,
    path: Option<PathBuf>,
    writing: Writing,
    #[cfg(test)]
    writer: Option<ProfileWriter>,
}

/// Saved connections and recent ones, shared by every view that lists them.
pub struct Connections {
    profiles: Profiles,
    recents: Recents,
    profiles_file: Option<PathBuf>,
    recents_file: Option<PathBuf>,
    /// Invalid user configuration remains read-only until fixed externally.
    load_error: Option<String>,
    profile_persistence_error: Option<String>,
    recents_persistence_error: Option<String>,
    queue: WriteQueue<PendingWrite>,
    writer: Option<Task<()>>,
    recents_writer: Option<Task<()>>,
    recents_revision: u64,
    recents_written_revision: u64,
    #[cfg(test)]
    test_writer: Option<ProfileWriter>,
}
struct GlobalConnections(Entity<Connections>);
impl Global for GlobalConnections {}

impl Connections {
    pub fn load(paths: &Paths) -> Self {
        let profiles_file = paths.connections_file();
        let recents_file = paths.recents_file();
        let (profiles, load_error) = load_profiles(&profiles_file);
        Self {
            profiles,
            recents: load_recents(&recents_file),
            profiles_file: Some(profiles_file),
            recents_file: Some(recents_file),
            load_error,
            ..Self::in_memory()
        }
    }
    pub fn in_memory() -> Self {
        Self {
            profiles: Profiles::default(),
            recents: Recents::default(),
            profiles_file: None,
            recents_file: None,
            load_error: None,
            profile_persistence_error: None,
            recents_persistence_error: None,
            queue: WriteQueue::new(MAX_PENDING_WRITES),
            writer: None,
            recents_writer: None,
            recents_revision: 0,
            recents_written_revision: 0,
            #[cfg(test)]
            test_writer: None,
        }
    }
    pub(crate) fn install(self, cx: &mut App) {
        let entity = cx.new(|_| self);
        shutdown::save_on_quit(entity.clone(), cx);
        cx.set_global(GlobalConnections(entity));
    }
    pub fn global(cx: &App) -> Entity<Connections> {
        cx.global::<GlobalConnections>().0.clone()
    }
    pub fn profiles(&self) -> &Profiles {
        &self.profiles
    }
    pub fn recents(&self) -> &Recents {
        &self.recents
    }
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }
    /// Failures remain visible in every Connections panel, including recent bookkeeping.
    pub fn persistence_error(&self) -> Option<&str> {
        self.profile_persistence_error
            .as_deref()
            .or(self.recents_persistence_error.as_deref())
    }

    /// Adds or updates a profile; completion means the write succeeded.
    pub fn save_profile(
        &mut self,
        profile: Profile,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
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
    ) -> Task<Result<(), String>> {
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
    ) -> Task<Result<(), String>> {
        self.place_profile(id, group, None, cx)
    }
    /// Files a connection under `group`, just before `before` (else last).
    pub fn place_profile(
        &mut self,
        id: ProfileId,
        group: Option<String>,
        before: Option<ProfileId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::Move { id, group, before }, cx)
    }
    pub fn delete_profile(
        &mut self,
        id: ProfileId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::Delete(id), cx)
    }
    /// Renames a group; an existing group with the new name absorbs it.
    pub fn rename_group(
        &mut self,
        from: String,
        to: String,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::RenameGroup { from, to }, cx)
    }
    /// Moves a group's connections to the top level and removes the group.
    pub fn ungroup(&mut self, group: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.enqueue(Mutation::Ungroup(group), cx)
    }
    /// Deletes a group and its connections, provided its members are still
    /// exactly `expected` (the ones the user was shown) when the write runs.
    pub fn delete_group(
        &mut self,
        group: String,
        expected: Vec<ProfileId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::DeleteGroup { group, expected }, cx)
    }
    #[cfg(test)]
    pub(crate) fn set_profiles_file_for_test(&mut self, path: Option<PathBuf>) {
        self.profiles_file = path;
    }
    pub fn associate_credential(
        &mut self,
        target: &Target,
        auth: &Auth,
        id: CredentialId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
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
        self.recents.record(Recent {
            options: spec.options.clone(),
            target: spec.target.clone(),
            auth: spec.auth.clone(),
            profile,
            last_used: now(),
            credential: spec.credential,
            launch: spec.launch.clone(),
        });
        self.schedule_recents(cx);
        cx.notify();
    }
    pub fn spec_for_recent(&self, recent: &Recent) -> SessionSpec {
        match recent.profile.and_then(|id| self.profiles.get(id)) {
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
