use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task, WeakEntity, Window};
use nocterm_core::{Paths, persist};
use nocterm_session::{Auth, CredentialId, Target};
use nocterm_workspace::{SessionSpec, Workspace};

use crate::store::{Profile, ProfileId, Profiles, Recent, Recents};

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
    },
    Delete(ProfileId),
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
    persistence_error: Option<String>,
    pending: VecDeque<PendingWrite>,
    writer: Option<Task<()>>,
    recents_writer: Option<Task<()>>,
    recents_revision: u64,
    recents_written_revision: u64,
    closing: bool,
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
            persistence_error: None,
            pending: VecDeque::new(),
            writer: None,
            recents_writer: None,
            recents_revision: 0,
            recents_written_revision: 0,
            closing: false,
            #[cfg(test)]
            test_writer: None,
        }
    }
    pub(crate) fn install(self, cx: &mut App) {
        let entity = cx.new(|_| self);
        let connections = entity.clone();
        cx.on_app_quit(move |cx| {
            connections.update(cx, |this, _| this.closing = true);
            let connections = connections.clone();
            let app = cx.to_async();
            let executor = cx.background_executor().clone();
            async move {
                // GPUI allows 200 ms for native shutdown callbacks.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(180);
                loop {
                    let busy = connections.read_with(&app, |this, _| {
                        this.writer.is_some() || this.recents_writer.is_some()
                    });
                    if !busy {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        tracing::warn!("timed out saving connections during shutdown; queued writes may be incomplete");
                        return;
                    }
                    executor.timer(std::time::Duration::from_millis(10)).await;
                }
                let final_snapshot = connections.read_with(&app, |this, _| {
                    (this.recents_revision != this.recents_written_revision)
                        .then(|| this.recents_file.clone().map(|path| (path, this.recents.clone()))).flatten()
                });
                if let Some((path, recents)) = final_snapshot {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let work = executor.spawn(async move { persist::save(&path, &recents) });
                    match futures::future::select(work, executor.timer(remaining)).await {
                        futures::future::Either::Left((Err(error), _)) => tracing::warn!(%error, "could not save final recent connections"),
                        futures::future::Either::Right(_) => tracing::warn!("timed out saving final recent connections"),
                        _ => {},
                    }
                }
            }
        })
        .detach();
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
        self.persistence_error.as_deref()
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
        self.enqueue(Mutation::Move { id, group }, cx)
    }
    pub fn delete_profile(
        &mut self,
        id: ProfileId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        self.enqueue(Mutation::Delete(id), cx)
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
    fn candidate(&self, mutation: &Mutation) -> Result<Profiles, String> {
        let mut profiles = self.profiles.clone();
        match mutation {
            Mutation::Save { profile, expected } => {
                let current = profiles.get(profile.id);
                let matches = match expected {
                    ProfileExpectation::Any => true,
                    ProfileExpectation::Absent => current.is_none(),
                    ProfileExpectation::Snapshot(expected) => current == Some(expected.as_ref()),
                };
                if !matches {
                    return Err("This connection changed or was deleted in another view. Reopen the editor to load its current values; your draft was not saved.".into());
                }
                profiles.upsert(*profile.clone());
            }
            Mutation::Move { id, group } => {
                let mut profile = profiles
                    .get(*id)
                    .cloned()
                    .ok_or("Connection no longer exists")?;
                profile.group = group.clone();
                profiles.upsert(profile);
            }
            Mutation::Delete(id) => {
                profiles.remove(*id);
            }
            Mutation::Credential { target, auth, id } => {
                for profile in profiles
                    .iter_mut()
                    .filter(|p| &p.target == target && &p.auth == auth)
                {
                    profile.credential = Some(*id);
                }
            }
        }
        if profiles != self.profiles
            && let Some(error) = &self.load_error
        {
            return Err(format!(
                "Saved connections are read-only until the file is fixed: {error}"
            ));
        }
        Ok(profiles)
    }
    fn enqueue(&mut self, mutation: Mutation, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        if self.closing {
            return Task::ready(Err("Connections service is shutting down.".into()));
        }
        let metadata_bytes = match &mutation {
            Mutation::Move { group, .. } => group.as_ref().map_or(0, String::len),
            Mutation::Credential { target, auth, .. } => target
                .host
                .len()
                .saturating_add(target.user.len())
                .saturating_add(
                    serde_json::to_string(auth).map_or(MAX_PROFILE_BYTES + 1, |text| text.len()),
                ),
            _ => 0,
        };
        if metadata_bytes > MAX_PROFILE_BYTES {
            return Task::ready(Err(
                "Connection mutation exceeds the 64 KiB size limit.".into()
            ));
        }
        if let Mutation::Save { profile, .. } = &mutation {
            match toml::to_string(profile.as_ref()) {
                Ok(text) if text.len() <= MAX_PROFILE_BYTES => {}
                Ok(_) => {
                    return Task::ready(Err(
                        "Connection fields exceed the 64 KiB size limit.".into()
                    ));
                }
                Err(error) => return Task::ready(Err(error.to_string())),
            }
        }
        if self.profiles_file.is_none() && self.pending.is_empty() {
            let result = self
                .candidate(&mutation)
                .map(|profiles| self.publish(profiles, &mutation, cx));
            return Task::ready(result);
        }
        if self.pending.len() >= MAX_PENDING_WRITES {
            return Task::ready(Err(
                "Connection save queue is full. Wait for pending saves to finish and retry.".into(),
            ));
        }
        let (done, result) = oneshot::channel();
        self.pending.push_back(PendingWrite { mutation, done });
        if self.writer.is_none() {
            self.writer = Some(cx.spawn(async move |this, cx| {
                loop {
                    let next = this.update(cx, |this, _| {
                        let Some(pending) = this.pending.pop_front() else {
                            this.writer = None;
                            return None;
                        };
                        let candidate = this.candidate(&pending.mutation);
                        let path = if candidate
                            .as_ref()
                            .is_ok_and(|profiles| profiles == &this.profiles)
                        {
                            None
                        } else {
                            this.profiles_file.clone()
                        };
                        Some(Write {
                            pending,
                            candidate,
                            path,
                            #[cfg(test)]
                            writer: this.test_writer.clone(),
                        })
                    });
                    let Ok(Some(write)) = next else {
                        break;
                    };
                    let Write {
                        pending,
                        candidate,
                        path,
                        #[cfg(test)]
                        writer,
                    } = write;
                    let result = match candidate {
                        Ok(profiles) => {
                            let (profiles, result) = cx
                                .background_executor()
                                .spawn(async move {
                                    #[cfg(test)]
                                    let result = if let Some(writer) = writer {
                                        writer(profiles.clone()).await
                                    } else {
                                        write_profiles(path, &profiles)
                                    };
                                    #[cfg(not(test))]
                                    let result = write_profiles(path, &profiles);
                                    (profiles, result)
                                })
                                .await;
                            match result {
                                Ok(()) => this
                                    .update(cx, |this, cx| {
                                        this.publish(profiles, &pending.mutation, cx);
                                        Ok(())
                                    })
                                    .unwrap_or_else(|_| {
                                        Err("Connections service closed during saving.".into())
                                    }),
                                Err(error) => Err(error),
                            }
                        }
                        Err(error) => Err(error),
                    };
                    let _ = this.update(cx, |this, cx| {
                        this.persistence_error = result.clone().err();
                        cx.notify();
                    });
                    let _ = pending.done.send(result);
                }
            }));
        }
        cx.spawn(async move |_, _| {
            result
                .await
                .unwrap_or_else(|_| Err("Connections service closed before saving.".into()))
        })
    }
    fn publish(&mut self, profiles: Profiles, mutation: &Mutation, cx: &mut Context<Self>) {
        self.profiles = profiles;
        match mutation {
            Mutation::Delete(id) => {
                self.recents.unlink(*id);
                self.schedule_recents(cx);
            }
            Mutation::Credential { target, auth, id } => {
                for recent in self
                    .recents
                    .iter_mut()
                    .filter(|r| &r.target == target && &r.auth == auth)
                {
                    recent.credential = Some(*id);
                }
                self.schedule_recents(cx);
            }
            _ => {}
        }
        cx.notify();
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
    fn schedule_recents(&mut self, cx: &mut Context<Self>) {
        self.recents_revision = self.recents_revision.wrapping_add(1);
        if self.closing || self.recents_file.is_none() || self.recents_writer.is_some() {
            return;
        }
        self.recents_writer = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok((path, recents, revision)) = this.update(cx, |this, _| {
                    (
                        this.recents_file.clone(),
                        this.recents.clone(),
                        this.recents_revision,
                    )
                }) else {
                    return;
                };
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        path.map_or(Ok(()), |path| {
                            persist::save(&path, &recents).map_err(|error| error.to_string())
                        })
                    })
                    .await;
                let again = this.update(cx, |this, cx| {
                    if result.is_ok() {
                        this.recents_written_revision = revision;
                    }
                    if let Err(error) = result {
                        this.persistence_error =
                            Some(format!("Could not save recent connections: {error}"));
                        cx.notify();
                    }
                    if this.recents_revision != revision {
                        true
                    } else {
                        this.recents_writer = None;
                        false
                    }
                });
                if !matches!(again, Ok(true)) {
                    return;
                }
            }
        }));
    }
    pub fn spec_for_recent(&self, recent: &Recent) -> SessionSpec {
        match recent.profile.and_then(|id| self.profiles.get(id)) {
            Some(profile) => spec_for_profile(profile),
            None => SessionSpec {
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

fn write_profiles(path: Option<PathBuf>, profiles: &Profiles) -> Result<(), String> {
    path.map_or(Ok(()), |path| {
        persist::save_preserving(&path, profiles).map_err(|error| error.to_string())
    })
}

fn load_profiles(path: &Path) -> (Profiles, Option<String>) {
    match persist::load::<Profiles>(path) {
        Ok(profiles) => (profiles.unwrap_or_default(), None),
        Err(error) => {
            tracing::error!(%error, "could not read saved connections");
            (Profiles::default(), Some(error.to_string()))
        }
    }
}

fn load_recents(path: &Path) -> Recents {
    persist::load(path)
        .unwrap_or_else(|error| {
            // Only bookkeeping: start afresh.
            tracing::warn!(%error, "could not read recent connections");
            None
        })
        .unwrap_or_default()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[gpui_kit::test]
    async fn moves_preserve_latest_attributes_and_roll_back_failed_storage(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let profile = Profile {
            id: ProfileId::generate(),
            name: "Build".into(),
            description: "Retained".into(),
            options: Default::default(),
            target: Target::new("ci", "build", 2222),
            auth: Auth::Password,
            credential: Some(CredentialId::generate().unwrap()),
            launch: Some(Default::default()),
            group: Some("Work".into()),
        };
        let entity = cx.new(|_| Connections::in_memory());
        entity
            .update(cx, |c, cx| c.save_profile(profile.clone(), cx))
            .await
            .unwrap();
        entity
            .update(cx, |c, cx| {
                c.move_profile(profile.id, Some("Personal".into()), cx)
            })
            .await
            .unwrap();
        let mut expected = profile.clone();
        expected.group = Some("Personal".into());
        assert_eq!(
            entity.read_with(cx, |c, _| c.profiles.get(profile.id).cloned()),
            Some(expected)
        );
        let before = entity.read_with(cx, |c, _| c.profiles.clone());
        entity.update(cx, |c, _| c.profiles_file = Some(dir.path().into()));
        assert!(
            entity
                .update(cx, |c, cx| c.move_profile(profile.id, None, cx))
                .await
                .is_err()
        );
        assert_eq!(entity.read_with(cx, |c, _| c.profiles.clone()), before);
        entity.update(cx, |c, _| {
            c.profiles_file = None;
            c.load_error = Some("Invalid TOML".into());
        });
        assert!(
            entity
                .update(cx, |c, cx| c.move_profile(profile.id, None, cx))
                .await
                .is_err()
        );
        assert_eq!(entity.read_with(cx, |c, _| c.profiles.clone()), before);
        entity
            .update(cx, |c, cx| {
                c.move_profile(profile.id, Some("Personal".into()), cx)
            })
            .await
            .unwrap();
    }

    fn profile(name: &str) -> Profile {
        Profile {
            id: ProfileId::generate(),
            name: name.into(),
            description: String::new(),
            options: Default::default(),
            target: Target::new("ci", "host", 22),
            auth: Auth::Password,
            credential: None,
            launch: None,
            group: None,
        }
    }

    #[gpui_kit::test]
    async fn ordered_writes_publish_after_success_and_reject_queued_stale_editor(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use futures::FutureExt as _;
        use std::sync::{Arc, Mutex};
        let original = profile("Build");
        let entity = cx.new(|_| Connections::in_memory());
        entity
            .update(cx, |c, cx| c.save_profile(original.clone(), cx))
            .await
            .unwrap();
        let (first_send, first_receive) = oneshot::channel::<Result<(), String>>();
        let (second_send, second_receive) = oneshot::channel::<Result<(), String>>();
        let gates = Arc::new(Mutex::new(VecDeque::from([first_receive, second_receive])));
        let writes = Arc::new(Mutex::new(Vec::<Profiles>::new()));
        let observed = writes.clone();
        entity.update(cx, |c, _| {
            c.profiles_file = Some("/not-used-by-injected-writer".into());
            c.test_writer = Some(Arc::new(move |profiles| {
                observed.lock().unwrap().push(profiles);
                let gate = gates.lock().unwrap().pop_front().unwrap();
                async { gate.await.unwrap() }.boxed()
            }));
        });
        let first = entity.update(cx, |c, cx| {
            c.move_profile(original.id, Some("A".into()), cx)
        });
        let second = entity.update(cx, |c, cx| {
            c.move_profile(original.id, Some("B".into()), cx)
        });
        let mut edited = original.clone();
        edited.description = "Old draft".into();
        let stale = entity.update(cx, |c, cx| {
            c.save_profile_checked(edited, Some(original.clone()), cx)
        });
        cx.run_until_parked();
        assert_eq!(writes.lock().unwrap().len(), 1);
        assert_eq!(
            entity.read_with(cx, |c, _| c.profiles.get(original.id).cloned()),
            Some(original.clone())
        );
        // A normal UI mutation remains runnable while disk completion is held.
        entity.update(cx, |c, _| {
            c.persistence_error = Some("UI remains responsive".into())
        });
        first_send.send(Ok(())).unwrap();
        cx.run_until_parked();
        assert_eq!(writes.lock().unwrap().len(), 2);
        assert_eq!(
            entity.read_with(cx, |c, _| c
                .profiles
                .get(original.id)
                .unwrap()
                .group
                .clone()),
            Some("A".into())
        );
        second_send.send(Ok(())).unwrap();
        first.await.unwrap();
        second.await.unwrap();
        assert!(stale.await.unwrap_err().contains("changed or was deleted"));
        assert_eq!(
            writes.lock().unwrap().len(),
            2,
            "stale draft never reaches persistence"
        );
        assert_eq!(
            entity.read_with(cx, |c, _| c
                .profiles
                .get(original.id)
                .unwrap()
                .group
                .clone()),
            Some("B".into())
        );
    }

    #[gpui_kit::test]
    async fn stale_edit_cannot_resurrect_deleted_profile_or_replace_credential(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let original = profile("Build");
        let entity = cx.new(|_| Connections::in_memory());
        entity
            .update(cx, |c, cx| c.save_profile(original.clone(), cx))
            .await
            .unwrap();
        let credential = CredentialId::generate().unwrap();
        entity
            .update(cx, |c, cx| {
                c.associate_credential(&original.target, &original.auth, credential, cx)
            })
            .await
            .unwrap();
        assert!(
            entity
                .update(cx, |c, cx| c.save_profile_checked(
                    original.clone(),
                    Some(original.clone()),
                    cx
                ))
                .await
                .is_err()
        );
        assert_eq!(
            entity.read_with(cx, |c, _| c.profiles.get(original.id).unwrap().credential),
            Some(credential)
        );
        entity
            .update(cx, |c, cx| c.delete_profile(original.id, cx))
            .await
            .unwrap();
        assert!(
            entity
                .update(cx, |c, cx| c.save_profile_checked(
                    original.clone(),
                    Some(original.clone()),
                    cx
                ))
                .await
                .is_err()
        );
        assert!(entity.read_with(cx, |c, _| c.profiles.is_empty()));
        let mut oversized = original.clone();
        oversized.description = "x".repeat(MAX_PROFILE_BYTES + 1);
        assert!(
            entity
                .update(cx, |c, cx| c.save_profile(oversized, cx))
                .await
                .is_err()
        );
    }

    #[gpui_kit::test]
    async fn queue_admission_and_shutdown_refusal_are_explicit(cx: &mut gpui_kit::TestAppContext) {
        let entity = cx.new(|_| Connections::in_memory());
        let directory = tempfile::tempdir().unwrap();
        let (_pending, rejected) = entity.update(cx, |c, cx| {
            c.profiles_file = Some(directory.path().into());
            let pending = (0..MAX_PENDING_WRITES)
                .map(|_| c.save_profile(profile("Queued"), cx))
                .collect::<Vec<_>>();
            let rejected = c.save_profile(profile("Overflow"), cx);
            (pending, rejected)
        });
        assert!(rejected.await.unwrap_err().contains("queue is full"));
        entity.update(cx, |c, _| c.closing = true);
        assert!(
            entity
                .update(cx, |c, cx| c.save_profile(profile("Shutdown"), cx))
                .await
                .unwrap_err()
                .contains("shutting down")
        );
    }

    #[test]
    fn saved_profile_keeps_description_and_session_overrides_when_opened() {
        let profile = Profile {
            id: ProfileId::generate(),
            name: "Legacy build".into(),
            description: "Windows-1251 machine\nUses a private proxy".into(),
            target: nocterm_session::Target::new("ci", "build.example", 22),
            auth: nocterm_session::Auth::Auto,
            group: None,
            credential: None,
            launch: None,
            options: nocterm_session::SessionOptions {
                term: Some("screen-256color".into()),
                charset: Some(nocterm_session::Charset::Windows1251),
                proxy: Some(nocterm_session::ProxyConfig::HttpConnect {
                    host: "127.0.0.1".into(),
                    port: 8080,
                }),
                logging: Some(nocterm_session::LoggingOptions::default()),
            },
        };
        let serialized = toml::to_string(&profile).unwrap();
        let restored: Profile = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.description, profile.description);
        assert_eq!(spec_for_profile(&restored).options, profile.options);
    }
    #[test]
    fn a_missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();

        let (profiles, error) = load_profiles(&dir.path().join("connections.toml"));

        assert!(profiles.is_empty());
        assert_eq!(error, None);
        assert_eq!(
            load_recents(&dir.path().join("recents.toml")),
            Recents::default()
        );
    }

    #[test]
    fn a_broken_file_is_reported_not_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.toml");
        fs::write(&path, "[[connection]]\nname = 3\n").unwrap();

        let (profiles, error) = load_profiles(&path);

        assert!(profiles.is_empty());
        assert!(error.unwrap().contains("connections.toml"));
    }

    #[test]
    fn broken_recents_start_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recents.toml");
        fs::write(&path, "not toml at all [").unwrap();

        assert_eq!(load_recents(&path), Recents::default());
    }
}
