use nocterm_keys::OpenError;
mod source_binding;

use super::*;
use crate::account::Slot;

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

pub(super) mod race {
    use std::path::Path;

    use nocterm_vault::VaultStatus;
    use tempfile::TempDir;

    use super::*;
    use crate::account::testing::{ACCOUNT_ID, HUB_UID, MemoryKeysets, PASSWORD, hub};
    use crate::cache::KeysetCopy;

    const PASSPHRASE: &str = "a long vault passphrase";

    /// One device of the account: its own cache directory and its own key flow.
    pub(super) struct Device {
        pub(super) flow: Arc<KeyFlow>,
        subjects: Arc<crate::account::testing::ScriptedSubjects>,
        accounts: Arc<Accounts>,
        dir: TempDir,
    }

    impl Device {
        pub(super) fn new() -> Self {
            Self::in_dir(tempfile::tempdir().unwrap())
        }

        fn in_dir(dir: TempDir) -> Self {
            let accounts = Arc::new(Accounts::new(dir.path().to_path_buf()));
            let subjects = Arc::new(crate::account::testing::ScriptedSubjects::for_slot(
                &accounts.slot(&hub()).unwrap(),
            ));
            Self {
                flow: KeyFlow::new(
                    pool(dir.path(), &accounts),
                    Arc::clone(&accounts),
                    Handle::current(),
                ),
                subjects,
                accounts,
                dir,
            }
        }

        /// The same device after the client was closed and opened again: what it kept on disk
        /// stays, what it held in memory is gone.
        pub(super) fn restarted(self) -> Self {
            Self::in_dir(self.dir)
        }

        pub(super) fn slot(&self) -> Arc<Slot> {
            self.accounts.slot(&hub()).unwrap()
        }

        pub(super) fn owner(&self) -> Arc<KeyAdmission> {
            if self.subjects.current(&hub()).is_none() {
                self.subjects.sign_in(hub(), ACCOUNT_ID, "device");
            }
            let provider = LocalSourceProvider::new(self.slot(), self.subjects.clone());
            KeyAdmission::scripted(provider.membership().unwrap())
        }
        pub(super) fn confirm(&self) {
            let owner = self.owner();
            self.flow
                .offer(&owner, Secret::new("displayed fixture key"))
                .unwrap()
                .confirm()
                .unwrap();
        }

        pub(super) async fn sign_in(&self, hub_keys: &MemoryKeysets, password: &str) -> KeyStatus {
            self.subjects.sign_in(hub(), ACCOUNT_ID, "device");
            let owner = self.owner();
            let secrets = LoginSecrets::derive(password, HUB_UID, "ada").unwrap();
            self.flow
                .after_password(&owner, hub_keys, ACCOUNT_ID, "ada", secrets)
                .await
                .unwrap()
        }

        async fn with_passphrase(&self, hub_keys: &MemoryKeysets, passphrase: &str) -> KeyStatus {
            let passphrase = Secret::new(passphrase);
            self.flow
                .open_with_passphrase(&self.owner(), hub_keys, ACCOUNT_ID, passphrase)
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
            .own_collection("shared", &envelope);
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
    async fn a_keyset_sealed_with_the_login_password_is_not_opened_with_a_passphrase() {
        let hub_keys = MemoryKeysets::default();
        let (first, second) = (Device::new(), Device::new());
        assert!(is_created(&first.sign_in(&hub_keys, PASSWORD).await));

        let refused = second
            .flow
            .open_with_passphrase(
                &second.owner(),
                &hub_keys,
                ACCOUNT_ID,
                Secret::new(PASSPHRASE),
            )
            .await;

        assert!(matches!(
            refused,
            Err(HubError::UnlockSourceMismatch {
                needs_login_password: true
            })
        ));
        assert!(second.slot().open().is_none());
        assert!(
            second.slot().recovery().is_none(),
            "a wrong source is not a wrong password: no recovery key is asked for"
        );
    }

    #[tokio::test]
    async fn a_keyset_sealed_with_a_vault_password_asks_for_it_instead_of_the_login_password() {
        let hub_keys = MemoryKeysets::default();
        let (first, second) = (Device::new(), Device::new());
        first.with_passphrase(&hub_keys, PASSPHRASE).await;

        let status = second.sign_in(&hub_keys, PASSWORD).await;

        assert_eq!(status, KeyStatus::PassphraseNeeded);
        assert!(second.slot().open().is_none());
        assert!(second.slot().recovery().is_none());
        assert_eq!(
            second.with_passphrase(&hub_keys, PASSPHRASE).await,
            KeyStatus::Open
        );
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
            .recover_with(&loser.owner(), &hub_keys, created.key().clone())
            .await;
        assert!(recovered.is_ok());
        assert_eq!(loser.slot().status(), VaultStatus::Unlocked);
    }
}

mod reissue {
    use nocterm_keys::{RecoveryKey, UnlockKey};

