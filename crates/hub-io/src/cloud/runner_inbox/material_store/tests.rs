//! Actual account DH grants survive runner restart without retaining a device key.
use super::*;
use nocterm_cloud_model::{
    identity::*,
    journal::{JournalCipherContext, JournalStorageClass},
};
use nocterm_keys::{
    AccountKeyPair,
    cloud::{ChatKey, seal_chat_key_to_runner},
};

struct Fixture {
    directory: tempfile::TempDir,
    binding: JournalCatalogBinding,
    identity: Arc<CloudRunnerIdentity>,
    account: AccountKeyPair,
    stream: StreamBinding,
    epoch: ChatKeyEpoch,
    grant: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let account = AccountKeyPair::generate().unwrap();
        let binding = JournalCatalogBinding::new(
            AccountScope::new(
                HubIdentity::new(
                    NamespaceId::new("hub").unwrap(),
                    Fingerprint::from_bytes([1; 32]),
                ),
                AccountIdentity::new(
                    NamespaceId::new("account").unwrap(),
                    Fingerprint::from_bytes(Sha256::digest(account.public_key().as_bytes()).into()),
                ),
            ),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(4).unwrap(),
        );
        let directory = tempfile::tempdir().unwrap();
        let identity = Arc::new(
            CloudRunnerIdentity::create_new(&directory.path().join("identity"), binding.clone())
                .unwrap(),
        );
        let stream = binding.stream(CloudChatId::from_bytes([5; 16]));
        let epoch = ChatKeyEpoch::new(6).unwrap();
        let coordinate = ChatCoordinate::new(
            super::super::super::metadata_crypto::namespace(&binding).unwrap(),
            super::super::super::metadata_crypto::runner(&binding),
            *stream.chat().as_bytes(),
            epoch.get(),
        )
        .unwrap();
        let recipient =
            PublicKey::from_bytes(identity.public_identity().unwrap().encryption_key()).unwrap();
        let grant = seal_chat_key_to_runner(
            &ChatKey::generate().unwrap(),
            &account,
            &recipient,
            &coordinate,
        )
        .unwrap();
        Self {
            directory,
            binding,
            identity,
            account,
            stream,
            epoch,
            grant,
        }
    }
    fn path(&self) -> std::path::PathBuf {
        self.directory.path().join("material")
    }
    fn create(&self) -> Arc<RunnerMaterialStore> {
        RunnerMaterialStore::create_new(
            &self.path(),
            self.binding.clone(),
            self.identity.clone(),
            2,
        )
        .unwrap()
    }
    fn install(&self, store: &RunnerMaterialStore) -> Result<(), MaterialStoreError> {
        store.admit(
            &self.stream,
            self.epoch,
            self.account.public_key().to_bytes(),
            [7; 16],
            &self.grant,
        )
    }
    fn context(&self) -> JournalCipherContext {
        JournalCipherContext::new(
            self.stream.clone(),
            JournalStorageClass::Record,
            [8; 16],
            JournalSequence::from_u64(1),
            StreamRevision::from_u64(2),
            WriterEpoch::new(3).unwrap(),
        )
    }
}

#[cfg(unix)]
#[test]
fn original_sealed_material_restores_without_online_device_and_rejects_replacement() {
    let f = Fixture::new();
    let store = f.create();
    assert!(
        RunnerMaterialStore::open_existing(&f.path(), f.binding.clone(), f.identity.clone(), 2)
            .is_err()
    );
    f.install(&store).unwrap();
    f.install(&store).unwrap();
    let cipher = store.journal_cipher();
    let context = f.context();
    let sealed = cipher
        .seal(&context, b"persisted encrypted transcript")
        .unwrap();
    drop(cipher);
    drop(store);
    let restored =
        RunnerMaterialStore::open_existing(&f.path(), f.binding.clone(), f.identity.clone(), 2)
            .unwrap();
    assert_eq!(
        restored.journal_cipher().open(&context, &sealed).unwrap(),
        b"persisted encrypted transcript"
    );
    assert!(matches!(
        restored.admit(
            &f.stream,
            ChatKeyEpoch::new(7).unwrap(),
            f.account.public_key().to_bytes(),
            [9; 16],
            &f.grant
        ),
        Err(MaterialStoreError::Crypto(_))
    ));
    assert!(
        !restored
            .retire_chat(&f.stream, ChatKeyEpoch::new(7).unwrap())
            .unwrap()
    );
    assert!(restored.retire_chat(&f.stream, f.epoch).unwrap());
    assert!(restored.journal_cipher().open(&context, &sealed).is_err());
    drop(restored);
    let retired =
        RunnerMaterialStore::open_existing(&f.path(), f.binding.clone(), f.identity.clone(), 2)
            .unwrap();
    assert!(retired.journal_cipher().open(&context, &sealed).is_err());
    assert!(matches!(
        f.install(&retired),
        Err(MaterialStoreError::Retired)
    ));
}

#[cfg(unix)]
#[test]
fn the_key_epoch_of_a_chat_is_known_until_its_material_is_retired() {
    let f = Fixture::new();
    let store = f.create();
    assert_eq!(store.chat_key_epoch(&f.stream), None);
    f.install(&store).unwrap();
    assert_eq!(store.chat_key_epoch(&f.stream), Some(f.epoch));
    assert!(store.retire_chat(&f.stream, f.epoch).unwrap());
    assert_eq!(store.chat_key_epoch(&f.stream), None);
}

#[cfg(unix)]
#[test]
fn material_store_refuses_moved_or_tampered_grants_before_key_installation() {
    let f = Fixture::new();
    let store = f.create();
    let moved = f.binding.stream(CloudChatId::from_bytes([9; 16]));
    assert!(
        store
            .admit(
                &moved,
                f.epoch,
                f.account.public_key().to_bytes(),
                [7; 16],
                &f.grant
            )
            .is_err()
    );
    let mut damaged = f.grant.clone();
    *damaged.last_mut().unwrap() ^= 1;
    assert!(
        store
            .admit(
                &f.stream,
                f.epoch,
                f.account.public_key().to_bytes(),
                [7; 16],
                &damaged
            )
            .is_err()
    );
    assert!(store.journal_cipher().chat_key_epoch(&f.stream).is_err());
    f.install(&store).unwrap();
    drop(store);
    let path = f.path().join("runner-material.v1");
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(
        RunnerMaterialStore::open_existing(&f.path(), f.binding.clone(), f.identity.clone(), 2)
            .is_err()
    );
}
