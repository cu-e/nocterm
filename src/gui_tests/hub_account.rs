//! The chosen hub account keeps the saved profiles and the vault in step with its other
//! devices, and choosing another one (or none) takes effect without a restart.
use super::*;
use crate::bootstrap::Booted;
use crate::hub_account::{AccountAccess, HubAccounts};
use crate::shell_setup::UserPluginMode;
use nocterm_connections_model::{
    profiles::Profile,
    replica::ReplicaScope,
    test_support::{MemoryReplica, profile_id_for},
};
use nocterm_hubs_model::{
    Fingerprint, HubAddress, HubError, HubId, HubRecord, HubsSettings, TrustPolicy,
};
use nocterm_session::{Auth, Secret, Target};
use nocterm_vault::{VaultAccess as _, VaultStatus, testing::ScriptedVaultAccess};
use std::sync::atomic::{AtomicBool, Ordering};

const ADDRESS: &str = "hub.home:7443";
const OTHER: &str = "hub.work:7443";
const PASSPHRASE: &str = "test unique master passphrase";

fn hub_id(address: &str) -> HubId {
    HubId::from(HubAddress::parse(address).unwrap())
}

fn record(address: &str) -> HubRecord {
    HubRecord::new(
        HubAddress::parse(address).unwrap(),
        TrustPolicy::Pinned(Fingerprint::from_bytes([7; 32])),
        address,
    )
}