    use super::race::Device;
    use super::*;
    use crate::account::testing::{ACCOUNT_ID, HUB_UID, MemoryKeysets, PASSWORD, hub};

    /// A device that created the account's keys, and the recovery key it was shown.
    async fn created() -> (Device, MemoryKeysets, RecoveryKey) {
        let (device, hub_keys) = (Device::new(), MemoryKeysets::default());
        let KeyStatus::Created(shown) = device.sign_in(&hub_keys, PASSWORD).await else {
            panic!("a first sign-in creates the keys");
        };
        let key = RecoveryKey::parse(shown.key().expose()).unwrap();
        (device, hub_keys, key)
    }

    async fn reissue(device: &Device, hub_keys: &MemoryKeysets) -> Result<RecoveryKey, HubError> {
        let slot = device.slot();
        let keys = slot.open().ok_or(HubError::KeysLocked)?;
        let shown = device
            .flow
            .reissue_with(&device.owner(), hub_keys, keys.keyset())
            .await?;
        Ok(RecoveryKey::parse(shown.expose()).unwrap())
    }

    fn opens_with(hub_keys: &MemoryKeysets, key: &RecoveryKey) -> bool {
        Keyset::recover(&hub_keys.stored().unwrap(), ACCOUNT_ID, key).is_ok()
    }

    fn password_opens(hub_keys: &MemoryKeysets) -> bool {
        let secrets = LoginSecrets::derive(PASSWORD, HUB_UID, "ada").unwrap();
        let unlock: UnlockKey = secrets.into_unlock();
        Keyset::open(&hub_keys.stored().unwrap(), ACCOUNT_ID, &unlock).is_ok()
    }

    #[tokio::test]
    async fn a_recovery_key_is_unconfirmed_from_its_creation_until_the_person_confirms_it() {
        let (device, ..) = created().await;
        assert!(device.flow.recovery_key_unconfirmed(&hub()));

        device.confirm();

        assert!(!device.flow.recovery_key_unconfirmed(&hub()));
    }

    #[tokio::test]
    async fn an_unconfirmed_key_is_offered_again_when_the_keys_open_after_a_restart() {
        let (device, hub_keys, _) = created().await;

        let device = device.restarted();
        assert!(
            !device.flow.recovery_key_unconfirmed(&hub()),
            "nothing is open before the password opens the keys"
        );
        let status = device.sign_in(&hub_keys, PASSWORD).await;
        assert!(matches!(status, KeyStatus::RecoveryKeyUnconfirmed(_)));
        assert!(device.flow.recovery_key_unconfirmed(&hub()));

        device.confirm();
        let device = device.restarted();
        assert_eq!(device.sign_in(&hub_keys, PASSWORD).await, KeyStatus::Open);
    }

    #[tokio::test]
    async fn another_device_of_the_account_does_not_inherit_the_marker() {
        let (_first, hub_keys, _) = created().await;

        let second = Device::new();

        assert_eq!(second.sign_in(&hub_keys, PASSWORD).await, KeyStatus::Open);
    }

