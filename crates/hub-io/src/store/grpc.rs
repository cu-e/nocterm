//! The store service of a signed-in hub.

use std::sync::Arc;

use futures::StreamExt as _;
use nocterm_hub_client::ClientError;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::HubId;

use super::{Call, OpError, StoreOps};
use crate::pool::Pool;

/// Reaches the store service of one hub with the pooled client.
pub(crate) struct HubStore {
    pool: Arc<Pool>,
    hub: HubId,
}

impl HubStore {
    pub(crate) fn new(pool: Arc<Pool>, hub: HubId) -> Self {
        Self { pool, hub }
    }

    fn failure(&self, error: ClientError) -> OpError {
        if error.is_conflict() {
            OpError::Conflict
        } else {
            OpError::Hub(self.pool.failed(&self.hub, error))
        }
    }
}

impl StoreOps for HubStore {
    fn keyset(&self) -> Call<'_, Option<v1::AccountKeyset>> {
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            client
                .keyset_record()
                .await
                .map_err(|error| self.failure(error))
        })
    }

    fn collections(&self, kind: &str) -> Call<'_, Vec<v1::Collection>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let list = client
                .store()
                .list_collections(v1::ListCollectionsRequest {
                    org_id: String::new(),
                    kind,
                })
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(list.into_inner().collections)
        })
    }

    fn create_collection(&self, request: v1::CreateCollectionRequest) -> Call<'_, v1::Collection> {
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let made = client
                .store()
                .create_collection(request)
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(made.into_inner())
        })
    }

    fn collection_key(&self, collection_id: &str) -> Call<'_, v1::CollectionKey> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let key = client
                .store()
                .get_collection_key(v1::GetCollectionKeyRequest { collection_id })
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(key.into_inner())
        })
    }

    fn members(&self, collection_id: &str) -> Call<'_, Vec<v1::CollectionMember>> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let list = client
                .store()
                .list_collection_members(v1::ListCollectionMembersRequest { collection_id })
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(list.into_inner().members)
        })
    }

    fn memberships(&self) -> Call<'_, Vec<v1::Membership>> {
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let me = client
                .directory()
                .who_am_i(v1::Empty {})
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(me.into_inner().memberships)
        })
    }

    fn put_object(&self, request: v1::PutObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let stored = client
                .store()
                .put_object(request)
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(stored.into_inner())
        })
    }

    fn delete_object(&self, request: v1::DeleteObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let stored = client
                .store()
                .delete_object(request)
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(stored.into_inner())
        })
    }

    fn changes(&self, collection_id: &str, after: u64) -> Call<'_, v1::ChangeBatch> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let batch = client
                .store()
                .list_changes(v1::ListChangesRequest {
                    collection_id,
                    after,
                    limit: 0,
                })
                .await
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(batch.into_inner())
        })
    }

    fn watch(&self, collections: Vec<String>) -> Call<'_, futures::stream::BoxStream<'static, ()>> {
        Box::pin(async move {
            let client = self.pool.fresh(&self.hub).await?;
            let stream = client
                .store()
                .watch_changes(v1::WatchChangesRequest {
                    collection_ids: collections,
                })
                .await
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
