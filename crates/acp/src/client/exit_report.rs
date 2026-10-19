//! The bounded agent event queue and its terminal exit report.
use nocterm_ai::AgentEvent;

use super::remote_errors::scrub;
use super::stderr_tail::StderrTail;

/// Queue shared by protocol handlers, command acknowledgements and the exit report.
#[derive(Clone)]
pub(super) struct EventQueue {
    sender: async_channel::Sender<AgentEvent>,
    receiver: async_channel::Receiver<AgentEvent>,
}
impl EventQueue {
    pub(super) fn new(capacity: usize) -> Self {
        let (sender, receiver) = async_channel::bounded(capacity);
        Self { sender, receiver }
    }
    pub(super) fn sender(&self) -> async_channel::Sender<AgentEvent> {
        self.sender.clone()
    }
    pub(super) fn receiver(&self) -> async_channel::Receiver<AgentEvent> {
        self.receiver.clone()
    }
    /// Publishes the final event, replacing the oldest one if the queue is full, then closes it.
    pub(super) fn finish(&self, report: ExitReport) {
        let event = AgentEvent::Exited {
            code: report.code,
            stderr_tail: report.stderr_tail,
        };
        if let Err(async_channel::TrySendError::Full(event)) = self.sender.try_send(event) {
            let _ = self.receiver.try_recv();
            let _ = self.sender.try_send(event);
        }
        self.sender.close();
    }
}

/// How the agent process ended, with diagnostics already scrubbed.
pub(super) struct ExitReport {
    code: Option<i32>,
    stderr_tail: String,
}
impl ExitReport {
    /// Stderr wins; otherwise the protocol failure explains the exit.
    pub(super) fn new(
        code: Option<i32>,
        tail: &StderrTail,
        secrets: &[String],
        failure: Option<String>,
    ) -> Self {
        let mut stderr_tail = tail.scrubbed(secrets);
        if stderr_tail.is_empty() {
            stderr_tail = failure.map_or_else(String::new, |failure| scrub(&failure, secrets));
        }
        Self { code, stderr_tail }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::oneshot;

    fn barrier() -> AgentEvent {
        AgentEvent::Barrier(oneshot::channel().0)
    }

    #[test]
    fn a_full_queue_gives_up_its_oldest_event_for_the_exit_report() {
        let queue = EventQueue::new(2);
        let events = queue.receiver();
        queue.sender().try_send(barrier()).unwrap();
        queue
            .sender()
            .try_send(AgentEvent::Exited {
                code: Some(9),
                stderr_tail: "older".into(),
            })
            .unwrap();
        queue.finish(ExitReport::new(
            Some(3),
            &StderrTail::default(),
            &[],
            Some("boom".into()),
        ));
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            seen.push(event);
        }
        assert_eq!(seen.len(), 2);
        assert!(matches!(seen[0], AgentEvent::Exited { code: Some(9), .. }));
        assert!(matches!(
            &seen[1],
            AgentEvent::Exited { code: Some(3), stderr_tail } if stderr_tail == "boom"
        ));
        assert!(events.is_closed());
    }

    #[test]
    fn the_failure_text_is_scrubbed_when_stderr_is_silent() {
        let report = ExitReport::new(
            None,
            &StderrTail::default(),
            &["swordfish".into()],
            Some("denied swordfish".into()),
        );
        assert_eq!(report.stderr_tail, "denied [REDACTED]");
    }
}
