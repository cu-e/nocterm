//! Controlled lexical fact storage; native source authentication stays in adapters.
use super::super::{DecodeLedger, JsonDecodeError, ledger::Reservation, nodes::product};
use super::StringRole;
use std::mem::size_of;
#[derive(Clone, Copy)]
pub struct LiteralTextFact {
    start: usize,
    end: usize,
    decoded_bytes: usize,
    role: StringRole,
}
impl LiteralTextFact {
    pub fn start(self) -> usize {
        self.start
    }
    pub fn end(self) -> usize {
        self.end
    }
    pub fn decoded_bytes(self) -> usize {
        self.decoded_bytes
    }
    pub fn role(self) -> StringRole {
        self.role
    }
}
/// Count is a capacity request, never an authentic claim about arbitrary bytes.
/// All slots are reserved exactly once; insertion cannot grow this allocation.
pub struct LiteralTextFacts {
    facts: Vec<LiteralTextFact>,
    _backing: Reservation,
    _fixed: Reservation,
}
impl LiteralTextFacts {
    pub fn new(count: usize, ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        let fixed = ledger.reserve(size_of::<Self>())?;
        let backing = ledger.reserve(product(count, size_of::<LiteralTextFact>())?)?;
        let mut facts = Vec::new();
        facts.try_reserve_exact(count)?;
        if facts.capacity() != count {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self {
            facts,
            _backing: backing,
            _fixed: fixed,
        })
    }
    pub fn record(
        &mut self,
        start: usize,
        end: usize,
        decoded_bytes: usize,
        role: StringRole,
    ) -> Result<(), JsonDecodeError> {
        if self.facts.len() == self.facts.capacity()
            || end <= start
            || self
                .facts
                .last()
                .is_some_and(|previous| previous.end > start)
        {
            return Err(JsonDecodeError::TextPlan);
        }
        self.facts.push(LiteralTextFact {
            start,
            end,
            decoded_bytes,
            role,
        });
        Ok(())
    }
    pub fn finish(self) -> Result<Self, JsonDecodeError> {
        if self.facts.len() != self.facts.capacity() {
            return Err(JsonDecodeError::TextPlan);
        }
        Ok(self)
    }
    pub fn get(&self, index: usize) -> Option<LiteralTextFact> {
        self.facts.get(index).copied()
    }
    pub fn len(&self) -> usize {
        self.facts.len()
    }
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_fact_slots_are_reserved_once_and_drop_with_their_actual_owner() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4096).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut facts = LiteralTextFacts::new(2, &ledger).unwrap();
        let allocated = ledger.live_requested_bytes();
        facts.record(0, 3, 1, StringRole::Key).unwrap();
        facts.record(4, 12, 1, StringRole::Value).unwrap();
        let facts = facts.finish().unwrap();
        assert_eq!(ledger.live_requested_bytes(), allocated);
        assert_eq!(facts.get(1).unwrap().decoded_bytes(), 1);
        assert_eq!(facts.get(1).unwrap().end(), 12);
        drop(facts);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
    #[test]
    fn overfull_overlapping_or_incomplete_facts_cannot_grow_or_escape() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4096).unwrap();
        let mut facts = LiteralTextFacts::new(1, &ledger).unwrap();
        facts.record(0, 3, 1, StringRole::Key).unwrap();
        let allocated = ledger.live_requested_bytes();
        assert!(matches!(
            facts.record(4, 7, 1, StringRole::Value),
            Err(JsonDecodeError::TextPlan)
        ));
        assert_eq!(ledger.live_requested_bytes(), allocated);
        let incomplete = LiteralTextFacts::new(1, &ledger).unwrap();
        assert!(matches!(
            incomplete.finish(),
            Err(JsonDecodeError::TextPlan)
        ));
        let mut overlap = LiteralTextFacts::new(2, &ledger).unwrap();
        overlap.record(0, 7, 1, StringRole::Key).unwrap();
        assert!(matches!(
            overlap.record(6, 9, 1, StringRole::Value),
            Err(JsonDecodeError::TextPlan)
        ));
    }
}
