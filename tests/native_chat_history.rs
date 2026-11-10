//! New environment-scoped (v3) chats save and reload through the production
//! held-directory filesystem on every native platform, including Windows.
use futures::executor::block_on;
use nocterm_agent_storage::FileChatRepository;
use nocterm_ai::{
    history::{ContextId, SavedChat, SavedPrompt, SharedChat, TranscriptVersion},
    local_environment::{ChatScope, LocalEnvironment},
    storage::{AgentStorageError, ChatRepository},
    thread::Entry,
};
use nocterm_hubs_model::source_identity::{
    AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition, NamespaceId,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const ID: &str = "00000000-0000-4000-8000-0000000000c3";

fn root() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path())
        .unwrap()
        .join("agent-history");
    (directory, root)
}
fn repository(root: &Path) -> FileChatRepository {
    FileChatRepository::new(
        root.to_owned(),
        Arc::new(nocterm_localfs::LocalFilesystem::new().unwrap()),
    )
}
fn account(hub: u8) -> LocalEnvironment {
    LocalEnvironment::HubAccount(LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("instance").unwrap(),
            Fingerprint::from_bytes([hub; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new("account").unwrap(),
            Fingerprint::from_bytes([7; 32]),
        ),
    )))
}
fn project() -> ContextId {
    ContextId::Project(nocterm_project_model::ProjectId::from_bytes([5; 16]))
}
/// The shape a new chat document has when its first message is queued.
fn new_chat(scope: ChatScope, title: &str) -> SharedChat {
    let mut chat = SavedChat::for_environment("codex".into(), ID.into(), 1, scope);
    assert_eq!(chat.version, TranscriptVersion::ENVIRONMENT.wire());
    chat.header.title = Some(title.into());
    chat.entries.push(Entry::Agent(format!("{title} answer")));
    let queued = Arc::new(SavedPrompt {
        id: 1,
        text: format!("{title} queued"),
        images: Vec::new(),
        attachments: Vec::new(),
    });
    chat.queue = vec![(*queued).clone()];
    SharedChat {
        metadata: chat,
        queue: vec![queued],
    }
}
fn document(root: &Path, scope: &ChatScope) -> PathBuf {
    let base = root
        .join("scoped-v3")
        .join(scope.environment().storage_key());
    let context = match scope.context() {
        ContextId::General => base.join("general"),
        ContextId::Project(id) => base.join("projects").join(id.to_string()),
    };
    context.join(format!("{ID}.json"))
}

#[test]
fn new_chats_in_every_environment_and_context_save_reload_and_stay_isolated() {
    let (_directory, root) = root();
    let scopes = [
        (ChatScope::this_computer(ContextId::General), "device"),
        (ChatScope::this_computer(project()), "device project"),
        (ChatScope::new(account(1), ContextId::General), "account"),
        (
            ChatScope::new(account(2), project()),
            "other account project",
        ),
    ];
    for (scope, title) in &scopes {
        block_on(repository(&root).save(Arc::new(new_chat(scope.clone(), title)))).unwrap();
        assert!(document(&root, scope).is_file(), "{title} was not written");
    }
    // A fresh repository stands for a restarted application.
    let restarted = repository(&root);
    for (scope, title) in &scopes {
        let rows = block_on(restarted.catalog_in(scope.clone())).unwrap();
        assert_eq!(rows.len(), 1, "{title}");
        let chat = block_on(restarted.load_in(scope.clone(), ID.into())).unwrap();
        assert_eq!(chat.header.title.as_deref(), Some(*title));
        assert_eq!(chat.version, TranscriptVersion::ENVIRONMENT.wire());
        assert_eq!(chat.queue.len(), 1, "{title} lost its queued message");
        assert_eq!(chat.queue[0].text, format!("{title} queued"));
    }
    assert!(matches!(
        block_on(restarted.load_in(ChatScope::new(account(3), ContextId::General), ID.into())),
        Err(AgentStorageError::NotFound(_))
    ));
    block_on(restarted.delete_in(scopes[2].0.clone(), ID.into())).unwrap();
    assert!(!document(&root, &scopes[2].0).exists());
    for index in [0, 1, 3] {
        assert!(document(&root, &scopes[index].0).is_file());
    }
}

#[test]
fn a_later_snapshot_replaces_the_saved_chat_without_leaving_staging_files() {
    let (_directory, root) = root();
    let scope = ChatScope::this_computer(ContextId::General);
    let repository = repository(&root);
    block_on(repository.save(Arc::new(new_chat(scope.clone(), "first")))).unwrap();
    let mut later = new_chat(scope.clone(), "second");
    later.metadata.queue.clear();
    later.queue.clear();
    block_on(repository.save(Arc::new(later))).unwrap();
    let chat = block_on(repository.load_in(scope.clone(), ID.into())).unwrap();
    assert_eq!(chat.header.title.as_deref(), Some("second"));
    assert!(chat.queue.is_empty());
    let names: Vec<_> = std::fs::read_dir(document(&root, &scope).parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from(format!("{ID}.json"))]);
}

#[cfg(windows)]
#[test]
fn a_junctioned_account_partition_is_refused_without_touching_its_target() {
    let (_directory, root) = root();
    let outside = tempfile::tempdir().unwrap();
    let scope = ChatScope::new(account(1), ContextId::General);
    let partition = root
        .join("scoped-v3")
        .join(scope.environment().storage_key());
    std::fs::create_dir_all(partition.parent().unwrap()).unwrap();
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&partition)
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success());
    let saved = block_on(repository(&root).save(Arc::new(new_chat(scope, "account"))));
    assert!(saved.is_err(), "a junctioned partition accepted a write");
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}
