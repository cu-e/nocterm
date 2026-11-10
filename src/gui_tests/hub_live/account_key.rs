//! A key one device keeps in the account vault signs in to a real `sshd` from another device.
//!
//! Needs the real hub of the parent module and an `sshd` binary; without `sshd` the test
//! passes vacuously, as the SSH suite does. The records it writes are removed again.

use super::{Hub, hub};
use futures::executor::block_on;
use nocterm_hub_io::{HubAccount, HubLinks, HubLinksConfig};
use nocterm_hubs_model::HubSignIn as _;
use nocterm_session::{Auth, Prompt, Secret, SecretRequest, Transport as _};
use nocterm_ssh_keys::KeyMaterial;
use nocterm_vault::{CredentialBinding, VaultAccess as _};
use std::sync::Arc;

/// The shared real `sshd` fixture of the SSH suite. This file uses a part of it.
#[allow(
    dead_code,
    reason = "the shared fixture offers more than this suite uses"
)]
#[path = "../../../crates/ssh/tests/support/sshd.rs"]
mod sshd;

/// One device: its own credential directory, signed in with the account's password.
struct Device {
    links: Arc<HubLinks>,
    _dir: tempfile::TempDir,
}

impl Device {
    fn sign_in(hub: &Hub) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let links = HubLinks::start(HubLinksConfig {
            credentials_dir: dir.path().join("hubs"),
            device_name: "gui test".into(),
        })
        .unwrap();
        let spec = nocterm_hubs_model::HubEndpointSpec {
            address: hub.address.clone(),
            trust: nocterm_hubs_model::TrustPolicy::Pinned(hub.fingerprint),
        };
        // The hub limits how often an account signs in; a refusal for that reason is waited out.
        for _ in 0..40 {
            let signed = block_on(links.password(
                spec.clone(),
                hub.username.clone(),
                Secret::new(hub.password.clone()),
            ));
            match signed {
                Err(error) if format!("{error:?}").contains("ResourceExhausted") => {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Ok(_) => return Self { links, _dir: dir },
                Err(error) => panic!("the account signs in: {error:?}"),
            }
        }
        panic!("the hub kept refusing to sign {} in", hub.username)
    }

    fn account(&self, hub: &Hub) -> HubAccount {
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

/// Waits for `future` for a while: a server that never answers must fail the test.
fn within<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), future)
                .await
                .expect("no answer in time")
        })
}

#[test]
fn a_key_kept_on_one_device_signs_in_to_a_server_from_another() {
    let Some(hub) = hub() else { return };
    let Some(sshd) = sshd::Sshd::start() else {
        return;
    };
    let text = std::fs::read_to_string(sshd.root().join("plain/id_ed25519")).unwrap();
    let key = KeyMaterial::import(&Secret::new(text), None).unwrap();
    let public_key = key.public_key().as_str().to_owned();
    let binding = CredentialBinding::PrivateKey {
        public_key: public_key.clone(),
    };

    // Device 1 keeps the key; the server trusts its public half.
    let first = Device::sign_in(&hub);
    let id = block_on(first.account(&hub).vault().put(
        None,
        format!("live-key-{}", uuid::Uuid::new_v4()),
        binding.clone(),
        key.to_secret(),
    ))
    .unwrap();

    // Device 2 knows nothing but the account, and still signs in with the key.
    let second = Device::sign_in(&hub);
    let from_account = block_on(second.account(&hub).vault().get(id, binding)).unwrap();
    let transport = sshd.transport("empty");
    let mut request = sshd.request();
    request.auth = Auth::VaultKey {
        credential: id,
        public_key,
    };
    let session = transport.open(request);
    within(sshd::connect(&session, |prompt| match prompt {
        Prompt::Secret {
            request: SecretRequest::PrivateKey { .. },
            reply,
            ..
        } => reply.send(Some(from_account.clone())),
        other => sshd::accept(other),
    }));
    drop(session);
    block_on(first.account(&hub).vault().delete(id)).unwrap();
}
