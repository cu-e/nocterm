//! One exact immutable personal object; uncertain writes read that selector once.
use super::*;
use crate::{
    account::operations::OpenedWork,
    store::{ExactPersonalCollection, Located, OpError},
};
use nocterm_hub_proto::v1;
use prost::Message as _;

pub(super) async fn load_or_register(
    work: &OpenedWork,
    context: &[u8],
    registration: Option<[u8; 32]>,
) -> Result<[u8; 32], CloudTrustError> {
    work.source().check().map_err(account)?;
    let store = work.transient_store();
    let collection_id = records::collection_id(context);
    let collection = ExactPersonalCollection::new(&store, records::KIND, &collection_id);
    let found = work
        .source()
        .during(collection.find())
        .await
        .map_err(account)?
        .map_err(store_error)?;
    let at = match found {
        Some(at) => at,
        None if registration.is_none() => return Err(CloudTrustError::NotConfigured),
        None => work
            .source()
            .during(collection.ensure())
            .await
            .map_err(account)?
            .map_err(store_error)?,
    };
    match read(work, &at, context).await? {
        Some(authority) => require_same(authority, registration),
        None => match registration {
            Some(authority) => publish(work, &at, context, authority).await,
            None => Err(CloudTrustError::NotConfigured),
        },
    }
}
fn require_same(
    authority: [u8; 32],
    registration: Option<[u8; 32]>,
) -> Result<[u8; 32], CloudTrustError> {
    if registration.is_some_and(|expected| expected != authority) {
        Err(CloudTrustError::RotationRequired)
    } else {
        Ok(authority)
    }
}
async fn publish(
    work: &OpenedWork,
    at: &Located,
    context: &[u8],
    authority: [u8; 32],
) -> Result<[u8; 32], CloudTrustError> {
    let plaintext = records::encode(context, authority)?;
    let ciphertext = at
        .key()
        .seal_object(at.id(), records::OBJECT, records::KIND, &plaintext)
        .map_err(backend)?;
    let request = v1::PutObjectRequest {
        collection_id: at.id().to_owned(),
        object_id: records::OBJECT.to_owned(),
        expected_version: 0,
        kind: records::KIND.to_owned(),
        ciphertext,
    };
    if request.encoded_len() > MAX_RESPONSE {
        return Err(CloudTrustError::Capacity);
    }
    let written = work
        .source()
        .during(work.ops().put_object(request))
        .await
        .map_err(account)?;
    match written {
        Ok(receipt)
            if receipt.collection_id == at.id()
                && receipt.object_id == records::OBJECT
                && receipt.version == 1
                && receipt.stream_version != 0 =>
        {
            Ok(authority)
        }
        Ok(_) => Err(CloudTrustError::Format),
        Err(error) => match read(work, at, context).await? {
            Some(winner) => require_same(winner, Some(authority)),
            None => Err(store_error(error)),
        },
    }
}
async fn read(
    work: &OpenedWork,
    at: &Located,
    context: &[u8],
) -> Result<Option<[u8; 32]>, CloudTrustError> {
    let object = work
        .source()
        .during(work.ops().object(at.id(), records::OBJECT))
        .await
        .map_err(account)?
        .map_err(store_error)?;
    let Some(object) = object else {
        return Ok(None);
    };
    if object.encoded_len() > MAX_RESPONSE || object.ciphertext.len() > records::MAX_PLAINTEXT + 42
    {
        return Err(CloudTrustError::Capacity);
    }
    if object.collection_id != at.id()
        || object.object_id != records::OBJECT
        || object.kind != records::KIND
        || object.updated_by_device.len() > 128
        || object.stream_version == 0
    {
        return Err(CloudTrustError::Binding);
    }
    if object.deleted {
        return Err(CloudTrustError::Deleted);
    }
    if object.version != 1 {
        return Err(CloudTrustError::Format);
    }
    let plaintext = at
        .key()
        .open_object(at.id(), records::OBJECT, records::KIND, &object.ciphertext)
        .map_err(backend)?;
    records::decode(context, &plaintext).map(Some)
}
fn store_error(error: OpError) -> CloudTrustError {
    match error {
        OpError::Hub(error) => account(error),
        error => backend(error),
    }
}
