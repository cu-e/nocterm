//! An in-memory profile replica with scripted remote writers, refusals and outages.
use crate::{
    profiles::{Profile, ProfileId},
    replica::{
        ProfileReplica, RemoteBody, RemoteProfile, ReplicaCursor, ReplicaError, ReplicaPage,
        ReplicaScope, ReplicaSpace, ReplicaVersion,
    },
};
use futures::{
    FutureExt as _, StreamExt as _, channel::mpsc, future::BoxFuture, stream::BoxStream,
};
use nocterm_session::CredentialId;
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

/// A call the replica received, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicaCall {
    Changes(Option<String>),
    Put {
        id: ProfileId,
        expected: Option<ReplicaVersion>,
    },
    Delete {
        id: ProfileId,
        expected: ReplicaVersion,
    },
    Carry {
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    },
    Copy {
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    },
}

struct Stored {
    body: RemoteBody,
    version: u64,
}

#[derive(Default)]
struct Inner {
    sequence: u64,
    objects: HashMap<(ReplicaScope, ProfileId), Stored>,
    feed: Vec<(u64, ReplicaScope, ProfileId)>,
    offline: bool,
    forbidden: HashSet<ProfileId>,
    forbidden_scopes: HashSet<ReplicaScope>,
    spaces: Vec<ReplicaSpace>,
    forced_conflicts: usize,
    concurrent: Option<(ReplicaScope, Profile)>,
    page_size: Option<usize>,
    calls: Vec<ReplicaCall>,
    secrets: HashMap<CredentialId, HashSet<ReplicaScope>>,
}

/// Versions and cursors come from one counter, so a cursor is "after write N".
#[derive(Default)]
pub struct MemoryReplica {
    inner: Mutex<Inner>,
    watchers: Mutex<Vec<mpsc::UnboundedSender<()>>>,
}

impl Inner {
    fn store(&mut self, id: ProfileId, scope: ReplicaScope, body: RemoteBody) -> ReplicaVersion {
        self.sequence += 1;
        self.feed.push((self.sequence, scope.clone(), id));
        self.objects.insert(
            (scope, id),
            Stored {
                body,
                version: self.sequence,
            },
        );
        ReplicaVersion::new(self.sequence)
    }

    fn check_write(
        &mut self,
        scope: &ReplicaScope,
        id: ProfileId,
        expected: Option<ReplicaVersion>,
    ) -> Result<(), ReplicaError> {
        if self.offline {
            return Err(ReplicaError::Unavailable);
        }
        if let Some((scope, profile)) = self.concurrent.take() {
            let body = RemoteBody::Present(Box::new(profile.clone()));
            self.store(profile.id, scope, body);
        }
        if self.forbidden.contains(&id) || self.forbidden_scopes.contains(scope) {
            return Err(ReplicaError::Forbidden { id });
        }
        if self.forced_conflicts > 0 {
            self.forced_conflicts -= 1;
            return Err(ReplicaError::Conflict { id });
        }
        let current = self
            .objects
            .get(&(scope.clone(), id))
            .map(|stored| ReplicaVersion::new(stored.version));
        if current == expected {
            Ok(())
        } else {
            Err(ReplicaError::Conflict { id })
        }
    }

    fn page(&self, after: Option<&ReplicaCursor>) -> ReplicaPage {
        let from = after
            .and_then(|cursor| cursor.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        let mut ids = Vec::new();
        let mut last = from;
        let mut more = false;
        for (sequence, scope, id) in self.feed.iter().filter(|(sequence, ..)| *sequence > from) {
            if self.page_size.is_some_and(|size| ids.len() >= size) {
                more = true;
                break;
            }
            last = *sequence;
            ids.retain(|(seen_scope, seen)| seen != id || seen_scope != scope);
            ids.push((scope.clone(), *id));
        }
        let changes = ids
            .into_iter()
            .filter_map(|(scope, id)| {
                let stored = self.objects.get(&(scope.clone(), id))?;
                Some(RemoteProfile {
                    scope,
                    id,
                    version: ReplicaVersion::new(stored.version),
                    body: stored.body.clone(),
                })
            })
            .collect();
        ReplicaPage {
            changes,
            cursor: ReplicaCursor::new(last.to_string()),
            more,
        }
    }
}

impl MemoryReplica {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, work: impl FnOnce(&mut Inner) -> R) -> R {
        work(&mut self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn notify(&self) {
        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|watcher| watcher.unbounded_send(()).is_ok());
    }

