use super::*;
use nocterm_session::Secret;
use ssh_key::{
    LineEnding, Mpint, PrivateKey,
    private::{DsaKeypair, DsaPrivateKey, Ed25519Keypair, KeypairData, SkEd25519},
    public::{DsaPublicKey, Ed25519PublicKey},
};

const ED25519: &str = include_str!("tests/fixtures/ed25519");
const ED25519_PUB: &str = include_str!("tests/fixtures/ed25519.pub");
const ED25519_ENCRYPTED: &str = include_str!("tests/fixtures/ed25519-encrypted");
const ECDSA_P256: &str = include_str!("tests/fixtures/ecdsa-p256");
const ECDSA_P521: &str = include_str!("tests/fixtures/ecdsa-p521");
const RSA_2048: &str = include_str!("tests/fixtures/rsa-2048");
const RSA_2048_PUB: &str = include_str!("tests/fixtures/rsa-2048.pub");
const RSA_1024: &str = include_str!("tests/fixtures/rsa-1024");
const PASSPHRASE: &str = "correct horse";

fn text(value: &str) -> Secret {
    Secret::new(value)
}

fn import(value: &str) -> Result<KeyMaterial, KeyImportError> {
    KeyMaterial::import(&text(value), None)
}

#[test]
fn ed25519_ecdsa_and_rsa_keys_import() {
    for fixture in [ED25519, ECDSA_P256, ECDSA_P521, RSA_2048] {
        assert!(import(fixture).is_ok());
    }
}

#[test]
fn fingerprint_matches_ssh_keygen() {
    // `ssh-keygen -lf` of the matching `.pub` fixtures.
    assert_eq!(
        import(ED25519).unwrap().fingerprint(),
        "SHA256:IcTCIphGPK3d9UZTQNsCnzJYrbdJNLARbB2m/D+T+io"
    );
    assert_eq!(
        import(RSA_2048).unwrap().fingerprint(),
        "SHA256:Hg6hPjFdxRUOyflF2elLWNpOwv3XKdnwzUOey/mSuWI"
    );
    assert_eq!(
        import(ECDSA_P256).unwrap().fingerprint(),
        "SHA256:/i3wXEA9O/yz/PQwAct2XNEHmGR8r2Ukj8tjSSk7Z3M"
    );
}

#[test]
fn public_key_text_drops_the_comment_and_matches_the_pub_file() {
    let material = import(ED25519).unwrap();
    let from_file = PublicKeyText::parse(ED25519_PUB).unwrap();
    assert_eq!(material.public_key(), from_file);
    assert!(!from_file.as_str().contains("fixture"));
    assert_eq!(from_file.as_str().split(' ').count(), 2);
    assert_eq!(from_file.fingerprint(), material.fingerprint());
    assert_eq!(
        import(RSA_2048).unwrap().public_key(),
        PublicKeyText::parse(RSA_2048_PUB).unwrap()
    );
}

#[test]
fn public_key_text_rejects_what_is_not_a_key() {
    assert!(matches!(
        PublicKeyText::parse("ssh-ed25519 not-base64!"),
        Err(KeyImportError::Malformed(_))
    ));
    assert!(matches!(
        PublicKeyText::parse(""),
        Err(KeyImportError::Malformed(_))
    ));
}

#[test]
fn normalization_is_idempotent_and_uses_lf() {
    let first = import(ED25519).unwrap().to_secret();
    assert!(!first.expose().contains('\r'));
    let second = KeyMaterial::import(&first, None).unwrap().to_secret();
    assert_eq!(first, second);
    let crlf = ED25519.replace('\n', "\r\n");
    assert_eq!(import(&crlf).unwrap().to_secret(), first);
}

#[test]
fn a_passphrase_is_removed_by_normalization() {
    let material = KeyMaterial::import(&text(ED25519_ENCRYPTED), Some(&text(PASSPHRASE))).unwrap();
    let stored = material.to_secret();
    // The stored text opens without a passphrase and is the same key.
    let again = KeyMaterial::import(&stored, None).unwrap();
    assert_eq!(again.fingerprint(), material.fingerprint());
    assert_eq!(
        again.fingerprint(),
        "SHA256:ldOV9Mf2de4K+4xGetZrah8ClNWVWHg+Fwr3MYMXpOQ"
    );
}

#[test]
fn an_encrypted_key_asks_for_its_passphrase() {
    assert!(matches!(
        import(ED25519_ENCRYPTED),
        Err(KeyImportError::PassphraseRequired)
    ));
}

#[test]
fn a_wrong_passphrase_is_told_apart_from_a_missing_one() {
    let wrong = KeyMaterial::import(&text(ED25519_ENCRYPTED), Some(&text("nope")));
    assert!(matches!(wrong, Err(KeyImportError::WrongPassphrase)));
}

#[test]
fn text_that_is_not_a_key_is_malformed() {
    assert!(matches!(
        import("not a key"),
        Err(KeyImportError::Malformed(_))
    ));
    assert!(matches!(import(""), Err(KeyImportError::Malformed(_))));
}

#[test]
fn rsa_below_2048_bits_is_too_weak() {
    assert!(matches!(
        import(RSA_1024),
        Err(KeyImportError::TooWeak { bits: 1024 })
    ));
}

