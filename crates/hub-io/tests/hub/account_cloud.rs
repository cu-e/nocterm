//! Normal account sign-in restores trusted Cloud bootstrap before a live R probe.
use super::{account::Device, hub};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, NamespaceId, RunnerId,
        RunnerKeyEpoch, Timestamp,
    },
    journal::{
        JournalCatalogBinding, PublicationAllocation, PublicationBudget, PublicationWorkBudget,
    },
    tool::ToolTimer,
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_io::cloud::*;
use nocterm_hubs_model::{HubAddress, KeyStatus};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{sync::Arc, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicSetup {
    format: String,
    hub_address: String,
    raw_hub_id: String,
    hub_namespace: String,
    hub_authority: String,
    account_id: String,
    account_key_fingerprint: String,
    runner: String,
    runner_key_epoch: u64,
    encryption_key: String,
    signing_key: String,
    proof_key: String,
}
impl PublicSetup {
    fn row(&self) -> RunnerRegistration {
        let authority = nocterm_hub_client::parse_fingerprint(&self.hub_authority).unwrap();
        let digest = Fingerprint::from_bytes(Sha256::digest(authority).into());
        let scope = AccountScope::new(
            HubIdentity::new(
                NamespaceId::new(self.hub_namespace.clone()).unwrap(),
                digest,
            ),
            AccountIdentity::new(
                NamespaceId::new(self.account_id.clone()).unwrap(),
                Fingerprint::from_bytes(
                    nocterm_hub_client::parse_fingerprint(&self.account_key_fingerprint).unwrap(),
                ),
            ),
        );
        assert_eq!(self.runner.len(), 32);
        let mut runner = [0u8; 16];
        for (index, byte) in runner.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&self.runner[index * 2..index * 2 + 2], 16).unwrap();
        }
        RunnerRegistration::new(
            JournalCatalogBinding::new(
                scope,
                RunnerId::from_bytes(runner),
                RunnerKeyEpoch::new(self.runner_key_epoch).unwrap(),
            ),
            ChatMaterialPins {
                encryption: nocterm_hub_client::parse_fingerprint(&self.encryption_key).unwrap(),
                signing: nocterm_hub_client::parse_fingerprint(&self.signing_key).unwrap(),
                proof: nocterm_hub_client::parse_fingerprint(&self.proof_key).unwrap(),
            },
            "Independently compared runner".into(),
        )
        .unwrap()
    }
    fn setup(&self) -> CloudRunnerSetup {
        assert_eq!(self.format, "nocterm-cloud-runner-setup-v1");
        CloudRunnerSetup::new(
            HubAddress::parse(&self.hub_address).unwrap().into(),
            self.raw_hub_id.clone(),
            nocterm_hub_client::parse_fingerprint(&self.hub_authority).unwrap(),
            self.row(),
        )
        .unwrap()
    }
}
struct Reservations {
    work: PublicationBudget,
    resident: PublicationBudget,
    work_bytes: usize,
}
impl Reservations {
    fn new() -> Self {
        let work_bytes = [
            cloud_trust_work_bytes().unwrap(),
            account_material_work_bytes().unwrap(),
            runner_registry_work_bytes().unwrap(),
            runner_catalog_work_bytes(),
        ]
        .into_iter()
        .max()
        .unwrap();
        Self {
            work: PublicationBudget::maintenance(
                IntakePoolLimits::new(work_bytes * 2, 2, 2).unwrap(),
            ),
            resident: PublicationBudget::maintenance(
                IntakePoolLimits::new(1024 * 1024, 24, 24).unwrap(),
            ),
            work_bytes,
        }
    }
    fn reserve(pool: &PublicationBudget, bytes: usize) -> AllocationLease {
        pool.try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
            .unwrap()
    }
    fn resident(&self) -> AllocationLease {
        Self::reserve(&self.resident, 16 * 1024)
    }
    fn work(&self) -> AllocationLease {
        Self::reserve(&self.work, self.work_bytes)
    }
    fn trust(&self) -> CloudTrustReservations {
        CloudTrustReservations::new(self.work(), self.resident()).unwrap()
    }
    fn material(&self) -> AccountMaterialReservations {
        AccountMaterialReservations::new(self.work(), self.resident()).unwrap()
    }
    fn registry(&self) -> RunnerRegistryReservations {
        RunnerRegistryReservations::new(self.work(), self.resident()).unwrap()
    }
}
struct NativeTimer;
impl ToolTimer for NativeTimer {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(nocterm_hub_client::now_ms().try_into().unwrap())
    }
    fn delay(&self, duration: Duration) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}
