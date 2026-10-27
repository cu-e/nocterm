use nocterm_keys::OpenError;

use super::*;

#[test]
fn a_device_that_loses_the_race_opens_the_keyset_that_exists() {
    assert!(matches!(
        creation(Err(KeysetError::AlreadyExists)),
        Ok(Creation::Raced)
    ));
}

#[test]
fn other_creation_failures_stay_failures() {
    assert!(matches!(
        creation(Err(KeysetError::Incomplete)),
        Err(KeysetError::Incomplete)
    ));
}

#[test]
fn a_keyset_the_hub_altered_is_reported_as_damaged_not_overwritten() {
    for error in [
        KeysetError::Damaged(OpenError::Rejected),
        KeysetError::Incomplete,
        KeysetError::WrongUnlockKey,
    ] {
        assert!(matches!(
            keyset_error(error),
            HubError::KeysetDamaged { .. }
        ));
    }
}