#[test]
fn dsa_keys_are_refused() {
    let number = || Mpint::from_positive_bytes(&[7]);
    let public = DsaPublicKey::new(number(), number(), number(), number()).unwrap();
    let private = DsaPrivateKey::new(number()).unwrap();
    let pair = DsaKeypair::new(public, private).unwrap();
    let key = PrivateKey::new(KeypairData::Dsa(pair), "fixture").unwrap();
    let text = key.to_openssh(LineEnding::LF).unwrap();
    assert!(matches!(
        import(&text),
        Err(KeyImportError::Unsupported { algorithm }) if algorithm == "ssh-dss"
    ));
}

#[test]
fn security_keys_are_refused() {
    let pair = Ed25519Keypair::from_seed(&[3; 32]);
    let public = ssh_key::public::SkEd25519::new(Ed25519PublicKey::from(&pair), "ssh:");
    let sk = SkEd25519::new(public, 1, vec![1, 2, 3]).unwrap();
    let key = PrivateKey::new(KeypairData::SkEd25519(sk), "fixture").unwrap();
    let text = key.to_openssh(LineEnding::LF).unwrap();
    assert!(matches!(
        import(&text),
        Err(KeyImportError::Unsupported { algorithm }) if algorithm.starts_with("sk-")
    ));
}

#[test]
fn ed25519_from_seed_is_deterministic() {
    let first = KeyMaterial::ed25519_from_seed(&[9; 32], "me@host").unwrap();
    let second = KeyMaterial::ed25519_from_seed(&[9; 32], "me@host").unwrap();
    let other = KeyMaterial::ed25519_from_seed(&[8; 32], "me@host").unwrap();
    assert_eq!(first.to_secret(), second.to_secret());
    assert_eq!(first.public_key(), second.public_key());
    assert_ne!(first.public_key(), other.public_key());
    assert!(first.public_key().as_str().starts_with("ssh-ed25519 "));
}

#[test]
fn a_generated_key_imports_back_unchanged() {
    let generated = KeyMaterial::ed25519_from_seed(&[1; 32], "fixture").unwrap();
    let again = KeyMaterial::import(&generated.to_secret(), None).unwrap();
    assert_eq!(again.public_key(), generated.public_key());
    assert_eq!(again.to_secret(), generated.to_secret());
}

#[test]
fn key_material_never_formats_the_private_key() {
    let material = import(ED25519).unwrap();
    let shown = format!("{material:?}");
    assert!(shown.contains(material.fingerprint()));
    let body = ED25519.lines().nth(1).unwrap();
    assert!(!shown.contains(body));
    let secret = format!("{:?}", material.to_secret());
    assert!(!secret.contains(body));
}

mod memory_port {
    use super::*;
    use crate::{
        KeySource, NoVaultSshKeys, VaultSshKeys, VaultSshKeysError, testing::MemoryVaultSshKeys,
    };
    use futures::executor::block_on;

    #[test]
    fn generated_and_imported_keys_are_listed_with_their_public_half() {
        let keys = MemoryVaultSshKeys::default();
        let generated = block_on(keys.generate("laptop".into())).unwrap();
        let imported =
            block_on(keys.import("work".into(), KeySource::Text(text(ED25519)), None)).unwrap();
        let listed = block_on(keys.list()).unwrap();
        assert_eq!(listed, vec![generated.clone(), imported.clone()]);
        assert_ne!(generated.credential, imported.credential);
        assert_eq!(
            imported.public_key,
            PublicKeyText::parse(ED25519_PUB).unwrap()
        );
    }

    #[test]
    fn import_reads_the_file_and_asks_for_a_passphrase() {
        let keys = MemoryVaultSshKeys::default().with_file("/keys/id", text(ED25519_ENCRYPTED));
        let source = || KeySource::File("/keys/id".into());
        let refused = block_on(keys.import("work".into(), source(), None));
        assert!(matches!(
            refused,
            Err(VaultSshKeysError::Key(KeyImportError::PassphraseRequired))
        ));
        let stored =
            block_on(keys.import("work".into(), source(), Some(text(PASSPHRASE)))).unwrap();
        let secret = keys.secret_of(stored.credential).unwrap();
        assert!(KeyMaterial::import(&secret, None).is_ok());
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let keys = MemoryVaultSshKeys::default();
        let result = block_on(keys.import("work".into(), KeySource::File("/nowhere".into()), None));
        assert!(matches!(result, Err(VaultSshKeysError::Read(_))));
    }

    #[test]
    fn a_locked_vault_refuses_everything() {
        let keys = MemoryVaultSshKeys::default();
        keys.set_locked(true);
        assert!(matches!(
            block_on(keys.list()),
            Err(VaultSshKeysError::Locked)
        ));
        assert!(matches!(
            block_on(keys.generate("x".into())),
            Err(VaultSshKeysError::Locked)
        ));
    }

    #[test]
    fn without_a_vault_every_call_says_so() {
        let keys = NoVaultSshKeys;
        assert!(matches!(
            block_on(keys.list()),
            Err(VaultSshKeysError::Unavailable)
        ));
    }
}

mod errors {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn a_malformed_key_keeps_the_original_cause() {
        let error = import("not a key").unwrap_err();
        assert!(error.source().is_some());
    }
}
