//! The small status an agent row shows, only when it tells the person something.

use nocterm_cloud_agents::StartPhase;
use nocterm_cloud_agents_model::AgentStatus;

/// What the row says beside the agent's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RowStatus {
    text: &'static str,
    /// Whether the text asks for the person's attention.
    attention: bool,
}

impl RowStatus {
    /// What the row says.
    pub(super) fn text(self) -> &'static str {
        self.text
    }

    /// Whether the text asks for the person's attention.
    pub(super) fn attention(self) -> bool {
        self.attention
    }
}

const fn plain(text: &'static str) -> Option<RowStatus> {
    Some(RowStatus {
        text,
        attention: false,
    })
}

const fn attention(text: &'static str) -> Option<RowStatus> {
    Some(RowStatus {
        text,
        attention: true,
    })
}

/// The status of a row: the progress of its start while one runs, else what the hub says of
/// the agent. A seat the person already has needs no capacity, so it is never "No capacity".
pub(super) fn row_status(
    status: AgentStatus,
    seated: bool,
    start: Option<&StartPhase>,
) -> Option<RowStatus> {
    match start {
        Some(StartPhase::Step(step)) => return plain(step.text()),
        Some(StartPhase::AwaitingTrust(_)) => return attention("Trust the pool?"),
        Some(StartPhase::Failed(_)) => return attention("Failed"),
        Some(StartPhase::Ready(_)) | None => {}
    }
    match status {
        AgentStatus::Ready => None,
        AgentStatus::Sleeping => plain("Asleep"),
        AgentStatus::Starting => plain("Starting…"),
        AgentStatus::NoCapacity if seated => plain("Asleep"),
        other => attention(other.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_agents_model::StartStep;

    #[test]
    fn a_ready_agent_shows_no_status() {
        assert_eq!(row_status(AgentStatus::Ready, false, None), None);
    }

    #[test]
    fn an_agent_the_person_has_a_seat_of_is_not_out_of_capacity() {
        let status = row_status(AgentStatus::NoCapacity, true, None).unwrap();
        assert_eq!(status.text, "Asleep");
        assert!(!status.attention);
        let status = row_status(AgentStatus::NoCapacity, false, None).unwrap();
        assert_eq!(status.text, "No capacity");
        assert!(status.attention);
    }

    #[test]
    fn a_start_under_way_shows_its_step() {
        let step = StartPhase::Step(StartStep::Registering);
        let status = row_status(AgentStatus::Ready, false, Some(&step)).unwrap();
        assert_eq!(status.text, StartStep::Registering.text());
    }
}
