//! The store service of a signed-in hub.

use std::sync::Arc;

use super::client::StoreClient;
use crate::account::source::{CapturedAccount, CapturedMembership};
use futures::StreamExt as _;
use nocterm_hub_client::ClientError;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};

use super::{Call, OpError, StoreOps};
use crate::pool::Pool;
use nocterm_core::allocation::AllocationLease;

/// Reaches the store service of one hub with the pooled client.
pub(crate) struct HubStore {
    client: StoreClient,
    bounded: Option<BoundedStore>,
}

struct BoundedStore {
    maximum: usize,
    allocation: AllocationLease,
}
// The exact same permit follows generated request/response bodies as well as
// this native store and its decoded buffers. Legacy callers keep their client.
macro_rules! store_rpc {
    ($owner:expr, $client:expr, $method:ident, $request:expr) => {{
        let request = $request;
        let response = if let Some(bound) = &$owner.bounded {
            $client
                .store_with_residency(Arc::new(bound.allocation.clone()), bound.maximum)
                .map_err(|error| $owner.failure(error))?
                .$method(request)
                .await
        } else {
            $client.store().$method(request).await
        };
        $owner.client.check()?;
        response
    }};
}

impl HubStore {
    pub(crate) fn captured(
        pool: Arc<Pool>,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Self, HubError> {
        Ok(Self {
            client: StoreClient::captured(pool, native, source)?,
            bounded: None,
        })
    }

    pub(crate) fn captured_bounded(
        pool: Arc<Pool>,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
        max_response_bytes: usize,
        allocation: AllocationLease,
    ) -> Result<Self, HubError> {
        if max_response_bytes == 0 {
            return Err(HubError::PermissionDenied);
        }
        Ok(Self {
            client: StoreClient::captured_cloud(pool, native, source)?,
            bounded: Some(BoundedStore {
                maximum: max_response_bytes,
                allocation,
            }),
        })
    }
    fn failure(&self, error: ClientError) -> OpError {
        self.client.failure(error)
    }

    pub(crate) fn membership(
        pool: Arc<Pool>,
        native: &CapturedMembership,
    ) -> Result<Self, HubError> {
        Ok(Self {
            client: StoreClient::membership(pool, native)?,
            bounded: None,
        })
    }
}

impl StoreOps for HubStore {
    fn keyset(&self) -> Call<'_, Option<v1::AccountKeyset>> {
        Box::pin(async move {
            if self.bounded.is_some() {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            let client = self.client.fresh().await?;
            client
                .keyset_record()
                .await
                .map_err(|error| self.failure(error))
        })
    }

    fn collections(&self, kind: &str) -> Call<'_, Vec<v1::Collection>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let list = store_rpc!(
                self,
                client,
                list_collections,
                v1::ListCollectionsRequest {
                    org_id: String::new(),
                    kind,
                }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(list.into_inner().collections)
        })
    }

    fn create_collection(&self, request: v1::CreateCollectionRequest) -> Call<'_, v1::Collection> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let made = store_rpc!(self, client, create_collection, request)
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(made.into_inner())
        })
    }

    fn collection_key(&self, collection_id: &str) -> Call<'_, v1::CollectionKey> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let key = store_rpc!(
                self,
                client,
                get_collection_key,
                v1::GetCollectionKeyRequest { collection_id }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(key.into_inner())
        })
    }

    fn members(&self, collection_id: &str) -> Call<'_, Vec<v1::CollectionMember>> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            if self.bounded.is_some() {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            let client = self.client.fresh().await?;
            let list = store_rpc!(
                self,
                client,
                list_collection_members,
                v1::ListCollectionMembersRequest { collection_id }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(list.into_inner().members)
        })
    }

    fn memberships(&self) -> Call<'_, Vec<v1::Membership>> {
        Box::pin(async move {
            if self.bounded.is_some() {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            let client = self.client.fresh().await?;
            let me = client
                .directory()
                .who_am_i(v1::Empty {})
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(me.into_inner().memberships)
        })
    }

    fn object(
        &self,
        collection_id: &str,
        object_id: &str,
    ) -> super::Call<'_, Option<v1::StoredObject>> {
        let collection_id = collection_id.to_owned();
        let object_id = object_id.to_owned();
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let result = store_rpc!(
                self,
                client,
                get_object,
                v1::GetObjectRequest {
                    collection_id,
                    object_id,
                }
            );
            match result {
                Ok(response) => Ok(Some(response.into_inner())),
                Err(status) => {
                    let error = ClientError::from(status);
                    if matches!(
                        error,
                        ClientError::Status {
                            code: nocterm_hub_client::StatusCode::NotFound,
                            ..
                        }
                    ) {
                        Ok(None)
                    } else {
                        Err(self.failure(error))
                    }
                }
            }
        })
    }

    fn put_object(&self, request: v1::PutObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let stored = store_rpc!(self, client, put_object, request)
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(stored.into_inner())
        })
    }

    fn delete_object(&self, request: v1::DeleteObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let stored = store_rpc!(self, client, delete_object, request)
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(stored.into_inner())
        })
    }

    fn changes(&self, collection_id: &str, after: u64) -> Call<'_, v1::ChangeBatch> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            if self.bounded.is_some() {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            let client = self.client.fresh().await?;
            let batch = store_rpc!(
                self,
                client,
                list_changes,
                v1::ListChangesRequest {
                    collection_id,
                    after,
                    limit: 0,
                }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(batch.into_inner())
        })
    }

    fn watch(&self, collections: Vec<String>) -> Call<'_, futures::stream::BoxStream<'static, ()>> {
        Box::pin(async move {
            if self.bounded.is_some() {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            let client = self.client.fresh().await?;
            let stream = store_rpc!(
                self,
                client,
                watch_changes,
                v1::WatchChangesRequest {
                    collection_ids: collections,
                }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?
            .into_inner();
            // A hint only says "read again"; the stream ends at the first failure.
            let hints = futures::stream::unfold(stream, |mut stream| async move {
                match stream.message().await {
                    Ok(Some(_)) => Some(((), stream)),
                    _ => None,
                }
            });
            Ok(hints.boxed())
        })
    }
}
