use super::{
    MAX_REPLAY_BYTES, MAX_REPLAY_EVENTS, ProjectionBinding, ProjectionCursor, ProjectionError,
};
use crate::{
    coordinator::CoordinatorEvent,
    identity::{ChatControlRevision, EventId, Fingerprint, Timestamp},
};

/// Semantic data supplied ONLY by the injected verifier after authenticating the
/// original source, event signature, ciphertext and canonical chain coordinates.
/// This shape constructor itself performs no cryptographic verification.
#[derive(Clone)]
pub struct ProjectionEvent {
    pub binding: ProjectionBinding,
    pub id: EventId,
    pub before: ProjectionCursor,
    pub head: Fingerprint,
    pub control: ChatControlRevision,
    pub time: Timestamp,
    pub event: CoordinatorEvent,
}
impl ProjectionEvent {
    pub fn after(&self) -> Result<ProjectionCursor, ProjectionError> {
        Ok(ProjectionCursor::at(
            self.before
                .sequence()
                .advance(1)
                .map_err(|_| ProjectionError::Capacity)?,
            self.head,
        ))
    }
}
/// A bounded verified page. Known head is retained by the adapter across reconnects;
/// an empty page may not erase it or advertise a shorter history.
#[derive(Clone)]
pub struct ProjectionBatch {
    binding: ProjectionBinding,
    events: Vec<ProjectionEvent>,
    known_head: ProjectionCursor,
    bytes: usize,
}
impl ProjectionBatch {
    pub fn new(
        binding: ProjectionBinding,
        events: Vec<ProjectionEvent>,
        known_head: ProjectionCursor,
    ) -> Result<Self, ProjectionError> {
        if events.len() > MAX_REPLAY_EVENTS {
            return Err(ProjectionError::Capacity);
        }
        let bytes = events.iter().try_fold(1024usize, |total, envelope| {
            let bytes = total.saturating_add(super::sizing::event_bytes(&envelope.event)?);
            if bytes > MAX_REPLAY_BYTES {
                return Err(ProjectionError::Capacity);
            }
            Ok(bytes)
        })?;
        Ok(Self {
            binding,
            events,
            known_head,
            bytes,
        })
    }
    pub fn binding(&self) -> &ProjectionBinding {
        &self.binding
    }
    pub fn events(&self) -> &[ProjectionEvent] {
        &self.events
    }
    pub fn known_head(&self) -> ProjectionCursor {
        self.known_head
    }
    pub fn resident_bytes(&self) -> usize {
        self.bytes
    }
}
