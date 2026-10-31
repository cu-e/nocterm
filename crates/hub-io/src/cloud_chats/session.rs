//! The account's Cloud owners a chat operation works through: hub trust, chat material and the
//! runner registry, captured together from one signed-in membership.

use std::sync::Arc;

use tokio::runtime::Handle;

use super::budget::ChatBudget;
use super::error::{self, CloudChatError};
use crate::cloud::{
    AccountRunnerRegistry, CloudAccountMaterial, RunnerRegistration, TrustedRunnerRoster,
};
use crate::hub_account::HubAccount;

/// What one operation learned from the account before touching a chat.
pub(super) struct OpenedAccount {
    material: Arc<CloudAccountMaterial>,
    roster: TrustedRunnerRoster,
}

impl OpenedAccount {
    pub(super) async fn open(
        account: &HubAccount,
        handle: &Handle,
        budget: &ChatBudget,
    ) -> Result<Self, CloudChatError> {
        let source = account.local_source().map_err(error::hub)?;
        let trust = account
            .cloud_trust(
                source.clone(),
                budget.resident(crate::cloud::CLOUD_TRUST_RESIDENT_BYTES)?,
            )
            .map_err(error::trust)?;
        let trusted = trust.load(budget.trust()?).await.map_err(error::trust)?;
        let material = account
            .cloud_material(source, trusted.authority(), budget.material()?)
            .await
            .map_err(error::material)?;
        let registry = AccountRunnerRegistry::capture(
            &material,
            trusted.scope().clone(),
            budget.resident(crate::cloud::RUNNER_REGISTRY_RESIDENT_BYTES)?,
            handle.clone(),
        )
        .map_err(error::registry)?;
        let roster = registry
            .load(budget.registry()?)
            .await
            .map_err(error::registry)?;
        Ok(Self {
            material: Arc::new(material),
            roster,
        })
    }

    pub(super) fn material(&self) -> &Arc<CloudAccountMaterial> {
        &self.material
    }

    pub(super) fn runners(&self) -> Result<&[RunnerRegistration], CloudChatError> {
        self.roster.runners().map_err(error::registry)
    }

    /// The registered runner with this hub identifier, or [`CloudChatError::NotEnrolled`].
    pub(super) fn runner(&self, id: &[u8; 16]) -> Result<&RunnerRegistration, CloudChatError> {
        self.runners()?
            .iter()
            .find(|row| row.binding().runner().as_bytes() == id)
            .ok_or(CloudChatError::NotEnrolled)
    }
}