    #[tokio::test]
    async fn reissuing_replaces_the_recovery_key_the_hub_can_open_and_keeps_the_password() {
        let (device, hub_keys, old) = created().await;
        device.confirm();
        let version = hub_keys.stored().unwrap().version;

        let new = reissue(&device, &hub_keys).await.unwrap();

        assert!(opens_with(&hub_keys, &new));
        assert!(!opens_with(&hub_keys, &old), "the old key no longer opens");
        assert!(password_opens(&hub_keys), "the password still opens it");
        let stored = hub_keys.stored().unwrap();
        assert_eq!(stored.version, version + 1);
        assert_eq!(
            device
                .slot()
                .read_cache(|cache| cache.unwrap().keyset.clone()),
            KeysetCopy::of(&stored),
            "the cache holds the record the hub now has"
        );
    }

    #[tokio::test]
    async fn a_reissued_key_is_unconfirmed_until_confirmed_and_survives_a_restart() {
        let (device, hub_keys, _) = created().await;
        device.confirm();

        reissue(&device, &hub_keys).await.unwrap();
        assert!(device.flow.recovery_key_unconfirmed(&hub()));

        let device = device.restarted();
        assert!(matches!(
            device.sign_in(&hub_keys, PASSWORD).await,
            KeyStatus::RecoveryKeyUnconfirmed(_)
        ));
    }

    #[tokio::test]
    async fn a_write_that_loses_to_another_device_is_repeated_against_the_new_version() {
        let (device, hub_keys, _) = created().await;
        let version = hub_keys.stored().unwrap().version;
        hub_keys.contend_recovery_writes(2);

        let new = reissue(&device, &hub_keys).await.unwrap();

        assert!(opens_with(&hub_keys, &new));
        assert_eq!(
            hub_keys.stored().unwrap().version,
            version + 3,
            "two writes of the other device and then ours"
        );
    }

    #[tokio::test]
    async fn a_keyset_that_keeps_changing_leaves_the_old_key_valid_and_the_marker_as_it_was() {
        let (device, hub_keys, old) = created().await;
        device.confirm();
        hub_keys.contend_recovery_writes(100);

        let result = reissue(&device, &hub_keys).await;

        assert!(result.is_err());
        assert!(opens_with(&hub_keys, &old), "nothing was replaced");
        assert!(!device.flow.recovery_key_unconfirmed(&hub()));
    }

    #[tokio::test]
    async fn reissuing_with_closed_keys_is_refused() {
        let (device, ..) = created().await;
        let device = device.restarted();

        let result = device.flow.reissue_recovery_key(&device.owner()).await;

        assert!(matches!(result, Err(HubError::KeysLocked)));
    }

    #[tokio::test]
    async fn a_record_of_another_key_pair_is_refused_and_not_written_over() {
        let (device, hub_keys, _) = created().await;
        hub_keys.edit_record(|record| {
            record.public_key = nocterm_keys::AccountKeyPair::generate()
                .unwrap()
                .public_key()
                .to_bytes()
                .to_vec();
        });
        let before = hub_keys.stored().unwrap();

        let result = reissue(&device, &hub_keys).await;

        assert!(matches!(result, Err(HubError::KeysetDamaged { .. })));
        assert_eq!(hub_keys.stored().unwrap(), before);
    }

    #[tokio::test]
    async fn typing_the_recovery_key_confirms_that_the_person_holds_it() {
        let (device, hub_keys, key) = created().await;
        let device = device.restarted();
        let other = device.sign_in(&hub_keys, "another long password").await;
        assert_eq!(other, KeyStatus::RecoveryNeeded);

        device
            .flow
            .recover_with(
                &device.owner(),
                &hub_keys,
                Secret::new(key.display().as_str()),
            )
            .await
            .unwrap();

        assert!(!device.flow.recovery_key_unconfirmed(&hub()));
    }
}
