//! Encrypted collections of one account on a hub.
//!
//! [`StoreOps`] is the narrow seam to the hub's store service: plain calls with sealed bytes.
//! [`Store`] adds what needs the account keys: finding or creating the account's personal
//! collection of a kind, and sealing what goes into it. Only the kinds this client knows
//! are asked for, so a collection of another kind is never listed, opened, changed or
//! deleted here.
//!
//! A group is an organization's collection of a kind. The account only ever uses one whose
//! key an administrator granted to it: the hub answers "not found" for the key of a
//! collection the member holds none of, and that collection is then left out. A grant opens
//! only under the key of the administrator who made it, as pinned on this device
//! ([`crate::AdminKeys`]): a group whose administrator is not pinned yet is left out and
//! reported as pending with the fingerprint to compare. This client never creates, deletes or
//! grants a group's collection; that is `hubctl`'s job.

mod grpc;
#[cfg(test)]
pub(crate) mod memory;

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::HubError;
use nocterm_keys::{CollectionKey, OpenError, PinError, PublicKey, TrustedKey, random_bytes};
use uuid::Builder;

use crate::account::{OpenKeys, PendingAdminKey};
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
    /// The key envelope the hub holds for the account, with who granted it.
    fn collection_key(&self, collection_id: &str) -> Call<'_, v1::CollectionKey>;
    /// The members of a collection the account can read, with their public keys.
    fn members(&self, collection_id: &str) -> Call<'_, Vec<v1::CollectionMember>>;
    /// The organizations the account belongs to, with its role in each.
    fn memberships(&self) -> Call<'_, Vec<v1::Membership>>;
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

/// A group's collection of one kind and what the member may do in it.
#[derive(Clone)]
pub(crate) struct Group {
    pub(crate) org_id: String,
    /// What the user calls the organization.
    pub(crate) name: String,
    /// Members and above may write; a guest only reads.
    pub(crate) writable: bool,
    pub(crate) at: Located,
}

/// The collections of one kind the account uses: its own and its groups'.
#[derive(Clone, Default)]
pub(crate) struct Layout {
    pub(crate) personal: Option<Located>,
    pub(crate) groups: Vec<Group>,
}

/// What the pins of this device say of the key a grant was made with.
enum Granter {
    Offered(TrustedKey),
    Unconfirmed(PendingAdminKey, PublicKey),
    Unusable,
}

/// What came of the key of a group's collection.
enum Granted {
    Opened(Located),
    /// The account holds no key of it, or the key does not open: the group is left out.
    Unusable,
    /// The administrator who granted it is not pinned, or is pinned with another key.
    Unconfirmed(PendingAdminKey, PublicKey),
}

/// The member of a collection the hub names as the granter, with the public key it lists for
/// them. A granter that is not a member, or has no usable key, leaves the group out.
fn listed_key<'a>(
    members: &'a [v1::CollectionMember],
    collection_id: &str,
    granted_by: &str,
) -> Option<(&'a v1::CollectionMember, PublicKey)> {
    let Some(member) = members.iter().find(|m| m.account_id == granted_by) else {
        tracing::warn!(collection = %collection_id, "the granter is not a member; the group is left out");
        return None;
    };
    match PublicKey::from_bytes(&member.public_key) {
        Ok(offered) => Some((member, offered)),
        Err(_) => {
            tracing::warn!(collection = %collection_id, "the granter has no usable public key; the group is left out");
            None
        }
    }
}

/// An administrator's key as it is shown for confirmation.
fn pending_key(
    member: &v1::CollectionMember,
    organization: &str,
    offered: &PublicKey,
    replaces: Option<String>,
) -> PendingAdminKey {
    let admin = if member.display_name.is_empty() {
        member.username.clone()
    } else {
        member.display_name.clone()
    };
    PendingAdminKey {
        organization: organization.to_owned(),
        account_id: member.account_id.clone(),
        admin,
        fingerprint: offered.fingerprint(),
        replaces,
    }
}

/// Whether the role may write to the organization's collections.
fn may_write(role: i32) -> bool {
    matches!(
        v1::Role::try_from(role),
        Ok(v1::Role::Member | v1::Role::Admin | v1::Role::Owner)
    )
}

