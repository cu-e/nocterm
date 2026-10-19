//! Managed diagnostic labels keep their actual text and shared header paid.
use crate::model::json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch};
use nocterm_core::SharedResidentError;

#[derive(Debug, thiserror::Error)]
#[error("{text}")]
pub struct StorageLabel {
    text: String,
}
impl StorageLabel {
    pub fn as_str(&self) -> &str {
        &self.text
    }
}
pub type ManagedLabelFailure = SharedResidentError<StorageLabel, NativeScratch>;

impl StorageLabel {
    /// Admit the actual shared body and original supplied label before copying.
    pub(super) fn prepare(
        ledger: &DecodeLedger,
        id: &str,
    ) -> Result<ManagedLabelFailure, NativeAllocationError> {
        let bytes = ManagedLabelFailure::allocation_bytes()
            .checked_add(id.len())
            .ok_or(NativeAllocationError::Arithmetic)?;
        let credit = NativeScratch::reserve_native(ledger, bytes)?;
        let label = StorageLabel {
            text: id.to_owned(),
        };
        Ok(SharedResidentError::new(label, credit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::retention::RetentionAuthority;
    use crate::storage::AgentStorageError;

    #[test]
    fn invalid_identity_clones_share_original_paid_label_until_last_drop() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
        let baseline = ledger.live_requested_bytes();
        let error =
            AgentStorageError::managed_invalid_id(&ledger, "original invalid 名字").unwrap();
        let pointer = error.invalid_identity().unwrap().as_ptr();
        let charged = ledger.live_requested_bytes();
        let last = error.clone();
        drop(error);
        assert_eq!(last.invalid_identity().unwrap().as_ptr(), pointer);
        assert_eq!(ledger.live_requested_bytes(), charged);
        assert_eq!(
            last.to_string(),
            "Invalid chat identity: original invalid 名字"
        );
        drop(last);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}
