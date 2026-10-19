//! Semantic keys keep first ordinal and last genuine value span, never total occurrences.
use super::{ValueError, source::SemanticSpan};
use crate::loading::json_frames::{ObjectFrames, RootFrames};
use nocterm_ai::model::json::deferred::{
    DecodeLedger, JsonDecodeError, NativeScratch, literal::LiteralText,
};
use std::{borrow::Borrow, cmp::Ordering, collections::BTreeMap, io::Read, mem::size_of};
struct GuardedKey(LiteralText);
impl Borrow<str> for GuardedKey {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}
impl PartialEq for GuardedKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}
impl Eq for GuardedKey {}
impl PartialOrd for GuardedKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for GuardedKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.as_str().cmp(other.0.as_str())
    }
}
struct LastOccurrence {
    first: usize,
    value: SemanticSpan,
}
pub(super) struct MemberPlan {
    key: LiteralText,
    value: SemanticSpan,
    first: usize,
}
impl MemberPlan {
    pub(super) fn into_parts(self) -> (LiteralText, SemanticSpan) {
        (self.key, self.value)
    }
}
struct MapIndex<'a> {
    members: BTreeMap<GuardedKey, LastOccurrence>,
    backing: NativeScratch,
    ledger: &'a DecodeLedger,
    _fixed: NativeScratch,
}
impl<'a> MapIndex<'a> {
    fn new(ledger: &'a DecodeLedger) -> Result<Self, ValueError> {
        Ok(Self {
            members: BTreeMap::new(),
            backing: NativeScratch::reserve(ledger, 0)?,
            ledger,
            _fixed: NativeScratch::reserve(ledger, size_of::<Self>())?,
        })
    }
    fn record(
        &mut self,
        key: LiteralText,
        value: SemanticSpan,
        ordinal: usize,
    ) -> Result<(), ValueError> {
        let key = GuardedKey(key);
        if let Some(existing) = self.members.get_mut(key.0.as_str()) {
            existing.value = value;
            return Ok(());
        }
        let entries = self
            .members
            .len()
            .checked_add(1)
            .ok_or(JsonDecodeError::Arithmetic)?;
        let bound = construction_bound(entries)?;
        let extra = bound
            .checked_sub(self.backing.requested_bytes())
            .ok_or(JsonDecodeError::Arithmetic)?;
        self.backing.grow_before_allocation(extra)?;
        self.members.insert(
            key,
            LastOccurrence {
                first: ordinal,
                value,
            },
        );
        Ok(())
    }
    fn finish(self) -> Result<MapPlan, ValueError> {
        let count = self.members.len();
        let bytes = count
            .checked_mul(size_of::<MemberPlan>())
            .ok_or(JsonDecodeError::Arithmetic)?;
        let allocation = NativeScratch::reserve(self.ledger, bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(JsonDecodeError::from)?;
        if values.capacity() != count {
            return Err(JsonDecodeError::CapacityProfile.into());
        }
        let MapIndex {
            members, backing, ..
        } = self;
        for (key, value) in members {
            values.push(MemberPlan {
                key: key.0,
                value: value.value,
                first: value.first,
            });
        }
        // The consuming B-tree iterator and its actual nodes have retired.
        drop(backing);
        values.sort_unstable_by_key(|member| member.first);
        Ok(MapPlan {
            members: values.into_iter(),
            _allocation: allocation,
        })
    }
}
/// Pinned Rust B-tree B=6: nonroot nodes have at least5 entries; root at least1.
/// Sixteen pointer widths cover parent/header/edges/padding, as Choices' pinned
/// node-envelope calculation. Two additional nodes cover split/root insertion.
fn construction_bound(entries: usize) -> Result<usize, JsonDecodeError> {
    let node = size_of::<GuardedKey>()
        .checked_add(size_of::<LastOccurrence>())
        .and_then(|v| v.checked_mul(11))
        .and_then(|v| v.checked_add(16 * size_of::<usize>()))
        .ok_or(JsonDecodeError::Arithmetic)?;
    let nodes = 1usize
        .checked_add((entries - 1) / 5)
        .and_then(|v| v.checked_add(2))
        .ok_or(JsonDecodeError::Arithmetic)?;
    node.checked_mul(nodes).ok_or(JsonDecodeError::Arithmetic)
}
pub(super) struct MapPlan {
    members: std::vec::IntoIter<MemberPlan>,
    _allocation: NativeScratch,
}
impl MapPlan {
    pub(super) fn read(
        reader: &mut dyn Read,
        parent: &SemanticSpan,
        ledger: &DecodeLedger,
    ) -> Result<Self, ValueError> {
        let mut input = ObjectFrames::new_literal_managed(reader, parent.start())?;
        let mut index = MapIndex::new(ledger)?;
        let mut ordinal = 0usize;
        while let Some((key, member)) = input.next_guarded_member(ledger)? {
            index.record(key, parent.member(&member)?, ordinal)?;
            ordinal = ordinal.checked_add(1).ok_or(JsonDecodeError::Arithmetic)?;
        }
        RootFrames::Object(input).finish()?;
        index.finish()
    }
    pub(super) fn next(&mut self) -> Option<MemberPlan> {
        self.members.next()
    }
}
