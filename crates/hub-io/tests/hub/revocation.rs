//! Actual server revocation reaches the native conditional cleanup boundary.
use super::{Hub, hub, sign_in};
use futures::executor::block_on;
use nocterm_connections_model::replica::ProfileReplica as _;
use nocterm_hub_client::{DeviceInfo, HubClient, HubEndpoint, Trust};
use nocterm_hub_io::{HubLinks, HubLinksConfig};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::HubSignIn as _;
use nocterm_keys::DeviceKey;
use nocterm_vault::{VaultAccess as _, VaultError, VaultStatus};
use tokio::runtime::Runtime;

/// A second real device of the same account, removed again even if an assertion fails.
struct Revoker {
    runtime: Runtime,
    client: HubClient,
    device: String,
}
impl Revoker {
    fn sign_in(hub: &Hub) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let endpoint = HubEndpoint::with_trust(
            &hub.address.to_string(),
            Trust::Pinned(*hub.fingerprint.as_bytes()),
        )
        .unwrap();
        let client = runtime
            .block_on(HubClient::connect(
                &endpoint,
                DeviceKey::generate().unwrap(),
                None,
            ))
            .unwrap();
        let device = DeviceInfo::new(super::account::unique("revoker"), "Nocterm test");
        let signed = runtime
            .block_on(client.password_login(&hub.username, &hub.password, &device))
            .unwrap();
        Self {
            runtime,
            client,
            device: signed.session.device_id,
        }
    }
    fn revoke_named(&self, name: &str) {
        self.runtime.block_on(async {
            let listed = self
                .client
                .directory()
                .list_devices(v1::Empty {})
                .await
                .unwrap()
                .into_inner();
            let device = listed
                .devices
                .into_iter()
                .find(|device| device.name == name)
                .expect("the native device is registered");
            self.client
                .directory()
                .revoke_device(v1::RevokeDeviceRequest {
                    device_id: device.device_id.clone(),
                })
                .await
                .unwrap();
            let listed = self
                .client
                .directory()
                .list_devices(v1::Empty {})
                .await
                .unwrap()
                .into_inner();
            assert!(
                listed
                    .devices
                    .iter()
                    .any(|known| known.device_id == device.device_id && known.revoked),
                "the server confirms the native device is revoked"
            );
        });
    }
}
impl Drop for Revoker {
    fn drop(&mut self) {
        let _ = self.runtime.block_on(self.client.directory().revoke_device(
            v1::RevokeDeviceRequest {
                device_id: self.device.clone(),
            },
        ));
    }
}

#[test]
fn current_server_revocation_erases_only_the_original_native_login_and_keys() {
    let Some(hub) = hub() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let label = super::account::unique("native-revoked");
    let links = HubLinks::start(HubLinksConfig {
        credentials_dir: directory.path().join("hubs"),
        device_name: label.clone(),
    })
    .unwrap();
    let signed = sign_in(&links, &hub).unwrap();
    let account = links.open_account(&hub.id()).unwrap();
    let source = account.local_source().unwrap();
    let vault = account.vault();
    let revoker = Revoker::sign_in(&hub);
    revoker.revoke_named(&DeviceInfo::new(label, "Nocterm").display_name());
    let replica = account.replica().unwrap();
    let denied = block_on(vault.list());
    let remote = block_on(replica.changes(None)).map(|page| page.changes.len());
    let source_current = source.check_current().is_ok();
    assert!(
        matches!(&denied, Err(VaultError::Stale)),
        "native vault outcome: {:?}; remote-only replica outcome: {remote:?}; original source current: {source_current}",
        denied.as_ref().map(Vec::len)
    );
    assert!(
        matches!(vault.status(), VaultStatus::Missing | VaultStatus::Locked),
        "confirmed auth rejection closes native keys; untyped expiry may retain ciphertext"
    );
    assert!(source.check_current().is_err());
    assert!(signed.login.check_current().is_err());
    assert!(!block_on(links.connect(hub.spec())).unwrap().signed_in);
}
