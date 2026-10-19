//! Real native publication failure keeps its original source and paid paths alive.
use super::*;
use nocterm_ai::{
    history::{self, HistoryEncoding},
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
use std::error::Error;

#[test]
fn failed_native_publication_clones_pin_original_path_and_refund_on_last_drop() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing-directory");
    std::fs::create_dir(&destination).unwrap();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_writer().unwrap();
    let ledger = nocterm_ai::testing::writer_ledger(&role).unwrap();
    let baseline = ledger.live_requested_bytes();
    let value = serde_json::json!({"original":"payload"});
    let decision = EncodingDecision {
        encoding: HistoryEncoding::Version2,
        size: history::measure_json(&value).unwrap(),
    };
    let publication =
        stage(&destination, &ledger, &value, decision).unwrap_or_else(|error| panic!("{error}"));
    let error = publication.publish().unwrap_err();
    let AgentStorageError::ManagedPublication(original) = &error else {
        panic!("paid native publication failure");
    };
    assert_eq!(original.as_ref().path(), destination);
    assert!(
        original
            .as_ref()
            .source()
            .unwrap()
            .downcast_ref::<nocterm_core::fs::FileError>()
            .is_some()
    );
    let pointer = original.as_ref() as *const _;
    let path_pointer = original
        .as_ref()
        .path()
        .as_os_str()
        .as_encoded_bytes()
        .as_ptr();
    let charged = ledger.live_requested_bytes();
    assert!(charged > baseline);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    let cloned = error.clone();
    drop(error);
    assert_eq!(ledger.live_requested_bytes(), charged);
    let AgentStorageError::ManagedPublication(cloned_cause) = &cloned else {
        panic!("same native category");
    };
    assert_eq!(cloned_cause.as_ref() as *const _, pointer);
    assert_eq!(
        cloned_cause
            .as_ref()
            .path()
            .as_os_str()
            .as_encoded_bytes()
            .as_ptr(),
        path_pointer
    );
    drop(cloned);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn managed_native_stage_cancel_removes_actual_temporary_and_refunds_all_path_credits() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("history");
    std::fs::write(&destination, b"old").unwrap();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_writer().unwrap();
    let ledger = nocterm_ai::testing::writer_ledger(&role).unwrap();
    let baseline = ledger.live_requested_bytes();
    let value = serde_json::json!(["actual", "wire"]);
    let decision = EncodingDecision {
        encoding: HistoryEncoding::Version2,
        size: history::measure_json(&value).unwrap(),
    };
    let publication =
        stage(&destination, &ledger, &value, decision).unwrap_or_else(|error| panic!("{error}"));
    assert!(ledger.live_requested_bytes() > baseline);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    drop(publication);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
}

#[test]
fn escaped_native_error_survives_three_new_writer_phases_with_same_original_cause() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing-directory");
    std::fs::create_dir(&destination).unwrap();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut role = roles.take_writer().unwrap();
    role.close_phase();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    role.begin_phase().unwrap();
    let ledger = nocterm_ai::testing::writer_ledger(&role).unwrap();
    let ledger_baseline = ledger.live_requested_bytes();
    let value = serde_json::json!({"actual":"payload"});
    let decision = EncodingDecision {
        encoding: HistoryEncoding::Version2,
        size: history::measure_json(&value).unwrap(),
    };
    let error = stage(&destination, &ledger, &value, decision)
        .unwrap_or_else(|error| panic!("{error}"))
        .publish()
        .unwrap_err();
    let cloned = error.clone();
    let AgentStorageError::ManagedPublication(original) = &error else {
        panic!("paid native failure");
    };
    let pointer = original.as_ref() as *const _;
    role.close_phase();
    let mut old_ledger = Some(ledger);
    let mut expected_drop = None;
    for turn in 0..3 {
        role.begin_phase().unwrap();
        let ledger = nocterm_ai::testing::writer_ledger(&role).unwrap();
        if let Some(old) = old_ledger.take() {
            let exclusive = old
                .live_requested_bytes()
                .checked_sub(ledger_baseline)
                .unwrap();
            let metadata = nocterm_ai::testing::unique_ledger_metadata(&old, &ledger).unwrap();
            expected_drop = Some(exclusive.checked_add(metadata).unwrap());
            drop(old); // The oracle observer must not pin the old header at last error drop.
        }
        let next = directory.path().join(format!("next-{turn}"));
        stage(&next, &ledger, &value, decision)
            .unwrap_or_else(|error| panic!("{error}"))
            .publish()
            .unwrap();
        role.close_phase();
        drop(ledger);
        let AgentStorageError::ManagedPublication(current) = &cloned else {
            panic!("same native failure");
        };
        assert_eq!(current.as_ref() as *const _, pointer);
        assert_eq!(current.as_ref().path(), destination);
        assert!(scope.retained_and_pending_bytes().unwrap() > baseline);
    }
    drop(error);
    assert!(scope.retained_and_pending_bytes().unwrap() > baseline);
    let before = scope.retained_and_pending_bytes().unwrap();
    drop(cloned);
    let after = scope.retained_and_pending_bytes().unwrap();
    assert_eq!(before.checked_sub(after).unwrap(), expected_drop.unwrap());
    assert_eq!(after, baseline);
}

#[test]
fn managed_directory_policy_keeps_original_message_and_actual_loan_until_final_clone_drop() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    std::fs::write(&path, b"old").unwrap();
    let original = nocterm_core::paths::ensure_private_dir(&path).unwrap_err();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_writer().unwrap();
    let ledger = nocterm_ai::testing::writer_ledger(&role).unwrap();
    let baseline = ledger.live_requested_bytes();
    let error = super::native_paths::private_directory(&path, &ledger).unwrap_err();
    let AgentStorageError::ManagedDirectory(cause) = &error else {
        panic!("managed directory category");
    };
    assert_eq!(cause.as_ref().kind(), original.kind());
    assert_eq!(
        cause.as_ref().native_source().to_string(),
        original.to_string()
    );
    assert_eq!(
        cause
            .as_ref()
            .native_source()
            .source()
            .map(ToString::to_string),
        original.source().map(ToString::to_string)
    );
    assert_eq!(
        cause
            .as_ref()
            .native_source()
            .get_ref()
            .unwrap()
            .to_string(),
        original.get_ref().unwrap().to_string()
    );
    let pointer = cause.as_ref() as *const _;
    let charged = ledger.live_requested_bytes();
    assert!(charged > baseline);
    let cloned = error.clone();
    drop(error);
    assert_eq!(ledger.live_requested_bytes(), charged);
    let AgentStorageError::ManagedDirectory(cause) = &cloned else {
        panic!("same original category");
    };
    assert_eq!(cause.as_ref() as *const _, pointer);
    drop(cloned);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
}
