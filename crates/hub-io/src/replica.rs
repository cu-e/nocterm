//! The saved profiles on the account hub, one sealed object per profile.
//!
//! [`HubProfileReplica`] implements the replica port of the connection store over the
//! account's `profiles` collection. It works only while the vault is unlocked, because the keys
//! that seal profiles are the vault's; locked or unreachable is "unavailable", never an error.
//! The account has one space, its own: a write to any other scope is
//! [`ReplicaError::Forbidden`].

mod watch;
mod wire;

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
use crate::account::operations::{BoundOperations, OpenedWork};
use crate::classify::unreachable;
use crate::store::{Located, OpError, PROFILES_KIND, Store};
use crate::task::spawn;

pub struct HubProfileReplica {
    handle: Handle,
    bound: BoundOperations,
}

/// A failure of a store call as the replica reports it. `id` names the profile a write was
/// about.
fn replica_error(error: OpError, id: Option<ProfileId>) -> ReplicaError {
    match (error, id) {
        (OpError::Hub(HubError::NotSignedIn | HubError::Stale), _) => ReplicaError::Stale,
        (OpError::Conflict, Some(id)) => ReplicaError::Conflict { id },
        (OpError::Hub(HubError::NotFound), Some(id)) => ReplicaError::Conflict { id },
        (OpError::Hub(HubError::PermissionDenied), Some(id)) => ReplicaError::Forbidden { id },
        (OpError::Hub(error), _) if unreachable(&error) => ReplicaError::Unavailable,
        (error, _) => ReplicaError::Other(SharedCause::new(error)),
    }
}

impl HubProfileReplica {
    pub(crate) fn new(handle: Handle, bound: BoundOperations) -> Self {
        Self { handle, bound }
    }

    fn run<T, F>(
        &self,
        work: impl FnOnce(OpenedWork) -> F + Send + 'static,
    ) -> BoxFuture<'static, Result<T, ReplicaError>>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, ReplicaError>> + Send + 'static,
    {
        let admitted = match self.bound.capture() {
            Ok(admitted) => admitted,
            Err(error) => return Box::pin(async move { Err(native_failure(error)) }),
        };
        let source = admitted.source().clone();
        let returned = source.clone();
        let task = spawn(&self.handle, async move {
            source
                .during(work(admitted))
                .await
                .map_err(native_failure)?
        });
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(ReplicaError::Unavailable));
            returned.check().map_err(native_failure)?;
            result
        })
    }
}

fn native_failure(error: HubError) -> ReplicaError {
    match error {
        HubError::KeysLocked => ReplicaError::Locked,
        HubError::NotSignedIn | HubError::Stale => ReplicaError::Stale,
        other => ReplicaError::Other(SharedCause::new(other)),
    }
}

async fn changes(
    work: OpenedWork,
    after: Option<ReplicaCursor>,
) -> Result<ReplicaPage, ReplicaError> {
    let store = work.store();
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
    let mut seen = Vec::new();
    let mut position = from;
    for object in &batch.changes {
        position = position.max(object.stream_version);
        if let Some(profile) = wire::read(ReplicaScope::Personal, at.id(), at.key(), object)? {
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

/// The collection a write goes to: the account's own, created by the first write.
async fn target(
    store: &Store,
    scope: &ReplicaScope,
    id: ProfileId,
    create: bool,
) -> Result<Located, ReplicaError> {
    if *scope != ReplicaScope::Personal {
        return Err(ReplicaError::Forbidden { id });
    }
    let located = if create {
        store.ensure(PROFILES_KIND).await.map(Some)
    } else {
        store.find(PROFILES_KIND).await
    }
    .map_err(|error| replica_error(error, None))?;
    located.ok_or(ReplicaError::Conflict { id })
}

async fn put(
    work: OpenedWork,
    scope: ReplicaScope,
    profile: Profile,
    expected: Option<ReplicaVersion>,
) -> Result<ReplicaVersion, ReplicaError> {
    let id = profile.id;
    let store = work.store();
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
    work: OpenedWork,
    scope: ReplicaScope,
    id: ProfileId,
    expected: ReplicaVersion,
) -> Result<ReplicaVersion, ReplicaError> {
    let store = work.store();
    let at = target(&store, &scope, id, false).await?;
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
        self.run(move |work| changes(work, after))
    }

    fn put(
        &self,
        scope: ReplicaScope,
        profile: Profile,
        expected: Option<ReplicaVersion>,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        self.run(move |work| put(work, scope, profile, expected))
    }

    fn delete(
        &self,
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        self.run(move |work| delete(work, scope, id, expected))
    }

    fn watch(&self) -> BoxStream<'static, ()> {
        let work = match self.bound.capture() {
            Ok(work) => work,
            Err(_) => return futures::stream::empty().boxed(),
        };
        let (ticks, receiver) = mpsc::channel(1);
        self.handle.spawn(watch::follow(work, ticks));
        let source = self.bound.source().clone();
        futures::stream::unfold((receiver, source), |(mut receiver, source)| async move {
            source.check().ok()?;
            receiver.recv().await?;
            source.check().ok()?;
            Some(((), (receiver, source)))
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests;
