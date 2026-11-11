use super::*;
use nocterm_ai::{history::ContextId, local_environment::LocalEnvironment};
use std::{path::PathBuf, sync::Arc};

fn snapshot_failure(error: impl Error + Send + Sync + 'static) -> AgentPersistenceError {
    nocterm_store::StoreError::backend(error).into()
}

#[test]
fn unsupported_storage_reports_missing_capability_instead_of_disk_space() {
    for error in [
        AgentStorageError::EnvironmentUnsupported(Arc::new(LocalEnvironment::ThisComputer)),
        AgentStorageError::ScopeUnsupported(ContextId::General),
    ] {
        for failure in [error.clone().into(), snapshot_failure(error)] {
            let message = persistence_failure(&failure);
            assert!(
                message.contains("lacks the native storage support"),
                "{message}"
            );
            assert!(
                message.contains("Queued messages are retained"),
                "{message}"
            );
            assert!(!message.contains("disk space"), "{message}");
        }
    }
}

#[test]
fn full_storage_reports_disk_space_from_the_preserved_native_cause() {
    for kind in [io::ErrorKind::StorageFull, io::ErrorKind::QuotaExceeded] {
        let native = LocalFsError::from_io("chat", io::Error::from(kind));
        let message = persistence_failure(&snapshot_failure(AgentStorageError::from(native)));
        assert!(
            message.contains("free disk space before retrying"),
            "{message}"
        );
        assert!(
            message.contains("Queued messages are retained"),
            "{message}"
        );
    }
}

#[test]
fn denied_storage_reports_access_and_other_write_failures_keep_the_cause() {
    let denied = AgentStorageError::from(LocalFsError::PermissionDenied(PathBuf::from("chat")));
    let message = persistence_failure(&snapshot_failure(denied));
    assert!(
        message.contains("allow access to the chat storage directory"),
        "{message}"
    );
    assert!(!message.contains("disk space"), "{message}");

    let native = LocalFsError::from_io("chat", io::Error::other("snapshot interrupted"));
    let message = persistence_failure(&snapshot_failure(AgentStorageError::from(native)));
    assert!(message.contains("snapshot interrupted"), "{message}");
    assert!(
        message.contains("resolve the storage error before retrying"),
        "{message}"
    );
    assert!(!message.contains("disk space"), "{message}");
}
