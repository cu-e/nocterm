//! The keys of an account on a real hub: what one device keeps, another device reads.
//!
//! Two devices are two credential directories. They sign in with the same account, so the
//! second one opens the keyset the first one created. The records the tests write are removed
//! again, and the account is left as it was found.

use super::{Hub, hub, links, sign_in};
use futures::executor::block_on;
use nocterm_connections_model::{
    profiles::Profile,
    replica::{ProfileReplica as _, RemoteBody, ReplicaScope},
    test_support::profile_id_for,
};
use nocterm_hub_io::HubLinks;
use nocterm_hubs_model::{HubSignIn as _, KeyStatus, source_identity::LocalAccountSourceError};
use nocterm_session::{Auth, Secret};
use nocterm_vault::{CredentialBinding, VaultAccess as _};
use std::sync::Arc;

/// One device: its own credential directory, signed in with the account's password.
pub(super) struct Device {
    links: Arc<HubLinks>,
    login: nocterm_hubs_model::LoginReceipt,
    _dir: tempfile::TempDir,
}

impl Device {
    pub(super) fn sign_in(hub: &Hub) -> (Self, KeyStatus) {
        let dir = tempfile::tempdir().unwrap();
        let links = links(&dir);
        let signed = sign_in(&links, hub).expect("the account signs in");
        (
            Self {
                links,
                login: signed.login,
                _dir: dir,
            },
            signed.keys,
        )
    }

    pub(super) fn account(&self, hub: &Hub) -> nocterm_hub_io::HubAccount {
        self.links.open_account(&hub.id()).expect("an account")
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // The device is not left registered with the account.
        if let Some(hub) = hub() {
            let _ = block_on(self.links.sign_out(&hub.id()));
        }
    }
}

pub(super) fn unique(name: &str) -> String {
    format!("{name}-{}", uuid::Uuid::new_v4())
}

#[test]
fn the_keys_of_an_account_open_on_a_second_device() {
    let Some(hub) = hub() else { return };
    let (_first, keys) = Device::sign_in(&hub);
    // The first sign-in of an account makes the keys; any later one opens them.
    assert!(
        matches!(keys, KeyStatus::Created(_) | KeyStatus::Open),
        "{keys:?}"
    );
    let (second, keys) = Device::sign_in(&hub);
    assert!(matches!(keys, KeyStatus::Open), "{keys:?}");
    assert!(second.account(&hub).vault().is_unlocked());
}

#[test]
fn local_history_capture_survives_reconnect_but_not_lock_or_a_replacement_login() {
    let Some(hub) = hub() else { return };
    let (device, _) = Device::sign_in(&hub);
    let account = device.account(&hub);
    let source = account.local_source().unwrap();
    let partition = source.partition();
    block_on(device.links.connect(hub.spec())).unwrap();
    assert_eq!(source.check_current(), Ok(()));

    let vault = account.vault();
    vault.lock();
    assert!(
        device.login.check_current().is_ok(),
        "a key lock is not a new login"
    );
    assert_eq!(source.check_current(), Err(LocalAccountSourceError::Stale));
    block_on(vault.unlock(Secret::new(hub.password.clone()))).unwrap();
    let unlocked = account.local_source().unwrap();
    assert_eq!(unlocked.partition(), partition);
    assert_eq!(source.check_current(), Err(LocalAccountSourceError::Stale));

    sign_in(&device.links, &hub).unwrap();
    assert_eq!(
        unlocked.check_current(),
        Err(LocalAccountSourceError::Stale)
    );
    let next = account.local_source().unwrap();
    assert!(matches!(
        device.login.discard(),
        Err(nocterm_hubs_model::HubError::Stale)
    ));
    assert!(
        next.check_current().is_ok(),
        "old same-account cleanup preserves the replacement"
    );
    assert_eq!(next.partition(), partition);
    block_on(device.links.sign_out(&hub.id())).unwrap();
    assert_eq!(next.check_current(), Err(LocalAccountSourceError::Stale));
}

#[test]
fn a_credential_kept_on_one_device_is_read_on_another() {
    let Some(hub) = hub() else { return };
    let (first, _) = Device::sign_in(&hub);
    let (second, _) = Device::sign_in(&hub);
    let label = unique("live-credential");
    let binding = CredentialBinding::KeyPassphrase {
        path: "/live/key".into(),
    };

    let id = block_on(first.account(&hub).vault().put(
        None,
        label.clone(),
        binding.clone(),
        Secret::new("hunter2"),
    ))
    .unwrap();
    let vault = second.account(&hub).vault();
    let listed = block_on(vault.list()).unwrap();
    assert!(
        listed
            .iter()
            .any(|info| info.id == id && info.label == label)
    );
    let secret = block_on(vault.get(id, binding)).unwrap();
    assert_eq!(secret.expose(), "hunter2");

    block_on(first.account(&hub).vault().delete(id)).unwrap();
}

