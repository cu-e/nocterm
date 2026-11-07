//! An in-memory profile replica with scripted remote writers, refusals and outages.
use crate::{
    profiles::{Profile, ProfileId},
    replica::{
        ProfileReplica, RemoteBody, RemoteProfile, ReplicaCursor, ReplicaError, ReplicaPage,
        ReplicaVersion,
    },
};
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::BoxFuture,
    stream::BoxStream,
};
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
}

struct Stored {
    body: RemoteBody,
    version: u64,
}

/// What writes are refused: single connections, or all of them.
#[derive(Default)]
struct Forbidden {
    profiles: HashSet<ProfileId>,
    all: bool,
}

#[derive(Default)]
struct Inner {
    sequence: u64,
    objects: HashMap<ProfileId, Stored>,
    feed: Vec<(u64, ProfileId)>,
    offline: bool,
    /// The account keys are closed: the server is reachable but every call is refused.
    locked: bool,
    /// Successful writes left before the replica goes offline.
    offline_after: Option<usize>,
    /// The answer of the next write waits for this signal.
    held: Option<oneshot::Receiver<()>>,
    forbidden: Forbidden,
    forced_conflicts: usize,
    concurrent: Option<Profile>,
    page_size: Option<usize>,
    calls: Vec<ReplicaCall>,
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

    fn store(&mut self, id: ProfileId, body: RemoteBody) -> ReplicaVersion {
        self.sequence += 1;
        self.feed.push((self.sequence, id));
        self.objects.insert(
            id,
            Stored {
                body,
                version: self.sequence,
            },
        );
        ReplicaVersion::new(self.sequence)
    }

    fn check_write(
        &mut self,
        id: ProfileId,
        expected: Option<ReplicaVersion>,
    ) -> Result<(), ReplicaError> {
        if let Some(error) = self.unserved() {
            return Err(error);
        }
        if let Some(profile) = self.concurrent.take() {
            let body = RemoteBody::Present(Box::new(profile.clone()));
            self.store(profile.id, body);
        }
        if self.forbidden.all || self.forbidden.profiles.contains(&id) {
            return Err(ReplicaError::Forbidden { id });
        }
        if self.forced_conflicts > 0 {
            self.forced_conflicts -= 1;
            return Err(ReplicaError::Conflict { id });
        }
        let current = self
            .objects
            .get(&id)
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
        for (sequence, id) in self.feed.iter().filter(|(sequence, _)| *sequence > from) {
            if self.page_size.is_some_and(|size| ids.len() >= size) {
                more = true;
                break;
            }
            last = *sequence;
            ids.retain(|seen| seen != id);
            ids.push(*id);
        }
        let changes = ids
            .into_iter()
            .filter_map(|id| {
                let stored = self.objects.get(&id)?;
                Some(RemoteProfile {
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
    pub fn write_remote(&self, profile: Profile) -> ReplicaVersion {
        let id = profile.id;
        let version = self.with(|inner| inner.store(id, RemoteBody::Present(Box::new(profile))));
        self.notify();
        version
    }

    /// Another device deletes the profile, leaving a tombstone.
    pub fn delete_remote(&self, id: ProfileId) -> ReplicaVersion {
        let version = self.with(|inner| inner.store(id, RemoteBody::Deleted));
        self.notify();
        version
    }

    /// A newer client wrote something this client cannot read.
    pub fn write_unrecognized(&self, id: ProfileId) -> ReplicaVersion {
        let version = self.with(|inner| inner.store(id, RemoteBody::Unrecognized));
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

    /// Every write is refused as `Forbidden`.
    pub fn refuse_writes(&self) {
        self.with(|inner| inner.forbidden.all = true);
    }

    /// The next `count` writes lose a race regardless of their expectation.
    pub fn conflict_next_writes(&self, count: usize) {
        self.with(|inner| inner.forced_conflicts = count);
    }

    /// Another device writes `profile` just before the next write is checked.
    pub fn write_concurrently_before_next_write(&self, profile: Profile) {
        self.with(|inner| inner.concurrent = Some(profile));
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

    /// The version and body of the latest write of a profile.
    pub fn stored(&self, id: ProfileId) -> Option<(ReplicaVersion, RemoteBody)> {
        self.with(|inner| {
            let stored = inner.objects.get(&id)?;
            Some((ReplicaVersion::new(stored.version), stored.body.clone()))
        })
    }

    /// The live profile stored under `id`, if any.
    pub fn profile(&self, id: ProfileId) -> Option<Profile> {
        match self.stored(id)?.1 {
            RemoteBody::Present(profile) => Some(*profile),
            RemoteBody::Deleted | RemoteBody::Unrecognized => None,
        }
    }

    fn write(
        &self,
        id: ProfileId,
        call: ReplicaCall,
        expected: Option<ReplicaVersion>,
        body: RemoteBody,
    ) -> Result<ReplicaVersion, ReplicaError> {
        let result = self.with(|inner| {
            inner.calls.push(call);
            inner.check_write(id, expected)?;
            let version = inner.store(id, body);
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
            RemoteBody::Present(Box::new(profile)),
        );
        self.answered(result)
    }

    fn delete(
        &self,
        id: ProfileId,
        expected: ReplicaVersion,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>> {
        let call = ReplicaCall::Delete { id, expected };
        let result = self.write(id, call, Some(expected), RemoteBody::Deleted);
        self.answered(result)
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
