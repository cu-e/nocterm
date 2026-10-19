use super::CleanupError;
use nocterm_ai::{AgentError, workspace::WorkdirError};
use nocterm_core::SharedError;
use std::{error::Error, io};

fn source_as<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> &'a E {
    error.source().unwrap().downcast_ref().unwrap()
}

#[test]
fn process_cleanup_retains_the_original_agent_error_across_clones() {
    let error = CleanupError::process(AgentError::Io("Process tree still alive".into()));
    let cloned = error.clone();
    let shared = source_as::<SharedError<AgentError>>(&error);
    let original = source_as::<AgentError>(shared);
    let cloned_original = source_as::<AgentError>(source_as::<SharedError<AgentError>>(&cloned));
    assert!(matches!(original, AgentError::Io(message) if message == "Process tree still alive"));
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
    let error = CleanupError::process(AgentError::Workspace(WorkdirError::resolve(
        "/workspace",
        native,
    )));
    let cloned = error.clone();
    let original = source_as::<AgentError>(source_as::<SharedError<AgentError>>(&error));
    assert!(matches!(
        original,
        AgentError::Workspace(WorkdirError::Resolve { .. })
    ));
    let workspace = source_as::<WorkdirError>(original);
    assert!(
        matches!(workspace, WorkdirError::Resolve { path, .. } if path.to_str() == Some("/workspace"))
    );
    let native = source_as::<io::Error>(source_as::<SharedError<io::Error>>(workspace));
    assert_eq!(native.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(native.raw_os_error(), Some(code));
    assert_eq!(native.to_string(), message);
    let cloned_original = source_as::<AgentError>(source_as::<SharedError<AgentError>>(&cloned));
    assert!(std::ptr::eq(original, cloned_original));
    let cloned_native =
        source_as::<io::Error>(source_as::<SharedError<io::Error>>(
            source_as::<WorkdirError>(cloned_original),
        ));
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
