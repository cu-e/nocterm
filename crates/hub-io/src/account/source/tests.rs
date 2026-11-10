use super::*;
use crate::{
    account::{Accounts, testing::*},
    cache::UnlockMethod,
};
use nocterm_hub_proto::v2;
use nocterm_keys::UnlockKey;

struct Fixture {
    _directory: tempfile::TempDir,
    slot: Arc<Slot>,
    subjects: Arc<ScriptedSubjects>,
    provider: LocalSourceProvider,
    record: v2::Keyset,
    unlock: UnlockKey,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let accounts = Accounts::new(directory.path().to_path_buf());
        let slot = accounts.slot(&hub()).unwrap();
        let unlock = unlock(PASSWORD);
        let (record, _) = record(&unlock);
        let subjects = Arc::new(ScriptedSubjects::for_slot(&slot));
        subjects.sign_in(hub(), ACCOUNT_ID, "device-1");
        slot.install(
            open(&record, &unlock),
            cache(&record, UnlockMethod::Passphrase),
        );
        let provider = LocalSourceProvider::new(slot.clone(), subjects.clone());
        Self {
            _directory: directory,
            slot,
            subjects,
            provider,
            record,
            unlock,
        }
    }
    fn capture(&self) -> Arc<dyn LocalAccountSource> {
        self.provider.capture().unwrap()
    }
    fn unlock(&self) {
        self.slot.unlocked(open(&self.record, &self.unlock));
    }
}

#[test]
fn local_partition_binds_the_configured_hub_and_the_account_but_not_its_keys() {
    let fixture = Fixture::new();
    let source = fixture.capture();
    let LocalAccountPartition::LegacyConfigured(partition) = source.partition() else {
        panic!("a hub window's history is partitioned by hub and account")
    };
    assert_eq!(partition.hub(), &hub());
    assert_eq!(partition.account().as_str(), ACCOUNT_ID);
    assert_eq!(source.check_current(), Ok(()));
    let membership = fixture.provider.membership().unwrap();
    fixture.slot.lock();
    assert_eq!(
        LocalAccountSource::partition(membership.as_ref()),
        source.partition(),
        "a locked vault keeps the same chats"
    );
    assert_eq!(
        LocalAccountSource::check_current(membership.as_ref()),
        Ok(())
    );
}

#[test]
fn locking_drops_open_keys_despite_retained_history_captures_and_providers() {
    let fixture = Fixture::new();
    let keys = Arc::downgrade(&fixture.slot.open().unwrap());
    let source = fixture.capture();
    fixture.slot.lock();
    assert_eq!(source.check_current(), Err(LocalAccountSourceError::Stale));
    assert!(
        keys.upgrade().is_none(),
        "the source cannot retain the secret owner"
    );
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::KeysLocked)
    ));
    fixture.unlock();
    assert_eq!(source.check_current(), Err(LocalAccountSourceError::Stale));
    let next = fixture.capture();
    assert_eq!(next.partition(), source.partition());
    assert_eq!(next.check_current(), Ok(()));
}

#[test]
fn a_same_account_relogin_invalidates_capture_even_before_keys_are_reinstalled() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    fixture.subjects.sign_in(hub(), ACCOUNT_ID, "device-1");
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::KeysLocked)
    ));
    fixture.unlock();
    let next = fixture.capture();
    assert_eq!(next.partition(), original.partition());
    assert_eq!(next.check_current(), Ok(()));
}

#[test]
fn an_account_round_trip_never_revives_an_old_capture() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    fixture
        .subjects
        .sign_in(hub(), "another-account", "device-2");
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::KeysLocked)
    ));
    fixture.subjects.sign_in(hub(), ACCOUNT_ID, "device-1");
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::KeysLocked)
    ));
    fixture.unlock();
    assert_eq!(fixture.capture().check_current(), Ok(()));
}

