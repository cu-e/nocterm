//! Scope authority and generated paths are checked through the public repository.
use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::thread::Entry;
use nocterm_session::{
    local_fs::LocalFsError,
    testing::{FsOperation, InMemoryFs},
};
use std::sync::Mutex;
fn chat(scope: ContextId) -> SavedChat {
    let mut chat = SavedChat::for_scope(
        "codex".into(),
        "00000000-0000-4000-8000-000000000001".into(),
        1,
        scope,
    );
    chat.entries = vec![
        Entry::User(vec![Content::text("project question")]),
        Entry::Agent("x".repeat(1024 * 1024)),
    ];
    chat
}
fn project() -> ContextId {
    serde_json::from_value(
        serde_json::json!({"kind":"project","id":"01010101010101010101010101010101"}),
    )
    .unwrap()
}
fn fake(directory: &Path) -> (FileChatRepository, Arc<InMemoryFs>) {
    let filesystem = Arc::new(InMemoryFs::default());
    for ancestor in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
        filesystem.seed_native_directory(ancestor).unwrap();
    }
    (
        FileChatRepository::new(directory.to_owned(), filesystem.clone()),
        filesystem,
    )
}
#[test]
fn warm_catalog_and_ordinary_autosave_only_read_the_bounded_authoritative_header() {
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    let chat = chat(project());
    let snapshot = || {
        Arc::new(SharedChat {
            metadata: chat.clone(),
            queue: Vec::new(),
        })
    };
    block_on(repository.save(snapshot())).unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let observed = reads.clone();
    filesystem.observe_local(Arc::new(move |operation, path: &Path| {
        if operation == FsOperation::Read {
            observed.lock().unwrap().push(path.to_owned());
        }
    }));
    assert_eq!(
        block_on(repository.catalog(project())).unwrap()[0].title(),
        "project question"
    );
    let transcript = document(directory.path(), project(), &chat.header.id, "json");
    assert_eq!(
        reads
            .lock()
            .unwrap()
            .iter()
            .filter(|path| **path == transcript)
            .count(),
        1
    );
    reads.lock().unwrap().clear();
    block_on(repository.save(snapshot())).unwrap();
    assert_eq!(
        reads
            .lock()
            .unwrap()
            .iter()
            .filter(|path| **path == transcript)
            .count(),
        1
    );
    reads.lock().unwrap().clear();
    assert_eq!(
        block_on(repository.load(project(), chat.header.id.clone()))
            .unwrap()
            .entries
            .len(),
        2
    );
    assert!(
        reads
            .lock()
            .unwrap()
            .iter()
            .filter(|path| **path == transcript)
            .count()
            > 1
    );
}
#[cfg(unix)]
#[test]
fn wrong_scope_inside_a_partition_cannot_be_listed_overwritten_or_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let chat = chat(project());
    save(directory.path(), &chat).unwrap();
    let path = document(directory.path(), project(), &chat.header.id, "json");
    let mut forged = chat.clone();
    forged.header.scope = ContextId::General;
    let bytes = history::encode(&forged).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(load_catalog(directory.path(), project()).is_empty());
    let repository = repository(directory.path());
    assert!(matches!(
        block_on(repository.load(project(), chat.header.id.clone())),
        Err(AgentStorageError::ScopeMismatch { .. })
    ));
    assert!(matches!(
        block_on(repository.save(Arc::new(SharedChat {
            metadata: chat.clone(),
            queue: Vec::new()
        }))),
        Err(AgentStorageError::ScopeMismatch { .. })
    ));
    assert!(matches!(
        block_on(repository.delete(project(), chat.header.id)),
        Err(AgentStorageError::ScopeMismatch { .. })
    ));
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[cfg(unix)]
#[test]
fn authoritative_header_can_offer_recovery_but_a_disagreeing_payload_cannot_hydrate() {
    let directory = tempfile::tempdir().unwrap();
    let chat = chat(project());
    save(directory.path(), &chat).unwrap();
    let path = document(directory.path(), project(), &chat.header.id, "json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["chat"]["scope"] = serde_json::json!({"kind":"general"});
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let rows = load_catalog(directory.path(), project());
    assert_eq!(rows[0].title(), "project question");
    assert_eq!(rows[0].scope, project());
    assert!(matches!(
        load_scoped(directory.path(), project(), &chat.header.id),
        Err(AgentStorageError::Codec(
            history::HistoryCodecError::HeaderMismatch
        ))
    ));
}
#[test]
fn a_failed_legacy_delete_keeps_the_preferred_scoped_document_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    let current = chat(ContextId::General);
    block_on(repository.save(Arc::new(SharedChat {
        metadata: current.clone(),
        queue: Vec::new(),
    })))
    .unwrap();
    let mut old = current.clone();
    old.version = 1;
    old.header.title = Some("stale legacy caption".into());
    let legacy = directory.path().join(format!("{}.json", old.header.id));
    filesystem
        .seed_native_file(&legacy, history::encode(&old).unwrap())
        .unwrap();
    filesystem.fail_next_local(
        FsOperation::RemoveFile,
        legacy.clone(),
        LocalFsError::PermissionDenied(legacy),
    );
    assert!(block_on(repository.delete(ContextId::General, current.header.id.clone())).is_err());
    assert_eq!(
        block_on(repository.catalog(ContextId::General)).unwrap()[0].title(),
        "project question"
    );
    block_on(repository.delete(ContextId::General, current.header.id.clone())).unwrap();
    block_on(repository.delete(ContextId::General, current.header.id)).unwrap();
    assert!(
        block_on(repository.catalog(ContextId::General))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn invalid_preferred_scoped_document_never_reveals_a_stale_legacy_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let current = chat(ContextId::General);
    save(directory.path(), &current).unwrap();
    let mut legacy = current.clone();
    legacy.version = 1;
    legacy.header.title = Some("stale fallback".into());
    fs::write(
        directory.path().join(format!("{}.json", current.header.id)),
        history::encode(&legacy).unwrap(),
    )
    .unwrap();
    fs::write(
        document(
            directory.path(),
            ContextId::General,
            &current.header.id,
            "json",
        ),
        b"broken",
    )
    .unwrap();
    assert!(load_catalog(directory.path(), ContextId::General).is_empty());
    assert!(load(directory.path(), &current.header.id).is_err());
}
#[cfg(unix)]
#[test]
fn generated_namespace_links_cannot_alias_general_or_another_project() {
    use std::os::unix::fs::symlink;
    for link_level in [
        "scoped-v2",
        "scoped-v2/projects",
        "scoped-v2/projects/01010101010101010101010101010101",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let link = directory.path().join(link_level);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(foreign.path(), &link).unwrap();
        assert!(save(directory.path(), &chat(project())).is_err());
        assert!(fs::read_dir(foreign.path()).unwrap().next().is_none());
    }
}

#[test]
fn legacy_metadata_after_a_large_payload_and_a_deep_first_user_ignore_forged_sidecars() {
    let directory = tempfile::tempdir().unwrap();
    let mut chat = chat(ContextId::General);
    chat.version = 1;
    chat.entries = vec![
        Entry::Agent("x".repeat(1024 * 1024)),
        Entry::User(vec![Content::text("deep first question")]),
    ];
    let entries = serde_json::to_string(&chat.entries).unwrap();
    let bytes = format!(
        "{{\"entries\":{entries},\"id\":\"{}\",\"agent_id\":\"codex\",\"updated\":99,\"version\":1,\"name\":\"Late name\",\"pinned\":true}}",
        chat.header.id
    );
    let path = directory.path().join(format!("{}.json", chat.header.id));
    fs::write(&path, &bytes).unwrap();
    let mut fake = history::ChatSummary::from_chat(&chat);
    fake.title = Some("forged fallback".into());
    fake.name = Some("forged name".into());
    fake.pinned = false;
    fs::write(
        directory.path().join(format!("{}.meta", chat.header.id)),
        serde_json::to_vec(&fake).unwrap(),
    )
    .unwrap();
    let rows = load_catalog(directory.path(), ContextId::General);
    assert_eq!(rows[0].title(), "Late name");
    assert_eq!(rows[0].title.as_deref(), Some("deep first question"));
    assert!(rows[0].pinned);
    assert_eq!(rows[0].updated, 99);
    assert_eq!(
        load(directory.path(), &chat.header.id)
            .unwrap()
            .entries
            .len(),
        2
    );
    assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
}
#[test]
fn failed_publication_and_cleanup_keep_both_typed_causes_and_the_old_document() {
    use std::io;
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    let mut chat = chat(project());
    let snapshot = |chat: &SavedChat| {
        Arc::new(SharedChat {
            metadata: chat.clone(),
            queue: Vec::new(),
        })
    };
    block_on(repository.save(snapshot(&chat))).unwrap();
    let partition = document(directory.path(), project(), &chat.header.id, "json")
        .parent()
        .unwrap()
        .to_owned();
    filesystem.fail_next_local(
        FsOperation::Write,
        partition.clone(),
        LocalFsError::from_io(&partition, io::Error::from(io::ErrorKind::StorageFull)),
    );
    filesystem.fail_next_local(
        FsOperation::Cancel,
        partition.clone(),
        LocalFsError::from_io(&partition, io::Error::from(io::ErrorKind::Interrupted)),
    );
    chat.header.title = Some("must not publish".into());
    let error = block_on(repository.save(snapshot(&chat))).unwrap_err();
    let AgentStorageError::StageCleanup { operation, cleanup } = &error else {
        panic!("retain failed operation and cleanup")
    };
    fn original<'a>(error: &'a (dyn std::error::Error + 'static)) -> &'a io::Error {
        let mut cause = Some(error);
        while let Some(error) = cause {
            if let Some(native) = error.downcast_ref::<io::Error>() {
                return native;
            }
            cause = error.source();
        }
        panic!("the concrete native cause must remain reachable")
    }
    assert_eq!(
        original(operation.as_ref()).kind(),
        io::ErrorKind::StorageFull
    );
    assert_eq!(
        original(cleanup.as_ref()).kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(original(&error).kind(), io::ErrorKind::StorageFull);
    assert!(std::ptr::eq(original(&error), original(&error.clone())));
    assert_eq!(
        block_on(repository.load(project(), chat.header.id))
            .unwrap()
            .header
            .title,
        None
    );
}

#[test]
fn a_namespace_replacement_after_admission_cannot_retarget_the_opened_payload() {
    use nocterm_session::local_fs::LocalMutations as _;
    use std::{
        ffi::OsStr,
        sync::atomic::{AtomicBool, Ordering},
    };
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    let own = chat(project());
    let mut foreign = chat(ContextId::General);
    foreign.header.title = Some("foreign general data".into());
    for chat in [own.clone(), foreign] {
        block_on(repository.save(Arc::new(SharedChat {
            metadata: chat,
            queue: Vec::new(),
        })))
        .unwrap();
    }
    let path = document(directory.path(), project(), &own.header.id, "json");
    let partition = path.parent().unwrap().to_owned();
    let general = document(directory.path(), ContextId::General, &own.header.id, "json")
        .parent()
        .unwrap()
        .to_owned();
    let swapped = Arc::new(AtomicBool::new(false));
    let seen = swapped.clone();
    let injected = filesystem.clone();
    filesystem.observe_local(Arc::new(move |operation, observed: &Path| {
        if operation == FsOperation::Download
            && observed == path
            && !seen.swap(true, Ordering::SeqCst)
        {
            injected
                .rename(&partition, OsStr::new("held-project"))
                .unwrap();
            injected.seed_native_alias(&partition, &general).unwrap();
        }
    }));
    let loaded = block_on(repository.load(project(), own.header.id.clone())).unwrap();
    assert!(swapped.load(Ordering::SeqCst));
    assert_eq!(loaded.header.scope, project());
    assert_eq!(loaded.header.title, None);
    assert_eq!(
        block_on(repository.load(ContextId::General, own.header.id))
            .unwrap()
            .header
            .title
            .as_deref(),
        Some("foreign general data")
    );
}
#[cfg(unix)]
#[test]
fn a_transcript_leaf_link_cannot_read_overwrite_or_delete_another_bucket() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let own = chat(project());
    let foreign = chat(ContextId::General);
    save(directory.path(), &own).unwrap();
    save(directory.path(), &foreign).unwrap();
    let path = document(directory.path(), project(), &own.header.id, "json");
    let target = document(directory.path(), ContextId::General, &own.header.id, "json");
    let before = fs::read(&target).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(load_catalog(directory.path(), project()).is_empty());
    assert!(load_scoped(directory.path(), project(), &own.header.id).is_err());
    assert!(save(directory.path(), &own).is_err());
    assert!(block_on(repository(directory.path()).delete(project(), own.header.id)).is_err());
    assert_eq!(fs::read(target).unwrap(), before);
    assert!(fs::symlink_metadata(path).unwrap().is_symlink());
}

#[test]
fn portable_identity_refuses_projects_while_general_history_remains_available() {
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    filesystem.use_portable_directory_identity(true);
    let general = chat(ContextId::General);
    block_on(repository.save(Arc::new(SharedChat {
        metadata: general.clone(),
        queue: Vec::new(),
    })))
    .unwrap();
    assert_eq!(
        block_on(repository.catalog(ContextId::General))
            .unwrap()
            .len(),
        1
    );
    assert!(block_on(repository.load(ContextId::General, general.header.id)).is_ok());
    assert!(matches!(
        block_on(repository.save(Arc::new(SharedChat {
            metadata: chat(project()),
            queue: Vec::new()
        }))),
        Err(AgentStorageError::ScopeUnsupported(_))
    ));
    assert!(matches!(
        block_on(repository.catalog(project())),
        Err(AgentStorageError::ScopeUnsupported(_))
    ));
}

#[test]
fn a_committed_directory_durability_warning_is_a_successful_publication() {
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    let mut chat = chat(project());
    block_on(repository.save(Arc::new(SharedChat {
        metadata: chat.clone(),
        queue: Vec::new(),
    })))
    .unwrap();
    let partition = document(directory.path(), project(), &chat.header.id, "json")
        .parent()
        .unwrap()
        .to_owned();
    filesystem.fail_next_local(
        FsOperation::DirectorySync,
        partition.clone(),
        LocalFsError::from_io(
            &partition,
            std::io::Error::from(std::io::ErrorKind::StorageFull),
        ),
    );
    chat.header.title = Some("already committed".into());
    block_on(repository.save(Arc::new(SharedChat {
        metadata: chat.clone(),
        queue: Vec::new(),
    })))
    .unwrap();
    assert_eq!(
        block_on(repository.load(project(), chat.header.id))
            .unwrap()
            .header
            .title
            .as_deref(),
        Some("already committed")
    );
}

#[test]
fn foreign_authoritative_scope_is_refused_before_the_payload_is_read() {
    let directory = tempfile::tempdir().unwrap();
    let (repository, filesystem) = fake(directory.path());
    let own = chat(project());
    block_on(repository.save(Arc::new(SharedChat {
        metadata: own.clone(),
        queue: Vec::new(),
    })))
    .unwrap();
    let path = document(directory.path(), project(), &own.header.id, "json");
    let foreign = chat(ContextId::General);
    filesystem
        .seed_native_file(&path, history::encode(&foreign).unwrap())
        .unwrap();
    let reads = Arc::new(Mutex::new(0));
    let observed = reads.clone();
    filesystem.observe_local(Arc::new(move |operation, file: &Path| {
        if operation == FsOperation::Read && file == path {
            *observed.lock().unwrap() += 1;
        }
    }));
    assert!(
        matches!(block_on(repository.load(project(), own.header.id)), Err(AgentStorageError::ScopeMismatch { expected, actual }) if expected == project() && actual == ContextId::General)
    );
    assert_eq!(*reads.lock().unwrap(), 1);
}
