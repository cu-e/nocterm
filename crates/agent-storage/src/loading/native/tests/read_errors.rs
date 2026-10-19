use super::*;
use nocterm_ai::{
    history::{NativeLoadError, ReaderDelivery, ReaderSource},
    model::retention::document::DocumentRetention,
};
use std::{error::Error, io, path::Path};

fn failed_load(directory: &Path) -> ReaderDelivery {
    failed_load_as(directory, ID)
}
fn failed_load_as(directory: &Path, id: &str) -> ReaderDelivery {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let repository = FileChatRepository::with_retention(directory.into(), authority);
    futures::executor::block_on(repository.load_into(id.into(), source.capture().unwrap()))
}
fn native<'a>(error: &'a (dyn Error + 'static)) -> &'a io::Error {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(native) = error.downcast_ref::<io::Error>() {
            return native;
        }
        current = error.source();
    }
    panic!("original native read cause is in the standard source chain")
}

#[cfg(unix)]
#[test]
fn escaped_native_read_failure_shares_original_path_cause_and_display() {
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("blocker");
    fs::write(&blocker, b"unchanged public blocker").unwrap();
    let delivery = failed_load(&blocker);
    let Some(NativeLoadError::Storage(error)) = delivery.error() else {
        panic!("native read failed through the storage boundary")
    };
    assert!(matches!(
        error,
        nocterm_ai::storage::AgentStorageError::ManagedRead(_)
    ));
    let original = native(error) as *const io::Error;
    let message = error.to_string();
    let escaped = error.clone();
    assert_eq!(native(&escaped).kind(), io::ErrorKind::NotADirectory);
    assert!(std::ptr::eq(native(&escaped), original));
    drop(delivery);
    assert!(std::ptr::eq(native(&escaped), original));
    assert_eq!(escaped.to_string(), message);
    assert_eq!(fs::read(blocker).unwrap(), b"unchanged public blocker");
}

#[test]
fn missing_managed_document_keeps_original_not_found_classification_and_id() {
    let directory = tempfile::tempdir().unwrap();
    let delivery = failed_load(directory.path());
    let Some(NativeLoadError::Storage(error)) = delivery.error() else {
        panic!("missing document failed through the storage boundary")
    };
    assert!(error.is_not_found());
    assert_eq!(error.to_string(), format!("Saved chat was not found: {ID}"));
    assert_eq!(native(error).kind(), io::ErrorKind::NotFound);
    assert!(directory.path().read_dir().unwrap().next().is_none());
}

#[test]
fn managed_invalid_identity_preserves_supplied_label_before_native_io() {
    let directory = tempfile::tempdir().unwrap();
    let delivery = failed_load_as(directory.path(), "original invalid 名字");
    let Some(NativeLoadError::Storage(error)) = delivery.error() else {
        panic!("invalid identity failed through the storage boundary")
    };
    assert_eq!(error.invalid_identity(), Some("original invalid 名字"));
    let original = error.invalid_identity().unwrap().as_ptr();
    let escaped = error.clone();
    drop(delivery);
    assert_eq!(escaped.invalid_identity().unwrap().as_ptr(), original);
    assert_eq!(
        escaped.to_string(),
        "Invalid chat identity: original invalid 名字"
    );
    assert!(directory.path().read_dir().unwrap().next().is_none());
}
