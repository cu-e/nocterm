//! One admitted material operation. Borrowed async work cannot outlive its owner.
use super::*;
use crate::account::operations::OpenedWork;

pub(super) struct MaterialOperation {
    work: OpenedWork,
    request: ChatMaterialRequest,
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
    resident: AllocationLease,
}
impl MaterialOperation {
    pub(super) fn new(
        work: OpenedWork,
        request: ChatMaterialRequest,
        bound: BoundOperations,
        cloud: Arc<CloudClient>,
        resident: AllocationLease,
    ) -> Self {
        Self {
            work,
            request,
            bound,
            cloud,
            resident,
        }
    }
    pub(super) async fn execute(
        &self,
        create: bool,
    ) -> Result<AccountChatMaterial, AccountMaterialError> {
        let record = storage::load_or_create(&self.work, self.request.clone(), create).await?;
        self.work.source().check()?;
        self.cloud.check_source().map_err(cause)?;
        let material = AccountChatMaterial {
            resident: Allocated::new(
                MaterialOwner {
                    record,
                    bound: self.bound.clone(),
                    cloud: self.cloud.clone(),
                },
                self.resident.clone(),
            ),
        };
        material.with_key(|_| Ok(()))?;
        self.work
            .source()
            .publish(|| material)
            .map_err(AccountMaterialError::from)
    }
}
