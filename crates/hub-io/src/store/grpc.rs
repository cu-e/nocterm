//! The store service of a signed-in hub.

use std::sync::Arc;

use super::client::StoreClient;
use crate::account::source::{CapturedAccount, CapturedMembership};
use futures::StreamExt as _;
use nocterm_hub_client::ClientError;
use nocterm_hub_proto::v2;
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};

use super::{Call, OpError, StoreOps};
use crate::pool::Pool;

/// Reaches the vault service of one hub with the pooled client.
pub(crate) struct HubStore {
    client: StoreClient,
}

macro_rules! store_rpc {
    ($client:expr, $method:ident, $request:expr) => {
        $client.vault().$method($request).await
    };
}

impl HubStore {
    pub(crate) fn captured(
        pool: Arc<Pool>,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Self, HubError> {
        Ok(Self {
            client: StoreClient::captured(pool, native, source)?,
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
        })
    }
}

impl StoreOps for HubStore {
    fn keyset(&self) -> Call<'_, Option<v2::Keyset>> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            client
                .keyset_record()
                .await
                .map_err(|error| self.failure(error))
        })
    }

    fn collections(&self, kind: &str) -> Call<'_, Vec<v2::Collection>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let list = store_rpc!(
                client,
                list_collections,
                v2::ListCollectionsRequest { kind }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(list.into_inner().collections)
        })
    }

    fn create_collection(&self, request: v2::CreateCollectionRequest) -> Call<'_, v2::Collection> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let made = store_rpc!(client, create_collection, request)
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(made.into_inner())
        })
    }

    fn collection_key(&self, collection_id: &str) -> Call<'_, v2::CollectionKey> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let key = store_rpc!(
                client,
                get_collection_key,
                v2::IdRequest { id: collection_id }
            )
            .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(key.into_inner())
        })
    }

    fn put_object(&self, request: v2::PutObjectRequest) -> Call<'_, v2::ObjectVersion> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let stored = store_rpc!(client, put_object, request)
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(stored.into_inner())
        })
    }

    fn delete_object(&self, request: v2::DeleteObjectRequest) -> Call<'_, v2::ObjectVersion> {
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let stored = store_rpc!(client, delete_object, request)
                .map_err(|status| self.failure(ClientError::from(status)))?;
            Ok(stored.into_inner())
        })
    }

    fn changes(&self, collection_id: &str, after: u64) -> Call<'_, v2::ChangeBatch> {
        let collection_id = collection_id.to_owned();
        Box::pin(async move {
            let client = self.client.fresh().await?;
            let batch = store_rpc!(
                client,
                list_changes,
                v2::ListChangesRequest {
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
            let client = self.client.fresh().await?;
            let stream = store_rpc!(
                client,
                watch_changes,
                v2::WatchChangesRequest {
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
