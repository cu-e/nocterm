//! Two actual fake native account owners sharing the production capture/ledger path.
use super::{self as testing, ScriptedSubjects};
use crate::{
    account::{
        Accounts, OpenKeys, Slot, operations::AccountOperations, source::LocalSourceProvider,
    },
    cache::{AccountCache, UnlockMethod},
    store::{
        StoreOps as _, VAULT_KIND,
        memory::{MemoryAccountStores, MemoryHub},
    },
    vault::{HubVault, credentials::CredentialCapture},
};
use nocterm_hub_client::Keyset;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{HubError, HubId, source_identity::LocalAccountSource};
use nocterm_session::{CredentialId, CredentialStore, Secret, SecretRequest, Target};
use std::sync::Arc;
use tokio::runtime::Handle;

pub(crate) struct NativeAccounts {
    _dir: tempfile::TempDir,
    slot: Arc<Slot>,
    subjects: Arc<ScriptedSubjects>,
    provider: LocalSourceProvider,
    operations: Arc<AccountOperations>,
    vault: Arc<HubVault>,
    hubs: [Arc<MemoryHub>; 2],
    records: [v1::AccountKeyset; 2],
    unlock: nocterm_keys::UnlockKey,
}
impl NativeAccounts {
    pub(crate) fn new() -> Self {
        Self::at(testing::hub())
    }
    pub(crate) fn at(hub: HubId) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let accounts = Accounts::new(dir.path().to_path_buf());
        let slot = accounts.slot(&hub).unwrap();
        let unlock = testing::unlock(testing::PASSWORD);
        let (a, _) = testing::record(&unlock);
        let b = testing::sealed_record(
            "account-2",
            &unlock,
            &testing::kdf(),
            &nocterm_keys::AccountKey::generate().unwrap(),
            &nocterm_keys::AccountKeyPair::generate().unwrap(),
            &nocterm_keys::RecoveryKey::generate().unwrap(),
        );
        let subjects = Arc::new(ScriptedSubjects::for_slot(&slot));
        let provider = LocalSourceProvider::new(slot.clone(), subjects.clone());
        let hubs = [
            Arc::new(MemoryHub::default()),
            Arc::new(MemoryHub::default()),
        ];
        let stores = Arc::new(MemoryAccountStores::new([
            (testing::ACCOUNT_ID.into(), hubs[0].clone()),
            ("account-2".into(), hubs[1].clone()),
        ]));
        let operations = Arc::new(AccountOperations::with_stores(provider.clone(), stores));
        let vault = Arc::new(HubVault::new(
            Handle::current(),
            slot.clone(),
            operations.clone(),
        ));
        let fixture = Self {
            _dir: dir,
            slot,
            subjects,
            provider,
            operations,
            vault,
            hubs,
            records: [a, b],
            unlock,
        };
        fixture.login(0);
        fixture
    }
    pub(crate) fn slot(&self) -> &Arc<Slot> {
        &self.slot
    }
    pub(crate) fn subjects(&self) -> &Arc<ScriptedSubjects> {
        &self.subjects
    }
    pub(crate) fn hub(&self, index: usize) -> &Arc<MemoryHub> {
        &self.hubs[index]
    }
    pub(crate) fn operations(&self) -> &Arc<AccountOperations> {
        &self.operations
    }
    pub(crate) fn vault(&self) -> &Arc<HubVault> {
        &self.vault
    }
    pub(crate) fn source(&self) -> Arc<dyn LocalAccountSource> {
        self.provider.capture().unwrap()
    }
    pub(crate) fn capture_credentials(
        &self,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn CredentialStore>, HubError> {
        CredentialCapture::new(
            Handle::current(),
            self.operations.stores().clone(),
            &self.vault.idle(),
        )
        .capture(&self.provider, source)
    }
    pub(crate) fn store(&self) -> Arc<dyn CredentialStore> {
        self.capture_credentials(self.source()).unwrap()
    }
    pub(crate) fn login(&self, index: usize) {
        let account = if index == 0 {
            testing::ACCOUNT_ID
        } else {
            "account-2"
        };
        self.subjects
            .sign_in(self.slot.hub().clone(), account, "device");
        self.hubs[index].set_keyset(Some(self.records[index].clone()));
        self.slot.install(
            OpenKeys::pinning(
                Keyset::open(&self.records[index], account, &self.unlock).unwrap(),
                nocterm_keys::PinnedKeys::new(),
            ),
            AccountCache::new(
                testing::HUB_UID.into(),
                account.into(),
                UnlockMethod::Passphrase,
                &self.records[index],
            ),
        );
    }
    pub(crate) async fn seed(&self) -> CredentialId {
        self.store()
            .save(
                "A".into(),
                &SecretRequest::Password {
                    target: Target::new("ann", "example.test", 22),
                    retry: false,
                },
                Secret::new("secret A"),
            )
            .await
            .unwrap()
    }
    pub(crate) async fn count(&self, index: usize) -> usize {
        self.hubs[index]
            .collections(VAULT_KIND)
            .await
            .unwrap()
            .iter()
            .map(|at| self.hubs[index].objects(&at.collection_id).len())
            .sum()
    }
}
