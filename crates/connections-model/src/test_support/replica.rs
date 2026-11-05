//! An in-memory profile replica with scripted remote writers, refusals and outages.
use crate::{
    profiles::{Profile, ProfileId},
    replica::{
        CredentialCopy, ProfileReplica, RemoteBody, RemoteProfile, ReplicaCursor, ReplicaError,
        ReplicaPage, ReplicaScope, ReplicaSpace, ReplicaVersion,
    },
};
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::BoxFuture,
    stream::BoxStream,
};
use nocterm_session::CredentialId;
use std::{
    collections::{HashMap, HashSet, VecDeque},
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

/// What writes are refused: single connections and whole spaces.
#[derive(Default)]
struct Forbidden {
    profiles: HashSet<ProfileId>,
    scopes: HashSet<ReplicaScope>,
}

#[derive(Default)]
struct Inner {
    sequence: u64,
    objects: HashMap<(ReplicaScope, ProfileId), Stored>,
    feed: Vec<(u64, ReplicaScope, ProfileId)>,
    offline: bool,
    /// The account keys are closed: the server is reachable but every call is refused.
    locked: bool,
    /// Successful writes left before the replica goes offline.
    offline_after: Option<usize>,
    /// The answer of the next write waits for this signal.
    held: Option<oneshot::Receiver<()>>,
    forbidden: Forbidden,
    spaces: Vec<ReplicaSpace>,
    forced_conflicts: usize,
    concurrent: Option<(ReplicaScope, Profile)>,
    page_size: Option<usize>,
    calls: Vec<ReplicaCall>,
    secrets: HashMap<CredentialId, HashSet<ReplicaScope>>,
    copies: u64,
}

/// Versions and cursors come from one counter, so a cursor is "after write N".
#[derive(Default)]
pub struct MemoryReplica {
    inner: Mutex<Inner>,
    watchers: Mutex<Vec<mpsc::UnboundedSender<()>>>,
    scripted_pages: Mutex<VecDeque<ReplicaPage>>,
    held_read: Mutex<Option<oneshot::Receiver<()>>>,
}

impl Inner {
    /// Why a call cannot be served now: closed keys are noticed before the network.
    fn unserved(&self) -> Option<ReplicaError> {
        if self.locked {
            Some(ReplicaError::Locked)
        } else if self.offline {
            Some(ReplicaError::Unavailable)
        } else {
            None
        }
    }

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
        if let Some(error) = self.unserved() {
            return Err(error);
        }
        if let Some((scope, profile)) = self.concurrent.take() {
            let body = RemoteBody::Present(Box::new(profile.clone()));
            self.store(profile.id, scope, body);
        }
        if self.forbidden.profiles.contains(&id) || self.forbidden.scopes.contains(scope) {
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

    /// The account keys: while locked every call answers `Locked`, even when reachable.
    /// Unlocking tells watchers, as the real replica does when its keys open.
    pub fn set_locked(&self, locked: bool) {
        self.with(|inner| inner.locked = locked);
        if !locked {
            self.notify();
        }
    }

    /// The link drops once `writes` more writes succeed, as when it breaks between two
    /// requests of one operation.
    pub fn go_offline_after_writes(&self, writes: usize) {
        self.with(|inner| inner.offline_after = Some(writes));
    }

    /// The next write is carried out, but its answer reaches the caller only after the
    /// returned sender is used: whatever happens meanwhile happens during the round.
    pub fn hold_next_write(&self) -> oneshot::Sender<()> {
        let (release, held) = oneshot::channel();
        self.with(|inner| inner.held = Some(held));
        release
    }

    /// Writes of this profile are refused as `Forbidden`.
    pub fn refuse_writes_of(&self, id: ProfileId) {
        self.with(|inner| inner.forbidden.profiles.insert(id));
    }

    /// Writes into this scope are refused as `Forbidden`, as for a read-only member.
    pub fn refuse_writes_in(&self, scope: ReplicaScope) {
        self.with(|inner| inner.forbidden.scopes.insert(scope));
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

    /// Supplies protocol pages without changing the coherent remote object store.
    /// Malformed cursors are allowed for consumer validation tests; response counts stay bounded.
    pub fn script_pages(&self, pages: Vec<ReplicaPage>) {
        assert!(pages.len() <= 512 && pages.iter().all(|page| page.changes.len() <= 20_000));
        *self
            .scripted_pages
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = pages.into();
    }
    /// Suspends the next changes response; dropping its consumer closes the receipt.
    pub fn hold_next_read(&self) -> oneshot::Sender<()> {
        let (send, receive) = oneshot::channel();
        *self
            .held_read
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(receive);
        send
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

    /// Puts the secrets kept in `from` into `to`, and answers the id each goes by there.
    /// A secret leaves `from` when `leave` is set; otherwise it is copied under a new id.
    fn relocate(
        &self,
        id: ProfileId,
        call: ReplicaCall,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
        leave: bool,
    ) -> Result<Vec<CredentialCopy>, ReplicaError> {
        self.with(|inner| {
            inner.calls.push(call);
            if let Some(error) = inner.unserved() {
                return Err(error);
            }
            if inner.forbidden.profiles.contains(&id)
                || inner.forbidden.scopes.contains(&from)
                || inner.forbidden.scopes.contains(&to)
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
                    kept.insert(to.clone());
                    moved.push(CredentialCopy {
                        from: credential,
                        to: credential,
                    });
                    continue;
                }
                inner.copies += 1;
                let copy =
                    CredentialId::from_bytes((u128::MAX - u128::from(inner.copies)).to_be_bytes());
                inner.secrets.insert(copy, HashSet::from([to.clone()]));
                moved.push(CredentialCopy {
                    from: credential,
                    to: copy,
                });
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
            let version = inner.store(id, scope, body);
            if let Some(left) = inner.offline_after.as_mut() {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    inner.offline = true;
                    inner.offline_after = None;
                }
            }
            Ok(version)
        });
        if result.is_ok() {
            self.notify();
        }
        result
    }

    /// The answer of a write, after the hold on it is released.
    fn answered(
        &self,
        result: Result<ReplicaVersion, ReplicaError>,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        match self.with(|inner| inner.held.take()) {
            Some(held) => async move {
                held.await.ok();
                result
            }
            .boxed(),
            None => futures::future::ready(result).boxed(),
        }
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
            match inner.unserved() {
                Some(error) => Err(error),
                None => Ok(self
                    .scripted_pages
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .pop_front()
                    .unwrap_or_else(|| inner.page(after.as_ref()))),
            }
        });
        let held = self
            .held_read
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        async move {
            if let Some(held) = held {
                let _ = held.await;
            }
            result
        }
        .boxed()
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
        self.answered(result)
    }

    fn delete(
        &self,
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        let call = ReplicaCall::Delete { id, expected };
        let result = self.write(id, call, Some(expected), scope, RemoteBody::Deleted);
        self.answered(result)
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
        let moved = self
            .relocate(id, call, from, to, credentials, true)
            .map(|moved| moved.into_iter().map(|copy| copy.from).collect());
        futures::future::ready(moved).boxed()
    }

    fn copy(
        &self,
        id: ProfileId,
        from: ReplicaScope,
        to: ReplicaScope,
        credentials: Vec<CredentialId>,
    ) -> BoxFuture<'static, Result<Vec<CredentialCopy>, ReplicaError>> {
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
