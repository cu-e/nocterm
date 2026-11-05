//! The provider key of an own-runner agent lives in the account's encrypted collection.

use std::sync::Arc;

use super::*;
use crate::account::{operations::OpenedWork, testing};
use crate::store::memory::MemoryHub;

fn work() -> (
    OpenedWork,
    Arc<MemoryHub>,
    tempfile::TempDir,
    Arc<crate::account::operations::AccountOperations>,
) {
    let hub = Arc::new(MemoryHub::default());
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, _record) = testing::signed_in(dir.path());
    let operations = testing::native_operations(&slot, hub.clone());
    (operations.opened().unwrap(), hub, dir, operations)
}

#[tokio::test]
async fn a_stored_key_is_read_back_on_the_account_and_replaced_by_the_next_save() {
    let (work, _hub, _dir, _operations) = work();
    put(
        &work,
        "agent-1",
        &OwnAgentKey::new("openai_chat", &["gpt".into()], "sk-first").unwrap(),
    )
    .await
    .unwrap();
    put(
        &work,
        "agent-1",
        &OwnAgentKey::new("openai_chat", &["gpt".into()], "sk-second").unwrap(),
    )
    .await
    .unwrap();
    let read = get(&work, "agent-1").await.unwrap().unwrap();
    assert_eq!(read.expose(), "sk-second");
    assert_eq!(read.provider(), "openai_chat");
    assert_eq!(read.model(), "gpt");
    assert!(get(&work, "agent-2").await.unwrap().is_none());
}

#[tokio::test]
async fn a_forgotten_key_is_gone_and_forgetting_twice_succeeds() {
    let (work, _hub, _dir, _operations) = work();
    put(
        &work,
        "agent-1",
        &OwnAgentKey::new("openai_chat", &["gpt".into()], "sk-first").unwrap(),
    )
    .await
    .unwrap();
    remove(&work, "agent-1").await.unwrap();
    assert!(get(&work, "agent-1").await.unwrap().is_none());
    remove(&work, "agent-1").await.unwrap();
    remove(&work, "never-stored").await.unwrap();
}

#[test]
fn the_key_is_never_shown_in_debug() {
    let shown = format!(
        "{:?}",
        OwnAgentKey::new("openai_chat", &["gpt".into()], "sk-secret-value").unwrap()
    );
    assert!(!shown.contains("sk-secret-value"));
    assert!(shown.contains("openai_chat"));
}

#[tokio::test]
async fn an_own_key_keeps_every_model_and_a_single_model_document_remains_compatible() {
    let (work, _hub, _dir, _operations) = work();
    let original = OwnAgentKey::new(
        "openai_chat",
        &["first".into(), "second".into()],
        "kept-secret",
    )
    .unwrap();
    put(&work, "agent-multi", &original).await.unwrap();
    let read = get(&work, "agent-multi").await.unwrap().unwrap();
    assert_eq!(read.model(), "first");
    assert_eq!(read.models(), ["first", "second"]);
    assert_eq!(read.expose(), "kept-secret");
    let canonical =
        nocterm_cloud_device::records::ProviderKeyRecord::decode(&read.encode().unwrap()).unwrap();
    assert_eq!(canonical.models(), ["first", "second"]);
    let legacy = br#"{"format":1,"provider":"openai_chat","model":"old","key":"a2VwdC1zZWNyZXQ="}"#;
    let old = OwnAgentKey::decode(legacy).unwrap();
    assert_eq!(old.models(), ["old"]);
    assert_eq!(old.encode().unwrap().as_slice(), legacy);
}

#[tokio::test]
async fn a_model_only_own_key_edit_keeps_its_authenticated_provider_and_secret() {
    let (work, _hub, _dir, _operations) = work();
    let stored = OwnAgentKey::new("openai_chat", &["first".into()], "kept-secret").unwrap();
    put(&work, "agent-edit", &stored).await.unwrap();
    let edited = get(&work, "agent-edit")
        .await
        .unwrap()
        .unwrap()
        .with_models(vec!["second".into(), "third".into()])
        .unwrap();
    put(&work, "agent-edit", &edited).await.unwrap();
    let read = get(&work, "agent-edit").await.unwrap().unwrap();
    assert_eq!(read.provider(), "openai_chat");
    assert_eq!(read.expose(), "kept-secret");
    assert_eq!(read.models(), ["second", "third"]);
}

#[tokio::test]
async fn a_model_only_write_never_overwrites_a_key_changed_after_its_read() {
    let (work, _hub, _dir, _operations) = work();
    put(
        &work,
        "agent-race",
        &OwnAgentKey::new("openai_chat", &["old".into()], "old-secret").unwrap(),
    )
    .await
    .unwrap();
    let (old, receipt) = get_versioned(&work, "agent-race").await.unwrap().unwrap();
    let edited = old.with_models(vec!["edited".into()]).unwrap();
    put(
        &work,
        "agent-race",
        &OwnAgentKey::new("openai_chat", &["new".into()], "new-secret").unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(
        put_expected(receipt, &edited).await,
        Err(CloudAgentError::Conflict)
    ));
    let actual = get(&work, "agent-race").await.unwrap().unwrap();
    assert_eq!(actual.models(), ["new"]);
    assert_eq!(actual.expose(), "new-secret");
}

#[tokio::test]
async fn competing_model_edit_receipts_conflict_without_losing_the_winning_models() {
    let (work, _hub, _dir, _operations) = work();
    put(
        &work,
        "agent-model-race",
        &OwnAgentKey::new("openai_chat", &["old".into()], "secret").unwrap(),
    )
    .await
    .unwrap();
    let (first, first_version) = get_versioned(&work, "agent-model-race")
        .await
        .unwrap()
        .unwrap();
    let (second, second_version) = get_versioned(&work, "agent-model-race")
        .await
        .unwrap()
        .unwrap();
    put_expected(
        first_version,
        &first.with_models(vec!["first".into()]).unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(
        put_expected(
            second_version,
            &second.with_models(vec!["second".into()]).unwrap()
        )
        .await,
        Err(CloudAgentError::Conflict)
    ));
    let actual = get(&work, "agent-model-race").await.unwrap().unwrap();
    assert_eq!(actual.models(), ["first"]);
    assert_eq!(actual.expose(), "secret");
}

#[tokio::test]
async fn repairing_absence_does_not_replace_a_key_created_after_the_absence_read() {
    let (work, _hub, _dir, _operations) = work();
    let (missing, receipt) = prepare_write(&work, "agent-repair").await.unwrap();
    assert!(missing.is_none());
    put(
        &work,
        "agent-repair",
        &OwnAgentKey::new("openai_chat", &["other".into()], "other-secret").unwrap(),
    )
    .await
    .unwrap();
    let repair = OwnAgentKey::new("openai_chat", &["repair".into()], "repair-secret").unwrap();
    assert!(matches!(
        put_expected(receipt, &repair).await,
        Err(CloudAgentError::Conflict)
    ));
    let actual = get(&work, "agent-repair").await.unwrap().unwrap();
    assert_eq!(actual.expose(), "other-secret");
    assert_eq!(actual.models(), ["other"]);
}
