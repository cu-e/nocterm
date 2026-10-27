use super::*;
use nocterm_connections_model::replica::ReplicaCursor;

fn state() -> ReplicaState {
    let mut state = ReplicaState::default();
    state.advance(ReplicaCursor::new("7"));
    state
}

#[test]
fn a_saved_state_is_read_back() {
    let directory = tempfile::tempdir().unwrap();
    let file = FileReplicaState::new(directory.path().join("replica.json"));
    file.save(&state()).unwrap();
    assert_eq!(file.load(), state());
}

#[test]
fn a_missing_file_starts_from_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let file = FileReplicaState::new(directory.path().join("replica.json"));
    assert_eq!(file.load(), ReplicaState::default());
}

#[test]
fn a_file_that_is_not_a_state_starts_from_nothing_and_is_replaced_by_the_next_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.json");
    std::fs::write(&path, b"{ not json").unwrap();
    let file = FileReplicaState::new(path);
    assert_eq!(file.load(), ReplicaState::default());
    file.save(&state()).unwrap();
    assert_eq!(file.load(), state());
}

#[cfg(unix)]
#[test]
fn the_file_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.json");
    FileReplicaState::new(path.clone()).save(&state()).unwrap();
    let mode = std::fs::metadata(path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0);
}
