//! A hub window shows the account of its hub: its connections, in a mirror that replicates with
//! the hub, and its vault. The window of this computer shows neither.
use super::*;
use crate::bootstrap::Booted;
use crate::hub_sources::{AccountAccess, HubAccounts};
use crate::shell_setup::UserPluginMode;
use nocterm_connections_model::{
    profiles::Profile,
    test_support::{MemoryReplica, profile_id_for},
};
use nocterm_hubs_model::{
    Environment, Fingerprint, HubAddress, HubError, HubId, HubRecord, HubsSettings, TrustPolicy,
    testing::{FakeHub, FakeServer},
};
use nocterm_session::{Auth, Secret, Target};
use nocterm_vault::{VaultAccess as _, VaultStatus, testing::ScriptedVaultAccess};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const ADDRESS: &str = "hub.home:7443";
pub(super) const OTHER: &str = "hub.work:7443";
pub(super) const PASSPHRASE: &str = "test unique master passphrase";

pub(super) fn hub_id(address: &str) -> HubId {
    HubId::from(HubAddress::parse(address).unwrap())
}

pub(super) fn record(address: &str) -> HubRecord {
    HubRecord::new(
        HubAddress::parse(address).unwrap(),
        TrustPolicy::Pinned(Fingerprint::from_bytes([7; 32])),
        address,
    )
}

