//! Native account routing over the shared MemoryHub storage implementation.
use super::MemoryHub;
use crate::{
    account::{
        operations::OperationStores,
        source::{CapturedAccount, CapturedMembership},
    },
    store::StoreOps,
};
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};
use std::{collections::HashMap, sync::Arc};

pub(crate) struct MemoryAccountStores(HashMap<String, Arc<MemoryHub>>);
impl MemoryAccountStores {
    pub(crate) fn new(accounts: impl IntoIterator<Item = (String, Arc<MemoryHub>)>) -> Self {
        Self(accounts.into_iter().collect())
    }
    fn account(&self, id: &str) -> Result<Arc<dyn StoreOps>, HubError> {
        self.0
            .get(id)
            .cloned()
            .map(|hub| hub as Arc<dyn StoreOps>)
            .ok_or(HubError::Stale)
    }
}
impl OperationStores for MemoryAccountStores {
    fn opened(
        &self,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn StoreOps>, HubError> {
        source.check_current().map_err(|_| HubError::Stale)?;
        self.account(native.subject()?.account())
    }
    fn membership(&self, native: &CapturedMembership) -> Result<Arc<dyn StoreOps>, HubError> {
        native.check()?;
        self.account(native.subject()?.account())
    }
}
