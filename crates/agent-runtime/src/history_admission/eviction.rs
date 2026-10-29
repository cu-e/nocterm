//! One weak participant's offered history or immutable-payload reclamation.
use super::{HistoryUsage, RegisteredHistory};
use gpui::App;
use nocterm_ai::execution_origin::ChatKey;
use std::rc::Rc;

enum EvictionKind {
    History,
    Payload,
}
pub(super) struct EvictionCandidate {
    participant: Rc<RegisteredHistory>,
    age: u64,
    kind: EvictionKind,
}
impl EvictionCandidate {
    pub(super) fn new(
        participant: Rc<RegisteredHistory>,
        usage: Option<HistoryUsage>,
        cx: &App,
    ) -> Option<Self> {
        if let Some(usage) = usage {
            if usage.shown() {
                return None;
            }
            if usage.bytes() > 0 {
                return Some(Self {
                    participant,
                    age: usage.updated(),
                    kind: EvictionKind::History,
                });
            }
        }
        let age = participant.participant.oldest_payload(cx)?;
        Some(Self {
            participant,
            age,
            kind: EvictionKind::Payload,
        })
    }
    // Reclaim cold metadata before hidden transcript history. Payload ordinals
    // and history timestamps are compared only within their own class.
    pub(super) fn priority(&self) -> (bool, u64) {
        (matches!(self.kind, EvictionKind::History), self.age)
    }
    pub(super) fn identity(&self) -> (*const RegisteredHistory, (bool, u64)) {
        (Rc::as_ptr(&self.participant), self.priority())
    }
    pub(super) fn evict(&self, cx: &mut App) -> Option<ChatKey> {
        match self.kind {
            EvictionKind::History => {
                let key = self.participant.participant.local_key(cx);
                self.participant
                    .participant
                    .evict(cx)
                    .then_some(key)
                    .flatten()
            }
            EvictionKind::Payload => {
                self.participant.participant.evict_payload(cx);
                None
            }
        }
    }
}
