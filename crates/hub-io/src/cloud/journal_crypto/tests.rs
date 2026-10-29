//! Actual SDK grants and native AEAD, exercised through consumer-facing methods.
use super::*;
use nocterm_cloud_model::{
    identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, JournalSequence, NamespaceId,
        RunnerId, RunnerKeyEpoch, StreamRevision, WriterEpoch,
    },
    journal::JournalCatalogBinding,
};
use nocterm_hub_model::cloud::{CloudNamespace, CloudTextId, RunnerCoordinate};
use nocterm_keys::{AccountKeyPair, cloud::seal_chat_key_to_runner};

struct Fixture {
    directory: tempfile::TempDir,
    binding: JournalCatalogBinding,
    identity: Arc<CloudRunnerIdentity>,
    account: AccountKeyPair,
    key: ChatKey,
    coordinate: ChatCoordinate,
    stream: StreamBinding,
    epoch: ChatKeyEpoch,
    grant: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let account = AccountKeyPair::generate().unwrap();
        let fingerprint = Sha256::digest(account.public_key().as_bytes()).into();
        let binding = JournalCatalogBinding::new(
            AccountScope::new(
                HubIdentity::new(
                    NamespaceId::new("hub").unwrap(),
                    Fingerprint::from_bytes([1; 32]),
                ),
                AccountIdentity::new(
                    NamespaceId::new("account").unwrap(),
                    Fingerprint::from_bytes(fingerprint),
                ),
            ),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(4).unwrap(),
        );
        let directory = tempfile::tempdir().unwrap();
        let identity =
            Arc::new(CloudRunnerIdentity::create_new(directory.path(), binding.clone()).unwrap());
        let epoch = ChatKeyEpoch::new(5).unwrap();
        let stream = binding.stream(CloudChatId::from_bytes([4; 16]));
        let coordinate = ChatCoordinate::new(
            CloudNamespace {
                hub_instance_id: CloudTextId::new("hub").unwrap(),
                account_id: CloudTextId::new("account").unwrap(),
            },
            RunnerCoordinate {
                id: [3; 16],
                key_epoch: 4,
            },
            [4; 16],
            5,
        )
        .unwrap();
        let key = ChatKey::generate().unwrap();
        let recipient =
            PublicKey::from_bytes(identity.public_identity().unwrap().encryption_key()).unwrap();
        let grant = seal_chat_key_to_runner(&key, &account, &recipient, &coordinate).unwrap();
        Self {
            directory,
            binding,
            identity,
            account,
            key,
            coordinate,
            stream,
            epoch,
            grant,
        }
    }
    fn install(&self, crypto: &RunnerJournalCrypto) {
        crypto
            .install_grant(
                &self.stream,
                self.epoch,
                self.account.public_key().to_bytes(),
                &self.grant,
            )
            .unwrap();
    }
    fn context(&self, class: JournalStorageClass) -> JournalCipherContext {
        JournalCipherContext::new(
            self.stream.clone(),
            class,
            [8; 16],
            JournalSequence::from_u64(1),
            StreamRevision::from_u64(2),
            WriterEpoch::new(3).unwrap(),
        )
    }
}

#[cfg(unix)]
#[test]
fn material_grants_use_actual_account_pins_chat_epochs_and_sdk_journal_purposes() {
    let fixture = Fixture::new();
    let crypto = RunnerJournalCrypto::new(fixture.identity.clone(), 1).unwrap();
    assert!(crypto.chat_key_epoch(&fixture.stream).is_err());
    fixture.install(&crypto);
    assert_eq!(
        crypto.chat_key_epoch(&fixture.stream).unwrap(),
        fixture.epoch
    );
    let context = fixture.context(JournalStorageClass::Record);
    let sealed = crypto.seal(&context, b"private native record").unwrap();
    let expected = JournalCoordinate::new(
        fixture.coordinate.clone(),
        JournalClass::Record,
        [8; 16],
        1,
        2,
        3,
    )
    .unwrap();
    assert_eq!(
        &**fixture.key.open_journal(&expected, &sealed).unwrap(),
        b"private native record"
    );
    assert_eq!(
        crypto.open(&context, &sealed).unwrap(),
        b"private native record"
    );
    assert!(
        crypto
            .open(&fixture.context(JournalStorageClass::Header), &sealed)
            .is_err()
    );
    let mut tampered = sealed;
    *tampered.last_mut().unwrap() ^= 1;
    assert!(crypto.open(&context, &tampered).is_err());
    assert!(matches!(
        crypto.install_grant(&fixture.stream, fixture.epoch, [9; 32], &fixture.grant),
        Err(RunnerMaterialError::Binding)
    ));
    assert!(matches!(
        crypto.install_grant(
            &fixture.stream,
            fixture.epoch.next().unwrap(),
            fixture.account.public_key().to_bytes(),
            &fixture.grant
        ),
        Err(RunnerMaterialError::RotationRequired)
    ));
    assert!(matches!(
        crypto.install_grant(
            &fixture.binding.stream(CloudChatId::from_bytes([6; 16])),
            fixture.epoch,
            fixture.account.public_key().to_bytes(),
            &fixture.grant
        ),
        Err(RunnerMaterialError::Capacity)
    ));
}

#[cfg(unix)]
#[test]
fn material_reopens_for_the_original_persistent_runner_and_retirement_is_conditional() {
    let fixture = Fixture::new();
    let crypto = RunnerJournalCrypto::new(fixture.identity.clone(), 1).unwrap();
    fixture.install(&crypto);
    let context = fixture.context(JournalStorageClass::Snapshot);
    let sealed = crypto.seal(&context, b"original archive").unwrap();
    let Fixture {
        directory,
        binding,
        identity,
        account,
        key: _,
        coordinate: _,
        stream,
        epoch,
        grant,
    } = fixture;
    drop(crypto);
    drop(identity);
    let restored = Arc::new(CloudRunnerIdentity::open_existing(directory.path(), binding).unwrap());
    let crypto = RunnerJournalCrypto::new(restored.clone(), 1).unwrap();
    crypto
        .install_grant(&stream, epoch, account.public_key().to_bytes(), &grant)
        .unwrap();
    assert_eq!(crypto.open(&context, &sealed).unwrap(), b"original archive");
    assert!(!crypto.retire_chat(&stream, epoch.next().unwrap()).unwrap());
    assert_eq!(crypto.open(&context, &sealed).unwrap(), b"original archive");
    assert!(crypto.retire_chat(&stream, epoch).unwrap());
    assert!(crypto.open(&context, &sealed).is_err());
    assert!(matches!(
        crypto.install_grant(&stream, epoch, account.public_key().to_bytes(), &grant),
        Err(RunnerMaterialError::Existing)
    ));
    crypto.retire();
    restored.retire();
    assert!(crypto.chat_key_epoch(&stream).is_err());
}
