//! Behaviour of the agent CA registry over an in-memory hub and fake servers.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use nocterm_hub_model::DeviceId;
use nocterm_hub_model::InstanceId;
use nocterm_hubs_model::{
    AgentAccessError, AgentHostTrust as _, HubError, HubFuture, HubId, ServerTrust,
};
use nocterm_keys::agents::{CaLine, SshCaGrant};
use nocterm_ssh_keys::ca::UserCa;
use nocterm_ssh_keys::testing::{FakeAgentHosts, FakeAuthorizedKeys};

use super::{AccountWork, AgentAccess, certify};
use crate::account::{
    Accounts,
    operations::{AccountOperations, OpenedWork},
    testing,
};
use crate::pool::Pool;
use crate::store::memory::MemoryHub;

struct Fixed {
    operations: Arc<AccountOperations>,
    device: DeviceId,
}

impl AccountWork for Fixed {
    fn work(&self, _hub: &HubId) -> Result<OpenedWork, HubError> {
        self.operations.opened()
    }

    fn device(&self, _hub: &HubId) -> HubFuture<DeviceId> {
        let device = self.device;
        Box::pin(async move { Ok(device) })
    }
}

struct Fixture {
    access: AgentAccess,
    hosts: FakeAgentHosts,
    hub: Arc<MemoryHub>,
    id: HubId,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let hub = Arc::new(MemoryHub::default());
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, _record) = testing::signed_in(dir.path());
    let operations = testing::native_operations(&slot, hub.clone());
    let hosts = FakeAgentHosts::default();
    let pool = Arc::new(Pool::new(
        dir.path().join("pool"),
        "test",
        Arc::new(Accounts::new(dir.path().join("pool"))),
    ));
    let clock = Arc::new(AtomicI64::new(1_000));
    let access = AgentAccess::with_parts(
        Arc::new(Fixed {
            operations,
            device: DeviceId::new(),
        }),
        Arc::new(hosts.clone()),
        Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)),
        pool,
    );
    Fixture {
        access,
        hosts,
        hub,
        id: testing::hub(),
        _dir: dir,
    }
}

const OTHER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOther me@laptop\n";

fn server(fx: &Fixture, connection: &str, text: &str) -> FakeAuthorizedKeys {
    let file = FakeAuthorizedKeys::with_text(text);
    fx.hosts
        .connect(connection, "deploy", "web.example", file.clone());
    file
}

#[tokio::test]
async fn allowing_a_server_adds_the_ca_line_and_keeps_the_other_lines() {
    let fx = fixture();
    let file = server(&fx, "web", OTHER);
    fx.access.allow(&fx.id, "web").await.unwrap();

    let text = file.text();
    assert!(text.starts_with(OTHER));
    let line = fx.access.ca_line(&fx.id).await.unwrap();
    assert!(text.lines().any(|existing| existing == line));
    let parsed = CaLine::parse(&line).unwrap();
    assert_eq!(parsed.to_line().unwrap(), line);

    let overview = fx.access.overview(&fx.id).await.unwrap();
    assert_eq!(overview.servers.len(), 1);
    assert_eq!(overview.servers[0].state, ServerTrust::Trusted);
    assert_eq!(
        (
            overview.servers[0].user.as_str(),
            overview.servers[0].host.as_str()
        ),
        ("deploy", "web.example")
    );
    assert!(overview.ca.is_some());
}

#[tokio::test]
async fn the_hub_holds_ciphertext_only() {
    let fx = fixture();
    server(&fx, "web", "");
    fx.access.allow(&fx.id, "web").await.unwrap();
    let collections = fx.hub.collections_of_kind("agent_ssh_trust");
    assert_eq!(collections.len(), 1);
    let objects = fx.hub.objects(&collections[0]);
    assert_eq!(objects.len(), 2);
    for object in objects {
        let text = String::from_utf8_lossy(&object.ciphertext).into_owned();
        assert!(!text.contains("web.example") && !text.contains("deploy"));
    }
}

