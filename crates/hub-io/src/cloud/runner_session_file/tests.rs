//! Explicit file actions preserve the exact rotation and original private source.
use super::*;
use crate::cloud::runner_test_support::{binding, session, subject};

#[test]
fn runner_session_restores_exact_rotation_with_exclusive_original_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = binding(2);
    let subject = subject();
    let mut file = RunnerSessionFile::acquire(directory.path(), &source, &subject).unwrap();
    file.require_absent().unwrap();
    assert!(RunnerSessionFile::acquire(directory.path(), &source, &subject).is_err());
    let first = SessionRecord::from_session(session(&subject, "first"), &file).unwrap();
    file.save(&first).unwrap();
    assert!(matches!(file.require_absent(), Err(Error::Existing)));
    let rotated = SessionRecord::from_session(session(&subject, "rotated"), &file).unwrap();
    file.save(&rotated).unwrap();
    // Retry the same successful rotation's disk write, without changing tokens.
    file.save(&rotated).unwrap();
    drop(file);
    let mut restored = RunnerSessionFile::acquire(directory.path(), &source, &subject).unwrap();
    let tokens = restored.read(&subject).unwrap().into_response();
    assert_eq!(tokens.access_token, "access-rotated");
    assert_eq!(tokens.refresh_token, "refresh-rotated");
    drop(restored);
    let mut other = RunnerSessionFile::acquire(directory.path(), &binding(9), &subject).unwrap();
    assert!(matches!(other.read(&subject), Err(Error::Format)));
    drop(other);
    let mut other_proof = subject.clone();
    other_proof.proof_key[0] ^= 1;
    let mut other = RunnerSessionFile::acquire(directory.path(), &source, &other_proof).unwrap();
    assert!(matches!(other.read(&other_proof), Err(Error::Format)));
}

#[cfg(unix)]
#[test]
fn runner_session_refuses_corruption_unsafe_permissions_and_final_links() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let source = binding(2);
    let subject = subject();
    let mut file = RunnerSessionFile::acquire(directory.path(), &source, &subject).unwrap();
    file.save(&SessionRecord::from_session(session(&subject, "first"), &file).unwrap())
        .unwrap();
    drop(file);
    let path = directory.path().join("runner-session.v1");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut file = RunnerSessionFile::acquire(directory.path(), &source, &subject).unwrap();
    assert!(file.read(&subject).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut corrupted = Zeroizing::new(std::fs::read(&path).unwrap());
    corrupted[70] ^= 1;
    std::fs::write(&path, corrupted.as_slice()).unwrap();
    assert!(matches!(file.read(&subject), Err(Error::Format)));
    let alternate = directory.path().join("original");
    std::fs::rename(&path, &alternate).unwrap();
    symlink(&alternate, &path).unwrap();
    assert!(file.read(&subject).is_err());
    assert!(file.require_absent().is_err());
}
