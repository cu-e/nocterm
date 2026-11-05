//! An in-memory hub store for tests: versions, conflicts, tombstones and change streams as the
//! hub keeps them, and an offline switch.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::StreamExt as _;
use nocterm_core::SharedCause;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::HubError;
use nocterm_session::testing::Gate;

use super::{Call, OpError, StoreOps};
use crate::account::testing::Admin;
mod accounts;
mod bounded;
pub(crate) use accounts::MemoryAccountStores;

#[derive(Default)]
struct Held {
    collections: Vec<v1::Collection>,
    keys: HashMap<String, v1::CollectionKey>,
    members: HashMap<String, Vec<v1::CollectionMember>>,
    objects: HashMap<String, BTreeMap<String, v1::StoredObject>>,
    heads: HashMap<String, u64>,
    clock: i64,
    keyset: Option<v1::AccountKeyset>,
    memberships: Vec<v1::Membership>,
    read_only: HashSet<String>,
    scripted_changes: VecDeque<v1::ChangeBatch>,
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
    ObjectReady,
    PutReady,
    Put,
}

impl MemoryHub {
    pub(crate) fn script_changes(&self, pages: Vec<v1::ChangeBatch>) {
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
    pub(crate) fn set_keyset(&self, record: Option<v1::AccountKeyset>) {
        self.held().keyset = record;
    }

    /// The organizations the account belongs to, with its role in each.
    pub(crate) fn set_memberships(&self, memberships: &[(&str, &str, v1::Role)]) {
        self.held().memberships = memberships
            .iter()
            .map(|(org_id, name, role)| v1::Membership {
                org: Some(v1::Org {
                    org_id: (*org_id).to_owned(),
                    name: (*name).to_owned(),
                    created_at_ms: 0,
                }),
                role: *role as i32,
                effective_permissions: Vec::new(),
            })
            .collect();
    }

    /// An organization's collection whose key `granter` granted to the account: `envelope` is
    /// the key sealed to the account's public key for `collection_id`. The hub lists the granter
    /// as a member with the public key it holds for them.
    pub(crate) fn add_group(
        &self,
        kind: &str,
        org_id: &str,
        collection_id: &str,
        envelope: Vec<u8>,
        granter: &Admin,
    ) {
        let mut held = self.held();
        held.clock += 1;
        let collection = v1::Collection {
            collection_id: collection_id.to_owned(),
            org_id: org_id.to_owned(),
            personal: false,
            kind: kind.to_owned(),
            sealed_name: vec![1],
            head: 0,
            created_at_ms: held.clock,
            deleted: false,
        };
        held.collections.push(collection);
        held.keys.insert(
            collection_id.to_owned(),
            v1::CollectionKey {
                collection_id: collection_id.to_owned(),
                key_version: 1,
                wrapped_key: envelope,
                granted_by: granter.account_id().to_owned(),
            },
        );
        held.members
            .insert(collection_id.to_owned(), vec![granter.as_member()]);
    }

    /// The hub names `account_id` as the granter of a collection's key and lists `key` as its
    /// public key, as a hostile hub could to pass off a key of its own.
    pub(crate) fn name_granter(&self, collection_id: &str, account_id: &str, key: &Admin) {
        let mut held = self.held();
        if let Some(record) = held.keys.get_mut(collection_id) {
            record.granted_by = account_id.to_owned();
        }
        let mut member = key.as_member();
        member.account_id = account_id.to_owned();
        held.members.insert(collection_id.to_owned(), vec![member]);
    }

    /// Writes into the collection are refused for lack of permission, as for a guest.
    pub(crate) fn refuse_writes_to(&self, collection_id: &str) {
        self.held().read_only.insert(collection_id.to_owned());
    }

    /// Another member writes an object, whatever the account may do.
    pub(crate) fn write_as_member(
        &self,
        collection_id: &str,
        object_id: &str,
        kind: &str,
        ciphertext: Vec<u8>,
    ) -> v1::ObjectVersion {
        let current = self
            .held()
            .objects
            .get(collection_id)
            .and_then(|objects| objects.get(object_id))
            .map_or(0, |object| object.version);
        self.write(
            collection_id,
            object_id,
            current,
            Some((kind.to_owned(), ciphertext)),
        )
        .expect("the member's write is accepted")
    }

    fn permitted(&self, collection_id: &str) -> Result<(), OpError> {
        if self.held().read_only.contains(collection_id) {
            return Err(OpError::Hub(HubError::PermissionDenied));
        }
        Ok(())
    }

    /// A collection another client made: of any kind, personal or an organization's.
    pub(crate) fn add_foreign(&self, kind: &str, org_id: &str) -> String {
        let mut held = self.held();
        held.clock += 1;
        let id = format!("foreign-{}", held.collections.len());
        let collection = v1::Collection {
            collection_id: id.clone(),
            org_id: org_id.to_owned(),
            personal: org_id.is_empty(),
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
    pub(crate) fn objects(&self, collection_id: &str) -> Vec<v1::StoredObject> {
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
    ) -> Result<v1::ObjectVersion, OpError> {
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
                v1::StoredObject {
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
        Ok(v1::ObjectVersion {
            collection_id: collection_id.to_owned(),
            object_id: object_id.to_owned(),
            version,
            stream_version,
        })
    }
}

impl StoreOps for MemoryHub {
    fn keyset(&self) -> Call<'_, Option<v1::AccountKeyset>> {
        Box::pin(async move {
            self.reach()?;
            let record = self.held().keyset.clone();
            self.boundary(StoreBoundary::KeysetReady).await?;
            Ok(record)
        })
    }

    fn collections(&self, kind: &str) -> Call<'_, Vec<v1::Collection>> {
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

    fn create_collection(&self, request: v1::CreateCollectionRequest) -> Call<'_, v1::Collection> {
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
                let collection = v1::Collection {
                    collection_id: request.collection_id.clone(),
                    org_id: request.org_id.clone(),
                    personal: request.org_id.is_empty(),
                    kind: request.kind,
                    sealed_name: request.sealed_name,
                    head: 0,
                    created_at_ms: held.clock,
                    deleted: false,
                };
                held.keys.insert(
                    request.collection_id.clone(),
                    v1::CollectionKey {
                        collection_id: request.collection_id,
                        key_version: 1,
                        wrapped_key: request.wrapped_key,
                        granted_by: String::new(),
                    },
                );
                held.collections.push(collection.clone());
                collection
            };
            self.boundary(StoreBoundary::CreateCollectionReady).await?;
            Ok(collection)
        })
    }

    fn collection_key(&self, collection_id: &str) -> Call<'_, v1::CollectionKey> {
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

    fn members(&self, collection_id: &str) -> Call<'_, Vec<v1::CollectionMember>> {
        let id = collection_id.to_owned();
        Box::pin(async move {
            self.reach()?;
            Ok(self.held().members.get(&id).cloned().unwrap_or_default())
        })
    }

    fn memberships(&self) -> Call<'_, Vec<v1::Membership>> {
        Box::pin(async move {
            self.reach()?;
            Ok(self.held().memberships.clone())
        })
    }

    fn object(&self, collection_id: &str, object_id: &str) -> Call<'_, Option<v1::StoredObject>> {
        let collection_id = collection_id.to_owned();
        let object_id = object_id.to_owned();
        Box::pin(async move {
            self.reach()?;
            let object = self
                .held()
                .objects
                .get(&collection_id)
                .and_then(|objects| objects.get(&object_id))
                .cloned();
            self.boundary(StoreBoundary::ObjectReady).await?;
            Ok(object)
        })
    }

    fn put_object(&self, request: v1::PutObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            self.boundary(StoreBoundary::Put).await?;
            self.reach()?;
            self.permitted(&request.collection_id)?;
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

    fn delete_object(&self, request: v1::DeleteObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            self.reach()?;
            self.permitted(&request.collection_id)?;
            self.write(
                &request.collection_id,
                &request.object_id,
                request.expected_version,
                None,
            )
        })
    }

    fn changes(&self, collection_id: &str, after: u64) -> Call<'_, v1::ChangeBatch> {
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
                changes.sort_by_key(|object: &v1::StoredObject| object.stream_version);
                v1::ChangeBatch {
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
