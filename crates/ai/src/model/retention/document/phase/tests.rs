//! A real escaped resident failure retains its original Writer phase metadata.
use super::*;
use crate::model::{
    json::deferred::{DecodeLedger, NativeScratch},
    retention::RetentionAuthority,
};
use nocterm_core::SharedResidentError;
use std::{error::Error, fmt};

struct NativeFailure {
    native: std::io::Error,
    _bytes: Vec<u8>,
    _allocation: NativeScratch,
}
impl fmt::Display for NativeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.native.fmt(formatter)
    }
}
impl fmt::Debug for NativeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.native, formatter)
    }
}
impl Error for NativeFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.native)
    }
}

#[test]
fn escaped_writer_failure_keeps_old_phase_paid_after_original_phase_replacement() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut writer = roles.take_writer().unwrap();
    let old = DecodeLedger::for_operation(&writer, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let ledger_baseline = old.live_requested_bytes();
    let buffer_credit = NativeScratch::reserve(&old, 4096).unwrap();
    let bytes = vec![1u8; 4096];
    assert_eq!(bytes.capacity(), 4096);
    let header_credit = NativeScratch::reserve(
        &old,
        SharedResidentError::<NativeFailure, NativeScratch>::allocation_bytes(),
    )
    .unwrap();
    let failure = SharedResidentError::new(
        NativeFailure {
            native: std::io::Error::from_raw_os_error(13),
            _bytes: bytes,
            _allocation: buffer_credit,
        },
        header_credit,
    );
    let original_exclusive = old.live_requested_bytes() - ledger_baseline;
    writer.close_phase();
    writer.begin_phase().unwrap();
    let current = DecodeLedger::for_operation(&writer, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let old_unique_metadata = crate::testing::unique_ledger_metadata(&old, &current).unwrap();
    assert!(
        old_unique_metadata > ledger_baseline,
        "the old original phase is a separate actual allocation"
    );
    drop(old);
    let before = scope.retained_and_pending_bytes().unwrap();
    drop(failure);
    let after = scope.retained_and_pending_bytes().unwrap();
    assert_eq!(
        before - after,
        original_exclusive + old_unique_metadata,
        "last escaped native failure releases payload, ledger and original phase only after physical destruction"
    );
}
