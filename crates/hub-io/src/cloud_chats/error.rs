//! Mapping of the adapter's typed failures to the port error, keeping each original cause.

use nocterm_cloud_device::CloudDeviceError;
use nocterm_cloud_device::flow::EffectFailure;
use nocterm_core::error::SharedCause;
use nocterm_hub_client::agents::AgentError;
use nocterm_hubs_model::{HubAddress, HubError};

use crate::agents::agent_error;
use crate::cloud_device::DeviceAccountError;

pub(super) use nocterm_cloud_agents_model::CloudChatError;

pub(super) fn hub(error: HubError) -> CloudChatError {
    match error {
        HubError::Stale => CloudChatError::Stale,
        other => CloudChatError::Hub(other),
    }
}

pub(super) fn account(error: DeviceAccountError) -> CloudChatError {
    match error {
        DeviceAccountError::Hub(error) => hub(error),
        DeviceAccountError::NotSetUp => CloudChatError::NotEnrolled,
        DeviceAccountError::Unsupported => CloudChatError::Unsupported,
        DeviceAccountError::Device(error) => device(error),
    }
}

/// A refusal of a device core flow.
pub(super) fn device(error: CloudDeviceError) -> CloudChatError {
    match error {
        CloudDeviceError::NoRunner | CloudDeviceError::NotEnrolled => CloudChatError::NotEnrolled,
        CloudDeviceError::NotClaimed => CloudChatError::RunnerNotReady,
        CloudDeviceError::Unsupported => CloudChatError::Unsupported,
        CloudDeviceError::Capacity => CloudChatError::Capacity,
        CloudDeviceError::NoProviderKey => CloudChatError::NoProviderKey,
        // A seat is started only when the agent has an enrolled one (the effect answers
        // `NotFound` otherwise): starting the agent enrols it.
        CloudDeviceError::Effect(EffectFailure::NotFound) => CloudChatError::AgentNotStarted,
        CloudDeviceError::Effect(EffectFailure::Unavailable | EffectFailure::Unknown) => {
            CloudChatError::RunnerNotReady
        }
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
            account(DeviceAccountError::NotSetUp),
            CloudChatError::NotEnrolled
        ));
        assert!(matches!(
            account(DeviceAccountError::Hub(HubError::Stale)),
            CloudChatError::Stale
        ));
    }

    #[test]
    fn a_runner_that_did_not_take_the_chat_asks_the_person_to_try_again() {
        assert!(device(CloudDeviceError::NotClaimed).is_retryable());
        assert!(matches!(
            device(CloudDeviceError::Effect(EffectFailure::NotFound)),
            CloudChatError::AgentNotStarted
        ));
    }

    #[test]
    fn an_unexpected_failure_keeps_its_typed_cause() {
        let failure = device(CloudDeviceError::Rollback);
        let source = std::error::Error::source(&failure).expect("cause");
        assert!(source.to_string().contains("behind"));
    }
}
