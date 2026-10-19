//! Closed PNG inspection over the original native parser and original source.
use ::png::budget::{AllocationBudget, AllocationRefused, BudgetVec};
use nocterm_ai::images::{
    self, ImageFacts, NativeImageAllocationScope, managed::ManagedImageBytes,
};
use std::io::Cursor;

#[derive(Clone, Copy)]
struct Budget<'a, 'ledger>(&'a NativeImageAllocationScope<'ledger>);
impl AllocationBudget for Budget<'_, '_> {
    fn admit(&self, bytes: usize) -> Result<(), AllocationRefused> {
        self.0
            .admit_before_native_allocation(bytes)
            .map_err(|_| AllocationRefused)
    }
    fn retire(&self, bytes: usize) {
        self.0.retire_after_native_drop(bytes);
    }
}
#[derive(Debug, thiserror::Error)]
pub(super) enum Failure {
    #[error(transparent)]
    Decoder(#[from] ::png::DecodingError),
    #[error(transparent)]
    Allocation(#[from] AllocationRefused),
    #[error("Image dimensions exceed the decoding limit")]
    Dimensions,
    #[error("Image codec returned inconsistent evidence")]
    Evidence,
}
/// All native allocations and inline receipts end before this function returns,
/// including errors ignored by optional metadata parsing. The caller MUST then
/// close the scope and surface its sticky first refusal before accepting facts.
pub(super) fn inspect(
    source: &ManagedImageBytes,
    facts: ImageFacts,
    scope: &NativeImageAllocationScope<'_>,
) -> Result<ImageFacts, Failure> {
    let budget = Budget(scope);
    let mut decoder = ::png::Decoder::new_with_budget(
        Cursor::new(source.bytes()),
        ::png::Limits {
            bytes: images::MAX_DECODE_BYTES as usize,
        },
        budget,
    )?;
    decoder.set_ignore_text_chunk(false);
    let header = decoder.read_header_info()?;
    if header.width > images::MAX_DECODE_DIMENSION || header.height > images::MAX_DECODE_DIMENSION {
        return Err(Failure::Dimensions);
    }
    if (header.width, header.height) != (facts.width(), facts.height()) {
        return Err(Failure::Evidence);
    }
    decoder.set_transformations(::png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let bytes = reader
        .output_buffer_size()
        .ok_or(::png::DecodingError::LimitsExceeded)?;
    if bytes as u64 > images::MAX_DECODE_BYTES {
        return Err(::png::DecodingError::LimitsExceeded.into());
    }
    let mut output = BudgetVec::with_capacity(budget, bytes)?;
    output.resize(bytes, 0)?;
    reader.next_frame(&mut output)?;
    Ok(facts)
}
