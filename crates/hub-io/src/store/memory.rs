//! An in-memory hub store for tests: versions, conflicts, tombstones and change streams as the
//! hub keeps them, and an offline switch.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use futures::StreamExt as _;
use nocterm_core::SharedCause;
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubError;
use nocterm_session::testing::Gate;

use super::{Call, OpError, StoreOps};
mod accounts;
pub(crate) use accounts::MemoryAccountStores;

#[derive(Default)]
struct Held {
    collections: Vec<v2::Collection>,
    keys: HashMap<String, v2::CollectionKey>,
    objects: HashMap<String, BTreeMap<String, v2::StoredObject>>,
    heads: HashMap<String, u64>,
    clock: i64,
    keyset: Option<v2::Keyset>,
    scripted_changes: VecDeque<v2::ChangeBatch>,
    change_calls: usize,
}

/// One account's collections on a pretend hub.
#[derive(Default)]
pub(crate) struct MemoryHub {
    held: Mutex<Held>,
    offline: AtomicBool,
    pauses: Mutex<HashMap<StoreBoundary, (Gate, Gate, Option<HubError>)>>,
}

/// Deterministic boundaries around actual MemoryHub behavior.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StoreBoundary {
    KeysetReady,
    WatchReady,
    Collections,
    CreateCollection,
    CreateCollectionReady,
    ChangesReady,
    PutReady,
    Put,
}

impl MemoryHub {
    pub(crate) fn script_changes(&self, pages: Vec<v2::ChangeBatch>) {
        self.held().scripted_changes = pages.into();
    }

