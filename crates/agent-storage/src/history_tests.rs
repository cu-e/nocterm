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
    chat.title = Some("Disk usage".into());
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
    let loaded = load(dir.path(), &saved.id).unwrap();
    assert_eq!(loaded.config, saved.config);
    assert_eq!(loaded.mode.as_deref(), Some("plan"));
}

#[test]
fn chats_round_trip_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut older = chat();
    older.updated = 10;
    let mut newer = chat();
    newer.updated = 20;
    save(dir.path(), &older).unwrap();
    save(dir.path(), &newer).unwrap();

    let loaded = load_all(dir.path());

    assert_eq!(
        loaded
            .iter()
            .map(|chat| chat.id.as_str())
            .collect::<Vec<_>>(),
        [newer.id.as_str(), older.id.as_str()]
    );
    assert_eq!(loaded[0].entries.len(), 3);
    assert!(matches!(&loaded[0].entries[2], Entry::Agent(text) if text == "Use `df -h`."));
    assert_eq!(loaded[0].session_id.as_deref(), Some("session-1"));
}

#[test]
fn pinned_chats_come_first_with_their_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut pinned = chat();
    pinned.updated = 1;
    pinned.pinned = true;
    pinned.name = Some("Disks".into());
    let mut newer = chat();
    newer.updated = 2;
    save(dir.path(), &pinned).unwrap();
    save(dir.path(), &newer).unwrap();

    let loaded = load_all(dir.path());

    assert_eq!(loaded[0].id, pinned.id);
    assert!(loaded[0].pinned);
    assert_eq!(loaded[0].name.as_deref(), Some("Disks"));
    assert!(!loaded[1].pinned);
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
    assert_eq!(mode(&dir.join(format!("{}.json", chat.id))), 0o600);
}

#[test]
fn deleting_removes_the_file_and_ids_cannot_walk_paths() {
    let dir = tempfile::tempdir().unwrap();
    let chat = chat();
    save(dir.path(), &chat).unwrap();
    delete(dir.path(), &chat.id).unwrap();
    delete(dir.path(), &chat.id).unwrap();
    assert!(load_all(dir.path()).is_empty());

    let mut evil = chat.clone();
    evil.id = "../escape".into();
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
    original.name = Some("keep the name".into());
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
            original.id.clone(),
            "other-agent".into(),
            draft.into(),
            original.updated + 1,
        ),
        queue: Vec::new(),
    };
    save_shared(directory.path(), &patch).unwrap();
    let loaded = load(directory.path(), &original.id).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded.entries).unwrap(),
        serde_json::to_value(&original.entries).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&loaded.queue).unwrap(),
        serde_json::to_value(&original.queue).unwrap()
    );
    assert_eq!(loaded.name, original.name);
    assert_eq!(loaded.agent_id, original.agent_id);
    assert_eq!(loaded.draft.as_deref(), Some(draft));
    assert_eq!(loaded.updated, original.updated + 1);
}
#[test]
fn best_effort_cache_failure_does_not_revoke_a_published_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let chat = chat();
    std::fs::create_dir(directory.path().join(format!("{}.meta", chat.id))).unwrap();
    save(directory.path(), &chat).unwrap();
    assert_eq!(
        serde_json::to_value(load(directory.path(), &chat.id).unwrap().entries).unwrap(),
        serde_json::to_value(chat.entries).unwrap()
    );
}

#[test]
fn a_version_one_file_loads_and_saves_back_byte_for_byte() {
    const ID: &str = "00000000-0000-4000-8000-0000000000a1";
    const CANONICAL: &[u8] = include_bytes!("../../ai/src/history/fixtures/v1/canonical.json");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{ID}.json"));
    std::fs::write(&path, CANONICAL).unwrap();

    let loaded = load(directory.path(), ID).unwrap();
    assert_eq!(loaded.entries.len(), 6);
    assert_eq!(loaded.queue.len(), 1);
    save(directory.path(), &loaded).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), CANONICAL);
}
