//! The sealed chat key grant of one chat, kept in the account before it is published.
//! A different blob for an installed key epoch fails closed at the runner, so every
//! publication and every retry sends the bytes first stored here.
use super::{
    record::{self, collection_id_of},
    *,
};
use crate::{
    account::operations::OpenedWork,
    store::{ExactPersonalCollection, Located, OpError},
};
use nocterm_hub_client::cloud::PreparedChatKeyGrant;
use nocterm_keys::PublicKey;

pub(super) const KIND: &str = "cloud-chat-grant";
const VERSION: u8 = 1;
const PUBLICATION_ID: usize = 16;
/// A sealed chat key is a few hundred bytes; the bound keeps hostile objects out of memory.
const MAX_CIPHERTEXT: usize = 1024;
pub(super) const MAX_RECORD: usize = 1 + PUBLICATION_ID + MAX_CIPHERTEXT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GrantRecord {
    publication_id: [u8; PUBLICATION_ID],
    ciphertext: Vec<u8>,
}
impl GrantRecord {
    pub(super) fn new(
        publication_id: [u8; PUBLICATION_ID],
        ciphertext: Vec<u8>,
    ) -> Result<Self, AccountMaterialError> {
        if ciphertext.is_empty() || ciphertext.len() > MAX_CIPHERTEXT {
            return Err(AccountMaterialError::Capacity);
        }
        Ok(Self {
            publication_id,
            ciphertext,
        })
    }
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1 + PUBLICATION_ID + self.ciphertext.len());
        bytes.push(VERSION);
        bytes.extend_from_slice(&self.publication_id);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }
    fn decode(bytes: &[u8]) -> Result<Self, AccountMaterialError> {
        if bytes.len() > MAX_RECORD {
            return Err(AccountMaterialError::Capacity);
        }
        let (version, rest) = bytes.split_first().ok_or(AccountMaterialError::Format)?;
        if *version != VERSION || rest.len() <= PUBLICATION_ID {
            return Err(AccountMaterialError::Format);
        }
        let (id, ciphertext) = rest.split_at(PUBLICATION_ID);
        Self::new(
            id.try_into().map_err(|_| AccountMaterialError::Format)?,
            ciphertext.to_vec(),
        )
    }
}

/// Compare-create: the first stored grant of a `(stream, epoch)` wins and is returned
/// verbatim to every later caller; `make` runs only when none exists.
pub(super) async fn load_or_create(
    work: &OpenedWork,
    request: &ChatMaterialRequest,
    make: impl FnOnce() -> Result<GrantRecord, AccountMaterialError>,
) -> Result<GrantRecord, AccountMaterialError> {
    let source = work.source();
    source.check()?;
    let store = work.transient_store();
    let collection_id = collection_id_of(request.stream.scope(), KIND)?;
    let collection = ExactPersonalCollection::new(&store, KIND, &collection_id);
    let found = source
        .during(collection.find())
        .await?
        .map_err(store_error)?;
    let at = match found {
        Some(at) => at,
        None => source
            .during(collection.ensure())
            .await?
            .map_err(store_error)?,
    };
    let id = record::object_id(request)?;
    if let Some(stored) = read(work, &at, &id).await? {
        return Ok(stored);
    }
    let grant = make()?;
    let plaintext = zeroize::Zeroizing::new(grant.encode());
    let written = source
        .during(store.put(&at, KIND, &id, &plaintext, 0))
        .await?;
    match written {
        Ok(version)
            if version.collection_id == at.id()
                && version.object_id == id
                && version.version == 1
                && version.stream_version != 0 =>
        {
            Ok(grant)
        }
        Ok(_) => Err(AccountMaterialError::Format),
        // A conflict or a lost ACK resolves to the stored winner, never to a second blob.
        Err(error) => match read(work, &at, &id).await? {
            Some(winner) => Ok(winner),
            None => Err(store_error(error)),
        },
    }
}

async fn read(
    work: &OpenedWork,
    at: &Located,
    id: &str,
) -> Result<Option<GrantRecord>, AccountMaterialError> {
    let object = work
        .source()
        .during(work.ops().object(at.id(), id))
        .await?
        .map_err(store_error)?;
    let Some(object) = object else {
        return Ok(None);
    };
    if object.collection_id != at.id()
        || object.object_id != id
        || object.kind != KIND
        || object.updated_by_device.len() > 128
        || object.stream_version == 0
    {
        return Err(AccountMaterialError::Binding);
    }
    if object.deleted {
        return Err(AccountMaterialError::Missing);
    }
    if object.version != 1 {
        return Err(AccountMaterialError::Format);
    }
    if object.ciphertext.len() > MAX_RECORD + 42 || object.ciphertext.len() < 42 {
        return Err(AccountMaterialError::Capacity);
    }
    let plaintext = at
        .key()
        .open_object(at.id(), id, KIND, &object.ciphertext)
        .map_err(cause)?;
    GrantRecord::decode(&plaintext).map(Some)
}