/// Of the live collections that satisfy `wanted`, the oldest: when two devices made one at
/// the same moment, it is the collection and the others stay unused.
fn oldest<'a>(
    listed: &'a [v1::Collection],
    kind: &str,
    wanted: impl Fn(&v1::Collection) -> bool,
) -> Option<&'a v1::Collection> {
    listed
        .iter()
        .filter(|c| !c.deleted && c.kind == kind && wanted(c))
        .min_by(|a, b| {
            (a.created_at_ms, &a.collection_id).cmp(&(b.created_at_ms, &b.collection_id))
        })
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
        match oldest(&listed, kind, |c| c.personal && c.org_id.is_empty()) {
            Some(chosen) => Ok(Some(self.open(chosen).await?)),
            None => Ok(None),
        }
    }

    /// The collection of `kind` that organization `org_id` keeps and the account holds the
    /// key of. `None` when the organization has none or has not granted the key yet.
    pub(crate) async fn shared(
        &self,
        kind: &str,
        org_id: &str,
    ) -> Result<Option<Located>, OpError> {
        let listed = self.ops.collections(kind).await?;
        let Some(chosen) = oldest(&listed, kind, |c| !c.personal && c.org_id == org_id) else {
            return Ok(None);
        };
        match self.granted(chosen, org_id).await? {
            Granted::Opened(at) => Ok(Some(at)),
            Granted::Unusable | Granted::Unconfirmed(..) => Ok(None),
        }
    }

    /// The account's own collection of `kind` and every group's it holds the key of. A group
    /// whose key is missing, or does not open, is left out: one group cannot take the
    /// account's own data away.
    pub(crate) async fn layout(&self, kind: &str) -> Result<Layout, OpError> {
        let listed = self.ops.collections(kind).await?;
        let personal = match oldest(&listed, kind, |c| c.personal && c.org_id.is_empty()) {
            Some(chosen) => Some(self.open(chosen).await?),
            None => None,
        };
        let mut groups = Vec::new();
        let mut pending = Vec::new();
        for membership in self.ops.memberships().await? {
            let Some(org) = membership.org else { continue };
            let Some(chosen) = oldest(&listed, kind, |c| !c.personal && c.org_id == org.org_id)
            else {
                continue;
            };
            match self.granted(chosen, &org.name).await? {
                Granted::Opened(at) => groups.push(Group {
                    org_id: org.org_id,
                    name: org.name,
                    writable: may_write(membership.role),
                    at,
                }),
                Granted::Unconfirmed(admin, key) => pending.push((admin, key)),
                Granted::Unusable => {}
            }
        }
        self.keys.note_pending(kind, pending);
        Ok(Layout { personal, groups })
    }

    /// The account's own collection: the envelope was sealed by this account.
    async fn open(&self, chosen: &v1::Collection) -> Result<Located, OpError> {
        let record = self.ops.collection_key(&chosen.collection_id).await?;
        let key = self
            .keys
            .own_collection(&chosen.collection_id, &record.wrapped_key)
            .map_err(OpError::Damaged)?;
        Ok(Located {
            id: chosen.collection_id.clone(),
            key,
            envelope: record.wrapped_key,
        })
    }

    /// A group's collection. An envelope the account sealed itself (a collection it created)
    /// opens as its own; a grant opens only under the pinned key of the account that made it.
    async fn granted(
        &self,
        chosen: &v1::Collection,
        organization: &str,
    ) -> Result<Granted, OpError> {
        let id = &chosen.collection_id;
        let record = match self.ops.collection_key(id).await {
            Ok(record) => record,
            Err(OpError::Hub(HubError::NotFound | HubError::PermissionDenied)) => {
                return Ok(Granted::Unusable);
            }
            Err(error) => return Err(error),
        };
        let envelope = record.wrapped_key;
        let opened = if record.granted_by.is_empty() {
            self.keys.own_collection(id, &envelope)
        } else {
            let offered = match self.granter(id, &record.granted_by, organization).await? {
                Granter::Offered(key) => key,
                Granter::Unconfirmed(admin, key) => return Ok(Granted::Unconfirmed(admin, key)),
                Granter::Unusable => return Ok(Granted::Unusable),
            };
            self.keys.granted_collection(id, &envelope, &offered)
        };
        match opened {
            Ok(key) => Ok(Granted::Opened(Located {
                id: id.clone(),
                key,
                envelope,
            })),
            Err(error) => {
                tracing::warn!(
                    collection = %id,
                    %error,
                    "a group's collection key does not open; the group is left out"
                );
                Ok(Granted::Unusable)
            }
        }
    }

    /// The key the hub names as the granter's, checked against the pins of this device.
    async fn granter(
        &self,
        collection_id: &str,
        granted_by: &str,
        organization: &str,
    ) -> Result<Granter, OpError> {
        let members = self.ops.members(collection_id).await?;
        let Some((member, offered)) = listed_key(&members, collection_id, granted_by) else {
            return Ok(Granter::Unusable);
        };
        let pending = |replaces| pending_key(member, organization, &offered, replaces);
        match self.keys.trust(granted_by, &offered) {
            Ok(trusted) => Ok(Granter::Offered(trusted)),
            Err(PinError::Unknown { .. }) => Ok(Granter::Unconfirmed(pending(None), offered)),
            Err(PinError::Changed { pinned, .. }) => {
                Ok(Granter::Unconfirmed(pending(Some(pinned)), offered))
            }
            Err(error) => {
                tracing::warn!(%error, "the granter's key cannot be trusted; the group is left out");
                Ok(Granter::Unusable)
            }
        }
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
