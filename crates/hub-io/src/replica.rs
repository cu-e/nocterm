//! The saved profiles on the account hub, one sealed object per profile.
//!
//! [`HubProfileReplica`] implements the replica port of the connection store over the hub's
//! `profiles` collections: the account's own (the personal scope) and every organization's
//! the account holds the key of (an organization scope). It works only while the vault is
//! unlocked, because the keys that seal profiles are the vault's; locked or unreachable is
//! "unavailable", never an error. The hub decides who may write to a group: a guest's write
//! is [`ReplicaError::Forbidden`], and an organization without a collection of profiles, or
//! without a key granted to the account, is a scope the account cannot write to either.

mod watch;
mod wire;

use std::sync::{Arc, Mutex, PoisonError};

use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use nocterm_connections_model::profiles::{Profile, ProfileId};
use nocterm_connections_model::replica::{
    CredentialCopy, ProfileReplica, ReplicaAccess, ReplicaCursor, ReplicaError, ReplicaPage,
    ReplicaScope, ReplicaSpace, ReplicaVersion,
};
use nocterm_core::SharedCause;
use nocterm_hubs_model::HubError;
use nocterm_session::CredentialId;
use nocterm_vault::VaultError;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use self::wire::{KIND, Positions};
use crate::account::{OpenKeys, Slot};
use crate::classify::unreachable;
use crate::store::{Layout, Located, OpError, PROFILES_KIND, Store, StoreOps};
use crate::task::spawn;
use crate::vault::{Ledger, Space};

/// The spaces as the last synchronization found them.
type Spaces = Arc<Mutex<Vec<ReplicaSpace>>>;

pub struct HubProfileReplica {
    handle: Handle,
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
    spaces: Spaces,
    /// The vaults, which move a profile's secrets with it.
    ledger: Arc<Ledger>,
}

/// A failure of a store call as the replica reports it. `id` names the profile a write was
/// about.
fn replica_error(error: OpError, id: Option<ProfileId>) -> ReplicaError {
    match (error, id) {
        (OpError::Conflict, Some(id)) => ReplicaError::Conflict { id },
        (OpError::Hub(HubError::NotFound), Some(id)) => ReplicaError::Conflict { id },
        (OpError::Hub(HubError::PermissionDenied), Some(id)) => ReplicaError::Forbidden { id },
        (OpError::Hub(error), _) if unreachable(&error) => ReplicaError::Unavailable,
        (error, _) => ReplicaError::Other(SharedCause::new(error)),
    }
}

impl HubProfileReplica {
    pub(crate) fn new(
        handle: Handle,
        slot: Arc<Slot>,
        ops: Arc<dyn StoreOps>,
        ledger: Arc<Ledger>,
    ) -> Self {
        Self {
            handle,
            slot,
            ops,
            spaces: Spaces::default(),
            ledger,
        }
    }

    fn store(slot: &Slot, ops: &Arc<dyn StoreOps>) -> Result<Store, ReplicaError> {
        let keys: Arc<OpenKeys> = slot.open().ok_or(ReplicaError::Unavailable)?;
        Ok(Store::new(Arc::clone(ops), keys))
    }

    /// Runs `work` on the adapter's threads.
    fn run<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = Result<T, ReplicaError>> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, ReplicaError>> {
        let task = spawn(&self.handle, work);
        Box::pin(async move { task.await.unwrap_or(Err(ReplicaError::Unavailable)) })
    }
}

/// Where the replica reads and writes: the account's own collection, then each group's.
fn scoped(layout: &Layout) -> Vec<(ReplicaScope, &Located)> {
    let personal = layout
        .personal
        .iter()
        .map(|at| (ReplicaScope::Personal, at));
    let groups = layout
        .groups
        .iter()
        .map(|group| (ReplicaScope::Organization(group.org_id.clone()), &group.at));
    personal.chain(groups).collect()
}

fn spaces_of(layout: &Layout) -> Vec<ReplicaSpace> {
    let personal = ReplicaSpace {
        scope: ReplicaScope::Personal,
        name: "Personal".to_owned(),
        access: ReplicaAccess::Write,
    };
    let groups = layout.groups.iter().map(|group| ReplicaSpace {
        scope: ReplicaScope::Organization(group.org_id.clone()),
        name: group.name.clone(),
        access: if group.writable {
            ReplicaAccess::Write
        } else {
            ReplicaAccess::Read
        },
    });
    std::iter::once(personal).chain(groups).collect()
}

/// A group that no longer lets the account read it is left out; anything else is a failure.
fn group_unreadable(error: &OpError) -> bool {
    matches!(
        error,
        OpError::Hub(HubError::NotFound | HubError::PermissionDenied)
    )
}

async fn changes(
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
    spaces: Spaces,
    after: Option<ReplicaCursor>,
) -> Result<ReplicaPage, ReplicaError> {
    let store = HubProfileReplica::store(&slot, &ops)?;
    let mut positions = Positions::parse(after.as_ref());
    let layout = store
        .layout(PROFILES_KIND)
        .await
        .map_err(|error| replica_error(error, None))?;
    *spaces.lock().unwrap_or_else(PoisonError::into_inner) = spaces_of(&layout);
    let mut seen = Vec::new();
    let mut more = false;
    for (scope, at) in scoped(&layout) {
        let from = positions.get(at.id());
        let batch = match store.changes(at, from).await {
            Ok(batch) => batch,
            Err(error) if scope != ReplicaScope::Personal && group_unreadable(&error) => continue,
            Err(error) => return Err(replica_error(error, None)),
        };
        let mut position = from;
        for object in &batch.changes {
            position = position.max(object.stream_version);
            if let Some(profile) = wire::read(scope.clone(), at.id(), at.key(), object)? {
                seen.push(profile);
            }
        }
        if !batch.more {
            position = position.max(batch.head);
        }
        positions.set(at.id(), position);
        more |= batch.more;
    }
    Ok(ReplicaPage {
        changes: seen,
        cursor: positions.cursor(),
        more,
    })
}

