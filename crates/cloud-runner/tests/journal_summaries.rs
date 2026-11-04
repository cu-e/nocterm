//! Journal v7 summary rows and the migration that adds them (ADR 0135).
mod support;
use futures::executor::block_on;
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use nocterm_cloud_runner::{DurableRunJournal, JournalLimits};
use std::sync::Arc;
use support::*;

const DATABASE: &str = "cloud-runs.sqlite3";

fn with_one_chat(root: &std::path::Path, cipher: &Arc<TestCipher>) -> DurableRunJournal {
    let store = journal(root, cipher.clone());
    let lease = claim(&store, None);
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    store
}
fn sql(root: &std::path::Path, statements: &str) {
    rusqlite::Connection::open(root.join(DATABASE))
        .unwrap()
        .execute_batch(statements)
        .unwrap();
}
fn version(root: &std::path::Path) -> i64 {
    rusqlite::Connection::open(root.join(DATABASE))
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}
fn summary_table(root: &std::path::Path) -> String {
    rusqlite::Connection::open(root.join(DATABASE))
        .unwrap()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='chat_summaries'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}
fn sealed(revision: u64, deleted: bool) -> SealedChatSummary {
    SealedChatSummary::new(
        stream(),
        ChatKeyEpoch::new(1).unwrap(),
        revision,
        deleted,
        bytes(&[revision as u8; 48]),
    )
    .unwrap()
}
fn at(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

#[test]
fn a_version_6_journal_gains_an_empty_summary_table_and_keeps_its_chats() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    drop(with_one_chat(root.path(), &cipher));
    let created = summary_table(root.path());
    sql(
        root.path(),
        "DROP TABLE chat_summaries; DROP TABLE chat_ssh_keys; PRAGMA user_version=6",
    );
    let store = journal(root.path(), cipher);
    assert_eq!(version(root.path()), 8);
    assert_eq!(summary_table(root.path()), created);
    assert_eq!(block_on(store.summary(stream())).unwrap(), None);
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.records().len(), 1);
}

#[test]
fn journals_of_other_versions_are_refused() {
    for other in [4, 9] {
        let root = private_root();
        let cipher = Arc::new(TestCipher::default());
        drop(with_one_chat(root.path(), &cipher));
        sql(root.path(), &format!("PRAGMA user_version={other}"));
        assert!(matches!(
            open_journal(root.path(), cipher, JournalLimits::default()),
            Err(JournalError::UnsupportedFormat)
        ));
        assert_eq!(version(root.path()), other);
    }
}

#[test]
fn a_staged_summary_keeps_its_exact_bytes_across_reopen_until_published() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = with_one_chat(root.path(), &cipher);
    block_on(store.stage_summary(sealed(1, false), at(10), at(20))).unwrap();
    drop(store);
    let store = journal(root.path(), cipher);
    let stored = block_on(store.summary(stream())).unwrap().unwrap();
    assert_eq!(stored.revision(), 1);
    assert_eq!(stored.published(), 0);
    assert_eq!(stored.created_at(), at(10));
    assert_eq!(stored.last_activity(), at(20));
    assert_eq!(stored.pending(), Some(&sealed(1, false)));
    block_on(store.summary_published(stream(), 1)).unwrap();
    let stored = block_on(store.summary(stream())).unwrap().unwrap();
    assert_eq!((stored.revision(), stored.published()), (1, 1));
    assert_eq!(stored.pending(), None);
}

#[test]
fn a_summary_revision_never_moves_backwards() {
    let root = private_root();
    let store = with_one_chat(root.path(), &Arc::new(TestCipher::default()));
    block_on(store.stage_summary(sealed(2, false), at(1), at(1))).unwrap();
    for stale in [1, 2] {
        assert!(matches!(
            block_on(store.stage_summary(sealed(stale, false), at(1), at(1))),
            Err(JournalError::Conflict)
        ));
    }
    block_on(store.summary_published(stream(), 2)).unwrap();
    block_on(store.summary_published(stream(), 1)).unwrap();
    block_on(store.stage_summary(sealed(3, true), at(1), at(5))).unwrap();
    let stored = block_on(store.summary(stream())).unwrap().unwrap();
    assert_eq!((stored.revision(), stored.published()), (3, 2));
    assert!(stored.deleted());
    assert_eq!(stored.pending(), Some(&sealed(3, true)));
}

#[test]
fn a_summary_of_a_chat_outside_the_catalog_binding_is_refused() {
    let root = private_root();
    let store = with_one_chat(root.path(), &Arc::new(TestCipher::default()));
    let other = StreamBinding::new(
        stream().scope().clone(),
        RunnerId::from_bytes([99; 16]),
        stream().runner_epoch(),
        stream().chat(),
    );
    assert!(matches!(
        block_on(store.summary(other)),
        Err(JournalError::Fenced)
    ));
}
