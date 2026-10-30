//! One exact append-only CAS object. Uncertain writes read that same selector once.
use super::*;
use crate::{
    account::operations::OpenedWork,
    store::{ExactPersonalCollection, Located, OpError},
};
use nocterm_hub_proto::v1;
use prost::Message as _;

pub(super) async fn load_or_register(
    work: &OpenedWork,
    scope: &AccountScope,
    context: &[u8],
    registration: Option<&RunnerRegistration>,
) -> Result<(u64, Vec<RunnerRegistration>), RunnerRegistryError> {
    work.source().check().map_err(account)?;
    let actual = work
        .source()
        .native()
        .with_current(|keys| {
            Ok(<[u8; 32]>::from(Sha256::digest(
                keys.keyset().public_key().to_bytes(),
            )))
        })
        .map_err(account)?;
    let subject = work.source().native().subject().map_err(account)?;
    if actual != *scope.account().key().as_bytes()
        || subject.account() != scope.account().id().as_str()
    {
        return Err(RunnerRegistryError::Binding);
    }
    if registration.is_some_and(|row| row.binding.scope() != scope) {
        return Err(RunnerRegistryError::Binding);
    }
    let store = work.transient_store();
    let collection_id = codec::collection_id(context);
    let collection = ExactPersonalCollection::new(&store, codec::KIND, &collection_id);
    let found = work
        .source()
        .during(collection.find())
        .await
        .map_err(account)?
        .map_err(store_error)?;
    let at = match found {
        Some(at) => at,
        None if registration.is_none() => return Ok((0, Vec::new())),
        None => work
            .source()
            .during(collection.ensure())
            .await
            .map_err(account)?
            .map_err(store_error)?,
    };
    let (version, mut rows) = read(work, &at, scope, context).await?;
    let Some(registration) = registration else {
        return Ok((version, rows));
    };
    if existing(&rows, registration)? {
        return Ok((version, rows));
    }
    if rows.len() >= codec::MAX_RUNNERS {
        return Err(RunnerRegistryError::Capacity);
    }
    rows.push(RunnerRegistration::new(
        registration.binding.clone(),
        registration.pins,
        registration.label.clone(),
    )?);
    rows.sort_by_key(|row| *row.binding.runner().as_bytes());
    publish(work, &at, scope, context, registration, version, rows).await
}
async fn publish(
    work: &OpenedWork,
    at: &Located,
    scope: &AccountScope,
    context: &[u8],
    registration: &RunnerRegistration,
    version: u64,
    rows: Vec<RunnerRegistration>,
) -> Result<(u64, Vec<RunnerRegistration>), RunnerRegistryError> {
    let next = version
        .checked_add(1)
        .ok_or(RunnerRegistryError::Capacity)?;
    let bytes = codec::encode(context, next, &rows)?;
    let ciphertext = at
        .key()
        .seal_object(at.id(), codec::OBJECT, codec::KIND, &bytes)
        .map_err(backend)?;
    let request = v1::PutObjectRequest {
        collection_id: at.id().to_owned(),
        object_id: codec::OBJECT.to_owned(),
        expected_version: version,
        kind: codec::KIND.to_owned(),
        ciphertext,
    };
    if request.encoded_len() > MAX_RESPONSE {
        return Err(RunnerRegistryError::Capacity);
    }
    let written = work
        .source()
        .during(work.ops().put_object(request))
        .await
        .map_err(account)?;
    match written {
        Ok(receipt)
            if receipt.collection_id == at.id()
                && receipt.object_id == codec::OBJECT
                && receipt.version == next
                && receipt.stream_version != 0 =>
        {
            Ok((next, rows))
        }
        Ok(_) => Err(RunnerRegistryError::Format),
        Err(error) => {
            let winner = read(work, at, scope, context).await?;
            if existing(&winner.1, registration)? {
                return Ok(winner);
            }
            match error {
                OpError::Conflict => Err(RunnerRegistryError::Conflict),
                error => Err(store_error(error)),
            }
        }
    }
}

fn existing(
    rows: &[RunnerRegistration],
    registration: &RunnerRegistration,
) -> Result<bool, RunnerRegistryError> {
    match rows
        .iter()
        .find(|row| row.binding.runner() == registration.binding.runner())
    {
        Some(row) if row == registration => Ok(true),
        Some(_) => Err(RunnerRegistryError::Rotation),
        None => Ok(false),
    }
}
async fn read(
    work: &OpenedWork,
    at: &Located,
    scope: &AccountScope,
    context: &[u8],
) -> Result<(u64, Vec<RunnerRegistration>), RunnerRegistryError> {
    let object = work
        .source()
        .during(work.ops().object(at.id(), codec::OBJECT))
        .await
        .map_err(account)?
        .map_err(store_error)?;
    let Some(object) = object else {
        return Ok((0, Vec::new()));
    };
    if object.encoded_len() > MAX_RESPONSE || object.ciphertext.len() > codec::MAX_PLAINTEXT + 42 {
        return Err(RunnerRegistryError::Capacity);
    }
    if object.collection_id != at.id()
        || object.object_id != codec::OBJECT
        || object.kind != codec::KIND
        || object.updated_by_device.len() > 128
        || object.stream_version == 0
    {
        return Err(RunnerRegistryError::Binding);
    }
    if object.deleted {
        return Err(RunnerRegistryError::Deleted);
    }
    if object.version == 0 || object.version > codec::MAX_RUNNERS as u64 {
        return Err(RunnerRegistryError::Format);
    }
    let plaintext = at
        .key()
        .open_object(at.id(), codec::OBJECT, codec::KIND, &object.ciphertext)
        .map_err(backend)?;
    let rows = codec::decode(scope, context, object.version, &plaintext)?;
    Ok((object.version, rows))
}
fn store_error(error: OpError) -> RunnerRegistryError {
    match error {
        OpError::Conflict => RunnerRegistryError::Conflict,
        OpError::Hub(error) => account(error),
        error => backend(error),
    }
}