#[test]
fn pool_eviction_invalidates_capture_before_key_erasure_or_remote_work() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    fixture.subjects.evict(&hub());
    assert!(
        fixture.slot.open().is_some(),
        "sign-out's remote work has not erased keys yet"
    );
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::NotSignedIn)
    ));
    fixture.subjects.sign_in(hub(), ACCOUNT_ID, "device-1");
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
}

#[test]
fn ordinary_refresh_retains_the_capture_but_device_replacement_does_not() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    fixture.subjects.renew(hub(), ACCOUNT_ID, "device-1");
    assert_eq!(original.check_current(), Ok(()));
    fixture.subjects.renew(hub(), ACCOUNT_ID, "device-2");
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::KeysLocked)
    ));
    fixture.unlock();
    assert_eq!(fixture.capture().partition(), original.partition());
}

#[test]
fn replacing_keys_invalidates_capture_but_keeps_its_local_partition() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    let (replacement, _) = record(&fixture.unlock);
    fixture.slot.install(
        open(&replacement, &fixture.unlock),
        cache(&replacement, UnlockMethod::Passphrase),
    );
    assert_eq!(
        original.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    assert_eq!(fixture.capture().partition(), original.partition());
}

#[test]
fn a_retained_provider_can_wait_for_the_first_unlock_without_capturing_keys() {
    let fixture = Fixture::new();
    fixture.slot.lock();
    let provider = LocalSourceProvider::new(fixture.slot.clone(), fixture.subjects.clone());
    assert!(matches!(provider.capture(), Err(HubError::KeysLocked)));
    fixture.unlock();
    assert_eq!(provider.capture().unwrap().check_current(), Ok(()));
}

#[test]
fn a_capture_alone_keeps_neither_the_account_owner_nor_its_open_keys_alive() {
    let (source, keys) = {
        let fixture = Fixture::new();
        let keys = Arc::downgrade(&fixture.slot.open().unwrap());
        (fixture.capture(), keys)
    };
    assert!(keys.upgrade().is_none());
    assert_eq!(source.check_current(), Err(LocalAccountSourceError::Stale));
}

#[tokio::test]
async fn locked_membership_admits_unlock_but_an_explicit_lock_retires_its_receipt() {
    let fixture = Fixture::new();
    fixture.slot.lock();
    let membership = fixture.provider.membership().unwrap();
    let subject = membership.subject().unwrap();
    let receipt = membership
        .with_current(|| fixture.slot.receipt(&subject))
        .unwrap();
    assert!(membership.check().is_ok());
    fixture.slot.lock();
    let next = cache(&fixture.record, UnlockMethod::Passphrase);
    let keys = Arc::new(open(&fixture.record, &fixture.unlock));
    let draft = fixture
        .slot
        .stage_cache(fixture.slot.cache_snapshot(), next)
        .await
        .unwrap();
    let result =
        membership.with_current(|| fixture.slot.commit_install(&subject, &receipt, keys, draft));
    assert!(matches!(result, Err(HubError::Stale)));
    assert!(matches!(
        fixture.provider.capture(),
        Err(HubError::KeysLocked)
    ));
}

#[tokio::test]
async fn a_late_unlock_never_installs_keys_after_another_login_is_published() {
    let fixture = Fixture::new();
    fixture.slot.lock();
    let original = fixture.provider.membership().unwrap();
    let subject = original.subject().unwrap();
    let receipt = fixture.slot.receipt(&subject).unwrap();
    let next = cache(&fixture.record, UnlockMethod::Passphrase);
    let keys = Arc::new(open(&fixture.record, &fixture.unlock));
    let draft = fixture
        .slot
        .stage_cache(fixture.slot.cache_snapshot(), next)
        .await
        .unwrap();
    fixture.subjects.sign_in(hub(), ACCOUNT_ID, "device-1");
    let result =
        original.with_current(|| fixture.slot.commit_install(&subject, &receipt, keys, draft));
    assert!(matches!(result, Err(HubError::Stale)));
    assert!(fixture.slot.open().is_none());
    fixture.unlock();
    assert!(fixture.provider.capture().is_ok());
    assert!(matches!(original.check(), Err(HubError::Stale)));
}
