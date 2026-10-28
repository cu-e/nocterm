//! End-to-end tests against a real hub.
//!
//! They need a running hub (`docker compose up` in the nocterm-server repository) and these
//! variables: `NOCTERM_HUB_ADDRESS`, `NOCTERM_HUB_FINGERPRINT` (SHA-256 of its certificate),
//! `NOCTERM_HUB_USERNAME` and `NOCTERM_HUB_PASSWORD`. Without `NOCTERM_REQUIRE_HUB` the tests
//! pass vacuously when the variables are missing; with it, missing variables fail the run, so
//! CI cannot silently skip them.
//!
//! The relay test that reaches a destination also needs `NOCTERM_HUB_RELAY_TARGET` (`host:port`
//! of an SSH server the hub is allowed to reach, for example a container of the compose file);
//! without it that test passes vacuously. The node tunnel test needs `NOCTERM_HUB_NODE` (a node
//! of the account's organization) and `NOCTERM_HUB_NODE_TARGET` (`host:port` of an SSH server only
//! that node reaches, for example `sshd-lan` of the compose file); without them it passes vacuously.

use futures::executor::block_on;
use nocterm_hub_io::{HubLinks, HubLinksConfig, HubRelay};
use nocterm_hubs_model::{
    Fingerprint, HubAddress, HubEndpointSpec, HubError, HubId, HubSessions as _, HubSignIn as _,
    HubTerminalTarget, HubTrust as _, SignedIn, TrustPolicy,
};
use nocterm_session::{
    CloseReason, ConnectRequest, Event, RelayDialer as _, RelayError, RelayRefusal, Secret,
    Session, Target,
};

#[path = "hub/account.rs"]
mod account;
#[path = "hub/groups.rs"]
mod groups;

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

