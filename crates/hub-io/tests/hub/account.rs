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
use nocterm_hubs_model::{HubSignIn as _, KeyStatus};
use nocterm_session::{Auth, Secret};
use nocterm_vault::{CredentialBinding, VaultAccess as _};
use std::sync::Arc;

/// One device: its own credential directory, signed in with the account's password.
pub(super) struct Device {
    links: Arc<HubLinks>,
    _dir: tempfile::TempDir,
}

impl Device {
    pub(super) fn sign_in(hub: &Hub) -> (Self, KeyStatus) {
        let dir = tempfile::tempdir().unwrap();
        let links = links(&dir);
        let signed = sign_in(&links, hub).expect("the account signs in");
        (Self { links, _dir: dir }, signed.keys)
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

    let written = block_on(first.account(&hub).replica().put(
        ReplicaScope::Personal,
        profile.clone(),
        None,
    ))
    .unwrap();
    let page = block_on(second.account(&hub).replica().changes(None)).unwrap();
    let seen = page
        .changes
        .iter()
        .find(|change| change.id == profile.id)
        .expect("the other device sees the profile");
    assert_eq!(seen.body, RemoteBody::Present(Box::new(profile.clone())));

    block_on(
        first
            .account(&hub)
            .replica()
            .delete(ReplicaScope::Personal, profile.id, written),
    )
    .unwrap();
}
