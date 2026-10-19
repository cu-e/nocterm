//! Exact constructors for the original text fields under explicit admission.
use super::TextDecodingError;
use crate::DecodingError;
use fdeflate::budget::{Allocation, AllocationBudget};

pub(super) fn latin<B: AllocationBudget>(
    bytes: &[u8],
    credit: &mut Allocation<B>,
) -> Result<String, DecodingError> {
    let count = bytes
        .iter()
        .try_fold(0usize, |n, b| n.checked_add(if *b < 128 { 1 } else { 2 }))
        .ok_or(DecodingError::LimitsExceeded)?;
    let loan = Allocation::new(credit.budget().clone(), count)
        .map_err(|_| DecodingError::LimitsExceeded)?;
    let text = if credit.budget().tracked() {
        let mut text = String::with_capacity(count);
        text.extend(bytes.iter().map(|b| *b as char));
        text
    } else {
        super::decode_iso_8859_1(bytes)
    };
    credit.absorb(loan);
    Ok(text)
}
pub(super) fn bytes<B: AllocationBudget>(
    bytes: &[u8],
    credit: &mut Allocation<B>,
) -> Result<Vec<u8>, DecodingError> {
    let loan = Allocation::new(credit.budget().clone(), bytes.len())
        .map_err(|_| DecodingError::LimitsExceeded)?;
    let value = bytes.to_vec();
    credit.absorb(loan);
    Ok(value)
}
pub(super) fn utf8<B: AllocationBudget>(
    bytes: &[u8],
    credit: &mut Allocation<B>,
) -> Result<String, DecodingError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| DecodingError::from(TextDecodingError::Unrepresentable))?;
    let loan = Allocation::new(credit.budget().clone(), text.len())
        .map_err(|_| DecodingError::LimitsExceeded)?;
    let text = text.to_owned();
    credit.absorb(loan);
    Ok(text)
}
