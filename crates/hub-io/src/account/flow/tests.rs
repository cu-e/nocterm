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

mod race {
    use std::path::Path;

    use nocterm_vault::VaultStatus;
    use tempfile::TempDir;

    use super::*;
    use crate::account::testing::{ACCOUNT_ID, HUB_UID, MemoryKeysets, PASSWORD, hub};
    use crate::cache::KeysetCopy;

    const PASSPHRASE: &str = "a long vault passphrase";

    /// One device of the account: its own cache directory and its own key flow.
    struct Device {
        flow: KeyFlow,
        accounts: Arc<Accounts>,
        _dir: TempDir,
    }

    impl Device {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let accounts = Arc::new(Accounts::new(dir.path().to_path_buf()));
            Self {
                flow: KeyFlow::new(pool(dir.path(), &accounts), Arc::clone(&accounts)),
                accounts,
                _dir: dir,
            }
        }

        fn slot(&self) -> Arc<Slot> {
            self.accounts.slot(&hub()).unwrap()
        }

        async fn sign_in(&self, hub_keys: &MemoryKeysets, password: &str) -> KeyStatus {
            let secrets = LoginSecrets::derive(password, HUB_UID, "ada").unwrap();
            self.flow
                .after_password(&hub(), hub_keys, ACCOUNT_ID, "ada", secrets)
                .await
                .unwrap()
        }

        async fn with_passphrase(&self, hub_keys: &MemoryKeysets, passphrase: &str) -> KeyStatus {
            let passphrase = Secret::new(passphrase);
            self.flow
                .open_with_passphrase(&hub(), hub_keys, ACCOUNT_ID, passphrase)
                .await
                .unwrap()
        }
    }

    fn pool(dir: &Path, accounts: &Arc<Accounts>) -> Arc<Pool> {
        Arc::new(Pool::new(dir.to_path_buf(), "test", Arc::clone(accounts)))
    }

    /// Both devices read "no keyset" before either creates one.
    fn racing_hub() -> MemoryKeysets {
        let hub_keys = MemoryKeysets::default();
        hub_keys.meet_before_creating(2);
        hub_keys
    }

    fn count(statuses: [&KeyStatus; 2], wanted: impl Fn(&KeyStatus) -> bool) -> usize {
        statuses.into_iter().filter(|status| wanted(status)).count()
    }

    fn is_created(status: &KeyStatus) -> bool {
        matches!(status, KeyStatus::Created(_))
    }

    #[tokio::test]
    async fn two_devices_creating_the_keyset_at_once_end_with_one_keyset_and_one_recovery_key() {
        let hub_keys = racing_hub();
        let (first, second) = (Device::new(), Device::new());

        let (a, b) = tokio::join!(
            first.sign_in(&hub_keys, PASSWORD),
            second.sign_in(&hub_keys, PASSWORD)
        );

        assert_eq!(hub_keys.creations(), 2, "both devices tried to create it");
        assert_eq!(count([&a, &b], is_created), 1, "{a:?} {b:?}");
        assert_eq!(count([&a, &b], |s| *s == KeyStatus::Open), 1, "{a:?} {b:?}");
        let stored = hub_keys.stored().unwrap();
        assert_eq!(stored.version, 1, "the loser did not write over the keyset");
        for device in [&first, &second] {
            let slot = device.slot();
            assert_eq!(slot.status(), VaultStatus::Unlocked);
            let cached = slot.read_cache(|cache| cache.map(|cache| cache.keyset.clone()));
            assert_eq!(
                cached,
                Some(KeysetCopy::of(&stored)),
                "both keep the winner's"
            );
        }
    }

    #[tokio::test]
    async fn the_device_that_lost_the_race_holds_the_keys_of_the_one_that_won() {
        let hub_keys = racing_hub();
        let (first, second) = (Device::new(), Device::new());
        tokio::join!(
            first.sign_in(&hub_keys, PASSWORD),
            second.sign_in(&hub_keys, PASSWORD)
        );

        let (_, envelope) = first
            .slot()
            .open()
            .unwrap()
            .new_collection("shared")
            .unwrap();
        let opened = second
            .slot()
            .open()
            .unwrap()
            .collection("shared", &envelope);
        assert!(
            opened.is_ok(),
            "one account key opens what the other sealed"
        );
    }

    #[tokio::test]
    async fn a_vault_passphrase_that_loses_the_race_opens_the_keyset_that_won() {
        // Each device chose a salt of its own before it knew of the other: the loser must
        // derive the key again with the salt of the keyset that exists.
        let hub_keys = racing_hub();
        let (first, second) = (Device::new(), Device::new());

        let (a, b) = tokio::join!(
            first.with_passphrase(&hub_keys, PASSPHRASE),
            second.with_passphrase(&hub_keys, PASSPHRASE)
        );

        assert_eq!(count([&a, &b], is_created), 1, "{a:?} {b:?}");
        assert_eq!(count([&a, &b], |s| *s == KeyStatus::Open), 1, "{a:?} {b:?}");
        for device in [&first, &second] {
            assert_eq!(device.slot().status(), VaultStatus::Unlocked);
        }
    }

    #[tokio::test]
    async fn a_device_with_another_password_loses_the_race_and_waits_for_the_recovery_key() {
        let hub_keys = racing_hub();
        let (first, second) = (Device::new(), Device::new());

        let (a, b) = tokio::join!(
            first.sign_in(&hub_keys, PASSWORD),
            second.sign_in(&hub_keys, "another long password")
        );

        let (winner, loser, created, waiting) = match (&a, &b) {
            (KeyStatus::Created(key), KeyStatus::RecoveryNeeded) => (&first, &second, key, &b),
            (KeyStatus::RecoveryNeeded, KeyStatus::Created(key)) => (&second, &first, key, &a),
            other => panic!("one creates, the other waits for the recovery key: {other:?}"),
        };
        assert_eq!(*waiting, KeyStatus::RecoveryNeeded);
        assert_eq!(winner.slot().status(), VaultStatus::Unlocked);
        assert!(
            loser.slot().open().is_none(),
            "nothing opens without the key"
        );

        // The recovery key made by the winner opens the loser's keys.
        let recovered = loser
            .flow
            .recover_with(&hub(), &hub_keys, created.clone())
            .await;
        assert!(recovered.is_ok());
        assert_eq!(loser.slot().status(), VaultStatus::Unlocked);
    }
}
