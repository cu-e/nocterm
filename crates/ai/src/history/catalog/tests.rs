use super::*;
use crate::{
    acp,
    history::{SavedAttachment, SavedPrompt},
    thread::Entry,
};
fn chat() -> SavedChat {
    let mut chat = SavedChat::new("codex".into());
    chat.entries.push(Entry::User(vec![acp::ContentBlock::Text(
        acp::TextContent::new("first message"),
    )]));
    chat
}
#[test]
fn catalog_is_small_and_payloads_load_only_when_requested() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    chat.entries.push(Entry::Agent("x".repeat(1024 * 1024)));
    super::super::save(dir.path(), &chat).unwrap();
    let rows = load_catalog(dir.path());
    assert_eq!(rows.len(), 1);
    assert!(serde_json::to_string(&rows[0]).unwrap().len() < 1024);
    assert_eq!(rows[0].title(), "first message");
    assert_eq!(load(dir.path(), &chat.id).unwrap().entries.len(), 2);
}
#[test]
fn stale_cache_rebuilds_and_corrupt_files_survive() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    super::super::save(dir.path(), &chat).unwrap();
    chat.name = Some("Changed outside nocterm".into());
    fs::write(
        file(dir.path(), &chat.id).unwrap(),
        serde_json::to_string(&chat).unwrap(),
    )
    .unwrap();
    let broken = dir.path().join(format!("{}.json", uuid::Uuid::new_v4()));
    fs::write(&broken, "{").unwrap();
    assert_eq!(load_catalog(dir.path())[0].name, chat.name);
    assert_eq!(fs::read_to_string(broken).unwrap(), "{");
}
#[test]
fn legacy_scan_discards_payloads_and_keeps_pinned_newest() {
    let dir = tempfile::tempdir().unwrap();
    let mut oldest = String::new();
    for index in 0..MAX_RESTORED + 5 {
        let mut chat = chat();
        chat.updated = index as u64;
        chat.pinned = index == 0;
        if chat.pinned {
            oldest = chat.id.clone();
        }
        fs::write(
            file(dir.path(), &chat.id).unwrap(),
            serde_json::to_string(&chat).unwrap(),
        )
        .unwrap();
    }
    let rows = load_catalog(dir.path());
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
    chat.name = Some("Named".into());
    chat.pinned = true;
    let prompt = SavedPrompt {
        id: 7,
        text: "pending".into(),
        images: vec![acp::ContentBlock::Image(acp::ImageContent::new(
            "A".repeat(600_000),
            "image/png",
        ))],
        attachments: vec![SavedAttachment::Connection("host".into())],
    };
    let id = chat.id.clone();
    super::super::save_shared(
        dir.path(),
        &super::super::SharedChat {
            metadata: chat,
            queue: vec![std::sync::Arc::new(prompt)],
        },
    )
    .unwrap();
    assert!(load_catalog(dir.path())[0].has_content);
    let loaded = load(dir.path(), &id).unwrap();
    assert_eq!(loaded.draft.as_deref(), Some("literal draft"));
    assert_eq!(loaded.queue[0].id, 7);
    assert_eq!(loaded.queue[0].attachments.len(), 1);
    assert!(
        matches!(&loaded.queue[0].images[0], acp::ContentBlock::Image(image) if image.data.len() == 600_000)
    );
}
#[test]
fn malformed_collection_and_nested_amplification_are_rejected_without_erasing_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut chat = chat();
    chat.entries = (0..2001).map(|_| Entry::Agent(String::new())).collect();
    let path = file(dir.path(), &chat.id).unwrap();
    fs::write(&path, serde_json::to_string(&chat).unwrap()).unwrap();
    assert!(load(dir.path(), &chat.id).is_err());
    chat.entries = vec![Entry::Tool(
        acp::ToolCall::new("tool", "amplification")
            .raw_input(serde_json::json!({"tiny":vec![0; 140_000]})),
    )];
    fs::write(&path, serde_json::to_string(&chat).unwrap()).unwrap();
    assert!(load(dir.path(), &chat.id).is_err());
    assert!(super::super::save(dir.path(), &chat).is_err());
    assert!(chat.validate_size().is_err());
    assert!(path.exists());
}
#[test]
fn cached_structural_estimates_cover_preflight_and_pending_only_rows() {
    let mut chat = chat();
    chat.queue.push(SavedPrompt {
        id: 1,
        text: "界\n\"".into(),
        images: vec![],
        attachments: vec![],
    });
    let wire = super::super::wire::Wire::new(&chat, Some(&chat.queue));
    let bytes = serde_json::to_vec(&wire).unwrap();
    let preflight = super::super::preflight::check(&bytes).unwrap();
    let estimated = super::super::size::structural_size(&wire).unwrap();
    assert!(estimated >= preflight, "{estimated} < {preflight}");
    assert!(estimated >= bytes.len());
}
