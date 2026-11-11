//! Native chat storage through the application's production repository/filesystem composition.
#![cfg(any(unix, windows))]

use futures::executor::block_on;
use nocterm_agent_storage::FileChatRepository;
use nocterm_ai::{
    history::{self, ContextId, SavedAttachment, SavedChat, SavedPrompt, SharedChat},
    local_environment::{ChatScope, LocalEnvironment},
    storage::{AgentStorageError, ChatRepository},
};
use nocterm_hubs_model::source_identity::{
    AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition, NamespaceId,
};
use nocterm_localfs::LocalFilesystem;
use nocterm_session::local_fs::LocalFs;
use std::{ffi::OsStr, io::Read, path::Path, path::PathBuf, sync::Arc};

const ID: &str = "00000000-0000-4000-8000-0000000000a1";

fn repository(root: &Path) -> FileChatRepository {
    let root = std::fs::canonicalize(root).unwrap();
    let filesystem = Arc::new(LocalFilesystem::with_base(root.clone()).unwrap());
    FileChatRepository::new(root, filesystem)
}

fn snapshot(scope: &ChatScope, text: &str) -> SharedChat {
    let mut metadata = SavedChat::for_environment("codex".into(), ID.into(), 1, scope.clone());
    metadata.header.name = Some(text.into());
    metadata.draft = Some(text.into());
    metadata.session_id = Some("retained-agent-session".into());
    SharedChat {
        metadata,
        queue: Vec::new(),
    }
}

fn record_path(root: &Path, scope: &ChatScope) -> PathBuf {
    let partition = root
        .join("scoped-v3")
        .join(scope.environment().storage_key());
    let context = match scope.context() {
        ContextId::General => partition.join("general"),
        ContextId::Project(id) => partition.join("projects").join(id.to_string()),
    };
    context.join(format!("{ID}.json"))
}

fn hub(account: &str, authority: u8) -> LocalEnvironment {
    LocalEnvironment::HubAccount(LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("same-hub-instance").unwrap(),
            Fingerprint::from_bytes([authority; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new(account).unwrap(),
            Fingerprint::from_bytes([3; 32]),
        ),
    )))
}

#[test]
fn native_v3_chat_is_saved_listed_and_reopened_from_its_history_directory() {
    let directory = tempfile::tempdir().unwrap();
    let scope = ChatScope::this_computer(ContextId::General);
    let original = snapshot(&scope, "new native chat");
    let repository = repository(directory.path());
    block_on(repository.save(Arc::new(original))).unwrap();
    let path = record_path(directory.path(), &scope);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(history::decode(&bytes, ID).unwrap().version, 3);
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
    drop(repository);

    let reopened = self::repository(directory.path());
    let catalog = block_on(reopened.catalog_in(scope.clone())).unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].id, ID);
    assert_eq!(catalog[0].name.as_deref(), Some("new native chat"));
    assert_eq!(catalog[0].environment, *scope.environment());
    let loaded = block_on(reopened.load_in(scope.clone(), ID.into())).unwrap();
    assert_eq!(loaded.header.chat_scope(), scope);
    assert_eq!(loaded.draft.as_deref(), Some("new native chat"));
    assert_eq!(loaded.session_id.as_deref(), Some("retained-agent-session"));
}

#[test]
fn native_snapshot_recovers_the_complete_pending_queue_after_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let scope = ChatScope::this_computer(ContextId::General);
    let mut original = snapshot(&scope, "queued chat");
    let prompts = ["first pending message", "second pending message"]
        .into_iter()
        .enumerate()
        .map(|(id, text)| SavedPrompt {
            id: id as u64 + 1,
            text: text.into(),
            images: Vec::new(),
            attachments: vec![SavedAttachment::Connection("saved-server".into())],
        })
        .collect::<Vec<_>>();
    original.queue = prompts.iter().cloned().map(Arc::new).collect();
    let repository = repository(directory.path());
    block_on(repository.save(Arc::new(original))).unwrap();
    drop(repository);

    let reopened = self::repository(directory.path());
    let restored = block_on(reopened.load_in(scope, ID.into())).unwrap();
    assert_eq!(restored.queue, prompts);
    assert_eq!(
        restored.session_id.as_deref(),
        Some("retained-agent-session")
    );
}

