//! Connections persistence owners and immutable projections of committed effects.
use super::{Connections, ConnectionsError, MAX_PENDING_WRITES};
use crate::store::{ProfileId, ProfileMutation, Profiles, Recent, Recents};
use futures::future::BoxFuture;
use gpui_kit::{App, AppContext as _, Context, Entity};
use nocterm_core::{Paths, persist};
use nocterm_session::{Auth, CredentialId, Target};
use nocterm_store::{
    CommitPhase, Committed, Persisted, Publication, ShutdownCoordinator, SnapshotWriter, StoreError,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

#[derive(Clone, PartialEq)]
pub(super) struct ProfileSnapshot {
    profiles: Profiles,
    generation: u64,
    effects: Vec<StampedEffect>,
}
#[derive(Clone, PartialEq)]
struct StampedEffect {
    generation: u64,
    effect: Effect,
}
#[derive(Clone, PartialEq)]
enum Effect {
    Unlink(Vec<ProfileId>),
    Credential {
        target: Target,
        auth: Auth,
        id: CredentialId,
    },
}

impl ProfileSnapshot {
    fn new(profiles: Profiles) -> Self {
        Self {
            profiles,
            generation: 0,
            effects: Vec::new(),
        }
    }
    pub(super) fn apply(
        &mut self,
        mutation: &ProfileMutation,
        consumed: u64,
        load_error: Option<&Arc<persist::PersistError>>,
    ) -> Result<(), ConnectionsError> {
        let before = self.profiles.clone();
        mutation.apply(&mut self.profiles)?;
        let effect = match mutation {
            ProfileMutation::Credential { target, auth, id } => Some(Effect::Credential {
                target: target.clone(),
                auth: auth.clone(),
                id: *id,
            }),
            _ => {
                let removed = before
                    .iter()
                    .map(|profile| profile.id)
                    .filter(|id| self.profiles.get(*id).is_none())
                    .collect::<Vec<_>>();
                (!removed.is_empty()).then_some(Effect::Unlink(removed))
            }
        };
        if self.profiles == before && effect.is_none() {
            return Ok(());
        }
        if let Some(error) = load_error {
            return Err(ConnectionsError::ReadOnly(error.clone()));
        }
        self.generation += 1;
        self.effects.retain(|effect| effect.generation > consumed);
        if let Some(effect) = effect {
            self.effects.push(StampedEffect {
                generation: self.generation,
                effect,
            });
        }
        Ok(())
    }
    fn apply_effects(&self, recents: &mut Recents, consumed: u64) {
        for effect in self
            .effects
            .iter()
            .filter(|effect| effect.generation > consumed)
        {
            match &effect.effect {
                Effect::Unlink(ids) => {
                    for id in ids {
                        recents.unlink(*id);
                    }
                }
                Effect::Credential { target, auth, id } => {
                    for recent in recents
                        .iter_mut()
                        .filter(|recent| &recent.target == target && &recent.auth == auth)
                    {
                        recent.credential = Some(*id);
                    }
                }
            }
        }
    }
}

pub(super) struct Prepared {
    profiles: Profiles,
    recents: Recents,
    profiles_writer: SnapshotWriter<Profiles>,
    recents_writer: SnapshotWriter<Recents>,
    memory: bool,
}

pub(super) enum ConnectionStore {
    Prepared(Prepared),
    Installed(Installed),
}

pub(super) struct Installed {
    profiles: Entity<Persisted<ProfileSnapshot>>,
    recents: Entity<Persisted<Recents>>,
    consumed: Rc<Cell<u64>>,
    memory: bool,
}

impl ConnectionStore {
    pub(super) fn memory(profiles: Profiles, recents: Recents) -> Self {
        Self::Prepared(Prepared {
            profiles,
            recents,
            profiles_writer: Arc::new(|_| Box::pin(async { Ok(()) })),
            recents_writer: Arc::new(|_| Box::pin(async { Ok(()) })),
            memory: true,
        })
    }
    pub(super) fn backends(
        profiles: Profiles,
        recents: Recents,
        profiles_writer: SnapshotWriter<Profiles>,
        recents_writer: SnapshotWriter<Recents>,
    ) -> Self {
        Self::Prepared(Prepared {
            profiles,
            recents,
            profiles_writer,
            recents_writer,
            memory: false,
        })
    }
    pub(super) fn files(profiles: Profiles, recents: Recents, paths: &Paths) -> Self {
        let profile_path = paths.connections_file();
        let recents_path = paths.recents_file();
        Self::backends(
            profiles,
            recents,
            Arc::new(move |profiles| {
                let path = profile_path.clone();
                Box::pin(async move {
                    persist::save_preserving(&path, &profiles).map_err(StoreError::backend)
                })
            }),
            Arc::new(move |recents| {
                let path = recents_path.clone();
                Box::pin(async move { persist::save(&path, &recents).map_err(StoreError::backend) })
            }),
        )
    }
    pub(super) fn install(&mut self, cx: &mut App) -> &Installed {
        if let Self::Prepared(prepared) = self {
            let recents = cx.new(|_| {
                Persisted::new(
                    prepared.recents.clone(),
                    prepared.recents_writer.clone(),
                    Publication::Optimistic,
                    MAX_PENDING_WRITES,
                )
            });
            let consumed = Rc::new(Cell::new(0));
            let hook_recents = recents.clone();
            let hook_consumed = consumed.clone();
            let writer = prepared.profiles_writer.clone();
            let writer: SnapshotWriter<ProfileSnapshot> =
                Arc::new(move |snapshot| writer(snapshot.profiles));
            let profiles = cx.new(|_| {
                Persisted::new(
                    ProfileSnapshot::new(prepared.profiles.clone()),
                    writer,
                    Publication::Durable,
                    MAX_PENDING_WRITES,
                )
                .with_publication_hook(move |_, snapshot, cx| {
                    let consumed = hook_consumed.get();
                    let snapshot = snapshot.clone();
                    let generation = snapshot.generation;
                    if snapshot
                        .effects
                        .iter()
                        .any(|effect| effect.generation > consumed)
                        && let Err(error) = hook_recents.update(cx, |state, cx| {
                            state.update_latest(
                                move |recents| snapshot.apply_effects(recents, consumed),
                                cx,
                            )
                        })
                    {
                        tracing::error!(%error, "could not admit recent connection projection");
                    }
                    hook_consumed.set(generation);
                })
            });
            *self = Self::Installed(Installed {
                profiles,
                recents,
                consumed,
                memory: prepared.memory,
            });
        }
        self.installed().unwrap()
    }
    pub(super) fn installed(&self) -> Option<&Installed> {
        match self {
            Self::Installed(installed) => Some(installed),
            Self::Prepared(_) => None,
        }
    }
    pub(super) fn profiles<'a>(&'a self, cx: &'a App) -> &'a Profiles {
        match self {
            Self::Prepared(prepared) => &prepared.profiles,
            Self::Installed(installed) => &installed.profiles.read(cx).visible().profiles,
        }
    }
    pub(super) fn recents<'a>(&'a self, cx: &'a App) -> &'a Recents {
        match self {
            Self::Prepared(prepared) => &prepared.recents,
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

impl Installed {
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
    pub(super) fn observe(&self, cx: &mut Context<Connections>) {
        cx.observe(&self.profiles, |_, _, cx| cx.notify()).detach();
        cx.observe(&self.recents, |_, _, cx| cx.notify()).detach();
    }
    pub(super) fn register(&self, shutdown: &ShutdownCoordinator) -> Result<(), StoreError> {
        let profiles = self.profiles.clone();
        let recents = self.recents.clone();
        let consumed = self.consumed.clone();
        shutdown.register_capture(move |cx| {
            let consumed = consumed.get();
            let profiles = profiles.update(cx, |state, _| state.capture());
            let recents = recents.update(cx, |state, _| state.capture());
            let selector = ProjectionSelector::new(&profiles, recents.final_snapshot(), consumed);
            let (profiles, feed) = profiles.run_observed();
            let recents = recents.run_projected(feed, move |phase, committed| {
                selector.select(phase, committed)
            });
            join_captured(profiles, recents, cx)
        })
    }
}

struct ProjectionSelector {
    durable: (u64, u64),
    prefix: Option<((u64, u64), Recents)>,
    final_branch: ((u64, u64), Recents),
}
impl ProjectionSelector {
    fn new(
        profiles: &nocterm_store::Captured<ProfileSnapshot>,
        baseline: &Recents,
        consumed: u64,
    ) -> Self {
        let prefix = profiles.active().map(|active| {
            let mut recents = baseline.clone();
            active.snapshot().apply_effects(&mut recents, consumed);
            ((active.revision(), active.snapshot().generation), recents)
        });
        let mut final_recents = baseline.clone();
        profiles
            .final_snapshot()
            .apply_effects(&mut final_recents, consumed);
        Self {
            durable: (
                profiles.durable().revision(),
                profiles.durable().snapshot().generation,
            ),
            prefix,
            final_branch: (
                (
                    profiles.final_revision(),
                    profiles.final_snapshot().generation,
                ),
                final_recents,
            ),
        }
    }
    fn select(
        &self,
        phase: CommitPhase,
        committed: &Committed<ProfileSnapshot>,
    ) -> Result<Option<Recents>, StoreError> {
        let key = (committed.revision(), committed.snapshot().generation);
        if phase == CommitPhase::Final && key == self.final_branch.0 {
            return Ok(Some(self.final_branch.1.clone()));
        }
        if let Some((prefix_key, recents)) = &self.prefix
            && key == *prefix_key
        {
            return Ok(Some(recents.clone()));
        }
        if key == self.durable {
            return Ok(None);
        }
        Err(StoreError::backend(UnknownProjection))
    }
}
#[derive(Debug, thiserror::Error)]
#[error("captured profile projection was not one of the prepared committed snapshots")]
struct UnknownProjection;

fn join_captured(
    profiles: BoxFuture<'static, nocterm_store::CapturedOutcome<ProfileSnapshot>>,
    recents: BoxFuture<'static, nocterm_store::CapturedOutcome<Recents>>,
    cx: &App,
) -> BoxFuture<'static, Result<(), StoreError>> {
    let executor = cx.background_executor().clone();
    Box::pin(async move {
        let profiles = executor.spawn(profiles);
        let recents = executor.spawn(recents);
        let (profiles, recents) = futures::join!(profiles, recents);
        match (profiles.into_result(), recents.into_result()) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(profiles), Err(recents)) => Err(StoreError::Joined {
                first: Arc::new(profiles),
                second: Arc::new(recents),
            }),
        }
    })
}
