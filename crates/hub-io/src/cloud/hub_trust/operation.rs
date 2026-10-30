//! Strong keys remain inside this one admitted operation, not idle trust results.
use super::*;
use crate::account::operations::OpenedWork;
pub(super) struct TrustOperation {
    work: OpenedWork,
    bound: BoundOperations,
    client: Arc<CapturedClient>,
    normal: codec::NormalAccount,
    setup: Option<CloudRunnerSetup>,
    resident: AllocationLease,
    allocation: AllocationLease,
}
impl TrustOperation {
    pub(super) fn new(
        work: OpenedWork,
        bound: BoundOperations,
        client: Arc<CapturedClient>,
        normal: codec::NormalAccount,
        setup: Option<CloudRunnerSetup>,
        resident: AllocationLease,
        allocation: AllocationLease,
    ) -> Self {
        Self {
            work,
            bound,
            client,
            normal,
            setup,
            resident,
            allocation,
        }
    }
    pub(super) async fn execute(&self) -> Result<TrustedCloudHub, CloudTrustError> {
        self.client.check_cloud().map_err(account)?;
        if let Some(setup) = &self.setup {
            self.normal.require_setup(setup)?;
            self.verify_candidate(setup).await?;
        }
        let authority = storage::load_or_register(
            &self.work,
            &self.normal.context(),
            self.setup.as_ref().map(|setup| setup.authority),
        )
        .await?;
        self.client.check_cloud().map_err(account)?;
        let result = TrustedCloudHub {
            data: Allocated::new(
                HubPinData {
                    authority,
                    scope: self.normal.scope(authority)?,
                    bound: self.bound.clone(),
                    client: self.client.clone(),
                },
                self.resident.clone(),
            ),
        };
        self.work.source().publish(|| result).map_err(account)
    }
    async fn verify_candidate(&self, setup: &CloudRunnerSetup) -> Result<(), CloudTrustError> {
        let sdk = self.client.captured_cloud().map_err(account)?;
        let offer = self
            .client
            .during_cloud(sdk.cloud_offer_with_residency(Arc::new(self.allocation.clone())))
            .await
            .map_err(account)?
            .map_err(backend)?;
        let cloud = offer
            .accept_with_residency(&setup.authority, Arc::new(self.resident.clone()))
            .map_err(backend)?;
        self.client.check_cloud().map_err(account)?;
        let scope = self.normal.scope(setup.authority)?;
        let subject = self.bound.source().native().subject().map_err(account)?;
        if cloud.namespace().hub_instance_id.as_str() != scope.hub().instance().as_str()
            || cloud.namespace().account_id.as_str() != scope.account().id().as_str()
            || cloud.subject().account().as_str() != subject.account()
            || cloud.subject().device().as_str() != subject.device()
        {
            return Err(CloudTrustError::Binding);
        }
        if cloud.limits().features
            & nocterm_hub_model::cloud::capabilities::CLOUD_FEATURE_CHAT_KEY_MATERIAL
            == 0
        {
            return Err(CloudTrustError::Unsupported);
        }
        self.bound.source().check().map_err(account)
    }
}
