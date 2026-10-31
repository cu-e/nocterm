//! The provider key of an own-runner agent lives in the account's encrypted collection.

use std::sync::Arc;

use super::*;
use crate::account::{operations::OpenedWork, testing};
use crate::store::memory::MemoryHub;

fn work() -> (OpenedWork, Arc<MemoryHub>, tempfile::TempDir) {
    let hub = Arc::new(MemoryHub::default());
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, _record) = testing::signed_in(dir.path());
    let operations = testing::native_operations(&slot, hub.clone());
    (operations.opened().unwrap(), hub, dir)
}

#[tokio::test]
async fn a_stored_key_is_read_back_on_the_account_and_replaced_by_the_next_save() {
    let (work, _hub, _dir) = work();
    put(&work, "agent-1", &OwnAgentKey::new("openai_chat", "gpt", "sk-first"))
        .await
        .unwrap();
    put(&work, "agent-1", &OwnAgentKey::new("openai_chat", "gpt", "sk-second"))
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
    let (work, _hub, _dir) = work();
    put(&work, "agent-1", &OwnAgentKey::new("openai_chat", "gpt", "sk-first"))
        .await
        .unwrap();
    remove(&work, "agent-1").await.unwrap();
    assert!(get(&work, "agent-1").await.unwrap().is_none());
    remove(&work, "agent-1").await.unwrap();
    remove(&work, "never-stored").await.unwrap();
}

#[test]
fn the_key_is_never_shown_in_debug() {
    let shown = format!("{:?}", OwnAgentKey::new("openai_chat", "gpt", "sk-secret-value"));
    assert!(!shown.contains("sk-secret-value"));
    assert!(shown.contains("openai_chat"));
}
