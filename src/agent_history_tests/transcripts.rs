use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::{
    history::{MAX_FILE_BYTES, SavedAttachment, SavedPrompt},
    thread::Entry,
};
const MAX_SAVED_IMAGE_BYTES: usize = 512 * 1024;
const MAX_SAVED_ENTRIES: usize = 2000;

fn chat() -> SavedChat {
    let mut chat = SavedChat::for_identity("codex".into(), uuid::Uuid::new_v4().to_string(), 1);
    chat.header.title = Some("Disk usage".into());
    chat.session_id = Some("session-1".into());
    chat.entries = vec![
        Entry::User(vec![Content::text("df -h?")]),
        Entry::Thought("checking".into()),
        Entry::Agent("Use `df -h`.".into()),
    ];
    chat
}

#[test]
fn a_chat_keeps_its_configuration_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut saved = chat();
    saved.config.insert(
        "model".into(),
        nocterm_ai::session_config::ConfigValue::Value("opus".into()),
    );
    saved.mode = Some("plan".into());
    save(dir.path(), &saved).unwrap();
    let loaded = load(dir.path(), &saved.header.id).unwrap();
    assert_eq!(loaded.config, saved.config);
    assert_eq!(loaded.mode.as_deref(), Some("plan"));
}

#[test]
fn chats_round_trip_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut older = chat();
    older.header.updated = 10;
    let mut newer = chat();
    newer.header.updated = 20;
    save(dir.path(), &older).unwrap();
    save(dir.path(), &newer).unwrap();

    let loaded = load_all(dir.path());

    assert_eq!(
        loaded
            .iter()
            .map(|chat| chat.header.id.as_str())
            .collect::<Vec<_>>(),
        [newer.header.id.as_str(), older.header.id.as_str()]
    );
    assert_eq!(loaded[0].entries.len(), 3);
    assert!(matches!(&loaded[0].entries[2], Entry::Agent(text) if text == "Use `df -h`."));
    assert_eq!(loaded[0].session_id.as_deref(), Some("session-1"));
}

#[test]
fn pinned_chats_come_first_with_their_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut pinned = chat();
    pinned.header.updated = 1;
    pinned.header.pinned = true;
    pinned.header.name = Some("Disks".into());
    let mut newer = chat();
    newer.header.updated = 2;
    save(dir.path(), &pinned).unwrap();
    save(dir.path(), &newer).unwrap();

    let loaded = load_all(dir.path());

    assert_eq!(loaded[0].header.id, pinned.header.id);
    assert!(loaded[0].header.pinned);
    assert_eq!(loaded[0].header.name.as_deref(), Some("Disks"));
    assert!(!loaded[1].header.pinned);
}

#[cfg(unix)]
#[test]
fn chats_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("chats");
    let chat = chat();
    save(&dir, &chat).unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(
        mode(&document(&dir, chat.header.scope, &chat.header.id, "json")),
        0o600
    );
}

#[test]
fn deleting_removes_the_file_and_ids_cannot_walk_paths() {
    let dir = tempfile::tempdir().unwrap();
    let chat = chat();
    save(dir.path(), &chat).unwrap();
    delete(dir.path(), &chat.header.id).unwrap();
    delete(dir.path(), &chat.header.id).unwrap();
    assert!(load_all(dir.path()).is_empty());

    let mut evil = chat.clone();
    evil.header.id = "../escape".into();
    assert!(save(dir.path(), &evil).is_err());
    assert!(delete(dir.path(), "../escape").is_err());
}

#[test]
fn large_images_and_old_entries_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    let image = Content::Image(nocterm_ai::model::content::Image::new(
        "A".repeat(MAX_SAVED_IMAGE_BYTES + 1),
        "image/png",
    ));
    chat.entries = (0..MAX_SAVED_ENTRIES + 5)
        .map(|ix| Entry::Agent(ix.to_string()))
        .collect();
    chat.entries.push(Entry::User(vec![image]));
    save(dir.path(), &chat).unwrap();

    let loaded = &load_all(dir.path())[0];

    assert_eq!(loaded.entries.len(), MAX_SAVED_ENTRIES);
    assert!(matches!(
        loaded.entries.last(),
        Some(Entry::User(blocks)) if matches!(&blocks[0], Content::Text(text) if text.text.contains("not kept"))
    ));
}