pub(super) fn profile(name: &str) -> Profile {
    Profile {
        id: profile_id_for(name),
        name: name.into(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("ci", format!("{name}.example"), 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    }
}

/// An account whose profiles and vault are in memory.
pub(super) struct Account {
    pub(super) replica: Arc<MemoryReplica>,
    pub(super) vault: Arc<ScriptedVaultAccess>,
    source: Arc<nocterm_hubs_model::testing::ScriptedLocalAccountFactory>,
    captures:
        Arc<std::sync::Mutex<Vec<nocterm_hubs_model::source_identity::LocalAccountPartition>>>,
}

impl Account {
    pub(super) fn new() -> Self {
        let vault = Arc::new(ScriptedVaultAccess::default());
        futures::executor::block_on(vault.create(Secret::new(PASSPHRASE))).unwrap();
        Self {
            replica: Arc::new(MemoryReplica::new()),
            vault,
            source: Arc::default(),
            captures: Arc::default(),
        }
    }
}

fn partition(
    hub: &HubId,
    account: &str,
) -> nocterm_hubs_model::source_identity::LocalAccountPartition {
    use nocterm_hubs_model::source_identity::{
        Fingerprint, LegacyConfiguredAccount, LocalAccountPartition, NamespaceId,
    };
    LocalAccountPartition::LegacyConfigured(LegacyConfiguredAccount::new(
        hub.clone(),
        NamespaceId::new(account).unwrap(),
        Fingerprint::from_bytes([8; 32]),
    ))
}

/// The accounts of the servers it knows; any other server has none to open.
pub(super) struct FakeAccounts {
    accounts: Vec<(HubId, Account)>,
    refusing: AtomicBool,
}

impl FakeAccounts {
    pub(super) fn of(accounts: Vec<(HubId, Account)>) -> Arc<Self> {
        for (hub, account) in &accounts {
            account.source.sign_in(partition(hub, "alice"));
        }
        Arc::new(Self {
            accounts,
            refusing: AtomicBool::new(false),
        })
    }
}

impl HubAccounts for FakeAccounts {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError> {
        if self.refusing.load(Ordering::SeqCst) {
            return Err(HubError::NotSignedIn);
        }
        let (_, account) = self
            .accounts
            .iter()
            .find(|(id, _)| id == hub)
            .ok_or(HubError::NotSignedIn)?;
        let source = account.source.clone();
        Ok(AccountAccess::new(
            {
                let replica = account.replica.clone();
                let captures = account.captures.clone();
                Arc::new(move |source| {
                    source.check_current().map_err(|_| HubError::Stale)?;
                    captures.lock().unwrap().push(source.partition());
                    Ok(replica.clone()
                        as Arc<dyn nocterm_connections_model::replica::ProfileReplica>)
                })
            },
            account.vault.clone(),
            source,
            {
                let vault = account.vault.clone();
                Arc::new(move |_| {
                    Ok(Arc::new(nocterm_vault_ui::VaultCredentials::new(
                        vault.clone(),
                    )))
                })
            },
        ))
    }
}

/// The application composed over two known hubs, signed in to both.
pub(super) struct World {
    pub(super) booted: Booted,
    transport: Arc<MockTransport>,
    hub: FakeHub,
}

pub(super) fn world(accounts: Arc<FakeAccounts>, cx: &mut TestAppContext) -> World {
    let settings =
        nocterm_store::SettingsConfig::in_memory(Default::default()).with(HubsSettings {
            servers: vec![record(ADDRESS), record(OTHER)],
            ..HubsSettings::default()
        });
    let hub = FakeHub::new()
        .with_server(FakeServer::new(ADDRESS, 7))
        .with_server(FakeServer::new(OTHER, 7));
    hub.sign_in_as(&hub_id(ADDRESS), "alice");
    hub.sign_in_as(&hub_id(OTHER), "alice");
    let transport = Arc::new(MockTransport::default());
    let booted = cx.update(|cx| {
        let deps = nocterm_hubs::Deps {
            trust: Arc::new(hub.clone()),
            sign_in: Arc::new(hub.clone()),
            devices: Arc::new(nocterm_hubs_model::testing::FakeAccountDevices::default()),
            account_keys: Arc::new(nocterm_hubs_model::testing::FakeAccountKeys::alongside(
                &hub, "password",
            )),
        };
        boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            settings,
            (
                transport.clone(),
                nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
            ),
            true,
            None,
            None,
            unsupported_sandbox(),
            UserPluginMode::default(),
            HubPorts {
                deps: Some(deps),
                accounts: Some(accounts),
            },
            cx,
        )
    });
    cx.run_until_parked();
    World {
        booted,
        transport,
        hub,
    }
}

impl World {
    pub(super) fn open(
        &self,
        environment: &Environment,
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, Entity<Workspace>) {
        let opened = cx.update(|cx| open_environment(&self.booted, environment, cx));
        cx.run_until_parked();
        opened
    }

    fn store_of(
        &self,
        environment: &Environment,
        cx: &mut TestAppContext,
    ) -> Entity<nocterm_connections::Connections> {
        let sources = cx
            .update(|cx| self.booted.window_sources(environment, cx))
            .expect("the window's sources");
        sources.connections().unwrap().connections().clone()
    }

    fn profiles_of(&self, environment: &Environment, cx: &mut TestAppContext) -> Vec<Profile> {
        self.store_of(environment, cx)
            .read_with(cx, |store, cx| store.profiles(cx).iter().cloned().collect())
    }

    pub(super) fn vault_of(
        &self,
        environment: &Environment,
        cx: &mut TestAppContext,
    ) -> Arc<dyn nocterm_vault::VaultAccess> {
        let sources = cx
            .update(|cx| self.booted.window_sources(environment, cx))
            .expect("the window's sources");
        sources.vault().expect("a vault").service()
    }
}

fn save(
    store: &Entity<nocterm_connections::Connections>,
    profile: Profile,
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        store
            .update(cx, |store, cx| store.save_profile(profile, cx))
            .now_or_never()
            .unwrap()
            .unwrap()
    });
    cx.run_until_parked();
}

pub(super) fn home() -> Environment {
    Environment::Hub(hub_id(ADDRESS))
}

#[gpui_kit::test]
fn a_hub_window_shows_the_profiles_of_its_account_and_the_local_window_shows_its_own(
    cx: &mut TestAppContext,
) {
    let account = Account::new();
    let remote = profile("remote");
    account.replica.write_remote(remote.clone());
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    let local = profile("local-box");
    save(
        &world.booted.connections().unwrap().connections().clone(),
        local.clone(),
        cx,
    );
    world.open(&home(), cx);

    assert_eq!(world.profiles_of(&home(), cx), [remote]);
    assert_eq!(world.profiles_of(&Environment::Local, cx), [local]);
}

#[gpui_kit::test]
fn a_connection_created_in_a_hub_window_is_saved_on_the_hub_and_not_on_this_computer(
    cx: &mut TestAppContext,
) {
    let account = Account::new();
    let replica = account.replica.clone();
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    world.open(&home(), cx);
    let created = profile("created");

    save(&world.store_of(&home(), cx), created.clone(), cx);

    assert!(replica.stored(created.id).is_some(), "the hub has it");
    assert_eq!(world.profiles_of(&home(), cx), [created]);
    assert!(world.profiles_of(&Environment::Local, cx).is_empty());
}