fn store_error(error: OpError) -> AccountMaterialError {
    match error {
        OpError::Hub(error) => AccountMaterialError::Account(error),
        error => cause(error),
    }
}

impl AccountChatMaterial {
    /// Seals this chat's key to the runner pinned in its request. The key is opened and
    /// used inside one native admission and never leaves it.
    fn seal_grant(
        &self,
        publication_id: [u8; PUBLICATION_ID],
    ) -> Result<GrantRecord, AccountMaterialError> {
        let owner = self.resident.value();
        self.check_current()?;
        let request = owner.record.request();
        let coordinate = record::coordinate(request)?;
        let pinned = PublicKey::from_bytes(&request.pins.encryption).map_err(cause)?;
        let sealed = owner.bound.source().native().with_current(|keys| {
            Ok(ChatKey::open_for_account(
                keys.keyset().account_key(),
                &coordinate,
                owner.record.wrapped(),
            )
            .map_err(cause)
            .and_then(|key| {
                owner
                    .cloud
                    .prepare_chat_key_grant(
                        keys.keyset(),
                        &pinned,
                        coordinate.clone(),
                        &key,
                        publication_id,
                    )
                    .map_err(cause)
            }))
        })??;
        GrantRecord::new(publication_id, sealed.ciphertext().to_vec())
    }
}

/// A grant whose exact bytes are durable in the account. It is the only way to publish one.
pub struct StoredChatKeyGrant {
    grant: Arc<PreparedChatKeyGrant>,
    cloud: Arc<CloudClient>,
    bound: BoundOperations,
}
impl StoredChatKeyGrant {
    /// Sends the stored bytes to the runner's inbox; repeating it is harmless.
    pub fn publish(&self) -> BoxFuture<'static, Result<(), AccountMaterialError>> {
        let source = self.bound.source().clone();
        let cloud = self.cloud.clone();
        let grant = self.grant.clone();
        Box::pin(async move {
            source.check()?;
            source
                .during(async move { cloud.publish_chat_key_grant(&grant).await })
                .await?
                .map(|_| ())
                .map_err(cause)
        })
    }
}

impl CloudAccountMaterial {
    /// Stores the chat key grant of `material` (compare-create) and returns it for publishing.
    /// A repeat for the same chat returns the first stored bytes.
    pub fn store_chat_key_grant(
        &self,
        material: Arc<AccountChatMaterial>,
        reservations: AccountMaterialReservations,
    ) -> BoxFuture<'static, Result<StoredChatKeyGrant, AccountMaterialError>> {
        let prepared: Result<_, AccountMaterialError> = (|| {
            reservations.require(account_material_work_bytes()?)?;
            self.cloud.check_source().map_err(cause)?;
            if !Arc::ptr_eq(&self.cloud, &material.resident.value().cloud) {
                return Err(AccountMaterialError::Binding);
            }
            let work = self
                .bound
                .capture_bounded(MAX_RESPONSE, reservations.work())?;
            Ok((work, self.bound.clone(), self.cloud.clone()))
        })();
        let (work, bound, cloud) = match prepared {
            Ok(value) => value,
            Err(error) => {
                let error = retain_error(error, reservations.work());
                return Box::pin(async move { Err(error) });
            }
        };
        let source = Allocated::new(bound.source().clone(), reservations.resident());
        let operation = Allocated::new((work, material, bound, cloud), reservations.work());
        let worker = crate::task::spawn(&self.handle, async move {
            let (work, material, bound, cloud) = operation.value();
            execute(work, material, bound, cloud)
                .await
                .map_err(|error| retain_error(error, operation.allocation().clone()))
        });
        Box::pin(async move {
            let result = source
                .value()
                .during(worker)
                .await?
                .ok_or(AccountMaterialError::Stale)??;
            source
                .value()
                .publish(|| result)
                .map_err(AccountMaterialError::from)
        })
    }
}

async fn execute(
    work: &OpenedWork,
    material: &Arc<AccountChatMaterial>,
    bound: &BoundOperations,
    cloud: &Arc<CloudClient>,
) -> Result<StoredChatKeyGrant, AccountMaterialError> {
    let request = material.request().clone();
    let publication_id = nocterm_keys::random_bytes::<PUBLICATION_ID>().map_err(cause)?;
    let stored = load_or_create(work, &request, || material.seal_grant(publication_id)).await?;
    let coordinate = record::coordinate(&request)?;
    let grant = cloud
        .stage_chat_key_grant(coordinate, stored.publication_id, stored.ciphertext)
        .map_err(cause)?;
    work.source().check()?;
    Ok(StoredChatKeyGrant {
        grant: Arc::new(grant),
        cloud: cloud.clone(),
        bound: bound.clone(),
    })
}
