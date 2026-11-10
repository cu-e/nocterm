//! Native deferred admission uses the original private-file generation.
use super::*;
use crate::{account::Accounts, credentials::CredentialDir};
use nocterm_hub_client::Session;
use nocterm_hubs_model::{Fingerprint, HubAddress};

struct Device {
    _dir: tempfile::TempDir,
    file: CredentialFile,
    connector: Connector,
    spec: HubEndpointSpec,
}
impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let address = HubAddress::parse("127.0.0.1:1").unwrap();
        let accounts = Arc::new(Accounts::new(dir.path().to_owned()));
        let pool = Arc::new(Pool::new(dir.path().to_owned(), "test", accounts.clone()));
        let keys = KeyFlow::new(pool.clone(), accounts, tokio::runtime::Handle::current());
        let file = CredentialDir::new(dir.path().to_owned())
            .file(&address)
            .unwrap();
        Self {
            _dir: dir,
            file,
            connector: Connector::new(pool, keys),
            spec: HubEndpointSpec {
                address,
                trust: TrustPolicy::Pinned(Fingerprint::from_bytes([7; 32])),
            },
        }
    }
    fn login(&self, name: &str) {
        self.file.load_or_create().unwrap();
        self.file
            .save_session(&Session {
                account_id: name.into(),
                device_id: "device".into(),
                access_token: format!("access-{name}"),
                refresh_token: format!("refresh-{name}"),
                access_expires_at_ms: 10,
                refresh_expires_at_ms: 20,
            })
            .unwrap();
    }
    fn account(&self) -> String {
        self.file
            .load()
            .unwrap()
            .unwrap()
            .session
            .unwrap()
            .account_id
    }
}
#[tokio::test]
async fn a_login_queued_without_credentials_never_adopts_a_newer_private_file() {
    let device = Device::new();
    let start = device.connector.begin(device.spec.clone()).unwrap();
    device.login("B");
    assert!(matches!(
        device
            .connector
            .password(start, "A".into(), Secret::new("password"))
            .await,
        Err(HubError::Stale)
    ));
    assert_eq!(device.account(), "B");
}
#[tokio::test]
async fn a_login_queued_with_a_session_cannot_replace_another_login_of_the_same_account() {
    let device = Device::new();
    device.login("A");
    let start = device.connector.begin(device.spec.clone()).unwrap();
    let original = device.file.load().unwrap().unwrap().login;
    device.login("A");
    let current = device.file.load().unwrap().unwrap().login;
    assert_ne!(original, current);
    assert!(matches!(
        device
            .connector
            .password(start, "A".into(), Secret::new("password"))
            .await,
        Err(HubError::Stale)
    ));
    assert_eq!(device.file.load().unwrap().unwrap().login, current);
}
#[tokio::test]
async fn an_offline_sign_out_captured_before_a_new_login_preserves_that_login() {
    let device = Device::new();
    device.login("A");
    let sign_out = device
        .connector
        .sign_out(&HubId::from(device.spec.address.clone()))
        .unwrap();
    device.login("B");
    assert!(matches!(
        sign_out.start(&tokio::runtime::Handle::current()).await,
        Err(HubError::Stale)
    ));
    assert_eq!(device.account(), "B");
}