#[gpui_kit::test]
fn two_windows_of_one_hub_share_one_mirror_and_another_hub_has_its_own(cx: &mut TestAppContext) {
    let world = world(
        FakeAccounts::of(vec![
            (hub_id(ADDRESS), Account::new()),
            (hub_id(OTHER), Account::new()),
        ]),
        cx,
    );
    world.open(&home(), cx);
    world.open(&home(), cx);
    let work = Environment::Hub(hub_id(OTHER));
    world.open(&work, cx);

    assert_eq!(
        world.store_of(&home(), cx).entity_id(),
        world.store_of(&home(), cx).entity_id()
    );
    assert_ne!(
        world.store_of(&home(), cx).entity_id(),
        world.store_of(&work, cx).entity_id()
    );
    save(&world.store_of(&home(), cx), profile("home-box"), cx);
    assert!(world.profiles_of(&work, cx).is_empty());
}

#[gpui_kit::test]
fn a_hub_window_of_an_unreachable_account_cannot_be_composed(cx: &mut TestAppContext) {
    let accounts = FakeAccounts::of(vec![(hub_id(ADDRESS), Account::new())]);
    accounts.refusing.store(true, Ordering::SeqCst);
    let world = world(accounts, cx);

    let result = cx.update(|cx| world.booted.window_sources(&home(), cx));

    assert!(matches!(
        result,
        Err(crate::window_sources::WindowSourcesError::Account(_))
    ));
}

#[gpui_kit::test]
fn signing_out_empties_the_hub_window_and_locks_its_vault(cx: &mut TestAppContext) {
    let account = Account::new();
    account.replica.write_remote(profile("remote"));
    let vault_of_account = account.vault.clone();
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    world.open(&home(), cx);
    assert_eq!(world.profiles_of(&home(), cx).len(), 1);

    cx.update(|cx| {
        let hubs = world.booted.hub_registry(cx).unwrap();
        hubs.update(cx, |hubs, cx| hubs.sign_out(&hub_id(ADDRESS), cx).detach())
    });
    cx.run_until_parked();

    assert!(
        world.profiles_of(&home(), cx).is_empty(),
        "nothing of the account stays"
    );
    assert_eq!(vault_of_account.status(), VaultStatus::Locked);
}

#[gpui_kit::test]
fn a_secret_typed_in_a_hub_window_is_kept_in_the_hub_vault_for_the_hub_connection(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let account = Account::new();
    let remote = profile("remote");
    account.replica.write_remote(remote.clone());
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    world.open(&home(), cx);
    let sources = cx
        .update(|cx| world.booted.window_sources(&home(), cx))
        .unwrap();
    let terminal = cx.update(|cx| {
        cx.new(|cx| {
            nocterm_terminal::Terminal::new(
                sources.terminal().clone(),
                SessionSpec {
                    profile: Some(remote.id.to_string().into()),
                    options: Default::default(),
                    title: "remote".into(),
                    target: remote.target.clone(),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                    jumps: Vec::new(),
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    let driver = world
        .transport
        .drivers
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap();
    let (reply, answer) = nocterm_session::Reply::channel();
    assert!(futures::executor::block_on(driver.emit(Event::Prompt(
        Prompt::Secret {
            request: SecretRequest::Password {
                target: remote.target.clone(),
                retry: false,
            },
            reply,
            hop: None,
        }
    ))));
    cx.run_until_parked();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("hub box password"), true, cx)
        })
    });
    futures::executor::block_on(answer).unwrap().unwrap();
    assert!(futures::executor::block_on(driver.emit(Event::Connected)));
    cx.run_until_parked();

    let kept = futures::executor::block_on(world.vault_of(&home(), cx).list()).unwrap();
    assert_eq!(kept.len(), 1, "the secret is in the vault of the hub");
    assert!(
        world.profiles_of(&home(), cx)[0].credential.is_some(),
        "and belongs to the connection of the hub"
    );
    assert_eq!(
        world.vault_of(&Environment::Local, cx).status(),
        VaultStatus::Missing,
        "the vault of this computer is untouched"
    );
}

#[gpui_kit::test]
fn a_locked_account_asks_for_its_vault_in_the_window_and_lists_after_the_unlock(
    cx: &mut TestAppContext,
) {
    let account = Account::new();
    account.replica.write_remote(profile("remote"));
    account.vault.lock();
    account.replica.set_locked(true);
    let (replica, vault_of_account) = (account.replica.clone(), account.vault.clone());
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    let (handle, _workspace) = world.open(&home(), cx);
    assert!(world.profiles_of(&home(), cx).is_empty());

    // The window asks for the vault a couple of frames after it is composed.
    deliver_frames(handle, 4, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("vault-unlock-prompt").is_some(),
            "the hub window opens the unlock dialog of its own vault"
        );
    })
    .unwrap();
    cx.simulate_input(handle, PASSPHRASE);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(vault_of_account.is_unlocked());
    assert_eq!(
        world.vault_of(&Environment::Local, cx).status(),
        VaultStatus::Missing,
        "the vault of this computer was not asked"
    );
    replica.set_locked(false);
    cx.run_until_parked();

    assert_eq!(world.profiles_of(&home(), cx).len(), 1);
}