#[test]
fn foreign_and_broken_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.json"), "{").unwrap();
    let chat = chat();
    let text = serde_json::to_string(&chat).unwrap();
    // A valid chat under someone else's name.
    std::fs::write(
        dir.path().join(format!("{}.json", uuid::Uuid::new_v4())),
        text,
    )
    .unwrap();
    assert!(load_all(dir.path()).is_empty());
}
#[test]
fn pending_images_and_attachment_snapshots_round_trip_without_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    let data = "A".repeat(MAX_SAVED_IMAGE_BYTES + 1);
    chat.attachments = vec![
        SavedAttachment::Connection("server".into()),
        SavedAttachment::Group("production".into()),
    ];
    chat.queue = vec![SavedPrompt {
        id: 42,
        text: "queued image".into(),
        images: vec![Content::Image(nocterm_ai::model::content::Image::new(
            data.clone(),
            "image/png",
        ))],
        attachments: chat.attachments.clone(),
    }];
    save(dir.path(), &chat).unwrap();
    let saved = load_all(dir.path()).remove(0);
    assert_eq!(saved.queue[0].text, "queued image");
    assert_eq!(saved.attachments, chat.attachments);
    assert_eq!(saved.queue[0].attachments, chat.attachments);
    assert!(matches!(&saved.queue[0].images[0], Content::Image(image) if image.data == data));
}

#[test]
fn oversized_pending_messages_are_rejected_and_old_chats_still_load() {
    let mut chat = chat();
    chat.version = 1;
    let mut old = serde_json::to_value(&chat).unwrap();
    old.as_object_mut().unwrap().remove("queue");
    old.as_object_mut().unwrap().remove("attachments");
    let old: SavedChat = serde_json::from_value(old).unwrap();
    assert!(old.queue.is_empty());
    assert!(old.attachments.is_empty());
    chat.queue = vec![SavedPrompt {
        id: 1,
        text: "x".repeat(MAX_FILE_BYTES as usize),
        images: Vec::new(),
        attachments: Vec::new(),
    }];
    assert!(chat.validate_size().is_err());
}