    /// Another device writes `profile`; watchers are told.
    pub fn write_remote(&self, scope: ReplicaScope, profile: Profile) -> ReplicaVersion {
        let id = profile.id;
        let version =
            self.with(|inner| inner.store(id, scope, RemoteBody::Present(Box::new(profile))));
        self.notify();
        version
    }

    /// Another device deletes the profile, leaving a tombstone.
    pub fn delete_remote(&self, scope: ReplicaScope, id: ProfileId) -> ReplicaVersion {
        let version = self.with(|inner| inner.store(id, scope, RemoteBody::Deleted));
        self.notify();
        version
    }

    /// A newer client wrote something this client cannot read.
    pub fn write_unrecognized(&self, scope: ReplicaScope, id: ProfileId) -> ReplicaVersion {
        let version = self.with(|inner| inner.store(id, scope, RemoteBody::Unrecognized));
        self.notify();
        version
    }

    /// Reachability: while offline every call answers `Unavailable`.
    pub fn set_offline(&self, offline: bool) {
        self.with(|inner| inner.offline = offline);
        if !offline {
            self.notify();
        }
    }

    /// Writes of this profile are refused as `Forbidden`.
    pub fn refuse_writes_of(&self, id: ProfileId) {
        self.with(|inner| inner.forbidden.insert(id));
    }

    /// Writes into this scope are refused as `Forbidden`, as for a read-only member.
    pub fn refuse_writes_in(&self, scope: ReplicaScope) {
        self.with(|inner| inner.forbidden_scopes.insert(scope));
    }

    /// A secret is kept in `scope`, as the vault of that space would hold it.
    pub fn keep_secret(&self, scope: ReplicaScope, credential: CredentialId) {
        self.with(|inner| {
            inner.secrets.entry(credential).or_default().insert(scope);
        });
    }

    /// The space that keeps a secret, when it is kept in one only.
    pub fn secret_scope(&self, credential: CredentialId) -> Option<ReplicaScope> {
        let mut scopes = self.secret_scopes(credential).into_iter();
        scopes.next().filter(|_| scopes.next().is_none())
    }

    /// Every space that keeps a secret now.
    pub fn secret_scopes(&self, credential: CredentialId) -> HashSet<ReplicaScope> {
        self.with(|inner| inner.secrets.get(&credential).cloned().unwrap_or_default())
    }

    /// The spaces the replica reports to the application.
    pub fn set_spaces(&self, spaces: Vec<ReplicaSpace>) {
        self.with(|inner| inner.spaces = spaces);
    }

    /// The next `count` writes lose a race regardless of their expectation.
    pub fn conflict_next_writes(&self, count: usize) {
        self.with(|inner| inner.forced_conflicts = count);
    }

    /// Another device writes `profile` just before the next write is checked.
    pub fn write_concurrently_before_next_write(&self, scope: ReplicaScope, profile: Profile) {
        self.with(|inner| inner.concurrent = Some((scope, profile)));
    }

    /// Changes come in pages of at most `size` writes.
    pub fn page_size(&self, size: usize) {
        self.with(|inner| inner.page_size = Some(size));
    }

    pub fn calls(&self) -> Vec<ReplicaCall> {
        self.with(|inner| inner.calls.clone())
    }

    pub fn writes(&self) -> Vec<ReplicaCall> {
        self.calls()
            .into_iter()
            .filter(|call| !matches!(call, ReplicaCall::Changes(_)))
            .collect()
    }

    /// The scope, version and body of the latest write of a profile, in any scope.
    pub fn stored(&self, id: ProfileId) -> Option<(ReplicaScope, ReplicaVersion, RemoteBody)> {
        self.with(|inner| {
            let (_, scope, _) = inner.feed.iter().rev().find(|(_, _, seen)| *seen == id)?;
            let stored = inner.objects.get(&(scope.clone(), id))?;
            Some((
                scope.clone(),
                ReplicaVersion::new(stored.version),
                stored.body.clone(),
            ))
        })
    }

