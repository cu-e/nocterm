//! The same memory store under an explicit finite material response policy.
use super::*;

pub(super) struct BoundedMemoryHub {
    hub: Arc<MemoryHub>,
    limit: usize,
    _allocation: nocterm_core::allocation::AllocationLease,
}
impl BoundedMemoryHub {
    pub(super) fn new(
        hub: Arc<MemoryHub>,
        limit: usize,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> Self {
        Self {
            hub,
            limit,
            _allocation: allocation,
        }
    }
    fn require(&self, bytes: usize) -> Result<(), OpError> {
        if bytes > self.limit {
            Err(OpError::Hub(HubError::PermissionDenied))
        } else {
            Ok(())
        }
    }
}
fn collection_bytes(row: &v1::Collection) -> usize {
    128 + row.collection_id.len() + row.org_id.len() + row.kind.len() + row.sealed_name.len()
}
impl StoreOps for BoundedMemoryHub {
    fn keyset(&self) -> Call<'_, Option<v1::AccountKeyset>> {
        Box::pin(async { Err(OpError::Hub(HubError::PermissionDenied)) })
    }
    fn collections(&self, kind: &str) -> Call<'_, Vec<v1::Collection>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            let rows = self.hub.collections(&kind).await?;
            self.require(rows.iter().map(collection_bytes).sum())?;
            Ok(rows)
        })
    }
    fn create_collection(&self, request: v1::CreateCollectionRequest) -> Call<'_, v1::Collection> {
        Box::pin(async move {
            let row = self.hub.create_collection(request).await?;
            self.require(collection_bytes(&row))?;
            Ok(row)
        })
    }
    fn collection_key(&self, collection: &str) -> Call<'_, v1::CollectionKey> {
        let collection = collection.to_owned();
        Box::pin(async move {
            let row = self.hub.collection_key(&collection).await?;
            self.require(
                128 + row.collection_id.len() + row.granted_by.len() + row.wrapped_key.len(),
            )?;
            Ok(row)
        })
    }
    fn members(&self, _: &str) -> Call<'_, Vec<v1::CollectionMember>> {
        Box::pin(async { Err(OpError::Hub(HubError::PermissionDenied)) })
    }
    fn memberships(&self) -> Call<'_, Vec<v1::Membership>> {
        Box::pin(async { Err(OpError::Hub(HubError::PermissionDenied)) })
    }
    fn object(&self, collection: &str, id: &str) -> Call<'_, Option<v1::StoredObject>> {
        let collection = collection.to_owned();
        let id = id.to_owned();
        Box::pin(async move {
            let row = self.hub.object(&collection, &id).await?;
            if let Some(row) = &row {
                self.require(
                    128 + row.collection_id.len()
                        + row.object_id.len()
                        + row.kind.len()
                        + row.updated_by_device.len()
                        + row.ciphertext.len(),
                )?;
            }
            Ok(row)
        })
    }
    fn put_object(&self, request: v1::PutObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async move {
            self.require(
                128 + request.collection_id.len()
                    + request.object_id.len()
                    + request.kind.len()
                    + request.ciphertext.len(),
            )?;
            let row = self.hub.put_object(request).await?;
            self.require(128 + row.collection_id.len() + row.object_id.len())?;
            Ok(row)
        })
    }
    fn delete_object(&self, _: v1::DeleteObjectRequest) -> Call<'_, v1::ObjectVersion> {
        Box::pin(async { Err(OpError::Hub(HubError::PermissionDenied)) })
    }
    fn changes(&self, _: &str, _: u64) -> Call<'_, v1::ChangeBatch> {
        Box::pin(async { Err(OpError::Hub(HubError::PermissionDenied)) })
    }
    fn watch(&self, _: Vec<String>) -> Call<'_, futures::stream::BoxStream<'static, ()>> {
        Box::pin(async { Err(OpError::Hub(HubError::PermissionDenied)) })
    }
}
