//! One deterministic personal object, never a changes scan or an overwrite.
use super::{
    record::{self, MaterialRecord},
    *,
};
use crate::{
    account::operations::{OpenedSource, OpenedWork},
    store::{ExactPersonalCollection, Located, OpError},
};

pub(super) async fn load_or_create(
    work: &OpenedWork,
    request: ChatMaterialRequest,
    create: bool,
) -> Result<MaterialRecord, AccountMaterialError> {
    let source = work.source();
    source.check()?;
    let store = work.transient_store();
    let collection_id = record::collection_id(request.stream.scope())?;
    let collection = ExactPersonalCollection::new(&store, record::KIND, &collection_id);
    let found = source
        .during(collection.find())
        .await?
        .map_err(store_error)?;
    let at = match found {
        Some(at) => at,
        None if create => source
            .during(collection.ensure())
            .await?
            .map_err(store_error)?,
        None => return Err(AccountMaterialError::Missing),
    };
    let id = record::object_id(&request)?;
    if let Some(record) = read(work, &at, &id, request.clone()).await? {
        return Ok(record);
    }
    if !create {
        return Err(AccountMaterialError::Missing);
    }
    if request.epoch.get() != 1 {
        return Err(AccountMaterialError::Rotation);
    }
    let coordinate = record::coordinate(&request)?;
    let wrapped = source.native().with_current(|keys| {
        Ok(ChatKey::generate().map_err(cause).and_then(|key| {
            key.seal_for_account(keys.keyset().account_key(), &coordinate)
                .map_err(cause)
        }))
    })??;
    let record = MaterialRecord::new(request.clone(), wrapped);
    let plaintext = zeroize::Zeroizing::new(record.encode()?);
    let written = source
        .during(store.put(&at, record::KIND, &id, &plaintext, 0))
        .await?;
    match written {
        Ok(version)
            if version.collection_id == at.id()
                && version.object_id == id
                && version.version == 1
                && version.stream_version != 0 =>
        {
            Ok(record)
        }
        Ok(_) => Err(AccountMaterialError::Format),
        // Conflict or lost ACK: one bounded read of the SAME selector resolves
        // only an actual stored winner; no retry put and no regenerated ID.
        Err(error) => match read(work, &at, &id, request).await? {
            Some(winner) => Ok(winner),
            None => Err(store_error(error)),
        },
    }
}
async fn read(
    work: &OpenedWork,
    at: &Located,
    id: &str,
    request: ChatMaterialRequest,
) -> Result<Option<MaterialRecord>, AccountMaterialError> {
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
        || object.kind != record::KIND
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
    if object.ciphertext.len() > record::MAX_RECORD + 42 || object.ciphertext.len() < 42 {
        return Err(AccountMaterialError::Capacity);
    }
    let plaintext = at
        .key()
        .open_object(at.id(), id, record::KIND, &object.ciphertext)
        .map_err(cause)?;
    let record = MaterialRecord::decode(request, &plaintext)?;
    // Check the nested purpose-bound account envelope before publishing a handle.
    with_key(work.source(), &record, |_| Ok(()))?;
    Ok(Some(record))
}
pub(super) fn with_key<T>(
    source: &OpenedSource,
    record: &MaterialRecord,
    action: impl FnOnce(&ChatKey) -> Result<T, AccountMaterialError>,
) -> Result<T, AccountMaterialError> {
    source.check()?;
    let coordinate = record::coordinate(record.request())?;
    source.native().with_current(|keys| {
        Ok(
            ChatKey::open_for_account(keys.keyset().account_key(), &coordinate, record.wrapped())
                .map_err(cause)
                .and_then(|key| action(&key)),
        )
    })?
}

fn store_error(error: OpError) -> AccountMaterialError {
    // Preserve the typed native account cause directly; its SharedCause fields
    // receive the same budget guard when the worker publishes an error.
    match error {
        OpError::Hub(error) => AccountMaterialError::Account(error),
        error => cause(error),
    }
}