/// Signs in with the account's password. The hub limits how often an account signs in and the
/// tests sign in at once, so a refusal for that reason is waited out.
fn sign_in(links: &HubLinks, hub: &Hub) -> Result<SignedIn, HubError> {
    for _ in 0..40 {
        let signed = block_on(links.password(
            hub.spec(),
            hub.username.clone(),
            Secret::new(hub.password.clone()),
        ));
        match signed {
            Err(error) if format!("{error:?}").contains("ResourceExhausted") => {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            other => return other,
        }
    }
    panic!("the hub kept refusing to sign {} in", hub.username)
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

    let account = sign_in(&first, &hub).unwrap().account;
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
    sign_in(&links, &hub).unwrap();
    let account = block_on(links.account(&hub.id())).unwrap();
    let org = account.orgs.first().expect("an organization").id.clone();
    let target = HubTerminalTarget::New {
        org: org.clone(),
        node: None,
        title: "hub-io test".into(),
    };

    let (first_transport, created) = links.terminal(&hub.id(), target).into_parts();
    let first = first_transport.open(request());
    first.input("echo nocterm-e2e-marker\r");
    read_until(&first, "nocterm-e2e-marker");
    first.close();
    drop(first);

    let listed = block_on(links.sessions(&hub.id(), &org)).unwrap();
    let mine = listed
        .iter()
        .find(|session| session.title == "hub-io test")
        .expect("the session outlives its tab");

    let created = block_on(created.expect("a new session reports its id")).unwrap();
    assert_eq!(
        created, mine.id,
        "the id the terminal learned is the listed one"
    );

    let second = links
        .terminal(&hub.id(), HubTerminalTarget::Existing(mine.id.clone()))
        .into_parts()
        .0
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

/// Waits for `future` for a while: a relay that never answers must fail the test, not hang it.
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

fn signed_in_relay(hub: &Hub, dir: &tempfile::TempDir) -> (std::sync::Arc<HubLinks>, HubRelay) {
    let links = links(dir);
    sign_in(&links, hub).unwrap();
    let relay = HubRelay::new(&links);
    (links, relay)
}

fn target(host: &str, port: u16) -> Target {
    Target {
        host: host.into(),
        port,
        user: "relay".into(),
    }
}

/// The `host:port` a variable names, or `None` when it is not set.
fn destination(variable: &str) -> Option<(String, u16)> {
    let destination = std::env::var(variable).ok()?;
    let (host, port) = destination.rsplit_once(':').expect(variable);
    Some((host.to_owned(), port.parse().expect(variable)))
}

/// Reads the server's banner from a relay, then half-closes it and waits for the server to end.
fn expect_banner_then_end(stream: nocterm_session::RelayStream) {
    let (to_server, from_server) = stream.into_parts();
    let banner = within(from_server.recv()).expect("the server sends its banner first");
    assert!(
        banner.starts_with(b"SSH-"),
        "{:?}",
        String::from_utf8_lossy(&banner)
    );
    // Ending our direction is a half-close: the server, which waits for our banner, closes.
    drop(to_server);
    within(async { while from_server.recv().await.is_ok() {} });
}

#[test]
fn a_relay_reaches_a_server_carries_its_banner_and_ends_with_it() {
    let Some(hub) = hub() else { return };
    let Some((host, port)) = destination("NOCTERM_HUB_RELAY_TARGET") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (_links, relay) = signed_in_relay(&hub, &dir);

    // The address is found as the user may type it, not only as it was signed in with.
    let address = format!("https://{}/", hub.address);
    let stream = within(relay.dial(&address, None, &target(&host, port))).unwrap();
    expect_banner_then_end(stream);
}

#[test]
fn a_relay_through_a_node_reaches_a_server_the_hub_cannot() {
    let Some(hub) = hub() else { return };
    let (Ok(node), Some((host, port))) = (
        std::env::var("NOCTERM_HUB_NODE"),
        destination("NOCTERM_HUB_NODE_TARGET"),
    ) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (links, relay) = signed_in_relay(&hub, &dir);
    let address = hub.address.to_string();
    // A relay names its node by id; the variable names it as an administrator does.
    let account = block_on(links.account(&hub.id())).unwrap();
    let node = account
        .orgs
        .iter()
        .flat_map(|org| block_on(links.nodes(&hub.id(), &org.id)).unwrap())
        .find(|found| found.name == node)
        .unwrap_or_else(|| panic!("no node {node} in the account's organizations"))
        .id;

    let direct = within(relay.dial(&address, None, &target(&host, port)));
    assert!(direct.is_err(), "the hub itself must not reach {host}");
    let stream = within(relay.dial(&address, Some(node.as_str()), &target(&host, port))).unwrap();
    expect_banner_then_end(stream);
}

#[test]
fn a_relay_refuses_the_hubs_own_network_and_the_metadata_service() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (_links, relay) = signed_in_relay(&hub, &dir);
    for (host, port) in [("127.0.0.1", 22), ("169.254.169.254", 80)] {
        let refused = within(relay.dial(&hub.address.to_string(), None, &target(host, port)));
        assert!(
            matches!(
                refused,
                Err(RelayError::Refused {
                    reason: RelayRefusal::Policy | RelayRefusal::PrivateNetwork
                })
            ),
            "{host}: {refused:?}"
        );
    }
}

#[test]
fn a_relay_for_an_unknown_node_is_not_opened() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (_links, relay) = signed_in_relay(&hub, &dir);
    let opened = within(relay.dial(
        &hub.address.to_string(),
        Some("no-such-node"),
        &target("example.com", 22),
    ));
    assert!(opened.is_err(), "{opened:?}");
}

#[test]
fn a_relay_needs_a_hub_that_was_signed_in_to_on_this_device() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    let relay = HubRelay::new(&links);
    let refused = within(relay.dial(&hub.address.to_string(), None, &target("example.com", 22)));
    assert!(
        matches!(refused, Err(RelayError::NotSignedIn { .. })),
        "{refused:?}"
    );
}
