//! Public identity actions exercise native persistence without exporting seeds.
use super::*;
use nocterm_cloud_model::identity::{
    AccountIdentity, AccountScope, Fingerprint, HubIdentity, NamespaceId, RunnerId, RunnerKeyEpoch,
};

fn binding(account_key: u8) -> JournalCatalogBinding {
    JournalCatalogBinding::new(
        AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new("account").unwrap(),
                Fingerprint::from_bytes([account_key; 32]),
            ),
        ),
        RunnerId::from_bytes([3; 16]),
        RunnerKeyEpoch::new(4).unwrap(),
    )
}

#[cfg(unix)]
#[test]
fn private_runner_identity_reopens_the_original_keys_and_refuses_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let expected = binding(2);
    let identity = CloudRunnerIdentity::create_new(directory.path(), expected.clone()).unwrap();
    let public = identity.public_identity().unwrap();
    assert_eq!(public.binding(), &expected);
    assert!(CloudRunnerIdentity::open_existing(directory.path(), expected.clone()).is_err());
    assert!(!format!("{identity:?}").contains("encryption"));
    assert!(!format!("{identity:?}").contains("signing"));
    drop(identity);
    assert!(CloudRunnerIdentity::create_new(directory.path(), expected.clone()).is_err());
    let restored = CloudRunnerIdentity::open_existing(directory.path(), expected).unwrap();
    assert_eq!(restored.public_identity().unwrap(), public);
    restored.retire();
    assert!(matches!(
        restored.public_identity(),
        Err(CloudRunnerIdentityError::Unavailable)
    ));
    drop(restored);
    let restarted = CloudRunnerIdentity::open_existing(directory.path(), binding(2)).unwrap();
    assert_eq!(restarted.public_identity().unwrap(), public);
}

#[cfg(unix)]
#[test]
fn missing_corrupt_or_cross_account_identity_never_generates_a_fallback() {
    let directory = tempfile::tempdir().unwrap();
    assert!(CloudRunnerIdentity::open_existing(directory.path(), binding(2)).is_err());
    assert!(!directory.path().join("runner-identity.v1").exists());
    let original = CloudRunnerIdentity::create_new(directory.path(), binding(2))
        .unwrap()
        .public_identity()
        .unwrap();
    assert!(matches!(
        CloudRunnerIdentity::open_existing(directory.path(), binding(9)),
        Err(CloudRunnerIdentityError::Binding)
    ));
    assert_eq!(
        CloudRunnerIdentity::open_existing(directory.path(), binding(2))
            .unwrap()
            .public_identity()
            .unwrap(),
        original
    );
    for bytes in [vec![0; 159], vec![0; 160], vec![0; 161]] {
        std::fs::write(directory.path().join("runner-identity.v1"), bytes).unwrap();
        assert!(matches!(
            CloudRunnerIdentity::open_existing(directory.path(), binding(2)),
            Err(CloudRunnerIdentityError::Format)
        ));
    }
}

#[cfg(unix)]
#[test]
fn seed_corruption_cannot_silently_restore_a_different_runner_key() {
    use std::{
        fs::OpenOptions,
        io::{Read as _, Seek as _, SeekFrom, Write as _},
    };
    use zeroize::Zeroizing;
    let directory = tempfile::tempdir().unwrap();
    drop(CloudRunnerIdentity::create_new(directory.path(), binding(2)).unwrap());
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("runner-identity.v1"))
        .unwrap();
    file.seek(SeekFrom::Start(64)).unwrap();
    let mut byte = Zeroizing::new([0u8; 1]);
    file.read_exact(byte.as_mut()).unwrap();
    byte[0] ^= 1;
    file.seek(SeekFrom::Start(64)).unwrap();
    file.write_all(byte.as_ref()).unwrap();
    file.sync_all().unwrap();
    assert!(matches!(
        CloudRunnerIdentity::open_existing(directory.path(), binding(2)),
        Err(CloudRunnerIdentityError::Binding)
    ));
    assert_eq!(
        std::fs::metadata(directory.path().join("runner-identity.v1"))
            .unwrap()
            .len(),
        160
    );
}

#[cfg(unix)]
#[test]
fn identity_permission_or_alias_refusal_leaves_the_existing_file_untouched() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let expected = binding(2);
    let original = CloudRunnerIdentity::create_new(directory.path(), expected.clone())
        .unwrap()
        .public_identity()
        .unwrap();
    let path = directory.path().join("runner-identity.v1");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        CloudRunnerIdentity::open_existing(directory.path(), expected.clone()),
        Err(CloudRunnerIdentityError::Private(_))
    ));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        CloudRunnerIdentity::open_existing(directory.path(), expected.clone())
            .unwrap()
            .public_identity()
            .unwrap(),
        original
    );
    let link = directory.path().join("backup-link");
    std::fs::hard_link(&path, &link).unwrap();
    assert!(matches!(
        CloudRunnerIdentity::open_existing(directory.path(), expected.clone()),
        Err(CloudRunnerIdentityError::Private(_))
    ));
    std::fs::remove_file(&link).unwrap();
    let moved = directory.path().join("moved");
    std::fs::rename(&path, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &path).unwrap();
    assert!(matches!(
        CloudRunnerIdentity::open_existing(directory.path(), expected),
        Err(CloudRunnerIdentityError::Private(_))
    ));
    assert_eq!(std::fs::metadata(moved).unwrap().len(), 160);
}
