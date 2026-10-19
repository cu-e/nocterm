//! Immutable measured rows own exact vector and shared-header reservations.
use super::Measurement;
use crate::{
    history::{HistoryCodecError, SavedPrompt},
    model::{
        json::{
            deferred::{DecodeLedger, JsonDecodeError, ledger::Reservation},
            lifetime::Shared,
        },
        retention::{
            CensusPermit, RetentionAuthority, RetentionCell, RetentionError,
            graph::{Count, OwnerGraph, sealed},
        },
    },
    thread::Entry,
};
use std::mem::size_of;

pub(super) struct Measurements {
    values: Vec<Measurement>,
    retention: RetentionCell,
    // Shared::into_inner frees its header before this value drops; declaration
    // order frees the vector before its genuine allocation credit is retired.
    allocation: Option<Reservation>,
}
impl Measurements {
    pub(super) fn rows(
        authority: &RetentionAuthority,
        rows: &[Entry],
        grammar: Option<&crate::history::retention::LegacyFormat>,
        ledger: Option<&DecodeLedger>,
    ) -> Result<Shared<Self>, HistoryCodecError> {
        Self::build(
            authority,
            rows.iter().map(|row| Measurement::row(row, grammar)),
            ledger,
        )
    }
    pub(super) fn prompts(
        authority: &RetentionAuthority,
        prompts: &[SavedPrompt],
        grammar: Option<&crate::history::retention::LegacyFormat>,
        ledger: Option<&DecodeLedger>,
    ) -> Result<Shared<Self>, HistoryCodecError> {
        Self::build(
            authority,
            prompts
                .iter()
                .map(|prompt| Measurement::prompt(prompt, grammar)),
            ledger,
        )
    }
    fn build(
        authority: &RetentionAuthority,
        items: impl ExactSizeIterator<Item = Result<Measurement, HistoryCodecError>>,
        ledger: Option<&DecodeLedger>,
    ) -> Result<Shared<Self>, HistoryCodecError> {
        if ledger.is_some_and(|ledger| !authority.same(ledger.authority())) {
            return Err(HistoryCodecError::Retention(
                nocterm_core::SharedError::from_error(RetentionError::ForeignAuthority),
            ));
        }
        let count = items.len();
        let bytes = count
            .checked_mul(size_of::<Measurement>())
            .and_then(|bytes| {
                bytes.checked_add(crate::model::allocations::shared_inline_bytes::<Self>())
            })
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        let allocation = ledger
            .map(|ledger| ledger.reserve(bytes))
            .transpose()
            .map_err(HistoryCodecError::from)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(JsonDecodeError::from)
            .map_err(HistoryCodecError::from)?;
        if values.capacity() != count {
            return Err(HistoryCodecError::from(JsonDecodeError::CapacityProfile));
        }
        for item in items {
            values.push(item?);
        }
        Ok(Shared::new(Self {
            values,
            retention: RetentionCell::new(authority),
            allocation,
        }))
    }
    pub(super) fn as_slice(&self) -> &[Measurement] {
        &self.values
    }
}
impl std::ops::Deref for Measurements {
    type Target = [Measurement];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl sealed::Source for Measurements {}
impl OwnerGraph for Measurements {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        let bytes = self
            .values
            .capacity()
            .checked_mul(size_of::<Measurement>())
            .and_then(|bytes| {
                bytes.checked_add(crate::model::allocations::shared_inline_bytes::<Self>())
            })
            .ok_or(RetentionError::Arithmetic)?;
        if count.enter(&self.retention, bytes)? {
            if let Some(allocation) = &self.allocation {
                allocation.validate_pending(count)?;
            }
            for row in &self.values {
                if let Some(legacy) = &row.legacy {
                    legacy.retention.count(count)?;
                }
            }
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if self.retention.clear_owned(permit) {
            if let Some(allocation) = &self.allocation {
                allocation.cleanup_pending(permit);
            }
            for row in &self.values {
                if let Some(retention) = row.legacy.as_ref().map(|legacy| &legacy.retention) {
                    retention.cleanup(permit);
                }
            }
        }
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        if publication.enter(&self.retention) {
            if let Some(allocation) = &self.allocation {
                allocation.adopt_under(publication);
            }
            for row in &self.values {
                if let Some(legacy) = &row.legacy {
                    legacy.retention.adopt(publication);
                }
            }
        }
    }
}