#[test]
fn native_history_separates_device_accounts_authorities_and_project_contexts() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    let contexts = [
        ContextId::General,
        ContextId::Project("00000000000000000000000000000001".parse().unwrap()),
        ContextId::Project("00000000000000000000000000000002".parse().unwrap()),
    ];
    let environments = [
        LocalEnvironment::ThisComputer,
        hub("alice", 1),
        hub("bob", 1),
        hub("alice", 2),
    ];
    let scopes = environments
        .into_iter()
        .flat_map(|environment| {
            contexts.map(|context| ChatScope::new(environment.clone(), context))
        })
        .collect::<Vec<_>>();
    for (index, scope) in scopes.iter().enumerate() {
        let chat = snapshot(scope, &format!("partition {index}"));
        block_on(repository.save(Arc::new(chat))).unwrap();
    }
    for (index, scope) in scopes.iter().enumerate() {
        let loaded = block_on(repository.load_in(scope.clone(), ID.into())).unwrap();
        assert_eq!(loaded.header.chat_scope(), *scope);
        assert_eq!(loaded.draft, Some(format!("partition {index}")));
        let catalog = block_on(repository.catalog_in(scope.clone())).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].name, Some(format!("partition {index}")));
    }
    block_on(repository.delete_in(scopes[0].clone(), ID.into())).unwrap();
    assert!(
        block_on(repository.catalog_in(scopes[0].clone()))
            .unwrap()
            .is_empty()
    );
    for scope in &scopes[1..] {
        assert!(block_on(repository.load_in(scope.clone(), ID.into())).is_ok());
    }
}

#[test]
fn copied_foreign_records_cannot_be_loaded_overwritten_deleted_or_listed() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository(directory.path());
    let original = ChatScope::this_computer(ContextId::General);
    block_on(repository.save(Arc::new(snapshot(&original, "original")))).unwrap();
    let original_bytes = std::fs::read(record_path(directory.path(), &original)).unwrap();
    let destinations = [
        ChatScope::new(hub("alice", 1), ContextId::General),
        ChatScope::this_computer(ContextId::Project(
            "00000000000000000000000000000001".parse().unwrap(),
        )),
    ];
    for scope in destinations {
        block_on(repository.save(Arc::new(snapshot(&scope, "owned")))).unwrap();
        let path = record_path(directory.path(), &scope);
        std::fs::write(&path, &original_bytes).unwrap();
        let load = block_on(repository.load_in(scope.clone(), ID.into())).unwrap_err();
        assert!(matches!(
            load,
            AgentStorageError::EnvironmentMismatch { .. } | AgentStorageError::ScopeMismatch { .. }
        ));
        assert!(block_on(repository.save(Arc::new(snapshot(&scope, "replacement")))).is_err());
        assert!(block_on(repository.delete_in(scope.clone(), ID.into())).is_err());
        assert!(block_on(repository.catalog_in(scope)).unwrap().is_empty());
        assert_eq!(std::fs::read(path).unwrap(), original_bytes);
    }
}

#[test]
fn replacing_a_native_snapshot_publishes_complete_bytes_and_preserves_the_old_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let scope = ChatScope::this_computer(ContextId::General);
    let repository = repository(directory.path());
    block_on(repository.save(Arc::new(snapshot(&scope, "old snapshot")))).unwrap();
    let path = record_path(directory.path(), &scope);
    let mut reader = std::fs::File::open(&path).unwrap();
    let mut old_bytes = Vec::new();
    #[cfg(windows)]
    {
        // Legacy Windows atomic rename may refuse an open target. Failure and
        // retry with retained staging are covered by the native LocalFs fixture.
        reader.read_to_end(&mut old_bytes).unwrap();
        drop(reader);
    }
    block_on(repository.save(Arc::new(snapshot(&scope, "replacement snapshot")))).unwrap();
    #[cfg(unix)]
    reader.read_to_end(&mut old_bytes).unwrap();
    let old = history::decode(&old_bytes, ID).unwrap();
    assert_eq!(old.draft.as_deref(), Some("old snapshot"));
    let replacement = block_on(repository.load_in(scope, ID.into())).unwrap();
    assert_eq!(replacement.draft.as_deref(), Some("replacement snapshot"));
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
}

#[test]
fn interrupted_native_staging_preserves_the_last_acknowledged_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let scope = ChatScope::this_computer(ContextId::General);
    let repository = repository(directory.path());
    block_on(repository.save(Arc::new(snapshot(&scope, "acknowledged")))).unwrap();
    let path = record_path(directory.path(), &scope);
    let old_bytes = std::fs::read(&path).unwrap();
    let filesystem =
        LocalFilesystem::with_base(std::fs::canonicalize(directory.path()).unwrap()).unwrap();
    let held = filesystem.open_directory(path.parent().unwrap()).unwrap();
    let mut abandoned = held.stage().unwrap();
    abandoned.write(b"incomplete replacement").unwrap();
    abandoned.sync().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), old_bytes);
    drop(abandoned);
    let mut canceled = held.stage().unwrap();
    canceled.write(b"canceled replacement").unwrap();
    canceled.cancel().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), old_bytes);
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
    assert_eq!(
        block_on(repository.load_in(scope, ID.into()))
            .unwrap()
            .draft
            .as_deref(),
        Some("acknowledged")
    );
    assert!(
        held.metadata(OsStr::new(&format!("{ID}.json")))
            .unwrap()
            .is_some()
    );
}