#[test]
fn captured_terminal_credentials_keep_the_original_account_and_expire_on_lock_and_login() {
    use nocterm_session::{CredentialStoreError, SecretRequest};
    let Some(hub) = hub() else { return };
    let (device, _) = Device::sign_in(&hub);
    let account = device.account(&hub);
    let source = account.local_source().unwrap();
    let store = account.local_credentials(source).unwrap();
    let request = SecretRequest::KeyPassphrase {
        path: "/live/captured-key".into(),
        retry: false,
    };
    let id = block_on(store.save(
        unique("captured-secret"),
        &request,
        Secret::new("original account"),
    ))
    .unwrap();
    block_on(device.links.connect(hub.spec())).unwrap();
    assert_eq!(
        block_on(store.get(id, &request)).unwrap().expose(),
        "original account"
    );
    account.vault().lock();
    assert!(!store.is_available());
    assert!(matches!(
        block_on(store.get(id, &request)),
        Err(CredentialStoreError::Stale)
    ));
    block_on(account.vault().unlock(Secret::new(hub.password.clone()))).unwrap();
    let next = account
        .local_credentials(account.local_source().unwrap())
        .unwrap();
    assert_eq!(
        block_on(next.get(id, &request)).unwrap().expose(),
        "original account"
    );
    sign_in(&device.links, &hub).unwrap();
    assert!(matches!(
        block_on(next.get(id, &request)),
        Err(CredentialStoreError::Stale)
    ));
    let current = account
        .local_credentials(account.local_source().unwrap())
        .unwrap();
    assert_eq!(
        block_on(current.get(id, &request)).unwrap().expose(),
        "original account"
    );
    block_on(account.vault().delete(id)).unwrap();
}

#[test]
fn a_profile_saved_on_one_device_is_read_on_another() {
    let Some(hub) = hub() else { return };
    let (first, _) = Device::sign_in(&hub);
    let (second, _) = Device::sign_in(&hub);
    let name = unique("live-profile");
    let profile = Profile {
        id: profile_id_for(&name),
        name: name.clone(),
        description: String::new(),
        options: Default::default(),
        target: nocterm_session::Target::new("ci", "live.example", 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    };

    let written = block_on(first.account(&hub).replica().unwrap().put(
        ReplicaScope::Personal,
        profile.clone(),
        None,
    ))
    .unwrap();
    let page = block_on(second.account(&hub).replica().unwrap().changes(None)).unwrap();
    let seen = page
        .changes
        .iter()
        .find(|change| change.id == profile.id)
        .expect("the other device sees the profile");
    assert_eq!(seen.body, RemoteBody::Present(Box::new(profile.clone())));

    block_on(first.account(&hub).replica().unwrap().delete(
        ReplicaScope::Personal,
        profile.id,
        written,
    ))
    .unwrap();
}

#[test]
fn captured_relay_cannot_follow_relogin_or_change_its_original_route() {
    use nocterm_session::{RelayError, Target};
    let Some(hub) = hub() else { return };
    let (device, _) = Device::sign_in(&hub);
    let account = device.account(&hub);
    let source = account.local_source().unwrap();
    let relay = account.local_relay(source).unwrap();
    let name = hub.address.to_string();
    let target = Target::new("relay", "example.com", 22);
    let captured = relay.capture(&name, None, &target).unwrap();
    assert!(matches!(
        captured.capture(&name, Some("substituted-node"), &target),
        Err(RelayError::CaptureUnsupported)
    ));
    let other = Target::new("another-principal", "example.com", 22);
    assert!(matches!(
        captured.capture(&name, None, &other),
        Err(RelayError::CaptureUnsupported)
    ));
    sign_in(&device.links, &hub).unwrap();
    let stale = super::within(captured.dial(&name, None, &target));
    assert!(
        matches!(stale, Err(RelayError::NotSignedIn { .. })),
        "an old native worker selected replacement membership"
    );
    let current = account
        .local_relay(account.local_source().unwrap())
        .unwrap();
    current.capture(&name, None, &target).unwrap();
}
#[test]
fn an_ordinary_relay_captures_before_its_future_is_polled() {
    use nocterm_hub_io::HubRelay;
    use nocterm_session::{RelayDialer as _, RelayError, Target};
    let Some(hub) = hub() else { return };
    let (device, _) = Device::sign_in(&hub);
    let relay = HubRelay::new(&device.links);
    let name = hub.address.to_string();
    let target = Target::new("relay", "example.com", 22);
    let original = relay.capture(&name, None, &target).unwrap();
    sign_in(&device.links, &hub).unwrap();
    assert!(matches!(
        super::within(original.dial(&name, None, &target)),
        Err(RelayError::NotSignedIn { .. })
    ));
    let current = relay.capture(&name, None, &target).unwrap();
    drop(current);
}