    pub(crate) fn change_calls(&self) -> usize {
        self.held().change_calls
    }
    /// Returns (entered, release). The next matching operation pauses once.
    pub(crate) fn pause_next(&self, at: StoreBoundary) -> (Gate, Gate) {
        self.pause_outcome(at, None)
    }
    pub(crate) fn pause_next_error(&self, at: StoreBoundary, error: HubError) -> (Gate, Gate) {
        self.pause_outcome(at, Some(error))
    }
    fn pause_outcome(&self, at: StoreBoundary, error: Option<HubError>) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.pauses
            .lock()
            .unwrap()
            .insert(at, (entered.clone(), release.clone(), error));
        (entered, release)
    }

    async fn boundary(&self, at: StoreBoundary) -> Result<(), OpError> {
        let pause = self.pauses.lock().unwrap().remove(&at);
        if let Some((entered, release, error)) = pause {
            entered.release();
            release.wait().await;
            if let Some(error) = error {
                return Err(OpError::Hub(error));
            }
        }
        Ok(())
    }

    pub(crate) fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reach(&self) -> Result<(), OpError> {
        if self.offline.load(Ordering::SeqCst) {
            let address = "hub.test".parse().expect("a valid address");
            return Err(OpError::Hub(HubError::Unreachable {
                address,
                cause: SharedCause::new(std::io::Error::other("offline")),
            }));
        }
        Ok(())
    }

    /// The keyset record the hub holds for the account.
    pub(crate) fn set_keyset(&self, record: Option<v2::Keyset>) {
        self.held().keyset = record;
    }

    /// A collection another client made, of any kind.
    pub(crate) fn add_foreign(&self, kind: &str) -> String {
        let mut held = self.held();
        held.clock += 1;
        let id = format!("foreign-{}", held.collections.len());
        let collection = v2::Collection {
            collection_id: id.clone(),
            kind: kind.to_owned(),
            sealed_name: vec![1],
            head: 0,
            created_at_ms: held.clock,
            deleted: false,
        };
        held.collections.push(collection);
        id
    }

    /// The ids of the collections of `kind`.
    pub(crate) fn collections_of_kind(&self, kind: &str) -> Vec<String> {
        self.held()
            .collections
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| c.collection_id.clone())
            .collect()
    }

    /// Every stored object of a collection, tombstones included, as the hub holds them.
    pub(crate) fn objects(&self, collection_id: &str) -> Vec<v2::StoredObject> {
        let held = self.held();
        held.objects
            .get(collection_id)
            .map(|objects| objects.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Replaces an object's ciphertext behind the client's back, as a hostile hub could.
    pub(crate) fn tamper(&self, collection_id: &str, object_id: &str, ciphertext: Vec<u8>) {
        let mut held = self.held();
        if let Some(object) = held
            .objects
            .get_mut(collection_id)
            .and_then(|objects| objects.get_mut(object_id))
        {
            object.ciphertext = ciphertext;
        }
    }

    /// Replaces the sealed key a collection hands out.
    pub(crate) fn tamper_key(&self, collection_id: &str, envelope: Vec<u8>) {
        if let Some(record) = self.held().keys.get_mut(collection_id) {
            record.wrapped_key = envelope;
        }
    }

    fn write(
        &self,
        collection_id: &str,
        object_id: &str,
        expected: u64,
        content: Option<(String, Vec<u8>)>,
    ) -> Result<v2::ObjectVersion, OpError> {
        let mut held = self.held();
        if !held
            .collections
            .iter()
            .any(|c| c.collection_id == collection_id)
        {
            return Err(OpError::Hub(HubError::NotFound));
        }
        let current = held
            .objects
            .get(collection_id)
            .and_then(|objects| objects.get(object_id))
            .map(|object| object.version)
            .unwrap_or(0);
        if current != expected {
            return Err(OpError::Conflict);
        }
        let head = held.heads.entry(collection_id.to_owned()).or_default();
        *head += 1;
        let stream_version = *head;
        let (kind, ciphertext, deleted) = match content {
            Some((kind, ciphertext)) => (kind, ciphertext, false),
            None => {
                let kind = held
                    .objects
                    .get(collection_id)
                    .and_then(|objects| objects.get(object_id))
                    .map(|object| object.kind.clone())
                    .unwrap_or_default();
                (kind, Vec::new(), true)
            }
        };
        let version = current + 1;
        held.objects
            .entry(collection_id.to_owned())
            .or_default()
            .insert(
                object_id.to_owned(),
                v2::StoredObject {
                    collection_id: collection_id.to_owned(),
                    object_id: object_id.to_owned(),
                    version,
                    kind,
                    ciphertext,
                    deleted,
                    updated_at_ms: 0,
                    updated_by_device: String::new(),
                    stream_version,
                },
            );
        Ok(v2::ObjectVersion {
            collection_id: collection_id.to_owned(),
            object_id: object_id.to_owned(),
            version,
            stream_version,
        })
    }
}

impl StoreOps for MemoryHub {
    fn keyset(&self) -> Call<'_, Option<v2::Keyset>> {
        Box::pin(async move {
            self.reach()?;
            let record = self.held().keyset.clone();
            self.boundary(StoreBoundary::KeysetReady).await?;
            Ok(record)
        })
    }

    fn collections(&self, kind: &str) -> Call<'_, Vec<v2::Collection>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            self.boundary(StoreBoundary::Collections).await?;
            self.reach()?;
            Ok(self
                .held()
                .collections
                .iter()
                .filter(|c| c.kind == kind)
                .cloned()
                .collect())
        })
    }

    fn create_collection(&self, request: v2::CreateCollectionRequest) -> Call<'_, v2::Collection> {
        Box::pin(async move {
            self.boundary(StoreBoundary::CreateCollection).await?;
            self.reach()?;
            let collection = {
                let mut held = self.held();
                if held
                    .collections
                    .iter()
                    .any(|row| row.collection_id == request.collection_id)
                {
                    return Err(OpError::Conflict);
                }
                held.clock += 1;
                let collection = v2::Collection {
                    collection_id: request.collection_id.clone(),
                    kind: request.kind,
                    sealed_name: request.sealed_name,
                    head: 0,
                    created_at_ms: held.clock,
                    deleted: false,
                };
                held.keys.insert(
                    request.collection_id.clone(),
                    v2::CollectionKey {
                        collection_id: request.collection_id,
                        key_version: 1,
                        wrapped_key: request.wrapped_key,
                    },
                );
                held.collections.push(collection.clone());
                collection
            };
            self.boundary(StoreBoundary::CreateCollectionReady).await?;
            Ok(collection)
        })
    }

    fn collection_key(&self, collection_id: &str) -> Call<'_, v2::CollectionKey> {
        let id = collection_id.to_owned();
        Box::pin(async move {
            self.reach()?;
            self.held()
                .keys
                .get(&id)
                .cloned()
                .ok_or(OpError::Hub(HubError::NotFound))
        })
    }

    fn put_object(&self, request: v2::PutObjectRequest) -> Call<'_, v2::ObjectVersion> {
        Box::pin(async move {
            self.boundary(StoreBoundary::Put).await?;
            self.reach()?;
            let written = self.write(
                &request.collection_id,
                &request.object_id,
                request.expected_version,
                Some((request.kind, request.ciphertext)),
            )?;
            self.boundary(StoreBoundary::PutReady).await?;
            Ok(written)
        })
    }

    fn delete_object(&self, request: v2::DeleteObjectRequest) -> Call<'_, v2::ObjectVersion> {
        Box::pin(async move {
            self.reach()?;
            self.write(
                &request.collection_id,
                &request.object_id,
                request.expected_version,
                None,
            )
        })
    }

    fn changes(&self, collection_id: &str, after: u64) -> Call<'_, v2::ChangeBatch> {
        let id = collection_id.to_owned();
        Box::pin(async move {
            self.reach()?;
            let scripted = {
                let mut held = self.held();
                held.change_calls += 1;
                held.scripted_changes.pop_front()
            };
            let batch = scripted.unwrap_or_else(|| {
                let held = self.held();
                let mut changes: Vec<_> = held
                    .objects
                    .get(&id)
                    .map(|objects| {
                        objects
                            .values()
                            .filter(|object| object.stream_version > after)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                changes.sort_by_key(|object: &v2::StoredObject| object.stream_version);
                v2::ChangeBatch {
                    changes,
                    head: held.heads.get(&id).copied().unwrap_or(0),
                    more: false,
                }
            });
            self.boundary(StoreBoundary::ChangesReady).await?;
            Ok(batch)
        })
    }

    fn watch(
        &self,
        _collections: Vec<String>,
    ) -> Call<'_, futures::stream::BoxStream<'static, ()>> {
        Box::pin(async move {
            self.reach()?;
            self.boundary(StoreBoundary::WatchReady).await?;
            Ok(futures::stream::pending().boxed())
        })
    }
}
