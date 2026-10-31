//! Mapping of the adapter's typed failures to the port error, keeping each original cause.

use nocterm_core::error::SharedCause;
use nocterm_hub_client::agents::AgentError;
use nocterm_hubs_model::{HubAddress, HubError};

use crate::agents::agent_error;
use crate::cloud::{
    AccountMaterialError, CloudTrustError, ProviderBootError, RunnerCatalogError,
    RunnerRegistryError,
};

pub(super) use nocterm_cloud_agents_model::CloudChatError;

pub(super) fn hub(error: HubError) -> CloudChatError {
    match error {
        HubError::Stale => CloudChatError::Stale,
        other => CloudChatError::Hub(other),
    }
}

pub(super) fn material(error: AccountMaterialError) -> CloudChatError {
    match error {
        AccountMaterialError::Stale | AccountMaterialError::Account(HubError::Stale) => {
            CloudChatError::Stale
        }
        AccountMaterialError::Unsupported => CloudChatError::Unsupported,
        AccountMaterialError::Capacity => CloudChatError::Capacity,
        AccountMaterialError::Account(error) => CloudChatError::Hub(error),
        other => CloudChatError::Backend(SharedCause::new(other)),
    }
}

pub(super) fn trust(error: CloudTrustError) -> CloudChatError {
    match error {
        CloudTrustError::NotConfigured => CloudChatError::NotEnrolled,
        CloudTrustError::Stale => CloudChatError::Stale,
        CloudTrustError::Unsupported => CloudChatError::Unsupported,
        CloudTrustError::Capacity => CloudChatError::Capacity,
        CloudTrustError::Storage(error) => material(error),
        other => CloudChatError::Backend(SharedCause::new(other)),
    }
}

pub(super) fn registry(error: RunnerRegistryError) -> CloudChatError {
    match error {
        RunnerRegistryError::Stale => CloudChatError::Stale,
        RunnerRegistryError::Capacity => CloudChatError::Capacity,
        RunnerRegistryError::Storage(error) => material(error),
        other => CloudChatError::Backend(SharedCause::new(other)),
    }
}

pub(super) fn catalog(error: RunnerCatalogError) -> CloudChatError {
    match error {
        RunnerCatalogError::Unavailable => CloudChatError::RunnerNotReady,
        RunnerCatalogError::Capacity => CloudChatError::Capacity,
        RunnerCatalogError::Unsupported(_) => CloudChatError::Unsupported,
        RunnerCatalogError::Backend(error) => material(error),
        RunnerCatalogError::Registry(error) => registry(error),
        other => CloudChatError::Backend(SharedCause::new(other)),
    }
}

pub(super) fn boot(error: ProviderBootError) -> CloudChatError {
    match error {
        ProviderBootError::Capacity => CloudChatError::Capacity,
        ProviderBootError::Stale => CloudChatError::Stale,
        // A boot that ended, or a clock that moved, is replaced by the runner's next one.
        ProviderBootError::Expired | ProviderBootError::Clock => CloudChatError::RunnerNotReady,
        ProviderBootError::Storage(error) => material(error),
        other => CloudChatError::Backend(SharedCause::new(other)),
    }
}

pub(super) fn agent(address: &HubAddress, error: AgentError) -> CloudChatError {
    CloudChatError::Agent(agent_error(address, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_without_cloud_trust_is_not_enrolled() {
        assert!(matches!(
            trust(CloudTrustError::NotConfigured),
            CloudChatError::NotEnrolled
        ));
    }

    #[test]
    fn a_replaced_account_is_stale_wherever_it_is_noticed() {
        assert!(matches!(
            material(AccountMaterialError::Account(HubError::Stale)),
            CloudChatError::Stale
        ));
        assert!(matches!(
            registry(RunnerRegistryError::Stale),
            CloudChatError::Stale
        ));
        assert!(matches!(
            trust(CloudTrustError::Storage(AccountMaterialError::Stale)),
            CloudChatError::Stale
        ));
    }

    #[test]
    fn a_boot_that_ended_asks_the_person_to_try_again() {
        assert!(boot(ProviderBootError::Expired).is_retryable());
        assert!(matches!(
            boot(ProviderBootError::Stale),
            CloudChatError::Stale
        ));
    }

    #[test]
    fn a_full_budget_asks_the_person_to_retry_later() {
        assert!(material(AccountMaterialError::Capacity).is_retryable());
    }

    #[test]
    fn an_unexpected_failure_keeps_its_typed_cause() {
        let failure = registry(RunnerRegistryError::Format);
        let source = std::error::Error::source(&failure).expect("cause");
        assert!(source.to_string().contains("registry"));
    }
}