#[gpui_kit::test]
fn a_refused_change_to_a_connection_is_undone_and_announced_once(cx: &mut TestAppContext) {
    let account = Account::new();
    let saved = profile("saved");
    account.replica.write_remote(saved.clone());
    account.replica.refuse_writes();
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    let (handle, _workspace) = world.open(&home(), cx);
    assert_eq!(world.profiles_of(&home(), cx), std::slice::from_ref(&saved));

    let mut edited = saved.clone();
    edited.name = "renamed".into();
    save(&world.store_of(&home(), cx), edited, cx);

    assert_eq!(
        world.profiles_of(&home(), cx),
        [saved],
        "the synchronized version is back"
    );
    let notices = cx
        .update_window(handle, |_, window, cx| {
            nocterm_ui::notice::count(window, cx)
        })
        .unwrap();
    assert_eq!(notices, 1, "one notice for the refusal");
}

#[gpui_kit::test]
fn replacement_logins_attach_new_replica_facades_without_reusing_the_old_capture(
    cx: &mut TestAppContext,
) {
    let account = Account::new();
    let source = account.source.clone();
    let captures = account.captures.clone();
    let accounts = FakeAccounts::of(vec![(hub_id(ADDRESS), account)]);
    let world = world(accounts, cx);
    world.open(&home(), cx);
    assert_eq!(
        captures.lock().unwrap().as_slice(),
        &[partition(&hub_id(ADDRESS), "alice")]
    );
    for name in ["alice", "bob", "alice"] {
        source.sign_in(partition(&hub_id(ADDRESS), name));
        // A native source changed; the existing keys notice makes Follow recapture it.
        world.hub.lock_keys(&hub_id(ADDRESS));
        world.hub.unlock_keys(&hub_id(ADDRESS));
        cx.run_until_parked();
    }
    let captured = captures.lock().unwrap().clone();
    assert_eq!(
        captured,
        vec![
            partition(&hub_id(ADDRESS), "alice"),
            partition(&hub_id(ADDRESS), "alice"),
            partition(&hub_id(ADDRESS), "bob"),
            partition(&hub_id(ADDRESS), "alice")
        ]
    );
}

#[gpui_kit::test]
fn removing_and_readding_a_hub_builds_a_fresh_mirror_and_keeps_the_old_window_retired(
    cx: &mut TestAppContext,
) {
    let account = Account::new();
    account.replica.write_remote(profile("remote"));
    let world = world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx);
    let (_window, _workspace) = world.open(&home(), cx);
    let old = world.store_of(&home(), cx);
    let old_id = old.entity_id();
    let removed = cx.update(|cx| {
        world
            .booted
            .hub_registry(cx)
            .unwrap()
            .update(cx, |hubs, cx| hubs.remove(&hub_id(ADDRESS), cx))
    });
    removed.detach();
    cx.run_until_parked();
    assert!(old.read_with(cx, |store, cx| store.profiles(cx).is_empty()));
    world.hub.sign_in_as(&hub_id(ADDRESS), "alice");
    let settings = world.booted.settings_deps();
    cx.update(|cx| {
        settings
            .update::<HubsSettings>(cx, |settings| settings.servers.push(record(ADDRESS)))
            .detach()
    });
    cx.run_until_parked();
    let new = world.store_of(&home(), cx);
    assert_ne!(new.entity_id(), old_id);
    assert!(old.read_with(cx, |store, cx| store.profiles(cx).is_empty()));
}
