//! Immutable selection of one caller-named personal collection.
use super::*;

/// This owner never selects an older/random collection of the same kind. A
/// failed or uncertain create reads this exact ID and opens the persisted key.
pub(crate) struct ExactPersonalCollection<'a> {
    store: &'a Store,
    kind: &'a str,
    id: &'a str,
}
impl<'a> ExactPersonalCollection<'a> {
    pub(crate) fn new(store: &'a Store, kind: &'a str, id: &'a str) -> Self {
        Self { store, kind, id }
    }
    pub(crate) async fn find(&self) -> Result<Option<Located>, OpError> {
        let listed = self.store.ops.collections(self.kind).await?;
        let mut selected = None;
        for row in &listed {
            if row.collection_id.len() > 128
                || row.org_id.len() > 128
                || row.kind.len() > 128
                || row.sealed_name.len() > 1024
            {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            if row.collection_id != self.id {
                continue;
            }
            if selected.is_some()
                || row.deleted
                || !row.personal
                || !row.org_id.is_empty()
                || row.kind != self.kind
            {
                return Err(OpError::Hub(HubError::PermissionDenied));
            }
            selected = Some(row);
        }
        match selected {
            Some(row) => self.store.open(row).await.map(Some),
            None => Ok(None),
        }
    }
    pub(crate) async fn ensure(&self) -> Result<Located, OpError> {
        if let Some(found) = self.find().await? {
            return Ok(found);
        }
        let (key, envelope) = self.store.keys.generate_collection(self.id)?;
        let sealed_name = key.seal_name(self.id, self.kind).map_err(remote)?;
        let created = self
            .store
            .ops
            .create_collection(v1::CreateCollectionRequest {
                org_id: String::new(),
                kind: self.kind.to_owned(),
                sealed_name,
                wrapped_key: envelope,
                collection_id: self.id.to_owned(),
            })
            .await;
        // Never return the locally generated key, even after success: a
        // concurrent create or lost ACK must resolve the actual stored envelope.
        match self.find().await? {
            Some(winner) => Ok(winner),
            None => {
                created?;
                Err(OpError::Hub(HubError::NotFound))
            }
        }
    }
}