#[tokio::test]
async fn a_server_that_cannot_be_reached_keeps_a_pending_removal_until_it_is() {
    let fx = fixture();
    let file = server(&fx, "web", OTHER);
    fx.access.allow(&fx.id, "web").await.unwrap();
    fx.hosts.disconnect("web");

    let state = fx.access.deny(&fx.id, "web").await.unwrap();
    assert_eq!(state, ServerTrust::RemovalPending);
    assert!(file.text().contains("cert-authority"));
    let overview = fx.access.overview(&fx.id).await.unwrap();
    assert_eq!(overview.servers[0].state, ServerTrust::RemovalPending);

    fx.hosts
        .connect("web", "deploy", "web.example", file.clone());
    let state = fx.access.reconcile(&fx.id, "web").await.unwrap();
    assert_eq!(state, ServerTrust::Removed);
    assert_eq!(file.text(), OTHER);
}

#[tokio::test]
async fn denying_a_reachable_server_removes_only_the_ca_line() {
    let fx = fixture();
    let file = server(&fx, "web", OTHER);
    fx.access.allow(&fx.id, "web").await.unwrap();
    assert_eq!(
        fx.access.deny(&fx.id, "web").await.unwrap(),
        ServerTrust::Removed
    );
    assert_eq!(file.text(), OTHER);
}

#[tokio::test]
async fn a_line_removed_on_the_server_is_noticed_as_drift_on_the_next_connection() {
    let fx = fixture();
    let file = server(&fx, "web", OTHER);
    fx.access.allow(&fx.id, "web").await.unwrap();
    server(&fx, "web", OTHER);
    let state = fx.access.reconcile(&fx.id, "web").await.unwrap();
    assert_eq!(state, ServerTrust::Drifted);
    assert!(!file.text().is_empty());

    fx.access.allow(&fx.id, "web").await.unwrap();
    let overview = fx.access.overview(&fx.id).await.unwrap();
    assert_eq!(overview.servers[0].state, ServerTrust::Trusted);
}

#[tokio::test]
async fn allowing_without_a_session_is_refused_and_registers_nothing() {
    let fx = fixture();
    let error = fx.access.allow(&fx.id, "web").await.unwrap_err();
    assert!(matches!(error, AgentAccessError::NotConnected { .. }));
    assert!(fx.access.overview(&fx.id).await.unwrap().servers.is_empty());
}

#[tokio::test]
async fn reissuing_the_ca_moves_reachable_servers_and_drifts_the_others() {
    let fx = fixture();
    let near = server(&fx, "near", "");
    let far = server(&fx, "far", "");
    fx.access.allow(&fx.id, "near").await.unwrap();
    fx.access.allow(&fx.id, "far").await.unwrap();
    let before = fx.access.overview(&fx.id).await.unwrap().ca.unwrap();
    let old_line = fx.access.ca_line(&fx.id).await.unwrap();
    fx.hosts.disconnect("far");

    let after = fx.access.reissue_ca(&fx.id).await.unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
    let new_line = fx.access.ca_line(&fx.id).await.unwrap();
    assert!(near.text().contains(&new_line) && !near.text().contains(&old_line));

    fx.hosts
        .connect("far", "deploy", "web.example", far.clone());
    assert_eq!(
        fx.access.reconcile(&fx.id, "far").await.unwrap(),
        ServerTrust::Drifted
    );
}

#[test]
fn a_certificate_follows_the_grant_and_refuses_another_ca() {
    let ca = UserCa::from_seed([3; 32]).unwrap();
    let grant = SshCaGrant {
        instance: InstanceId::new(),
        principal: "nocterm-agent-0011223344556677".into(),
        ca_key: ca.public_key(),
        subject_key: [9; 32],
        serial: 7,
        valid_after_ms: 1_700_000_000_000,
        valid_before_ms: 1_700_000_000_000 + 3_600_000,
        source_addresses: vec!["203.0.113.5".into()],
    };
    let issued = certify(&ca, &grant, [1; 32]).unwrap();
    let certificate = nocterm_ssh_keys::ca::IssuedCertificate::parse(issued.openssh()).unwrap();
    assert_eq!(certificate.serial(), 7);
    let other = UserCa::from_seed([4; 32]).unwrap();
    assert!(certify(&other, &grant, [1; 32]).is_err());
}
