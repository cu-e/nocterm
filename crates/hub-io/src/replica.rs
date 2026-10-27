//! The saved profiles on the account hub, one sealed object per profile.
//!
//! [`HubProfileReplica`] implements the replica port of the connection store over the hub's
//! `profiles` collection. It works only while the vault is unlocked, because the keys that
//! seal profiles are the vault's; locked or unreachable is "unavailable", never an error.

mod watch;
mod wire;

use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use nocterm_connections_model::profiles::{Profile, ProfileId};
use nocterm_connections_model::replica::{
    ProfileReplica, ReplicaCursor, ReplicaError, ReplicaPage, ReplicaScope, ReplicaVersion,
};
use nocterm_core::SharedCause;
use nocterm_hubs_model::HubError;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use self::wire::{KIND, Positions};
use crate::account::{OpenKeys, Slot};
use crate::classify::unreachable;
use crate::store::{OpError, PROFILES_KIND, Store, StoreOps};
use crate::task::spawn;

/// Writes of a profile in the scope of an organization belong to the organization's own
/// space, which this replica does not serve yet.
pub struct HubProfileReplica {
    handle: Handle,
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
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
    pub(crate) fn new(handle: Handle, slot: Arc<Slot>, ops: Arc<dyn StoreOps>) -> Self {
        Self { handle, slot, ops }
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

async fn changes(
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
    after: Option<ReplicaCursor>,
) -> Result<ReplicaPage, ReplicaError> {
    let store = HubProfileReplica::store(&slot, &ops)?;
    let mut positions = Positions::parse(after.as_ref());
    let Some(at) = store
        .find(PROFILES_KIND)
        .await
        .map_err(|error| replica_error(error, None))?
    else {
        return Ok(ReplicaPage {
            changes: Vec::new(),
            cursor: positions.cursor(),
            more: false,
        });
    };
    let from = positions.get(at.id());
    let batch = store
        .changes(&at, from)
        .await
        .map_err(|error| replica_error(error, None))?;
    let mut seen = Vec::with_capacity(batch.changes.len());
    let mut position = from;
    for object in &batch.changes {
        position = position.max(object.stream_version);
        if let Some(profile) = wire::read(at.id(), at.key(), object)? {
            seen.push(profile);
        }
    }
    if !batch.more {
        position = position.max(batch.head);
    }
    positions.set(at.id(), position);
    Ok(ReplicaPage {
        changes: seen,
        cursor: positions.cursor(),
        more: batch.more,
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
    if scope != ReplicaScope::Personal {
        return Err(ReplicaError::Forbidden { id });
    }
    let store = HubProfileReplica::store(&slot, &ops)?;
    let plaintext =
        wire::encode(&profile).map_err(|error| ReplicaError::Other(SharedCause::new(error)))?;
    let at = store
        .ensure(PROFILES_KIND)
        .await
        .map_err(|error| replica_error(error, None))?;
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
    if scope != ReplicaScope::Personal {
        return Err(ReplicaError::Forbidden { id });
    }
    let store = HubProfileReplica::store(&slot, &ops)?;
    let at = store
        .find(PROFILES_KIND)
        .await
        .map_err(|error| replica_error(error, None))?
        .ok_or(ReplicaError::Conflict { id })?;
    let stored = store
        .remove(&at, &id.to_string(), expected.get())
        .await
        .map_err(|error| replica_error(error, Some(id)))?;
    Ok(ReplicaVersion::new(stored.version))
}

impl ProfileReplica for HubProfileReplica {
    fn changes(
        &self,
        after: Option<ReplicaCursor>,
    ) -> BoxFuture<'static, Result<ReplicaPage, ReplicaError>> {
        self.run(changes(
            Arc::clone(&self.slot),
            Arc::clone(&self.ops),
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
