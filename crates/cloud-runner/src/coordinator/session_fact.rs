//! Native versioned session descriptions; decoding never restores a live owner.
use super::format::Destination;
use crate::codec::identity::Run;
use nocterm_cloud_model::{
    coordinator::{
        CoordinatorError, RunDestination, SessionAttempt, SessionFailure, SessionObservation,
        SessionOutcome,
    },
    grant::GrantWindow,
    identity::{AttemptId, Fingerprint, Timestamp, WriterEpoch},
    journal::{JournalError, JournalRecord, JournalRecordKind},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Outcome {
    Established,
    Lost(Failure),
    Refused(Failure),
}
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Failure {
    Denied,
    Expired,
    Cancelled,
    SourceChanged,
    Unavailable,
    Unsupported,
    Capacity,
    ConnectionLost,
}
impl From<SessionFailure> for Failure {
    fn from(value: SessionFailure) -> Self {
        match value {
            SessionFailure::Denied => Self::Denied,
            SessionFailure::Expired => Self::Expired,
            SessionFailure::Cancelled => Self::Cancelled,
            SessionFailure::SourceChanged => Self::SourceChanged,
            SessionFailure::Unavailable => Self::Unavailable,
            SessionFailure::Unsupported => Self::Unsupported,
            SessionFailure::Capacity => Self::Capacity,
            SessionFailure::ConnectionLost => Self::ConnectionLost,
        }
    }
}
impl From<Failure> for SessionFailure {
    fn from(value: Failure) -> Self {
        match value {
            Failure::Denied => Self::Denied,
            Failure::Expired => Self::Expired,
            Failure::Cancelled => Self::Cancelled,
            Failure::SourceChanged => Self::SourceChanged,
            Failure::Unavailable => Self::Unavailable,
            Failure::Unsupported => Self::Unsupported,
            Failure::Capacity => Self::Capacity,
            Failure::ConnectionLost => Self::ConnectionLost,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionFact {
    writer: u64,
    run: Run,
    destination: Destination,
    attempt: [u8; 16],
    nonce: [u8; 32],
    issued: u64,
    expires: u64,
    outcome: Outcome,
}
impl SessionFact {
    /// The caller reserves native work before cloning these bounded coordinates.
    pub(super) fn capture(
        value: &SessionObservation,
        writer: WriterEpoch,
    ) -> Result<Self, CoordinatorError> {
        let binding = value.binding();
        let (receipt, outcome) = match value.outcome() {
            SessionOutcome::Established(receipt) => (Some(receipt), Outcome::Established),
            SessionOutcome::Lost { session, reason } => {
                (Some(session), Outcome::Lost((*reason).into()))
            }
            SessionOutcome::Refused(reason) => (None, Outcome::Refused((*reason).into())),
        };
        let destination = RunDestination::new(
            binding.target().clone(),
            binding.authentication().clone(),
            receipt.cloned(),
        )?;
        Ok(Self {
            writer: writer.get(),
            run: binding.run().into(),
            destination: (&destination).into(),
            attempt: *binding.attempt().as_bytes(),
            nonce: *binding.nonce().as_bytes(),
            issued: binding.window().issued().as_millis(),
            expires: binding.window().expires().as_millis(),
            outcome,
        })
    }
    pub(super) fn writer(&self) -> Result<WriterEpoch, CoordinatorError> {
        WriterEpoch::new(self.writer).map_err(CoordinatorError::from)
    }
    pub(super) fn restore(self) -> Result<SessionObservation, CoordinatorError> {
        self.writer()?;
        let destination = self.destination.restore()?;
        let binding = SessionAttempt::new(
            self.run.restore()?,
            destination.target().clone(),
            destination.authentication().clone(),
            AttemptId::from_bytes(self.attempt),
            Fingerprint::from_bytes(self.nonce),
            GrantWindow::new(
                Timestamp::from_millis(self.issued),
                Timestamp::from_millis(self.expires),
            )?,
        )?;
        let outcome = match (self.outcome, destination.session()) {
            (Outcome::Established, Some(session)) => SessionOutcome::Established(session.clone()),
            (Outcome::Lost(reason), Some(session)) => SessionOutcome::Lost {
                session: session.clone(),
                reason: reason.into(),
            },
            (Outcome::Refused(reason), None) => SessionOutcome::Refused(reason.into()),
            _ => return Err(JournalError::Corrupt.into()),
        };
        SessionObservation::new(binding, outcome)
    }
}

/// Narrow native-reader bridge; no public wire codec or live capability escapes.
pub(crate) fn decode_record(
    record: &JournalRecord,
) -> Result<(WriterEpoch, SessionObservation), JournalError> {
    if !matches!(record.kind(), JournalRecordKind::RunStatus)
        || record.payload().len() > nocterm_cloud_model::journal::MAX_SESSION_FACT_BYTES
    {
        return Err(JournalError::InvalidBatch);
    }
    let super::frame::Frame::Session(fact) =
        super::frame::Frame::read(record.payload()).map_err(journal_error)?
    else {
        return Err(JournalError::Conflict);
    };
    let writer = fact.writer().map_err(journal_error)?;
    let observation = fact.restore().map_err(journal_error)?;
    if record.context().run() != Some(observation.binding().run()) {
        return Err(JournalError::Corrupt);
    }
    Ok((writer, observation))
}
fn journal_error(error: CoordinatorError) -> JournalError {
    match error {
        CoordinatorError::Journal(error) => error,
        error => JournalError::Storage(nocterm_core::SharedCause::new(error)),
    }
}
