//! Original typed connection failures survive closed-session classification.
use super::registry::HeadlessSessionStatus;
use nocterm_session::{CloseReason, ConnectionFailureKind, SessionError};
#[derive(Default)]
pub(super) struct TerminalFailure {
    error: Option<SessionError>,
}
impl TerminalFailure {
    pub(super) fn observe(&mut self, reason: CloseReason) -> HeadlessSessionStatus {
        self.error = match reason {
            CloseReason::Failed(
                error @ (SessionError::Connection(_) | SessionError::Source(_)),
            ) => Some(error),
            _ => None,
        };
        if matches!(&self.error,Some(SessionError::Connection(error)) if error.kind()==ConnectionFailureKind::NeedsUnlock)
        {
            HeadlessSessionStatus::NeedsUnlock
        } else {
            HeadlessSessionStatus::Closed
        }
    }
    pub(super) fn error(&self) -> Option<SessionError> {
        self.error.clone()
    }
}
