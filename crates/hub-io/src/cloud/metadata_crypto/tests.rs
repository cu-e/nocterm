//! Actual persisted SDK identities seal both metadata domains without content keys.
use super::*;
use nocterm_cloud_model::{authority::StreamBinding, identity::*};

fn binding() -> JournalCatalogBinding {
    JournalCatalogBinding::new(
        AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new("account").unwrap(),
                Fingerprint::from_bytes([2; 32]),
            ),
        ),
        RunnerId::from_bytes([3; 16]),
        RunnerKeyEpoch::new(4).unwrap(),
    )
}
fn head(binding: &JournalCatalogBinding, chat: u8, writer: u64) -> JournalChatHeadContext {
    JournalChatHeadContext::new(
        binding.stream(CloudChatId::from_bytes([chat; 16])),
        Some(WriterEpoch::new(writer).unwrap()),
        8,
    )
    .unwrap()
}

#[cfg(unix)]
#[test]
fn native_metadata_crypto_restores_actual_persistent_keys_with_separate_purposes() {
    let directory = tempfile::tempdir().unwrap();
    let binding = binding();
    let identity =
        Arc::new(CloudRunnerIdentity::create_new(directory.path(), binding.clone()).unwrap());
    let crypto = RunnerMetadataCrypto::new(identity.clone());
    let catalog = JournalCatalogContext::new(binding.clone(), 9).unwrap();
    let current = head(&binding, 5, 1);
    let sealed_catalog = JournalCatalogCipher::seal(&crypto, &catalog, b"catalog").unwrap();
    let sealed_head =
        JournalChatHeadCipher::seal(&crypto, &current, b"actual protected head").unwrap();
    assert_eq!(
        JournalCatalogCipher::open(&crypto, &catalog, &sealed_catalog).unwrap(),
        b"catalog"
    );
    assert_eq!(
        JournalChatHeadCipher::open(&crypto, &current, &sealed_head).unwrap(),
        b"actual protected head"
    );
    assert!(JournalChatHeadCipher::open(&crypto, &current, &sealed_catalog).is_err());
    assert!(JournalCatalogCipher::open(&crypto, &catalog, &sealed_head).is_err());
    drop(crypto);
    drop(identity);
    let restored = Arc::new(CloudRunnerIdentity::open_existing(directory.path(), binding).unwrap());
    let crypto = RunnerMetadataCrypto::new(restored.clone());
    assert_eq!(
        JournalChatHeadCipher::open(&crypto, &current, &sealed_head).unwrap(),
        b"actual protected head"
    );
    assert_eq!(
        JournalCatalogCipher::open(&crypto, &catalog, &sealed_catalog).unwrap(),
        b"catalog"
    );
    restored.retire();
    assert!(JournalChatHeadCipher::open(&crypto, &current, &sealed_head).is_err());
    assert!(JournalCatalogCipher::seal(&crypto, &catalog, b"new").is_err());
}

#[cfg(unix)]
#[test]
fn metadata_crypto_refuses_moved_tampered_oversized_and_cross_source_values() {
    let directory = tempfile::tempdir().unwrap();
    let binding = binding();
    let identity =
        Arc::new(CloudRunnerIdentity::create_new(directory.path(), binding.clone()).unwrap());
    let crypto = RunnerMetadataCrypto::new(identity);
    let original = head(&binding, 5, 1);
    let sealed = JournalChatHeadCipher::seal(&crypto, &original, b"head").unwrap();
    for context in [
        head(&binding, 6, 1),
        head(&binding, 5, 2),
        JournalChatHeadContext::new(original.stream().clone(), original.writer(), 9).unwrap(),
    ] {
        assert!(JournalChatHeadCipher::open(&crypto, &context, &sealed).is_err());
    }
    let wrong_scope = AccountScope::new(
        binding.scope().hub().clone(),
        AccountIdentity::new(
            binding.scope().account().id().clone(),
            Fingerprint::from_bytes([9; 32]),
        ),
    );
    let wrong = StreamBinding::new(
        wrong_scope,
        binding.runner(),
        binding.epoch(),
        original.stream().chat(),
    );
    let wrong = JournalChatHeadContext::new(wrong, original.writer(), original.revision()).unwrap();
    assert!(matches!(
        JournalChatHeadCipher::seal(&crypto, &wrong, b"head"),
        Err(JournalError::Fenced)
    ));
    let mut changed = sealed;
    *changed.last_mut().unwrap() ^= 1;
    assert!(JournalChatHeadCipher::open(&crypto, &original, &changed).is_err());
    assert!(matches!(
        JournalChatHeadCipher::seal(&crypto, &original, &vec![0; MAX_NATIVE_CHAT_HEAD_BYTES + 1]),
        Err(JournalError::Capacity)
    ));
    assert!(matches!(
        JournalChatHeadCipher::open(
            &crypto,
            &original,
            &vec![0; MAX_NATIVE_CHAT_HEAD_CIPHERTEXT_BYTES + 1]
        ),
        Err(JournalError::Capacity)
    ));
    assert!(JournalChatHeadCipher::seal(&crypto, &original, b"").is_err());
}
