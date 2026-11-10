//! Actual held-directory persistence keeps Local account and device histories separate.
use super::*;
use nocterm_ai::local_environment::{ChatScope, LocalEnvironment};
use nocterm_hubs_model::source_identity::{
    AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition, NamespaceId,
};
const ID: &str = "00000000-0000-0000-0000-000000000042";
fn scope(hub: u8, account: &str) -> ChatScope {
    ChatScope::new(
        LocalEnvironment::HubAccount(LocalAccountPartition::Stable(AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("same-instance-id").unwrap(),
                Fingerprint::from_bytes([hub; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new(account).unwrap(),
                Fingerprint::from_bytes([8; 32]),
            ),
        ))),
        ContextId::General,
    )
}
fn chat(scope: ChatScope, title: &str) -> SavedChat {
    let mut chat = SavedChat::for_environment("codex".into(), ID.into(), 1, scope);
    chat.header.title = Some(title.into());
    chat.entries
        .push(nocterm_ai::thread::Entry::Agent("saved message".into()));
    chat
}
fn path(root: &Path, scope: &ChatScope, extension: &str) -> PathBuf {
    let base = root
        .join("scoped-v3")
        .join(scope.environment().storage_key());
    let context = match scope.context() {
        ContextId::General => base.join("general"),
        ContextId::Project(project) => base.join("projects").join(project.to_string()),
    };
    context.join(format!("{ID}.{extension}"))
}
#[test]
fn identical_chat_ids_and_account_ids_remain_separate_across_hubs_and_this_computer() {
    let directory = tempfile::tempdir().unwrap();
    let a = scope(1, "same-account-id");
    let b = scope(2, "same-account-id");
    let local = ChatScope::this_computer(ContextId::General);
    let project = ChatScope::new(
        a.environment().clone(),
        ContextId::Project(nocterm_project_model::ProjectId::from_bytes([4; 16])),
    );
    for (scope, title) in [
        (a.clone(), "A"),
        (b.clone(), "B"),
        (local.clone(), "Device"),
        (project.clone(), "Project A"),
    ] {
        save(directory.path(), &chat(scope, title)).unwrap();
    }
    let repository = repository(directory.path());
    for (scope, title) in [
        (a.clone(), "A"),
        (b.clone(), "B"),
        (local.clone(), "Device"),
        (project.clone(), "Project A"),
    ] {
        let rows = block_on(repository.catalog_in(scope.clone())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title(), title);
        assert_eq!(
            block_on(repository.load_in(scope, ID.into()))
                .unwrap()
                .header
                .title
                .as_deref(),
            Some(title)
        );
    }
    block_on(repository.delete_in(a.clone(), ID.into())).unwrap();
    assert!(!path(directory.path(), &a, "json").exists());
    assert!(path(directory.path(), &b, "json").exists());
    assert!(path(directory.path(), &local, "json").exists());
    assert!(path(directory.path(), &project, "json").exists());
}
#[test]
fn old_versions_are_visible_only_in_this_computer_and_never_moved_to_an_account() {
    let directory = tempfile::tempdir().unwrap();
    let mut old = SavedChat::for_identity("codex".into(), ID.into(), 1);
    old.header.title = Some("Old device history".into());
    old.entries
        .push(nocterm_ai::thread::Entry::Agent("old message".into()));
    save(directory.path(), &old).unwrap();
    let before = fs::read(document(directory.path(), ContextId::General, ID, "json")).unwrap();
    let repository = repository(directory.path());
    assert_eq!(
        block_on(repository.catalog(ContextId::General))
            .unwrap()
            .len(),
        1
    );
    assert!(
        block_on(repository.catalog_in(scope(1, "alice")))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        block_on(repository.load_in(scope(1, "alice"), ID.into())),
        Err(AgentStorageError::NotFound(_))
    ));
    block_on(repository.delete_in(scope(1, "alice"), ID.into())).unwrap();
    assert_eq!(
        fs::read(document(directory.path(), ContextId::General, ID, "json")).unwrap(),
        before
    );
    assert!(!directory.path().join("scoped-v3").exists());
}
#[test]
fn a_foreign_header_in_an_account_namespace_cannot_be_loaded_overwritten_or_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let a = scope(1, "alice");
    let b = scope(1, "bob");
    save(directory.path(), &chat(a.clone(), "A")).unwrap();
    save(directory.path(), &chat(b.clone(), "B")).unwrap();
    let foreign = fs::read(path(directory.path(), &b, "json")).unwrap();
    fs::write(path(directory.path(), &a, "json"), &foreign).unwrap();
    let repository = repository(directory.path());
    assert!(matches!(
        block_on(repository.load_in(a.clone(), ID.into())),
        Err(AgentStorageError::EnvironmentMismatch { .. })
    ));
    assert!(
        block_on(repository.catalog_in(a.clone()))
            .unwrap()
            .is_empty()
    );
    assert!(save(directory.path(), &chat(a.clone(), "replacement")).is_err());
    assert!(block_on(repository.delete_in(a.clone(), ID.into())).is_err());
    assert_eq!(
        fs::read(path(directory.path(), &a, "json")).unwrap(),
        foreign
    );
    assert_eq!(
        block_on(repository.load_in(b, ID.into()))
            .unwrap()
            .header
            .title
            .as_deref(),
        Some("B")
    );
}
#[test]
fn obsolete_sidecars_cannot_redirect_account_membership_or_override_v3_headers() {
    let directory = tempfile::tempdir().unwrap();
    let a = scope(1, "alice");
    let b = scope(1, "bob");
    save(directory.path(), &chat(a.clone(), "A")).unwrap();
    save(directory.path(), &chat(b.clone(), "B")).unwrap();
    let obsolete = serde_json::to_vec(&nocterm_ai::history::ChatSummary::from_chat(&chat(
        b.clone(),
        "B",
    )))
    .unwrap();
    fs::write(path(directory.path(), &a, "meta"), &obsolete).unwrap();
    let rows = block_on(repository(directory.path()).catalog_in(a.clone())).unwrap();
    assert_eq!(rows[0].title(), "A");
    assert_eq!(rows[0].chat_scope(), a.clone());
    assert_eq!(
        fs::read(path(directory.path(), &a, "meta")).unwrap(),
        obsolete
    );
}
#[test]
fn corrupt_v3_does_not_reveal_shadowed_device_v2_history() {
    let directory = tempfile::tempdir().unwrap();
    let local = ChatScope::this_computer(ContextId::General);
    let mut old = SavedChat::for_identity("codex".into(), ID.into(), 1);
    old.header.title = Some("old".into());
    old.entries
        .push(nocterm_ai::thread::Entry::Agent("old message".into()));
    save(directory.path(), &old).unwrap();
    save(directory.path(), &chat(local.clone(), "new")).unwrap();
    fs::write(path(directory.path(), &local, "json"), b"broken").unwrap();
    let repository = repository(directory.path());
    assert!(
        block_on(repository.catalog(ContextId::General))
            .unwrap()
            .is_empty()
    );
    assert!(block_on(repository.load(ContextId::General, ID.into())).is_err());
}
#[cfg(unix)]
#[test]
fn a_symlinked_account_directory_cannot_read_or_mutate_another_partition() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let a = scope(1, "alice");
    let b = scope(1, "bob");
    save(directory.path(), &chat(b.clone(), "B")).unwrap();
    let b_path = directory
        .path()
        .join("scoped-v3")
        .join(b.environment().storage_key());
    let a_path = directory
        .path()
        .join("scoped-v3")
        .join(a.environment().storage_key());
    symlink(&b_path, &a_path).unwrap();
    let before = fs::read(path(directory.path(), &b, "json")).unwrap();
    let repository = repository(directory.path());
    assert!(block_on(repository.load_in(a.clone(), ID.into())).is_err());
    assert!(save(directory.path(), &chat(a.clone(), "intruder")).is_err());
    assert!(block_on(repository.delete_in(a, ID.into())).is_err());
    assert_eq!(
        fs::read(path(directory.path(), &b, "json")).unwrap(),
        before
    );
}