fn profile(name: &str) -> Profile {
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
struct Account {
    replica: Arc<MemoryReplica>,
    vault: Arc<ScriptedVaultAccess>,
}

impl Account {
    fn new() -> Self {
        let vault = Arc::new(ScriptedVaultAccess::default());
        futures::executor::block_on(vault.create(Secret::new(PASSPHRASE))).unwrap();
        Self {
            replica: Arc::new(MemoryReplica::new()),
            vault,
        }
    }
}

/// The accounts of the servers it knows; any other server has none to open.
struct FakeAccounts {
    accounts: Vec<(HubId, Account)>,
    refusing: AtomicBool,
}

impl FakeAccounts {
    fn of(accounts: Vec<(HubId, Account)>) -> Arc<Self> {
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
        Ok(AccountAccess {
            replica: account.replica.clone(),
            vault: account.vault.clone(),
        })
    }
}

fn boot_with(accounts: Arc<FakeAccounts>, chosen: Option<&str>, cx: &mut TestAppContext) -> Booted {
    let settings =
        nocterm_store::SettingsConfig::in_memory(Default::default()).with(HubsSettings {
            servers: vec![record(ADDRESS), record(OTHER)],
            account: chosen.map(str::to_owned),
        });
    let hub = nocterm_hubs_model::testing::FakeHub::new();
    let booted = cx.update(|cx| {
        let deps = nocterm_hubs::Deps {
            trust: Arc::new(hub.clone()),
            sign_in: Arc::new(hub.clone()),
            admin_keys: Arc::new(nocterm_hubs_model::testing::FakeAdminKeys::alongside(&hub)),
        };
        boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            settings,
            (
                Arc::new(MockTransport::default()),
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
    booted
}

fn choose(booted: &Booted, hub: Option<&str>, cx: &mut TestAppContext) {
    let hub = hub.map(hub_id);
    let settings = booted.settings_deps().settings().clone();
    cx.update(|cx| {
        settings
            .update_checked::<HubsSettings>(cx, move |settings| {
                settings.set_account_hub(hub.as_ref());
                Ok(())
            })
            .detach();
    });
    cx.run_until_parked();
}

fn vault(booted: &Booted) -> Arc<dyn nocterm_vault::VaultAccess> {
    booted.vault().expect("the vault opened").service()
}

fn local_profiles(booted: &Booted, cx: &mut TestAppContext) -> Vec<Profile> {
    let store = booted.connections().unwrap().connections().clone();
    store.read_with(cx, |store, cx| store.profiles(cx).iter().cloned().collect())
}

#[gpui_kit::test]
fn the_chosen_account_supplies_the_vault_and_the_profiles(cx: &mut TestAppContext) {
    let account = Account::new();
    let remote = profile("remote");
    account
        .replica
        .write_remote(ReplicaScope::Personal, remote.clone());
    let booted = boot_with(
        FakeAccounts::of(vec![(hub_id(ADDRESS), account)]),
        Some(ADDRESS),
        cx,
    );

    assert!(
        vault(&booted).is_unlocked(),
        "the vault in use is the account's"
    );
    assert_eq!(local_profiles(&booted, cx), [remote]);
}

#[gpui_kit::test]
fn without_a_chosen_account_the_vault_and_the_profiles_stay_on_this_device(
    cx: &mut TestAppContext,
) {
    let account = Account::new();
    account
        .replica
        .write_remote(ReplicaScope::Personal, profile("remote"));
    let booted = boot_with(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), None, cx);

    assert_eq!(vault(&booted).status(), VaultStatus::Missing);
    assert!(local_profiles(&booted, cx).is_empty());
}

#[gpui_kit::test]
fn choosing_an_account_takes_effect_without_a_restart(cx: &mut TestAppContext) {
    let account = Account::new();
    let remote = profile("remote");
    account
        .replica
        .write_remote(ReplicaScope::Personal, remote.clone());
    let booted = boot_with(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), None, cx);
    let service = vault(&booted);
    let seen = service.revision();

    choose(&booted, Some(ADDRESS), cx);

    assert!(service.is_unlocked());
    assert!(service.revision() > seen, "observers are told of the move");
    assert_eq!(local_profiles(&booted, cx), [remote]);
}

#[gpui_kit::test]
fn choosing_none_goes_back_to_the_device_vault_and_stops_replicating(cx: &mut TestAppContext) {
    let account = Account::new();
    let replica = account.replica.clone();
    let vault_of_account = account.vault.clone();
    let booted = boot_with(
        FakeAccounts::of(vec![(hub_id(ADDRESS), account)]),
        Some(ADDRESS),
        cx,
    );

    choose(&booted, None, cx);

    assert_eq!(
        vault_of_account.status(),
        VaultStatus::Locked,
        "the account's vault does not stay open"
    );
    assert_eq!(vault(&booted).status(), VaultStatus::Missing);
    replica.write_remote(ReplicaScope::Personal, profile("later"));
    cx.run_until_parked();
    assert!(local_profiles(&booted, cx).is_empty());
}

#[gpui_kit::test]
fn switching_accounts_keeps_the_profiles_of_each_account_apart(cx: &mut TestAppContext) {
    let home = Account::new();
    let work = Account::new();
    let at_home = profile("home-box");
    let at_work = profile("work-box");
    home.replica
        .write_remote(ReplicaScope::Personal, at_home.clone());
    work.replica
        .write_remote(ReplicaScope::Personal, at_work.clone());
    let work_replica = work.replica.clone();
    let booted = boot_with(
        FakeAccounts::of(vec![(hub_id(ADDRESS), home), (hub_id(OTHER), work)]),
        Some(ADDRESS),
        cx,
    );
    assert_eq!(local_profiles(&booted, cx), std::slice::from_ref(&at_home));

    choose(&booted, Some(OTHER), cx);

    // The profiles that were agreed with the first account are not offered to the second.
    assert!(
        work_replica.stored(at_home.id).is_none(),
        "{} is not copied into another account",
        at_home.name
    );
    assert!(local_profiles(&booted, cx).contains(&at_work));
}

#[gpui_kit::test]
fn an_account_that_cannot_be_opened_leaves_the_device_as_it_was(cx: &mut TestAppContext) {
    let account = Account::new();
    let accounts = FakeAccounts::of(vec![(hub_id(ADDRESS), account)]);
    accounts.refusing.store(true, Ordering::SeqCst);
    let booted = boot_with(accounts, Some(ADDRESS), cx);

    assert_eq!(vault(&booted).status(), VaultStatus::Missing);
    assert!(local_profiles(&booted, cx).is_empty());
}

#[gpui_kit::test]
fn a_refused_change_to_a_shared_connection_is_undone_and_announced_once(cx: &mut TestAppContext) {
    use futures::FutureExt as _;
    use nocterm_connections_model::replica::{ReplicaAccess, ReplicaSpace};
    let account = Account::new();
    let acme = ReplicaScope::Organization("acme".into());
    let shared = profile("shared");
    account.replica.set_spaces(vec![
        ReplicaSpace {
            scope: ReplicaScope::Personal,
            name: "Personal".into(),
            access: ReplicaAccess::Write,
        },
        ReplicaSpace {
            scope: acme.clone(),
            name: "Acme".into(),
            access: ReplicaAccess::Read,
        },
    ]);
    account.replica.write_remote(acme.clone(), shared.clone());
    account.replica.refuse_writes_in(acme);
    let booted = boot_with(
        FakeAccounts::of(vec![(hub_id(ADDRESS), account)]),
        Some(ADDRESS),
        cx,
    );
    assert_eq!(local_profiles(&booted, cx), std::slice::from_ref(&shared));
    let (handle, _workspace) = cx.update(|cx| open_workspace(&booted, cx));

    let mut edited = shared.clone();
    edited.name = "renamed".into();
    let store = booted.connections().unwrap().connections().clone();
    cx.update(|cx| {
        store
            .update(cx, |store, cx| store.save_profile(edited, cx))
            .now_or_never()
            .unwrap()
            .unwrap()
    });
    cx.run_until_parked();

    assert_eq!(
        local_profiles(&booted, cx),
        [shared],
        "the shared version is back"
    );
    let notices = cx
        .update_window(handle, |_, window, cx| {
            nocterm_ui::notice::count(window, cx)
        })
        .unwrap();
    assert_eq!(notices, 1, "one notice for the refusal");
}
