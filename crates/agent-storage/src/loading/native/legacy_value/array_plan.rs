//! Only surviving array elements receive a prepaid span cursor and final backing.
use super::{ValueError, source::SemanticSpan};
use crate::loading::{json_frames::RootFrames, literal_reader::LiteralReadError};
use nocterm_ai::{
    history::LiteralPreflight,
    model::json::deferred::{DecodeLedger, JsonDecodeError, NativeScratch},
};
use std::{io::Read, mem::size_of};
pub(super) struct ArrayPlan {
    values: std::vec::IntoIter<SemanticSpan>,
    _allocation: NativeScratch,
}
impl ArrayPlan {
    pub(super) fn count(reader: &mut dyn Read, parent: &SemanticSpan) -> Result<usize, ValueError> {
        let mut frames = RootFrames::new_literal_managed(reader, parent.start())?;
        let RootFrames::Array(array) = &mut frames else {
            return Err(ValueError::Punctuation);
        };
        let mut count = 0usize;
        let mut minimum_policy = LiteralPreflight::default();
        minimum_policy.node().map_err(LiteralReadError::from)?;
        while array.next_element()?.is_some() {
            count = count.checked_add(1).ok_or(JsonDecodeError::Arithmetic)?;
            minimum_policy.node().map_err(LiteralReadError::from)?;
        }
        frames.finish()?;
        Ok(count)
    }
    pub(super) fn read(
        reader: &mut dyn Read,
        parent: &SemanticSpan,
        count: usize,
        ledger: &DecodeLedger,
    ) -> Result<Self, ValueError> {
        let bytes = count
            .checked_mul(size_of::<SemanticSpan>())
            .ok_or(JsonDecodeError::Arithmetic)?;
        let allocation = NativeScratch::reserve(ledger, bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(JsonDecodeError::from)?;
        if values.capacity() != count {
            return Err(JsonDecodeError::CapacityProfile.into());
        }
        let mut frames = RootFrames::new_literal_managed(reader, parent.start())?;
        let RootFrames::Array(array) = &mut frames else {
            return Err(ValueError::Punctuation);
        };
        while let Some(element) = array.next_element()? {
            if values.len() == count {
                return Err(ValueError::ForeignSource);
            }
            values.push(parent.element(&element)?);
        }
        frames.finish()?;
        if values.len() != count {
            return Err(ValueError::ForeignSource);
        }
        Ok(Self {
            values: values.into_iter(),
            _allocation: allocation,
        })
    }
    pub(super) fn next(&mut self) -> Option<SemanticSpan> {
        self.values.next()
    }
}
