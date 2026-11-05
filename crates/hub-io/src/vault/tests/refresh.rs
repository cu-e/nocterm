//! Refresh ordering and unchanged-cache behavior through the actual native vault port.
use super::*;
use crate::store::memory::StoreBoundary;
use nocterm_hub_proto::v1;

#[tokio::test(flavor = "multi_thread")]
async fn an_unordered_change_page_after_the_requested_cursor_is_accepted() {
    let (vault, hub, _dir) = vault();
    let first = vault
        .put(None, "First".into(), binding(), Secret::new("one"))
        .await
        .unwrap();
    let second = vault
        .put(None, "Second".into(), binding(), Secret::new("two"))
        .await
        .unwrap();
    let collection = hub
        .collections_of_kind(crate::store::VAULT_KIND)
        .pop()
        .unwrap();
    let mut changes = hub.objects(&collection);
    assert_eq!(changes.len(), 2);
    changes[0].stream_version = 4;
    changes[1].stream_version = 3;
    hub.script_changes(vec![v1::ChangeBatch {
        changes,
        head: 4,
        more: false,
    }]);
    assert_eq!(vault.list().await.unwrap().len(), 2);
    hub.set_offline(true);
    assert_eq!(vault.get(first, binding()).await.unwrap().expose(), "one");
    assert_eq!(vault.get(second, binding()).await.unwrap().expose(), "two");
}

#[tokio::test(flavor = "multi_thread")]
async fn unordered_changes_for_one_object_keep_its_newest_version() {
    let (vault, hub, _dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("old"))
        .await
        .unwrap();
    let collection = hub
        .collections_of_kind(crate::store::VAULT_KIND)
        .pop()
        .unwrap();
    let mut old = hub.objects(&collection).pop().unwrap();
    vault
        .put(Some(id), "Changed".into(), binding(), Secret::new("new"))
        .await
        .unwrap();
    let mut new = hub.objects(&collection).pop().unwrap();
    new.stream_version = 4;
    new.version = 4;
    old.stream_version = 3;
    old.version = 3;
    hub.script_changes(vec![v1::ChangeBatch {
        changes: vec![new, old],
        head: 4,
        more: false,
    }]);
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "new");
    hub.set_offline(true);
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "new");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_write_remains_successful_when_only_its_offline_copy_cannot_be_published() {
    let (vault, hub, dir) = vault();
    let first = vault
        .put(None, "First".into(), binding(), Secret::new("one"))
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
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let second = vault
        .put(None, "Second".into(), binding(), Secret::new("two"))
        .await
        .unwrap();
    hub.set_offline(true);
    assert_eq!(vault.get(first, binding()).await.unwrap().expose(), "one");
    assert_eq!(vault.get(second, binding()).await.unwrap().expose(), "two");
    assert_eq!(vault.list().await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nonadvancing_change_page_is_refused_once_and_preserves_offline_credentials() {
    let (vault, hub, _dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("pw"))
        .await
        .unwrap();
    let before = hub.change_calls();
    hub.script_changes(vec![v1::ChangeBatch {
        changes: Vec::new(),
        head: 100,
        more: true,
    }]);
    assert!(matches!(vault.list().await, Err(VaultError::Io(_))));
    assert_eq!(hub.change_calls(), before + 1);
    hub.set_offline(true);
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "pw");
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_snapshot_content_is_refused_before_cache_publication() {
    let (vault, hub, _dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("pw"))
        .await
        .unwrap();
    hub.script_changes(vec![v1::ChangeBatch {
        changes: vec![v1::StoredObject {
            object_id: "new-format".into(),
            kind: "future".into(),
            stream_version: 2,
            version: 1,
            ciphertext: vec![0; 19 * 1024 * 1024],
            ..Default::default()
        }],
        head: 2,
        more: false,
    }]);
    assert!(matches!(vault.list().await, Err(VaultError::Io(_))));
    hub.set_offline(true);
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "pw");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_progressing_but_unending_change_stream_has_a_finite_page_budget() {
    let (vault, hub, _dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("pw"))
        .await
        .unwrap();
    let before = hub.change_calls();
    hub.script_changes(
        (2..=1_026)
            .map(|cursor| v1::ChangeBatch {
                changes: vec![v1::StoredObject {
                    object_id: "future-object".into(),
                    kind: "future".into(),
                    stream_version: cursor,
                    version: cursor,
                    deleted: true,
                    ..Default::default()
                }],
                head: cursor,
                more: true,
            })
            .collect(),
    );
    assert!(matches!(vault.list().await, Err(VaultError::Io(_))));
    assert_eq!(hub.change_calls(), before + 1_025);
    hub.set_offline(true);
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "pw");
}

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
