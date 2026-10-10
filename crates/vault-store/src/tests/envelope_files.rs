use crate::{Vault, VaultService};
use fs2::FileExt as _;
use nocterm_session::{Secret, Target};
use nocterm_vault::*;
use std::{
    fs::{self, OpenOptions},
    path::Path,
};

fn password() -> Secret {
    Secret::new("unique test master passphrase")
}
fn binding() -> CredentialBinding {
    CredentialBinding::Password {
        target: Target::new("vault-user", "vault-secret-host", 22),
    }
}
fn created(path: &Path) -> Vault {
    let mut vault = Vault::new(path);
    vault.create(password()).unwrap();
    vault
}
#[test]
fn wrong_password_tamper_and_lock_do_not_expose_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let mut vault = created(&path);
    let id = vault
        .put(
            None,
            "private metadata".into(),
            binding(),
            Secret::new("very-private-password"),
        )
        .unwrap();
    let original = fs::read(&path).unwrap();
    for plaintext in [
        "very-private-password",
        "private metadata",
        "vault-user",
        "vault-secret-host",
    ] {
        assert!(
            !original
                .windows(plaintext.len())
                .any(|bytes| bytes == plaintext.as_bytes())
        );
    }
    assert!(!format!("{vault:?}").contains("very-private-password"));
    vault.lock();
    assert!(matches!(vault.get(id, &binding()), Err(VaultError::Locked)));
    assert!(matches!(
        vault.unlock(Secret::new("wrong password")),
        Err(VaultError::Authentication)
    ));
    assert!(!vault.is_unlocked());
    let mut corrupted = original.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    fs::write(&path, &corrupted).unwrap();
    assert!(matches!(
        vault.unlock(password()),
        Err(VaultError::Authentication)
    ));
    fs::write(&path, original).unwrap();
    vault.unlock(password()).unwrap();
    assert_eq!(
        vault.get(id, &binding()).unwrap().expose(),
        "very-private-password"
    );
}
#[test]
fn hostile_and_truncated_headers_reject_before_kdf() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let _vault = created(&path);
    let original = fs::read(&path).unwrap();
    for length in [0, 7, 24, 108, 171] {
        assert!(matches!(
            EnvelopeInfo::parse(&original[..length.min(original.len())]),
            Err(VaultError::Format)
        ));
    }
    for offset in [8, 10, 11, 12, 16, 20, 104] {
        let mut bytes = original.clone();
        bytes[offset] ^= 255;
        assert!(matches!(
            EnvelopeInfo::parse(&bytes),
            Err(VaultError::Format)
        ));
    }
    let mut bytes = original;
    bytes.push(0);
    assert!(matches!(
        EnvelopeInfo::parse(&bytes),
        Err(VaultError::Format)
    ));
}
#[test]
fn rotation_changes_master_password_and_uses_fresh_nonces() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let mut vault = created(&path);
    let before = fs::read(&path).unwrap();
    let id = vault
        .put(
            None,
            "credential".into(),
            binding(),
            Secret::new("password"),
        )
        .unwrap();
    let after = fs::read(&path).unwrap();
    assert_ne!(&before[56..80], &after[56..80]);
    assert_ne!(&before[80..104], &after[80..104]);
    vault
        .change_password(Secret::new("different master passphrase"))
        .unwrap();
    let rotated = fs::read(&path).unwrap();
    assert_ne!(&after[24..40], &rotated[24..40]);
    vault.lock();
    assert!(vault.unlock(password()).is_err());
    vault
        .unlock(Secret::new("different master passphrase"))
        .unwrap();
    assert_eq!(vault.get(id, &binding()).unwrap().expose(), "password");
}
#[test]
fn stale_writers_and_cancelled_commit_preserve_last_good_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let mut first = created(&path);
    let mut second = Vault::new(&path);
    second.unlock(password()).unwrap();
    first
        .put(None, "one".into(), binding(), Secret::new("one-secret"))
        .unwrap();
    let good = fs::read(&path).unwrap();
    assert!(matches!(
        second.put(None, "two".into(), binding(), Secret::new("two-secret")),
        Err(VaultError::Conflict)
    ));
    assert_eq!(fs::read(&path).unwrap(), good);
    assert!(second.list().unwrap().is_empty());
    first = first.with_cancellation(DeviceCancellation::for_epoch(
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2)),
        1,
    ));
    assert!(matches!(
        first.put(None, "three".into(), binding(), Secret::new("three-secret")),
        Err(VaultError::Cancelled)
    ));
    assert_eq!(fs::read(&path).unwrap(), good);
    assert_eq!(first.list().unwrap().len(), 1);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .unwrap();
    lock.try_lock_exclusive().unwrap();
    first = first.with_cancellation(DeviceCancellation::never());
    assert!(matches!(
        first.delete(first.list().unwrap()[0].id),
        Err(VaultError::Busy)
    ));
    assert_eq!(fs::read(&path).unwrap(), good);
}
#[test]
fn credential_binding_and_private_permissions_are_enforced() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let mut vault = created(&path);
    let id = vault
        .put(None, "one".into(), binding(), Secret::new("secret"))
        .unwrap();
    let other = CredentialBinding::Password {
        target: Target::new("other", "elsewhere", 22),
    };
    assert!(matches!(vault.get(id, &other), Err(VaultError::Binding)));
    assert!(matches!(
        vault.put(Some(id), "other".into(), other, Secret::new("other")),
        Err(VaultError::Binding)
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn worker_lock_invalidates_inflight_unlock_and_autolocks() {
    use futures::executor::block_on;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let service = VaultService::new(&path, std::time::Duration::from_secs(1)).unwrap();
    block_on(service.create(password())).unwrap();
    assert!(service.is_unlocked());
    service.lock();
    assert!(!service.is_unlocked());
    let unlock = service.unlock(password());
    service.lock();
    assert!(matches!(block_on(unlock), Err(VaultError::Cancelled)));
    assert!(!service.is_unlocked());
    block_on(service.unlock(password())).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1400));
    assert!(!service.is_unlocked());
    assert!(matches!(block_on(service.list()), Err(VaultError::Locked)));
}

#[cfg(unix)]
#[test]
fn failed_staging_retains_disk_and_unlocked_records() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let mut vault = created(&path);
    let original = fs::read(&path).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = vault.put(None, "rejected".into(), binding(), Secret::new("secret"));
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(result, Err(VaultError::Io(_))));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(vault.list().unwrap().is_empty());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn bounded_file_reads_reject_oversized_envelopes_and_non_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let mut vault = Vault::new(&path);
    let file = fs::File::create(&path).unwrap();
    file.set_len(2_000_000).unwrap();
    assert!(matches!(vault.unlock(password()), Err(VaultError::Limit)));
    assert!(!vault.is_unlocked());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(matches!(vault.unlock(password()), Err(VaultError::Format)));
}

#[cfg(unix)]
#[test]
fn envelope_symlink_rejection_preserves_its_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let victim = directory.path().join("victim");
    fs::write(&victim, b"unrelated bytes").unwrap();
    let path = directory.path().join("vault");
    symlink(&victim, &path).unwrap();
    let mut vault = Vault::new(&path);
    assert!(matches!(vault.unlock(password()), Err(VaultError::Format)));
    assert!(!vault.is_unlocked());
    assert_eq!(fs::read(&victim).unwrap(), b"unrelated bytes");
}
