//! Reading the state of a seat the hub reports while it is being prepared.

use nocterm_cloud_agents_model::{CloudAgentError, InstanceId, ReplyProblem};
use nocterm_hub_proto::{instance_failure_from_wire, v1};

/// What the start does next with a seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Progress {
    /// The host is still preparing the seat: ask again.
    Wait,
    /// The seat waits for the runner to enrol.
    Enrol,
    /// The seat is usable as it is.
    Ready,
}

/// Where the seat is, or why it cannot be used.
pub(super) fn progress(instance: &v1::Instance) -> Result<Progress, CloudAgentError> {
    match v1::InstanceState::try_from(instance.state) {
        Ok(v1::InstanceState::AwaitingEnrolment) => Ok(Progress::Enrol),
        Ok(v1::InstanceState::Ready | v1::InstanceState::Suspended) => Ok(Progress::Ready),
        Ok(v1::InstanceState::Provisioning) => Ok(Progress::Wait),
        Ok(v1::InstanceState::Deleting) => Err(CloudAgentError::SeatBeingDeleted),
        Ok(v1::InstanceState::Failed) => Err(failed(instance)),
        Ok(v1::InstanceState::Unspecified) | Err(_) => Err(CloudAgentError::UnexpectedReply(
            ReplyProblem::InstanceState(instance.state),
        )),
    }
}

/// The failure of a seat. The single place that reads why the hub gave up: the operator-facing
/// description of the code the hub stored, when it stored one.
fn failed(instance: &v1::Instance) -> CloudAgentError {
    match InstanceId::new(instance.instance_id.clone()) {
        Ok(id) => CloudAgentError::ProvisioningFailed {
            instance: id,
            reason: instance_failure_from_wire(instance).map(|why| why.describe().to_owned()),
        },
        Err(_) => CloudAgentError::UnexpectedReply(ReplyProblem::InstanceId),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_hub_model::agents::ProvisionFailure;

    fn seat(state: i32) -> v1::Instance {
        v1::Instance {
            instance_id: "inst-1".into(),
            state,
            ..Default::default()
        }
    }

    #[test]
    fn a_failed_seat_names_its_instance() {
        let error = progress(&seat(v1::InstanceState::Failed as i32)).unwrap_err();
        assert!(matches!(
            error,
            CloudAgentError::ProvisioningFailed { instance, reason: None }
                if instance.as_str() == "inst-1"
        ));
    }

    #[test]
    fn a_failed_seat_states_who_must_act_when_the_hub_stored_a_code() {
        let mut failed = seat(v1::InstanceState::Failed as i32);
        failed.failure_code = "account_unpinned".into();
        let CloudAgentError::ProvisioningFailed { reason, .. } = progress(&failed).unwrap_err()
        else {
            panic!("expected a provisioning failure");
        };
        let expected = ProvisionFailure::AccountUnpinned.describe();
        assert_eq!(reason.as_deref(), Some(expected));
    }

    #[test]
    fn a_seat_being_deleted_is_transient() {
        let error = progress(&seat(v1::InstanceState::Deleting as i32)).unwrap_err();
        assert!(matches!(error, CloudAgentError::SeatBeingDeleted));
    }

    #[test]
    fn an_unknown_state_is_an_unexpected_reply() {
        let error = progress(&seat(99)).unwrap_err();
        assert!(matches!(
            error,
            CloudAgentError::UnexpectedReply(ReplyProblem::InstanceState(99))
        ));
    }

    #[test]
    fn preparing_and_usable_seats_progress() {
        assert_eq!(
            progress(&seat(v1::InstanceState::Provisioning as i32)).unwrap(),
            Progress::Wait
        );
        assert_eq!(
            progress(&seat(v1::InstanceState::AwaitingEnrolment as i32)).unwrap(),
            Progress::Enrol
        );
        assert_eq!(
            progress(&seat(v1::InstanceState::Suspended as i32)).unwrap(),
            Progress::Ready
        );
    }
}
