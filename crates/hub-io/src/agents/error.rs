//! Mapping of contract failures to the Cloud agents port error.

use nocterm_cloud_agents_model::{CloudAgentError, Refusal, SetupProblem};
use nocterm_core::error::SharedCause;
use nocterm_hub_client::agents::{AgentError, SetupError};
use nocterm_hub_model::agents::RefusalReason;
use nocterm_hubs_model::{HubAddress, HubError};

use crate::classify::{classify, remote};
use crate::cloud::EnrolmentError;

pub(super) fn hub_error(error: HubError) -> CloudAgentError {
    match error {
        HubError::Stale => CloudAgentError::Stale,
        HubError::NotSignedIn => CloudAgentError::NotSignedIn,
        other => CloudAgentError::Hub(other),
    }
}

pub(super) fn agent_error(address: &HubAddress, error: AgentError) -> CloudAgentError {
    match error {
        AgentError::Unsupported => CloudAgentError::Unsupported,
        AgentError::Conflict { .. } => CloudAgentError::Conflict,
        AgentError::Refused(reason) => CloudAgentError::Refused(match reason {
            RefusalReason::ScopeQuota { .. } => Refusal::ScopeQuota,
            RefusalReason::MemberQuota => Refusal::MemberQuota,
            RefusalReason::Budget => Refusal::Budget,
            _ => Refusal::HostCapacity,
        }),
        AgentError::Hub(client) => hub_error(classify(address, client)),
        other => CloudAgentError::Hub(remote(other)),
    }
}

pub(super) fn setup_problem(error: &SetupError) -> SetupProblem {
    match error {
        SetupError::Missing => SetupProblem::Missing,
        SetupError::DocumentMismatch => SetupProblem::Substituted,
        _ => SetupProblem::NotAttested,
    }
}

pub(super) fn enrolment_error(error: EnrolmentError) -> CloudAgentError {
    match error {
        EnrolmentError::Account(HubError::Stale) => CloudAgentError::Stale,
        EnrolmentError::Account(HubError::NotSignedIn) => CloudAgentError::NotSignedIn,
        other => CloudAgentError::Registration(SharedCause::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use nocterm_hub_model::agents::QuotaScope;

    use super::*;

    fn address() -> HubAddress {
        HubAddress::parse("hub.example.org").expect("address")
    }

    #[test]
    fn an_unimplemented_hub_is_unsupported_and_refusals_keep_their_reason() {
        assert!(matches!(
            agent_error(&address(), AgentError::Unsupported),
            CloudAgentError::Unsupported
        ));
        let quota = AgentError::Refused(RefusalReason::ScopeQuota {
            scope: QuotaScope::Instance,
        });
        assert!(matches!(
            agent_error(&address(), quota),
            CloudAgentError::Refused(Refusal::ScopeQuota)
        ));
        assert!(matches!(
            agent_error(&address(), AgentError::Refused(RefusalReason::Budget)),
            CloudAgentError::Refused(Refusal::Budget)
        ));
    }

    #[test]
    fn an_ended_membership_is_stale_not_a_hub_failure() {
        assert!(matches!(hub_error(HubError::Stale), CloudAgentError::Stale));
    }
}
