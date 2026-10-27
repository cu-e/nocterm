//! End-to-end tests against a real hub.
//!
//! They need a running hub (`docker compose up` in the nocterm-server repository) and these
//! variables: `NOCTERM_HUB_ADDRESS`, `NOCTERM_HUB_FINGERPRINT` (SHA-256 of its certificate),
//! `NOCTERM_HUB_USERNAME` and `NOCTERM_HUB_PASSWORD`. Without `NOCTERM_REQUIRE_HUB` the tests
//! pass vacuously when the variables are missing; with it, missing variables fail the run, so
//! CI cannot silently skip them.

use futures::executor::block_on;
use nocterm_hub_io::{HubLinks, HubLinksConfig};
use nocterm_hubs_model::{
    Fingerprint, HubAddress, HubEndpointSpec, HubError, HubId, HubSessions as _, HubSignIn as _,
    HubTerminalTarget, HubTrust as _, TrustPolicy,
};
use nocterm_session::{CloseReason, ConnectRequest, Event, Secret, Session};

struct Hub {
    address: HubAddress,
    fingerprint: Fingerprint,
    username: String,
    password: String,
}

impl Hub {
    fn spec(&self) -> HubEndpointSpec {
        HubEndpointSpec {
            address: self.address.clone(),
            trust: TrustPolicy::Pinned(self.fingerprint),
        }
    }

    fn id(&self) -> HubId {
        HubId::from(self.address.clone())
    }
}

fn hub() -> Option<Hub> {
    let variable = |name: &str| std::env::var(name).ok();
    let settings = (
        variable("NOCTERM_HUB_ADDRESS"),
        variable("NOCTERM_HUB_FINGERPRINT"),
        variable("NOCTERM_HUB_USERNAME"),
        variable("NOCTERM_HUB_PASSWORD"),
    );
    let (Some(address), Some(fingerprint), Some(username), Some(password)) = settings else {
        assert!(
            std::env::var_os("NOCTERM_REQUIRE_HUB").is_none(),
            "NOCTERM_REQUIRE_HUB is set but NOCTERM_HUB_ADDRESS, NOCTERM_HUB_FINGERPRINT, \
             NOCTERM_HUB_USERNAME and NOCTERM_HUB_PASSWORD are not all set"
        );
        return None;
    };
    Some(Hub {
        address: HubAddress::parse(&address).expect("NOCTERM_HUB_ADDRESS"),
        fingerprint: Fingerprint::parse(&fingerprint).expect("NOCTERM_HUB_FINGERPRINT"),
        username,
        password,
    })
}

fn links(dir: &tempfile::TempDir) -> std::sync::Arc<HubLinks> {
    HubLinks::start(HubLinksConfig {
        credentials_dir: dir.path().join("hubs"),
        device_name: "hub-io test".into(),
    })
    .unwrap()
}

#[test]
fn probe_shows_the_certificate_that_was_pinned() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let probed = block_on(links(&dir).probe(&hub.address)).unwrap();
    assert_eq!(probed.fingerprint, hub.fingerprint);
}

#[test]
fn another_pinned_certificate_is_a_changed_certificate() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let wrong = Fingerprint::from_bytes([0xAB; 32]);
    let spec = HubEndpointSpec {
        address: hub.address.clone(),
        trust: TrustPolicy::Pinned(wrong),
    };
    match block_on(links(&dir).connect(spec)) {
        Err(HubError::CertificateChanged {
            pinned, presented, ..
        }) => {
            assert_eq!(pinned, wrong);
            assert_eq!(presented, hub.fingerprint);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn sign_in_survives_a_restart_and_sign_out_ends_it() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();

    let first = links(&dir);
    let info = block_on(first.connect(hub.spec())).unwrap();
    assert!(!info.signed_in);
    assert!(info.password_login);

    let refused = block_on(first.password(hub.spec(), hub.username.clone(), Secret::new("wrong")));
    assert!(
        matches!(refused, Err(HubError::InvalidCredentials)),
        "{refused:?}"
    );

    let account = block_on(first.password(
        hub.spec(),
        hub.username.clone(),
        Secret::new(hub.password.clone()),
    ))
    .unwrap();
    assert_eq!(account.username, hub.username);
    drop(first);

    // A new process reads the credentials without a new sign-in.
    let second = links(&dir);
    let info = block_on(second.connect(hub.spec())).unwrap();
    assert!(info.signed_in);
    let again = block_on(second.account(&hub.id())).unwrap();
    assert_eq!(again.id, account.id);
    if let Some(org) = account.orgs.first() {
        block_on(second.nodes(&hub.id(), &org.id)).unwrap();
        block_on(second.sessions(&hub.id(), &org.id)).unwrap();
    }

    block_on(second.sign_out(&hub.id())).unwrap();
    assert!(matches!(
        block_on(second.account(&hub.id())),
        Err(HubError::NotSignedIn)
    ));
    let info = block_on(second.connect(hub.spec())).unwrap();
    assert!(!info.signed_in);
}

fn request() -> ConnectRequest {
    ConnectRequest {
        proxy: Default::default(),
        launch: Default::default(),
        target: nocterm_session::Target {
            host: "hub".into(),
            port: 7443,
            user: "hub".into(),
        },
        auth: nocterm_session::Auth::Password,
        term: "xterm-256color".into(),
        size: Default::default(),
        connect_timeout: std::time::Duration::from_secs(10),
        keepalive_interval: None,
        jumps: Vec::new(),
    }
}

/// Reads events until the output contains `needle`; returns the output seen.
fn read_until(session: &Session, needle: &str) -> String {
    let mut seen = String::new();
    block_on(async {
        while !seen.contains(needle) {
            match session.next_event().await {
                Some(Event::Output(bytes) | Event::Restore(bytes)) => {
                    seen.push_str(&String::from_utf8_lossy(&bytes));
                }
                Some(Event::Closed(reason)) => panic!("closed early: {reason:?}"),
                Some(_) => {}
                None => panic!("the session ended"),
            }
        }
    });
    seen
}

#[test]
fn a_session_survives_detaching_and_is_restored_when_attached_again() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    block_on(links.password(
        hub.spec(),
        hub.username.clone(),
        Secret::new(hub.password.clone()),
    ))
    .unwrap();
    let account = block_on(links.account(&hub.id())).unwrap();
    let org = account.orgs.first().expect("an organization").id.clone();
    let target = HubTerminalTarget::New {
        org: org.clone(),
        node: None,
        title: "hub-io test".into(),
    };

    let first = links.terminal(&hub.id(), target).open(request());
    first.input("echo nocterm-e2e-marker\r");
    read_until(&first, "nocterm-e2e-marker");
    first.close();
    drop(first);

    let listed = block_on(links.sessions(&hub.id(), &org)).unwrap();
    let mine = listed
        .iter()
        .find(|session| session.title == "hub-io test")
        .expect("the session outlives its tab");

    let second = links
        .terminal(&hub.id(), HubTerminalTarget::Existing(mine.id.clone()))
        .open(request());
    let screen = read_until(&second, "nocterm-e2e-marker");
    assert!(screen.contains("nocterm-e2e-marker"));

    block_on(links.end(&hub.id(), &mine.id)).unwrap();
    block_on(async {
        while let Some(event) = second.next_event().await {
            if let Event::Closed(reason) = event {
                assert!(matches!(
                    reason,
                    CloseReason::Exited(_) | CloseReason::Failed(_)
                ));
                return;
            }
        }
    });
}
