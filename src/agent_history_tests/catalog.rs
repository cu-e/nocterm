use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::tools::ToolRecord;
fn file(dir: &Path, id: &str) -> Option<PathBuf> {
    let path = document(dir, ContextId::General, id, "json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    Some(path)
}
use nocterm_ai::{
    history::{SavedAttachment, SavedPrompt},
    thread::Entry,
};
fn chat() -> SavedChat {
    let mut chat = SavedChat::for_identity("codex".into(), uuid::Uuid::new_v4().to_string(), 1);
    chat.entries.push(Entry::User(vec![Content::Text(
        nocterm_ai::model::content::Text::new("first message"),
    )]));
    chat
}
#[test]
fn catalog_is_small_and_payloads_load_only_when_requested() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    chat.entries.push(Entry::Agent("x".repeat(1024 * 1024)));
    save(dir.path(), &chat).unwrap();
    let rows = load_catalog(dir.path(), ContextId::General);
    assert_eq!(rows.len(), 1);
    assert!(serde_json::to_string(&rows[0]).unwrap().len() < 1024);
    assert_eq!(rows[0].title(), "first message");
    assert_eq!(load(dir.path(), &chat.header.id).unwrap().entries.len(), 2);
}
#[test]
fn stale_cache_rebuilds_and_corrupt_files_survive() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    save(dir.path(), &chat).unwrap();
    chat.header.name = Some("Changed outside nocterm".into());
    fs::write(
        file(dir.path(), &chat.header.id).unwrap(),
        serde_json::to_string(&chat).unwrap(),
    )
    .unwrap();
    let broken = dir.path().join(format!("{}.json", uuid::Uuid::new_v4()));
    fs::write(&broken, "{").unwrap();
    assert_eq!(
        load_catalog(dir.path(), ContextId::General)[0].name,
        chat.header.name
    );
    assert_eq!(fs::read_to_string(broken).unwrap(), "{");
}
#[test]
fn legacy_scan_discards_payloads_and_keeps_pinned_newest() {
    let dir = tempfile::tempdir().unwrap();
    let mut oldest = String::new();
    for index in 0..MAX_RESTORED + 5 {
        let mut chat = chat();
        chat.header.updated = index as u64;
        chat.header.pinned = index == 0;
        if chat.header.pinned {
            oldest = chat.header.id.clone();
        }
        chat.version = 1;
        fs::write(
            dir.path().join(format!("{}.json", chat.header.id)),
            serde_json::to_string(&chat).unwrap(),
        )
        .unwrap();
    }
    let rows = load_catalog(dir.path(), ContextId::General);
    assert_eq!(rows.len(), MAX_RESTORED);
    assert_eq!(rows[0].id, oldest);
    assert!(rows[1].updated > rows[2].updated);
}
#[test]
fn queued_images_drafts_and_metadata_survive_lazy_loading() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    chat.entries.clear();
    chat.draft = Some("literal draft".into());
    chat.header.name = Some("Named".into());
    chat.header.pinned = true;
    let prompt = SavedPrompt {
        id: 7,
        text: "pending".into(),
        images: vec![Content::Image(nocterm_ai::model::content::Image::new(
            "A".repeat(600_000),
            "image/png",
        ))],
        attachments: vec![SavedAttachment::Connection("host".into())],
    };
    let id = chat.header.id.clone();
    save_shared(
        dir.path(),
        &nocterm_ai::history::SharedChat {
            metadata: chat,
            queue: vec![std::sync::Arc::new(prompt)],
        },
    )
    .unwrap();
    assert!(load_catalog(dir.path(), ContextId::General)[0].has_content);
    let loaded = load(dir.path(), &id).unwrap();
    assert_eq!(loaded.draft.as_deref(), Some("literal draft"));
    assert_eq!(loaded.queue[0].id, 7);
    assert_eq!(loaded.queue[0].attachments.len(), 1);
    assert!(
        matches!(&loaded.queue[0].images[0], Content::Image(image) if image.data.len() == 600_000)
    );
}
#[test]
fn malformed_collection_and_nested_amplification_are_rejected_without_erasing_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    chat.entries = (0..2001).map(|_| Entry::Agent(String::new())).collect();
    let path = file(dir.path(), &chat.header.id).unwrap();
    fs::write(&path, serde_json::to_string(&chat).unwrap()).unwrap();
    assert!(load(dir.path(), &chat.header.id).is_err());
    chat.entries = vec![Entry::Tool(
        ToolRecord::new("tool", "amplification")
            .raw_input(serde_json::json!({"tiny":vec![0; 140_000]})),
    )];
    fs::write(&path, serde_json::to_string(&chat).unwrap()).unwrap();
    assert!(load(dir.path(), &chat.header.id).is_err());
    assert!(save(dir.path(), &chat).is_err());
    assert!(chat.validate_size().is_err());
    assert!(path.exists());
}

#[cfg(unix)]
#[test]
fn forged_cache_cannot_change_authoritative_scope_or_displayed_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    chat.header.scope = serde_json::from_value(serde_json::json!({
        "kind": "project", "id": "01010101010101010101010101010101"
    }))
    .unwrap();
    save(dir.path(), &chat).unwrap();
    let path = document(dir.path(), chat.header.scope, &chat.header.id, "meta");
    let mut cache: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    cache["scope"] = serde_json::json!({"kind": "general"});
    cache["title"] = serde_json::json!("forged title");
    cache["name"] = serde_json::json!("forged name");
    cache["pinned"] = serde_json::json!(true);
    fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
    assert!(load_catalog(dir.path(), ContextId::General).is_empty());
    let rows = load_catalog(dir.path(), chat.header.scope);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title(), "first message");
    assert_eq!(rows[0].name, None);
    assert!(!rows[0].pinned);
    assert_eq!(rows[0].scope, chat.header.scope);
}

#[cfg(unix)]
#[test]
fn catalog_filters_scope_before_its_restored_record_budget() {
    let dir = tempfile::tempdir().unwrap();
    let general = chat();
    save(dir.path(), &general).unwrap();
    for index in 0..MAX_RESTORED + 1 {
        let mut project = chat();
        project.header.scope = serde_json::from_value(serde_json::json!({
            "kind": "project", "id": "01010101010101010101010101010101"
        }))
        .unwrap();
        project.header.updated = index as u64 + 2;
        save(dir.path(), &project).unwrap();
    }
    let rows = load_catalog(dir.path(), ContextId::General);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, general.header.id);
}

#[cfg(unix)]
#[test]
fn a_linked_sidecar_is_rebuilt_without_touching_its_foreign_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let chat = chat();
    save(directory.path(), &chat).unwrap();
    let path = document(
        directory.path(),
        ContextId::General,
        &chat.header.id,
        "meta",
    );
    let foreign = directory.path().join("foreign-cache");
    fs::write(&foreign, b"foreign private bytes").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&foreign, &path).unwrap();
    assert_eq!(
        load_catalog(directory.path(), ContextId::General)[0].title(),
        "first message"
    );
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign private bytes");
    assert!(!fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}