#[test]
fn archived_draft_publication_merges_only_draft_and_updated_without_hydration() {
    let directory = tempfile::tempdir().unwrap();
    let mut original = chat();
    original.header.name = Some("keep the name".into());
    original.queue.push(SavedPrompt {
        id: 9,
        text: "keep queued text".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    });
    save(directory.path(), &original).unwrap();
    let draft = "literal quote\"\n界";
    let patch = history::SharedChat {
        metadata: SavedChat::archive_draft(
            original.header.id.clone(),
            "other-agent".into(),
            draft.into(),
            original.header.updated + 1,
        ),
        queue: Vec::new(),
    };
    save_shared(directory.path(), &patch).unwrap();
    let loaded = load(directory.path(), &original.header.id).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded.entries).unwrap(),
        serde_json::to_value(&original.entries).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&loaded.queue).unwrap(),
        serde_json::to_value(&original.queue).unwrap()
    );
    assert_eq!(loaded.header.name, original.header.name);
    assert_eq!(loaded.header.agent_id, original.header.agent_id);
    assert_eq!(loaded.draft.as_deref(), Some(draft));
    assert_eq!(loaded.header.updated, original.header.updated + 1);
}
#[test]
fn best_effort_cache_failure_does_not_revoke_a_published_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let chat = chat();
    let meta = document(directory.path(), chat.header.scope, &chat.header.id, "meta");
    std::fs::create_dir_all(&meta).unwrap();
    save(directory.path(), &chat).unwrap();
    assert_eq!(
        serde_json::to_value(load(directory.path(), &chat.header.id).unwrap().entries).unwrap(),
        serde_json::to_value(chat.entries).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn archived_draft_publication_requires_the_original_documents_scope() {
    let directory = tempfile::tempdir().unwrap();
    let mut original = chat();
    original.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    save(directory.path(), &original).unwrap();
    let path = document(
        directory.path(),
        original.header.scope,
        &original.header.id,
        "json",
    );
    let before = std::fs::read(&path).unwrap();
    let patch = SharedChat {
        metadata: SavedChat::archive_draft(
            original.header.id.clone(),
            "codex".into(),
            "foreign".into(),
            2,
        ),
        queue: Vec::new(),
    };
    assert!(matches!(
        save_shared(directory.path(), &patch),
        Err(AgentStorageError::NotFound(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let patch = SavedChat::archive_draft_in_scope(
        original.header.id.clone(),
        "codex".into(),
        "owned draft".into(),
        3,
        original.header.scope,
    );
    save_shared(
        directory.path(),
        &SharedChat {
            metadata: patch,
            queue: Vec::new(),
        },
    )
    .unwrap();
    let loaded = load_scoped(directory.path(), original.header.scope, &original.header.id).unwrap();
    assert_eq!(loaded.header.scope, original.header.scope);
    assert_eq!(loaded.draft.as_deref(), Some("owned draft"));
    assert_eq!(loaded.entries.len(), original.entries.len());
}

#[cfg(unix)]
#[test]
fn repository_load_and_delete_require_the_captured_context() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = chat();
    project.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    let repository = repository(directory.path());
    let shared = Arc::new(SharedChat {
        metadata: project.clone(),
        queue: Vec::new(),
    });
    futures::executor::block_on(repository.save(shared)).unwrap();
    let general = history::ContextId::General;
    let path = document(
        directory.path(),
        project.header.scope,
        &project.header.id,
        "json",
    );
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        block_on(repository.load(general, project.header.id.clone())),
        Err(AgentStorageError::NotFound(_))
    ));
    block_on(repository.delete(general, project.header.id.clone())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut foreign = project.clone();
    foreign.header.scope = general;
    block_on(repository.save(Arc::new(SharedChat {
        metadata: foreign,
        queue: Vec::new(),
    })))
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    block_on(repository.delete(general, project.header.id.clone())).unwrap();
    let loaded = futures::executor::block_on(
        repository.load(project.header.scope, project.header.id.clone()),
    )
    .unwrap();
    assert_eq!(loaded.header.scope, project.header.scope);
    futures::executor::block_on(repository.delete(project.header.scope, project.header.id.clone()))
        .unwrap();
    assert!(matches!(
        load_scoped(directory.path(), project.header.scope, &project.header.id),
        Err(AgentStorageError::NotFound(_))
    ));
    futures::executor::block_on(repository.delete(project.header.scope, project.header.id))
        .unwrap();
}

#[cfg(unix)]
#[test]
fn project_documents_are_absent_from_legacy_flat_directory_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = chat();
    project.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    save(directory.path(), &project).unwrap();
    let legacy_files = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json" || extension == "meta")
        })
        .count();
    assert_eq!(legacy_files, 0);
}

#[cfg(unix)]
#[test]
fn an_identical_chat_id_in_two_contexts_never_aliases_the_other_document() {
    let directory = tempfile::tempdir().unwrap();
    let general = chat();
    let mut project = general.clone();
    project.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    project.header.title = Some("private project title".into());
    let repository = repository(directory.path());
    for chat in [general.clone(), project.clone()] {
        futures::executor::block_on(repository.save(Arc::new(SharedChat {
            metadata: chat,
            queue: Vec::new(),
        })))
        .unwrap();
    }
    let loaded = futures::executor::block_on(
        repository.load(general.header.scope, general.header.id.clone()),
    )
    .unwrap();
    assert_eq!(loaded.header.title, general.header.title);
    let loaded = futures::executor::block_on(
        repository.load(project.header.scope, project.header.id.clone()),
    )
    .unwrap();
    assert_eq!(loaded.header.title, project.header.title);
    futures::executor::block_on(repository.delete(project.header.scope, project.header.id.clone()))
        .unwrap();
    assert!(
        futures::executor::block_on(repository.load(general.header.scope, general.header.id))
            .is_ok()
    );
}

#[test]
fn a_version_one_file_loads_and_saves_back_byte_for_byte() {
    const ID: &str = "00000000-0000-4000-8000-0000000000a1";
    const CANONICAL: &[u8] =
        include_bytes!("../../crates/ai/src/history/fixtures/v1/canonical.json");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{ID}.json"));
    std::fs::write(&path, CANONICAL).unwrap();

    let loaded = load(directory.path(), ID).unwrap();
    assert_eq!(loaded.entries.len(), 6);
    assert_eq!(loaded.queue.len(), 1);
    save(directory.path(), &loaded).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), CANONICAL);
}
