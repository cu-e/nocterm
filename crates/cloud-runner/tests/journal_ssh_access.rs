//! Journal SSH rows, the v8 migration that adds them (ADR 0137) and the v9 migration that
//! drops rows sealed under the effect class (ADR 0139).
mod support;
use futures::executor::block_on;
use nocterm_cloud_model::{
    coordinator::{GrantClass, GrantReference},
    identity::*,
    journal::*,
    ssh_access::*,
};
use nocterm_cloud_runner::DurableRunJournal;
use std::sync::Arc;
use support::*;

const DATABASE: &str = "cloud-runs.sqlite3";
const SEED: [u8; 32] = [0x5e; 32];

fn with_one_chat(root: &std::path::Path, cipher: &Arc<TestCipher>) -> DurableRunJournal {
    let store = journal(root, cipher.clone());
    let lease = claim(&store, None);
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    store
}
fn database(root: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open(root.join(DATABASE)).unwrap()
}
fn bound() -> StoredSshAccess {
    let offer = SshBindingOffer::new(
        GrantReference::new(
            GrantClass::SshBinding,
            GrantId::from_bytes([1; 16]),
            GrantEpoch::new(1).unwrap(),
        ),
        Timestamp::from_millis(1_000),
        Timestamp::from_millis(1_000 + 3_600_000),
        [2; 32],
        vec![BoundTarget {
            server: SshServerKey::from_bytes([3; 16]),
            endpoint: SshEndpoint {
                host: "db.example".into(),
                port: 22,
                user: "deploy".into(),
            },
            host_key: fp(4),
            principal: "nocterm-agent-0000000000000000-0303030303030303".into(),
        }],
        bytes(&[5; 40]),
    )
    .unwrap();
    let reach = SshReach::new(
        vec![(offer.targets()[0].endpoint.clone(), None)],
        vec!["203.0.113.7".into()],
    )
    .unwrap();
    let seed = SshSubjectSeed::new(SEED);
    let mut access = ChatSshAccess::new();
    access
        .bind(
            offer,
            &reach,
            &seed.public_key(),
            Timestamp::from_millis(2_000),
            [6; 16],
        )
        .unwrap();
    StoredSshAccess {
        seed: Some(seed),
        access: access.parts().clone(),
    }
}

#[test]
fn a_version_7_journal_gains_an_empty_ssh_table_and_keeps_its_chats() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    drop(with_one_chat(root.path(), &cipher));
    database(root.path())
        .execute_batch("DROP TABLE chat_ssh_keys; PRAGMA user_version=7")
        .unwrap();
    let store = journal(root.path(), cipher);
    let version: i64 = database(root.path())
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
    assert!(block_on(store.ssh_access(stream())).unwrap().is_none());
    assert!(block_on(store.ssh_streams()).unwrap().is_empty());
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.records().len(), 1);
}

#[test]
fn a_version_8_journal_drops_its_ssh_rows_and_keeps_its_chats_and_records() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = with_one_chat(root.path(), &cipher);
    block_on(store.store_ssh_access(stream(), bound())).unwrap();
    drop(store);
    database(root.path())
        .execute_batch("PRAGMA user_version=8")
        .unwrap();
    let store = journal(root.path(), cipher);
    let version: i64 = database(root.path())
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
    assert!(block_on(store.ssh_access(stream())).unwrap().is_none());
    assert!(block_on(store.ssh_streams()).unwrap().is_empty());
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.records().len(), 1);
    block_on(store.store_ssh_access(stream(), bound())).unwrap();
    assert!(block_on(store.ssh_access(stream())).unwrap().is_some());
}

#[test]
fn an_ssh_row_survives_reopening_sealed_without_its_seed_in_the_clear() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = with_one_chat(root.path(), &cipher);
    let stored = bound();
    block_on(store.store_ssh_access(stream(), stored.clone())).unwrap();
    drop(store);
    let body: Vec<u8> = database(root.path())
        .query_row("SELECT body FROM chat_ssh_keys", [], |row| row.get(0))
        .unwrap();
    assert!(!body.windows(SEED.len()).any(|window| window == SEED));
    let store = journal(root.path(), cipher);
    let restored = block_on(store.ssh_access(stream())).unwrap().unwrap();
    assert_eq!(restored.seed.unwrap().expose(), &SEED);
    assert_eq!(restored.access, stored.access);
    assert_eq!(block_on(store.ssh_streams()).unwrap(), vec![stream()]);
}

#[test]
fn a_forgotten_key_leaves_only_the_replay_guard_and_a_changed_row_is_refused() {
    let root = private_root();
    let cipher = Arc::new(TestCipher::default());
    let store = with_one_chat(root.path(), &cipher);
    let mut access = ChatSshAccess::restore(bound().access).unwrap();
    access.forget();
    let forgotten = StoredSshAccess {
        seed: None,
        access: access.parts().clone(),
    };
    block_on(store.store_ssh_access(stream(), forgotten)).unwrap();
    let restored = block_on(store.ssh_access(stream())).unwrap().unwrap();
    assert!(restored.seed.is_none() && restored.access.binding.is_none());
    assert_eq!(restored.access.epochs.len(), 1);
    database(root.path())
        .execute("UPDATE chat_ssh_keys SET revision=revision+1", [])
        .unwrap();
    assert!(matches!(
        block_on(store.ssh_access(stream())),
        Err(JournalError::Corrupt)
    ));
}

#[test]
fn a_bound_row_without_its_seed_is_corrupt() {
    let root = private_root();
    let store = with_one_chat(root.path(), &Arc::new(TestCipher::default()));
    let mut stored = bound();
    stored.seed = None;
    block_on(store.store_ssh_access(stream(), stored)).unwrap();
    assert!(matches!(
        block_on(store.ssh_access(stream())),
        Err(JournalError::Corrupt)
    ));
}