    /// The version and body of a profile in one scope.
    pub fn stored_in(
        &self,
        scope: &ReplicaScope,
        id: ProfileId,
    ) -> Option<(ReplicaVersion, RemoteBody)> {
        self.with(|inner| {
            let stored = inner.objects.get(&(scope.clone(), id))?;
            Some((ReplicaVersion::new(stored.version), stored.body.clone()))
        })
    }

    /// The live profile stored under `id`, if any.
    pub fn profile(&self, id: ProfileId) -> Option<Profile> {
        match self.stored(id)?.2 {
            RemoteBody::Present(profile) => Some(*profile),
            RemoteBody::Deleted | RemoteBody::Unrecognized => None,
        }
    }

    /// Puts the secrets kept in `from` into `to`, and out of `from` when `leave` is set.
    fn relocate(
        &self,
        id: ProfileId,
        call: ReplicaCall,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
        leave: bool,
    ) -> Result<Vec<CredentialId>, ReplicaError> {
        self.with(|inner| {
            inner.calls.push(call);
            if inner.offline {
                return Err(ReplicaError::Unavailable);
            }
            if inner.forbidden.contains(&id)
                || inner.forbidden_scopes.contains(&from)
                || inner.forbidden_scopes.contains(&to)
            {
                return Err(ReplicaError::Forbidden { id });
            }
            let mut moved = Vec::new();
            for credential in credentials {
                let Some(kept) = inner.secrets.get_mut(&credential) else {
                    continue;
                };
                if !kept.contains(&from) {
                    continue;
                }
                if leave {
                    kept.remove(&from);
                }
                kept.insert(to.clone());
                moved.push(credential);
            }
            Ok(moved)
        })
    }

    fn write(
        &self,
        id: ProfileId,
        call: ReplicaCall,
        expected: Option<ReplicaVersion>,
        scope: ReplicaScope,
        body: RemoteBody,
    ) -> Result<ReplicaVersion, ReplicaError> {
        let result = self.with(|inner| {
            inner.calls.push(call);
            inner.check_write(&scope, id, expected)?;
            Ok(inner.store(id, scope, body))
        });
        if result.is_ok() {
            self.notify();
        }
        result
    }
}

impl ProfileReplica for MemoryReplica {
    fn changes(
        &self,
        after: Option<ReplicaCursor>,
    ) -> BoxFuture<'static, Result<ReplicaPage, ReplicaError>> {
        let result = self.with(|inner| {
            inner.calls.push(ReplicaCall::Changes(
                after.as_ref().map(|c| c.as_str().to_owned()),
            ));
            if inner.offline {
                Err(ReplicaError::Unavailable)
            } else {
                Ok(inner.page(after.as_ref()))
            }
        });
        futures::future::ready(result).boxed()
    }

    fn put(
        &self,
        scope: ReplicaScope,
        profile: Profile,
        expected: Option<ReplicaVersion>,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        let call = ReplicaCall::Put {
            id: profile.id,
            expected,
        };
        let result = self.write(
            profile.id,
            call,
            expected,
            scope,
            RemoteBody::Present(Box::new(profile)),
        );
        futures::future::ready(result).boxed()
    }

    fn delete(
        &self,
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        let call = ReplicaCall::Delete { id, expected };
        let result = self.write(id, call, Some(expected), scope, RemoteBody::Deleted);
        futures::future::ready(result).boxed()
    }

    fn carry(
        &self,
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    ) -> BoxFuture<'static, Result<Vec<CredentialId>, ReplicaError>> {
        let call = ReplicaCall::Carry {
            id,
            from: from.clone(),
            to: to.clone(),
            credentials: credentials.clone(),
        };
        futures::future::ready(self.relocate(id, call, from, to, credentials, true)).boxed()
    }

    fn copy(
        &self,
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    ) -> BoxFuture<'static, Result<Vec<CredentialId>, ReplicaError>> {
        let call = ReplicaCall::Copy {
            id,
            from: from.clone(),
            to: to.clone(),
            credentials: credentials.clone(),
        };
        futures::future::ready(self.relocate(id, call, from, to, credentials, false)).boxed()
    }

    fn spaces(&self) -> Vec<ReplicaSpace> {
        self.with(|inner| inner.spaces.clone())
    }

    fn watch(&self) -> BoxStream<'static, ()> {
        let (sender, receiver) = mpsc::unbounded();
        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(sender);
        receiver.boxed()
    }
}