#[test]
fn ordinary_second_device_restores_encrypted_trust_roster_and_live_runner_catalog() {
    let Some(hub) = hub() else { return };
    let setup_path =
        std::env::var_os("NOCTERM_RUNNER_SETUP_FILE").expect("actual CLI public setup is required");
    let bytes =
        nocterm_core::fs::read_regular_bounded(std::path::Path::new(&setup_path), 4096).unwrap();
    let setup: PublicSetup = toml::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let budget = Reservations::new();
    let (first, _) = Device::sign_in(&hub);
    let account_a = first.account(&hub);
    let source_a = account_a.local_source().unwrap();
    let trust_a = account_a
        .cloud_trust(source_a.clone(), budget.resident())
        .unwrap();
    let trusted_a = runtime
        .block_on(trust_a.register_new(setup.setup(), budget.trust()))
        .unwrap();
    let material_a = runtime
        .block_on(account_a.cloud_material(
            source_a.clone(),
            trusted_a.authority(),
            budget.material(),
        ))
        .unwrap();
    let registry_a = AccountRunnerRegistry::capture(
        &material_a,
        trusted_a.scope().clone(),
        budget.resident(),
        runtime.handle().clone(),
    )
    .unwrap();
    let roster_a = runtime
        .block_on(registry_a.register_new(setup.row(), budget.registry()))
        .unwrap();
    let expected = trusted_a.scope().clone();
    assert_eq!(roster_a.runners().unwrap().len(), 1);
    drop(setup);
    drop(first);
    assert!(source_a.check_current().is_err());
    assert!(trusted_a.check_current().is_err());
    drop((
        account_a, material_a, registry_a, roster_a, trusted_a, trust_a,
    ));
    verify_second_device(&hub, &runtime, &budget, expected);
}
fn verify_second_device(
    hub: &super::Hub,
    runtime: &tokio::runtime::Runtime,
    budget: &Reservations,
    expected: AccountScope,
) {
    let (second, keys) = Device::sign_in(hub);
    assert!(matches!(keys, KeyStatus::Open), "{keys:?}");
    let account = second.account(hub);
    let source = account.local_source().unwrap();
    let trust = account
        .cloud_trust(source.clone(), budget.resident())
        .unwrap();
    let trusted = runtime.block_on(trust.load(budget.trust())).unwrap();
    assert_eq!(trusted.scope(), &expected);
    let material = runtime
        .block_on(account.cloud_material(source, trusted.authority(), budget.material()))
        .unwrap();
    let registry = AccountRunnerRegistry::capture(
        &material,
        trusted.scope().clone(),
        budget.resident(),
        runtime.handle().clone(),
    )
    .unwrap();
    let roster = runtime.block_on(registry.load(budget.registry())).unwrap();
    let rows = roster.runners().unwrap();
    assert_eq!(rows.len(), 1);
    let catalog = AccountRunnerCatalog::capture(
        &material,
        &rows[0],
        budget.resident(),
        runtime.handle().clone(),
        Arc::new(NativeTimer),
    )
    .unwrap();
    let page = runtime
        .block_on(catalog.first(
            budget.work(),
            Reservations::reserve(&budget.resident, runner_catalog_page_bytes()),
        ))
        .unwrap();
    assert_eq!(page.binding(), rows[0].binding());
    assert_eq!(page.total_chats(), 0);
    assert!(page.entries().unwrap().is_empty());
    assert!(!page.has_more());
    drop(second);
    assert!(catalog.check_current().is_err());
    assert!(page.entries().is_err());
}
