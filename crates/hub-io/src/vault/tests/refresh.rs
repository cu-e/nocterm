//! Refresh ordering and unchanged-cache behavior through the actual native vault port.
use super::*;
use crate::store::memory::StoreBoundary;

#[tokio::test(flavor = "multi_thread")]
async fn a_refresh_started_before_deletion_cannot_restore_the_deleted_offline_credential() {
    let (vault, hub, _dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("pw"))
        .await
        .unwrap();
    let (entered, release) = hub.pause_next(StoreBoundary::ChangesReady);
    let delayed = vault.get(id, binding());
    tokio::time::timeout(Duration::from_secs(5), entered.wait())
        .await
        .unwrap();
    vault.delete(id).await.unwrap();
    release.release();
    let _ = delayed.await;
    hub.set_offline(true);
    assert!(matches!(
        vault.get(id, binding()).await,
        Err(VaultError::NotFound)
    ));
    assert!(vault.list().await.unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn reading_an_unchanged_vault_does_not_replace_its_cache_file() {
    use std::os::unix::fs::MetadataExt as _;
    let (vault, _hub, dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("pw"))
        .await
        .unwrap();
    let path = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "cache")
        })
        .unwrap();
    let original = std::fs::File::open(&path).unwrap();
    let inode = original.metadata().unwrap().ino();
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "pw");
    assert_eq!(vault.list().await.unwrap().len(), 1);
    assert_eq!(std::fs::metadata(path).unwrap().ino(), inode);
}