/// The collection a write in `scope` goes to. A group's must exist and be keyed for the
/// account; this client never creates one.
async fn target(
    store: &Store,
    scope: &ReplicaScope,
    id: ProfileId,
    create: bool,
) -> Result<Located, ReplicaError> {
    let located = match scope {
        ReplicaScope::Personal if create => store.ensure(PROFILES_KIND).await.map(Some),
        ReplicaScope::Personal => store.find(PROFILES_KIND).await,
        ReplicaScope::Organization(org_id) => store.shared(PROFILES_KIND, org_id).await,
    }
    .map_err(|error| replica_error(error, None))?;
    located.ok_or(match scope {
        ReplicaScope::Personal => ReplicaError::Conflict { id },
        ReplicaScope::Organization(_) => ReplicaError::Forbidden { id },
    })
}

async fn put(
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
    scope: ReplicaScope,
    profile: Profile,
    expected: Option<ReplicaVersion>,
) -> Result<ReplicaVersion, ReplicaError> {
    let id = profile.id;
    let store = HubProfileReplica::store(&slot, &ops)?;
    let plaintext =
        wire::encode(&profile).map_err(|error| ReplicaError::Other(SharedCause::new(error)))?;
    let at = target(&store, &scope, id, true).await?;
    let stored = store
        .put(
            &at,
            KIND,
            &id.to_string(),
            &plaintext,
            expected.map_or(0, ReplicaVersion::get),
        )
        .await
        .map_err(|error| replica_error(error, Some(id)))?;
    Ok(ReplicaVersion::new(stored.version))
}

async fn delete(
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
    scope: ReplicaScope,
    id: ProfileId,
    expected: ReplicaVersion,
) -> Result<ReplicaVersion, ReplicaError> {
    let store = HubProfileReplica::store(&slot, &ops)?;
    let at = target(&store, &scope, id, false).await?;
    let stored = store
        .remove(&at, &id.to_string(), expected.get())
        .await
        .map_err(|error| replica_error(error, Some(id)))?;
    Ok(ReplicaVersion::new(stored.version))
}

fn space_of(scope: ReplicaScope) -> Space {
    match scope {
        ReplicaScope::Personal => Space::Personal,
        ReplicaScope::Organization(org_id) => Space::Group(org_id),
    }
}

/// A failure of a vault call as the replica reports it: a vault that is read-only, or holds
/// no key of the group, refuses the write like the group's profiles would.
fn carry_error(error: VaultError, id: ProfileId) -> ReplicaError {
    match error {
        VaultError::ReadOnly => ReplicaError::Forbidden { id },
        VaultError::Locked | VaultError::Offline => ReplicaError::Unavailable,
        VaultError::Conflict => ReplicaError::Conflict { id },
        other => ReplicaError::Other(SharedCause::new(other)),
    }
}

impl ProfileReplica for HubProfileReplica {
    fn carry(
        &self,
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    ) -> BoxFuture<'static, Result<Vec<CredentialId>, ReplicaError>> {
        let ledger = Arc::clone(&self.ledger);
        self.run(async move {
            ledger
                .carry(&space_of(from), &space_of(to), &credentials)
                .await
                .map_err(|error| carry_error(error, id))
        })
    }

    fn copy(
        &self,
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    ) -> BoxFuture<'static, Result<Vec<CredentialCopy>, ReplicaError>> {
        let ledger = Arc::clone(&self.ledger);
        self.run(async move {
            let copies = ledger
                .copy(&space_of(from), &space_of(to), &credentials)
                .await
                .map_err(|error| carry_error(error, id))?;
            Ok(copies
                .into_iter()
                .map(|(from, to)| CredentialCopy { from, to })
                .collect())
        })
    }

    fn changes(
        &self,
        after: Option<ReplicaCursor>,
    ) -> BoxFuture<'static, Result<ReplicaPage, ReplicaError>> {
        self.run(changes(
            Arc::clone(&self.slot),
            Arc::clone(&self.ops),
            Arc::clone(&self.spaces),
            after,
        ))
    }

    fn put(
        &self,
        scope: ReplicaScope,
        profile: Profile,
        expected: Option<ReplicaVersion>,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        self.run(put(
            Arc::clone(&self.slot),
            Arc::clone(&self.ops),
            scope,
            profile,
            expected,
        ))
    }

    fn delete(
        &self,
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        self.run(delete(
            Arc::clone(&self.slot),
            Arc::clone(&self.ops),
            scope,
            id,
            expected,
        ))
    }

    fn spaces(&self) -> Vec<ReplicaSpace> {
        self.spaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn watch(&self) -> BoxStream<'static, ()> {
        let (ticks, receiver) = mpsc::channel(1);
        self.handle.spawn(watch::follow(
            Arc::clone(&self.slot),
            Arc::clone(&self.ops),
            ticks,
        ));
        futures::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|tick| (tick, receiver))
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests;
