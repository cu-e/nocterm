//! Behaviour of the agent CA registry over an in-memory hub and fake servers.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use nocterm_hub_model::DeviceId;
use nocterm_hub_model::cloud::ssh::SshServerId;
use nocterm_hubs_model::{
    AgentAccessError, AgentHostTrust as _, HubError, HubFuture, HubId, ServerTrust,
};
use nocterm_keys::agents::ssh_ca::agent_principal;
use nocterm_keys::agents::{CaLine, CaLineForm};
use nocterm_ssh_keys::testing::{FakeAgentHosts, FakeAuthorizedKeys};

use super::registry::Registry;
use super::{AccountWork, AgentAccess};
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
    operations: Arc<AccountOperations>,
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
            operations: operations.clone(),
            device: DeviceId::new(),
        }),
        Arc::new(hosts.clone()),
        Arc::new(move || clock.fetch_add(1, Ordering::SeqCst)),
        pool,
    );
    Fixture {
        access,
        operations,
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
    let line = fx.access.ca_line(&fx.id, "web").await.unwrap();
    assert!(text.lines().any(|existing| existing == line));
    let parsed = CaLine::parse(&line).unwrap();
    assert_eq!(parsed.to_line().unwrap(), line);
    // The line admits this server's own principal only.
    let web = SshServerId::of_profile("web");
    assert!(parsed.admits_server(&parsed.ca_key, &web));
    assert!(!parsed.admits_server(&parsed.ca_key, &SshServerId::of_profile("db")));
    assert!(!parsed.principals.contains(&agent_principal(&parsed.ca_key)));

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
    assert!(!overview.servers[0].needs_ca_update);
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
    let old_line = fx.access.ca_line(&fx.id, "near").await.unwrap();
    fx.hosts.disconnect("far");

    let after = fx.access.reissue_ca(&fx.id).await.unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
    let new_line = fx.access.ca_line(&fx.id, "near").await.unwrap();
    assert!(near.text().contains(&new_line) && !near.text().contains(&old_line));

    fx.hosts
        .connect("far", "deploy", "web.example", far.clone());
    assert_eq!(
        fx.access.reconcile(&fx.id, "far").await.unwrap(),
        ServerTrust::Drifted
    );
}

/// What an earlier client left on `connection`: the shared principal of line version 1 in the
/// file and a trust record of schema 1. Returns the shared line.
async fn shared_principal(fx: &Fixture, connection: &str, file: &FakeAuthorizedKeys) -> String {
    let store = fx.operations.opened().unwrap().transient_store();
    let mut registry = Registry::load(&store, false).await.unwrap().unwrap();
    let ca_key = registry.ca.as_ref().unwrap().value.ca().public_key();
    let shared = CaLine {
        ca_key,
        principals: vec![agent_principal(&ca_key)],
        comment: "nocterm agent access".into(),
    }
    .to_line()
    .unwrap();
    file.set_text(&format!("{OTHER}{shared}\n"));
    let mut record = registry.servers[connection].value.clone();
    record.set_line(CaLineForm::Shared, 1);
    registry.save_server(&store, record).await.unwrap();
    shared
}

#[tokio::test]
async fn a_server_with_the_shared_principal_gets_its_own_line_on_its_next_connection() {
    let fx = fixture();
    let file = server(&fx, "web", OTHER);
    fx.access.allow(&fx.id, "web").await.unwrap();
    let shared = shared_principal(&fx, "web", &file).await;
    let overview = fx.access.overview(&fx.id).await.unwrap();
    assert!(overview.servers[0].needs_ca_update);

    let state = fx.access.reconcile(&fx.id, "web").await.unwrap();
    assert_eq!(state, ServerTrust::Trusted);
    let own = fx.access.ca_line(&fx.id, "web").await.unwrap();
    let text = file.text();
    assert!(text.starts_with(OTHER));
    assert!(text.lines().any(|line| line == own));
    assert!(!text.lines().any(|line| line == shared));
    let overview = fx.access.overview(&fx.id).await.unwrap();
    assert!(!overview.servers[0].needs_ca_update);
}

#[tokio::test]
async fn a_shared_principal_that_cannot_be_rewritten_still_needs_an_update() {
    let fx = fixture();
    let file = server(&fx, "web", OTHER);
    fx.access.allow(&fx.id, "web").await.unwrap();
    let shared = shared_principal(&fx, "web", &file).await;
    file.fail_writes();
    assert_eq!(
        fx.access.reconcile(&fx.id, "web").await.unwrap(),
        ServerTrust::Trusted
    );
    assert!(file.text().contains(&shared));
    let overview = fx.access.overview(&fx.id).await.unwrap();
    assert!(overview.servers[0].needs_ca_update);
}
