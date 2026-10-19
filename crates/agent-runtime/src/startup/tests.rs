use super::CleanupError;
use nocterm_ai::{
    model::{failure::ManagedNativeFailure, retention::RetentionAuthority},
    thread::ThreadState,
    workspace::WorkdirError,
};
use nocterm_core::SharedError;
use std::{error::Error, io};

fn source_as<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> &'a E {
    error.source().unwrap().downcast_ref().unwrap()
}
/// The failure is admitted against a real document Source, like an adapter's cleanup result.
fn paid_failure<E: Error + Send + Sync + 'static>(error: E, nested: usize) -> ManagedNativeFailure {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let state = ThreadState::new(
        &authority,
        "codex",
        "00000000-0000-0000-0000-000000000001",
        1,
    )
    .unwrap();
    let source = state.source().capture().unwrap();
    let slot = source
        .context()
        .unwrap()
        .prepare_error::<E>(nested)
        .unwrap();
    slot.capture(error)
}

#[test]
fn process_cleanup_retains_the_original_native_cause_across_clones() {
    let error = CleanupError::Process(paid_failure(
        io::Error::other("Process tree still alive"),
        0,
    ));
    let cloned = error.clone();
    let failure = source_as::<ManagedNativeFailure>(&error);
    let original = source_as::<io::Error>(failure);
    let cloned_original = source_as::<io::Error>(source_as::<ManagedNativeFailure>(&cloned));
    assert_eq!(original.to_string(), "Process tree still alive");
    assert!(std::ptr::eq(original, cloned_original));
    assert!(original.source().is_none());
    assert_eq!(
        error.to_string(),
        "agent process cleanup failed: Process tree still alive"
    );
}

#[test]
fn process_cleanup_preserves_workspace_and_native_source_frames() {
    let code = if cfg!(windows) { 5 } else { 13 };
    let native = io::Error::from_raw_os_error(code);
    let message = native.to_string();
    let error = CleanupError::Process(paid_failure(
        WorkdirError::resolve("/workspace", native),
        "/workspace".len(),
    ));
    let cloned = error.clone();
    let original = source_as::<WorkdirError>(source_as::<ManagedNativeFailure>(&error));
    assert!(
        matches!(original, WorkdirError::Resolve { path, .. } if path.to_str() == Some("/workspace"))
    );
    let native = source_as::<io::Error>(source_as::<SharedError<io::Error>>(original));
    assert_eq!(native.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(native.raw_os_error(), Some(code));
    assert_eq!(native.to_string(), message);
    let cloned_original = source_as::<WorkdirError>(source_as::<ManagedNativeFailure>(&cloned));
    assert!(std::ptr::eq(original, cloned_original));
    let cloned_native =
        source_as::<io::Error>(source_as::<SharedError<io::Error>>(cloned_original));
    assert!(std::ptr::eq(native, cloned_native));
}

#[test]
fn lost_startup_acknowledgement_has_no_fabricated_source() {
    let error = CleanupError::AcknowledgementDropped;
    assert_eq!(
        error.to_string(),
        "agent startup cleanup acknowledgement was lost"
    );
    assert!(error.source().is_none());
}
