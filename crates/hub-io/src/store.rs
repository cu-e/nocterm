//! Encrypted collections of one account on a hub.
//!
//! [`StoreOps`] is the narrow seam to the hub's store service: plain calls with sealed bytes.
//! [`Store`] adds what needs the account keys: finding or creating the account's personal
//! collection of a kind, and sealing what goes into it. Only the kinds this client knows
//! are asked for, so a collection of another kind (or an organization's) is never listed,
//! opened, changed or deleted here.

mod grpc;
#[cfg(test)]
pub(crate) mod memory;

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::HubError;
use nocterm_keys::{CollectionKey, OpenError, random_bytes};
use uuid::Builder;

use crate::account::OpenKeys;
use crate::classify::remote;

pub(crate) use grpc::HubStore;

/// The kind of the collection that holds the vault's credentials.
pub(crate) const VAULT_KIND: &str = "vault";
/// The kind of the collection that holds the saved profiles.
pub(crate) const PROFILES_KIND: &str = "profiles";

/// Why a store call failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OpError {
    /// The version the write expected is not the stored one.
    #[error("the object changed on the hub")]
    Conflict,
    /// The hub returned a key or an object that does not authenticate: it was altered.
    #[error("the hub returned data that does not authenticate")]
    Damaged(#[source] OpenError),
    #[error(transparent)]
    Hub(#[from] HubError),
}

pub(crate) type Call<'a, T> = BoxFuture<'a, Result<T, OpError>>;

/// The calls of the store service this client makes, with sealed bytes only.
pub(crate) trait StoreOps: Send + Sync {
    /// The account's keyset record as the hub stores it; `None` before one was created.
    fn keyset(&self) -> Call<'_, Option<v1::AccountKeyset>>;
    fn collections(&self, kind: &str) -> Call<'_, Vec<v1::Collection>>;
    fn create_collection(&self, request: v1::CreateCollectionRequest) -> Call<'_, v1::Collection>;
    fn collection_key(&self, collection_id: &str) -> Call<'_, Vec<u8>>;
    fn put_object(&self, request: v1::PutObjectRequest) -> Call<'_, v1::ObjectVersion>;
    fn delete_object(&self, request: v1::DeleteObjectRequest) -> Call<'_, v1::ObjectVersion>;
    fn changes(&self, collection_id: &str, after: u64) -> Call<'_, v1::ChangeBatch>;
    /// Yields whenever a head of one of `collections` moved; ends when the connection does.
    fn watch(&self, collections: Vec<String>) -> Call<'_, BoxStream<'static, ()>>;
}

/// A collection of the account and the key that opens its objects.
#[derive(Clone)]
pub(crate) struct Located {
    id: String,
    key: Arc<CollectionKey>,
    envelope: Vec<u8>,
}

impl Located {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn key(&self) -> &CollectionKey {
        &self.key
    }

    /// The sealed key as the hub holds it, for the cache.
    pub(crate) fn envelope(&self) -> &[u8] {
        &self.envelope
    }
}

/// The account's collections on one hub, with the keys to read them.
pub(crate) struct Store {
    ops: Arc<dyn StoreOps>,
    keys: Arc<OpenKeys>,
}

impl Store {
    pub(crate) fn new(ops: Arc<dyn StoreOps>, keys: Arc<OpenKeys>) -> Self {
        Self { ops, keys }
    }

    /// The account's personal collection of `kind`, if the hub has one. When several exist
    /// (two devices made one at once) the oldest is the collection; the others stay unused.
    pub(crate) async fn find(&self, kind: &str) -> Result<Option<Located>, OpError> {
        let listed = self.ops.collections(kind).await?;
        let Some(chosen) = listed
            .into_iter()
            .filter(|c| !c.deleted && c.personal && c.org_id.is_empty() && c.kind == kind)
            .min_by(|a, b| {
                (a.created_at_ms, &a.collection_id).cmp(&(b.created_at_ms, &b.collection_id))
            })
        else {
            return Ok(None);
        };
        let envelope = self.ops.collection_key(&chosen.collection_id).await?;
        let key = self
            .keys
            .collection(&chosen.collection_id, &envelope)
            .map_err(OpError::Damaged)?;
        Ok(Some(Located {
            id: chosen.collection_id,
            key,
            envelope,
        }))
    }

    /// The personal collection of `kind`, created first if the account has none.
    pub(crate) async fn ensure(&self, kind: &str) -> Result<Located, OpError> {
        if let Some(found) = self.find(kind).await? {
            return Ok(found);
        }
        let id = Builder::from_random_bytes(random_bytes::<16>().map_err(remote)?)
            .into_uuid()
            .to_string();
        let (key, envelope) = self.keys.new_collection(&id)?;
        let sealed_name = key.seal_name(&id, kind).map_err(remote)?;
        self.ops
            .create_collection(v1::CreateCollectionRequest {
                org_id: String::new(),
                kind: kind.to_owned(),
                sealed_name,
                wrapped_key: envelope,
                collection_id: id,
            })
            .await?;
        // A device that created one at the same moment may have made an older one.
        self.find(kind)
            .await?
            .ok_or(OpError::Hub(HubError::NotFound))
    }

    /// Seals `plaintext` for the object and stores it. `expected` is the stored version, or 0
    /// to create.
    pub(crate) async fn put(
        &self,
        at: &Located,
        kind: &str,
        object_id: &str,
        plaintext: &[u8],
        expected: u64,
    ) -> Result<v1::ObjectVersion, OpError> {
        let ciphertext = at
            .key
            .seal_object(&at.id, object_id, kind, plaintext)
            .map_err(remote)?;
        self.ops
            .put_object(v1::PutObjectRequest {
                collection_id: at.id.clone(),
                object_id: object_id.to_owned(),
                expected_version: expected,
                kind: kind.to_owned(),
                ciphertext,
            })
            .await
    }

    /// Replaces the object with a tombstone.
    pub(crate) async fn remove(
        &self,
        at: &Located,
        object_id: &str,
        expected: u64,
    ) -> Result<v1::ObjectVersion, OpError> {
        self.ops
            .delete_object(v1::DeleteObjectRequest {
                collection_id: at.id.clone(),
                object_id: object_id.to_owned(),
                expected_version: expected,
            })
            .await
    }

    /// The changes of the collection after stream version `after`.
    pub(crate) async fn changes(
        &self,
        at: &Located,
        after: u64,
    ) -> Result<v1::ChangeBatch, OpError> {
        self.ops.changes(&at.id, after).await
    }
}

#[cfg(test)]
mod tests;
