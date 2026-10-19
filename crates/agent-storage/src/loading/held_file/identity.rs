//! Original strong and weak identities keep both actual shared headers paid.
use nocterm_ai::model::json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch};
use nocterm_core::SharedResidentLease;
use std::sync::{Arc, Weak};

pub(super) struct StrongIdentity {
    original: Arc<()>,
    payment: Option<SharedResidentLease<NativeScratch>>,
}
#[derive(Clone)]
pub(super) struct WeakIdentity {
    original: Weak<()>,
    _payment: Option<SharedResidentLease<NativeScratch>>,
}
impl StrongIdentity {
    #[cfg(test)]
    pub(super) fn legacy() -> Self {
        Self {
            original: Arc::new(()),
            payment: None,
        }
    }
    pub(super) fn managed(ledger: &DecodeLedger) -> Result<Self, NativeAllocationError> {
        let identity = nocterm_ai::model::allocations::shared_inline::<()>()
            .ok_or(NativeAllocationError::Arithmetic)?;
        let bytes = identity
            .checked_add(SharedResidentLease::<NativeScratch>::allocation_bytes())
            .ok_or(NativeAllocationError::Arithmetic)?;
        let credit = NativeScratch::reserve_native(ledger, bytes)?;
        let payment = SharedResidentLease::new(credit);
        Ok(Self {
            original: Arc::new(()),
            payment: Some(payment),
        })
    }
    pub(super) fn downgrade(&self) -> WeakIdentity {
        WeakIdentity {
            original: Arc::downgrade(&self.original),
            _payment: self.payment.clone(),
        }
    }
}
impl WeakIdentity {
    pub(super) fn matches(&self, original: &StrongIdentity) -> bool {
        self.original.ptr_eq(&Arc::downgrade(&original.original))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::model::retention::RetentionAuthority;

    #[test]
    fn original_weak_receipts_retain_both_headers_until_the_last_drop() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
        let baseline = ledger.live_requested_bytes();
        let original = StrongIdentity::managed(&ledger).unwrap();
        let charged = ledger.live_requested_bytes();
        let receipt = original.downgrade();
        let last = receipt.clone();
        assert!(receipt.matches(&original));
        assert!(charged > baseline);
        assert_eq!(ledger.live_requested_bytes(), charged);
        drop(original);
        assert_eq!(ledger.live_requested_bytes(), charged);
        drop(receipt);
        assert_eq!(ledger.live_requested_bytes(), charged);
        drop(last);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }

    #[test]
    fn equal_source_values_cannot_substitute_an_original_identity() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
        let original = StrongIdentity::managed(&ledger).unwrap();
        let other = StrongIdentity::managed(&ledger).unwrap();
        let receipt = original.downgrade();
        assert!(receipt.matches(&original));
        assert!(!receipt.matches(&other));
    }
}
