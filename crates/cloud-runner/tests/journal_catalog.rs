#[path = "journal_catalog/current_heads.rs"]
mod current_heads;
mod support;
use futures::executor::block_on;
use nocterm_cloud_model::{authority::*, identity::*, journal::*};
use nocterm_cloud_runner::{DurableRunJournal, JournalLimits};
use std::sync::Arc;
use support::*;

fn create(
    root: &std::path::Path,
    catalog: Arc<TestCipher>,
) -> Result<DurableRunJournal, JournalError> {
    DurableRunJournal::create_new(
        root,
        catalog_binding(),
        Arc::new(TestCipher::default()),
        catalog,
        Arc::new(TestCipher::default()),
        JournalLimits::default(),
    )
}
fn reopen(root: &std::path::Path) -> Result<DurableRunJournal, JournalError> {
    DurableRunJournal::open_existing(
        root,
        catalog_binding(),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        JournalLimits::default(),
    )
}
fn sql(root: &std::path::Path, command: &str) {
    // This connection models offline file tampering, not a trusted native
    // mutation. Keep the production schema/foreign-key checks unchanged.
    let corruptor = rusqlite::Connection::open(root.join("cloud-runs.sqlite3")).unwrap();
    corruptor
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    corruptor.execute_batch(command).unwrap();
}
fn second_stream() -> StreamBinding {
    catalog_binding().stream(CloudChatId::from_bytes([6; 16]))
}
fn claim_second(store: &DurableRunJournal) -> Result<WriterLease, JournalError> {
    block_on(store.claim_writer(
        WriterClaim::new(second_stream(), None, WriterEpoch::new(1).unwrap()).unwrap(),
    ))
}
fn read(store: &DurableRunJournal) -> Result<JournalPage, JournalError> {
    block_on(store.read(stream(), JournalSequence::default(), limits()))
}
fn catalog_row(root: &std::path::Path) -> (i64, Vec<u8>) {
    rusqlite::Connection::open(root.join("cloud-runs.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT revision,body FROM account_catalog WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[test]
fn creation_and_reopen_are_explicit_and_an_empty_catalog_is_already_authenticated() {
    let root = private_root();
    assert!(reopen(root.path()).is_err());
    let store = create(root.path(), Arc::new(TestCipher::default())).unwrap();
    let original_file = std::fs::read(root.path().join("cloud-runs.sqlite3")).unwrap();
    assert!(create(root.path(), Arc::new(TestCipher::default())).is_err());
    assert_eq!(
        std::fs::read(root.path().join("cloud-runs.sqlite3")).unwrap(),
        original_file
    );
    assert_eq!(catalog_row(root.path()).0, 1);
    let target = TargetBinding::new(fp(1), fp(2), NamespaceId::new("principal").unwrap(), fp(3));
    assert!(
        block_on(store.unresolved_target(stream().scope().clone(), target, 1))
            .unwrap()
            .is_empty()
    );
    drop(store);
    reopen(root.path()).unwrap();
    sql(root.path(), "DELETE FROM account_catalog");
    assert!(matches!(reopen(root.path()), Err(JournalError::Corrupt)));
    assert!(create(root.path(), Arc::new(TestCipher::default())).is_err());
}

#[test]
fn failed_initial_catalog_sealing_leaves_no_implicitly_recoverable_database() {
    let root = private_root();
    let catalog = Arc::new(TestCipher::default());
    catalog.fail_sealing(true);
    assert!(matches!(
        create(root.path(), catalog),
        Err(JournalError::Corrupt)
    ));
    assert!(matches!(reopen(root.path()), Err(JournalError::Corrupt)));
    assert!(create(root.path(), Arc::new(TestCipher::default())).is_err());
}

#[test]
fn the_unreleased_old_format_requires_explicit_recovery_without_index_rebuilding() {
    let root = private_root();
    create(root.path(), Arc::new(TestCipher::default())).unwrap();
    sql(root.path(), "PRAGMA user_version=1");
    assert!(matches!(
        reopen(root.path()),
        Err(JournalError::UnsupportedFormat)
    ));
}

#[test]
fn missing_catalog_or_required_chat_cannot_be_replaced_by_a_new_claim() {
    for corruption in ["DELETE FROM account_catalog", "DELETE FROM chats"] {
        let root = private_root();
        let store = create(root.path(), Arc::new(TestCipher::default())).unwrap();
        claim(&store, None);
        sql(root.path(), corruption);
        assert!(matches!(read(&store), Err(JournalError::Corrupt)));
        assert!(matches!(claim_second(&store), Err(JournalError::Corrupt)));
        assert!(matches!(reopen(root.path()), Err(JournalError::Corrupt)));
    }
}

#[test]
fn every_routing_column_is_checked_against_the_catalog_without_a_scope_filter() {
    for corruption in [
        "UPDATE chats SET scope=zeroblob(32)",
        "UPDATE chats SET binding=zeroblob(32)",
        "UPDATE chats SET chat_id=zeroblob(16)",
        "UPDATE chats SET chat=zeroblob(32)",
        "UPDATE chats SET runner=zeroblob(16)",
        "UPDATE chats SET key_epoch=key_epoch+1",
        "UPDATE chats SET deleted=1",
    ] {
        let root = private_root();
        let store = create(root.path(), Arc::new(TestCipher::default())).unwrap();
        claim(&store, None);
        sql(root.path(), corruption);
        assert!(
            matches!(read(&store), Err(JournalError::Corrupt)),
            "{corruption}"
        );
        assert!(
            matches!(reopen(root.path()), Err(JournalError::Corrupt)),
            "{corruption}"
        );
    }
}

#[test]
fn unknown_extra_physical_chats_are_not_ignored_or_added_to_the_catalog() {
    let root = private_root();
    let store = create(root.path(), Arc::new(TestCipher::default())).unwrap();
    claim(&store, None);
    sql(
        root.path(),
        "INSERT INTO chats SELECT zeroblob(32),binding,scope,zeroblob(16),runner,key_epoch,writer,revision,records,events,control,retained,deleted,charge,body,header_digest FROM chats",
    );
    assert!(matches!(read(&store), Err(JournalError::Corrupt)));
    assert!(matches!(reopen(root.path()), Err(JournalError::Corrupt)));
}

#[test]
fn catalog_authentication_binds_account_worker_epoch_revision_and_body() {
    let root = private_root();
    let store = create(root.path(), Arc::new(TestCipher::default())).unwrap();
    claim(&store, None);
    let binding = catalog_binding();
    let other_scope = AccountScope::new(
        binding.scope().hub().clone(),
        AccountIdentity::new(NamespaceId::new("another-account").unwrap(), fp(2)),
    );
    for changed in [
        JournalCatalogBinding::new(other_scope, binding.runner(), binding.epoch()),
        JournalCatalogBinding::new(
            binding.scope().clone(),
            RunnerId::from_bytes([99; 16]),
            binding.epoch(),
        ),
        JournalCatalogBinding::new(
            binding.scope().clone(),
            binding.runner(),
            RunnerKeyEpoch::new(2).unwrap(),
        ),
    ] {
        assert!(matches!(
            DurableRunJournal::open_existing(
                root.path(),
                changed.clone(),
                Arc::new(TestCipher::default()),
                Arc::new(TestCipher::default()),
                Arc::new(TestCipher::default()),
                JournalLimits::default()
            ),
            Err(JournalError::Corrupt)
        ));
        let foreign = changed.stream(stream().chat());
        assert!(matches!(
            block_on(store.claim_writer(
                WriterClaim::new(foreign.clone(), None, WriterEpoch::new(1).unwrap()).unwrap()
            )),
            Err(JournalError::Fenced)
        ));
        assert!(matches!(
            block_on(store.read(foreign, JournalSequence::default(), limits())),
            Err(JournalError::Fenced)
        ));
    }
    let original = catalog_row(root.path());
    for corruption in [
        "UPDATE account_catalog SET revision=revision+1",
        "UPDATE account_catalog SET body=zeroblob(length(body))",
    ] {
        sql(root.path(), corruption);
        assert!(matches!(read(&store), Err(JournalError::Corrupt)));
        assert!(matches!(reopen(root.path()), Err(JournalError::Corrupt)));
        let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
        db.execute(
            "UPDATE account_catalog SET revision=?1,body=?2",
            rusqlite::params![original.0, &original.1],
        )
        .unwrap();
    }
}

#[test]
fn failed_catalog_growth_rolls_back_the_new_chat_and_keeps_the_original_catalog() {
    let root = private_root();
    let catalog = Arc::new(TestCipher::default());
    let store = create(root.path(), catalog.clone()).unwrap();
    claim(&store, None);
    let original = catalog_row(root.path());
    catalog.fail_sealing(true);
    assert!(matches!(claim_second(&store), Err(JournalError::Corrupt)));
    assert_eq!(catalog_row(root.path()), original);
    assert!(matches!(
        block_on(store.read(second_stream(), JournalSequence::default(), limits())),
        Err(JournalError::NotFound)
    ));
    catalog.fail_sealing(false);
    claim_second(&store).unwrap();
    assert_eq!(catalog_row(root.path()).0, original.0 + 1);
    reopen(root.path()).unwrap();
}

#[test]
fn a_live_owner_rejects_old_valid_catalog_while_restart_has_no_external_rollback_anchor() {
    let root = private_root();
    let store = create(root.path(), Arc::new(TestCipher::default())).unwrap();
    claim(&store, None);
    let original = catalog_row(root.path());
    claim_second(&store).unwrap();
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    db.execute(
        "DELETE FROM current_heads WHERE chat=(SELECT chat FROM chats WHERE chat_id=?1)",
        [second_stream().chat().as_bytes().as_slice()],
    )
    .unwrap();
    db.execute(
        "DELETE FROM chats WHERE chat_id=?1",
        [second_stream().chat().as_bytes().as_slice()],
    )
    .unwrap();
    db.execute(
        "UPDATE account_catalog SET revision=?1,body=?2",
        rusqlite::params![original.0, &original.1],
    )
    .unwrap();
    drop(db);
    assert!(matches!(read(&store), Err(JournalError::Corrupt)));
    // This is a coherent old authenticated database state. Without an external
    // trusted head, a restarted owner cannot distinguish that historical state.
    let reopened = reopen(root.path()).unwrap();
    assert!(read(&reopened).is_ok());
}

#[test]
fn a_lower_catalog_byte_quota_rolls_back_growth_without_a_phantom_chat() {
    let root = private_root();
    let limits = JournalLimits::new(4 * 1024 * 1024, 256 * 1024, 100, 100)
        .unwrap()
        .with_catalog_bytes(1024)
        .unwrap();
    let store = DurableRunJournal::create_new(
        root.path(),
        catalog_binding(),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        limits,
    )
    .unwrap();
    let mut accepted = 0i64;
    for id in 0..100u8 {
        let stream = catalog_binding().stream(CloudChatId::from_bytes([id; 16]));
        let result =
            block_on(store.claim_writer(
                WriterClaim::new(stream, None, WriterEpoch::new(1).unwrap()).unwrap(),
            ));
        match result {
            Ok(_) => accepted += 1,
            Err(JournalError::Capacity) => break,
            Err(error) => panic!("unexpected failure: {error:?}"),
        }
    }
    assert!(accepted > 0 && accepted < 100);
    let db = rusqlite::Connection::open(root.path().join("cloud-runs.sqlite3")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM chats", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, accepted);
    assert_eq!(catalog_row(root.path()).0, accepted + 1);
    DurableRunJournal::open_existing(
        root.path(),
        catalog_binding(),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        Arc::new(TestCipher::default()),
        limits,
    )
    .unwrap();
}
