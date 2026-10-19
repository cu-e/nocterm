//! Sealed prepaid byte construction; arbitrary existing vectors cannot be adopted.
use super::{DecodeLedger, JsonDecodeError, NativeOwned, Reservation};

/// A byte buffer whose exact backing is paid before allocation.
/// Appending never grows implicitly; callers explicitly prepay replacement growth.
#[must_use = "finish the bytes or retain the builder until construction is cancelled"]
pub struct NativeBytes {
    // Declaration order retires the payload before returning its credit.
    values: Vec<u8>,
    allocation: Reservation,
}

impl NativeBytes {
    pub fn new(ledger: &DecodeLedger, capacity: usize) -> Result<Self, JsonDecodeError> {
        let allocation = ledger.reserve(capacity)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        if values.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(Self { values, allocation })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.values
    }

    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }

    /// Copy a borrowed fragment only when it fits the already prepaid backing.
    /// Refusal leaves the payload, capacity and reservation unchanged.
    pub fn append(&mut self, fragment: &[u8]) -> Result<(), JsonDecodeError> {
        let required = self
            .values
            .len()
            .checked_add(fragment.len())
            .ok_or(JsonDecodeError::Arithmetic)?;
        if required > self.values.capacity() {
            return Err(JsonDecodeError::ConstructionOrder);
        }
        self.values.extend_from_slice(fragment);
        Ok(())
    }

    /// Grow to an exact total capacity, not an additional byte count.
    /// Requests at or below the current capacity are no-ops (never shrink).
    /// The full replacement is reserved and allocated while the old payload and
    /// its credit remain alive. Any failure leaves this builder unchanged.
    pub fn grow(&mut self, capacity: usize) -> Result<(), JsonDecodeError> {
        if capacity <= self.values.capacity() {
            return Ok(());
        }
        let mut replacement = Self::new(self.allocation.original_ledger(), capacity)?;
        replacement.values.extend_from_slice(&self.values);
        // Drop the old Vec before its reservation; the replacement already owns
        // its independent credit. No in-place realloc can mutate us on refusal.
        drop(std::mem::replace(self, replacement));
        Ok(())
    }

    pub fn finish(self) -> NativeOwned<Vec<u8>> {
        NativeOwned::from_parts(self.values, self.allocation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::retention::{RetentionAuthority, document::DocumentRetention};

    #[test]
    fn prepaid_bytes_finish_with_original_ledger_and_exact_capacity() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4096).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut bytes = NativeBytes::new(&ledger, 5).unwrap();
        assert_eq!(bytes.capacity(), 5);
        assert_eq!(ledger.live_requested_bytes(), baseline + 5);
        bytes.append(&[0, 255]).unwrap();
        bytes.append(b"abc").unwrap();
        let owned = bytes.finish();
        assert_eq!(owned.get().as_slice(), &[0, 255, b'a', b'b', b'c']);
        assert_eq!(owned.get().capacity(), 5);
        assert!(owned.allocation.belongs_to(&ledger));
        assert_eq!(ledger.live_requested_bytes(), baseline + 5);
        drop(owned);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }

    #[test]
    fn bounded_append_refuses_without_changing_payload_capacity_or_credit() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4096).unwrap();
        let mut bytes = NativeBytes::new(&ledger, 3).unwrap();
        bytes.append(b"ab").unwrap();
        let live = ledger.live_requested_bytes();
        assert!(matches!(
            bytes.append(b"cd"),
            Err(JsonDecodeError::ConstructionOrder)
        ));
        assert_eq!(bytes.as_slice(), b"ab");
        assert_eq!(bytes.capacity(), 3);
        assert_eq!(ledger.live_requested_bytes(), live);
        bytes.append(b"c").unwrap();
        assert_eq!(bytes.as_slice(), b"abc");
    }

    #[test]
    fn growth_prepays_full_replacement_while_original_remains_live() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4096).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut bytes = NativeBytes::new(&ledger, 4).unwrap();
        bytes.append(b"ab").unwrap();
        bytes.grow(9).unwrap();
        assert_eq!(bytes.as_slice(), b"ab");
        assert_eq!(bytes.capacity(), 9);
        assert_eq!(ledger.peak_requested_bytes(), baseline + 4 + 9);
        assert_eq!(ledger.live_requested_bytes(), baseline + 9);
        bytes.append(b"cdefghi").unwrap();
        assert_eq!(bytes.finish().get().as_slice(), b"abcdefghi");
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }

    #[test]
    fn growth_refusal_preserves_builder_even_when_capacity_delta_would_fit() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let probe = DecodeLedger::new(&authority, 4096).unwrap();
        let baseline = probe.live_requested_bytes();
        let ledger = DecodeLedger::new(&authority, baseline + 9).unwrap();
        let mut bytes = NativeBytes::new(&ledger, 4).unwrap();
        bytes.append(b"ab").unwrap();
        assert!(matches!(bytes.grow(9), Err(JsonDecodeError::Limit { .. })));
        assert_eq!(bytes.as_slice(), b"ab");
        assert_eq!(bytes.capacity(), 4);
        assert_eq!(ledger.live_requested_bytes(), baseline + 4);
        assert_eq!(ledger.peak_requested_bytes(), baseline + 4);
        bytes.append(b"cd").unwrap();
        assert_eq!(bytes.finish().get().as_slice(), b"abcd");
        assert_eq!(ledger.live_requested_bytes(), baseline);
        assert!(matches!(
            NativeBytes::new(&ledger, 10),
            Err(JsonDecodeError::Limit { .. })
        ));
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }

    #[test]
    fn allocation_failure_refunds_replacement_and_keeps_original() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
        let mut bytes = NativeBytes::new(&ledger, 4).unwrap();
        bytes.append(b"ab").unwrap();
        let live = ledger.live_requested_bytes();
        let impossible = (isize::MAX as usize) + 1;
        assert!(matches!(
            bytes.grow(impossible),
            Err(JsonDecodeError::Allocation(_))
        ));
        assert_eq!(bytes.as_slice(), b"ab");
        assert_eq!(bytes.capacity(), 4);
        assert_eq!(ledger.live_requested_bytes(), live);
        assert!(matches!(
            NativeBytes::new(&ledger, impossible),
            Err(JsonDecodeError::Allocation(_))
        ));
        assert_eq!(ledger.live_requested_bytes(), live);
    }

    #[test]
    fn empty_builder_and_non_growing_requests_do_not_allocate() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 4096).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut bytes = NativeBytes::new(&ledger, 0).unwrap();
        bytes.append(b"").unwrap();
        bytes.grow(0).unwrap();
        assert_eq!(bytes.capacity(), 0);
        assert_eq!(ledger.live_requested_bytes(), baseline);
        bytes.grow(4).unwrap();
        bytes.append(b"ab").unwrap();
        let peak = ledger.peak_requested_bytes();
        bytes.grow(4).unwrap();
        bytes.grow(1).unwrap();
        assert_eq!(bytes.capacity(), 4);
        assert_eq!(bytes.as_slice(), b"ab");
        assert_eq!(ledger.peak_requested_bytes(), peak);
    }

    #[test]
    fn scoped_credit_is_refunded_on_failure_cancellation_and_finished_owner_drop() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let (document, mut roles) = DocumentRetention::new(&authority).unwrap();
        let role = roles.take_foreground().unwrap();
        let ledger = DecodeLedger::for_operation(&role, usize::MAX).unwrap();
        let account_baseline = document.retained_and_pending_bytes().unwrap();
        let ledger_baseline = ledger.live_requested_bytes();
        let mut bytes = NativeBytes::new(&ledger, 4).unwrap();
        bytes.append(b"ab").unwrap();
        let account_live = document.retained_and_pending_bytes().unwrap();
        assert!(account_live > account_baseline);
        assert!(bytes.grow(crate::history::MAX_RESIDENT_BYTES).is_err());
        assert!(NativeBytes::new(&ledger, crate::history::MAX_RESIDENT_BYTES).is_err());
        assert_eq!(document.retained_and_pending_bytes().unwrap(), account_live);
        assert_eq!(ledger.live_requested_bytes(), ledger_baseline + 4);
        assert_eq!(bytes.as_slice(), b"ab");
        assert_eq!(bytes.capacity(), 4);
        bytes.grow(8).unwrap();
        let owned = bytes.finish();
        assert_eq!(owned.get().as_slice(), b"ab");
        assert!(document.retained_and_pending_bytes().unwrap() > account_baseline);
        drop(owned);
        assert_eq!(ledger.live_requested_bytes(), ledger_baseline);
        assert_eq!(
            document.retained_and_pending_bytes().unwrap(),
            account_baseline
        );
        drop(NativeBytes::new(&ledger, 7).unwrap());
        assert_eq!(ledger.live_requested_bytes(), ledger_baseline);
        assert_eq!(
            document.retained_and_pending_bytes().unwrap(),
            account_baseline
        );
    }
}
