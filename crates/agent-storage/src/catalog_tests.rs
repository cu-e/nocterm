use super::*;
use crate::{
    chats::load,
    history_fixtures::{edited, image, text, tool},
};
use nocterm_ai::{
    history::{SavedAttachment, SavedPrompt, SharedChat},
    model::content::Content,
    thread::Entry,
};
fn file(dir: &Path, id: &str) -> Option<PathBuf> {
    crate::loading::file(dir, id).ok()
}
fn chat(authority: &nocterm_ai::model::retention::RetentionAuthority) -> SavedChat {
    edited(
        SavedChat::for_identity(
            authority,
            "codex".into(),
            uuid::Uuid::new_v4().to_string(),
            1,
        ),
        |_, _, transcript, _| {
            transcript
                .entries
                .push(Entry::User(vec![text("first message")]));
        },
    )
}
#[test]
fn catalog_is_small_and_payloads_load_only_when_requested() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = edited(chat(&authority), |_, _, transcript, _| {
        transcript
            .entries
            .push(Entry::Agent("x".repeat(1024 * 1024)))
    });
    crate::chats::save(dir.path(), &chat).unwrap();
    let rows = load_catalog(dir.path());
    assert_eq!(rows.len(), 1);
    assert!(serde_json::to_string(&rows[0]).unwrap().len() < 1024);
    assert_eq!(rows[0].title(), "first message");
    assert_eq!(
        load(dir.path(), chat.id(), &authority)
            .unwrap()
            .transcript()
            .entries
            .len(),
        2
    );
}
#[test]
fn stale_cache_rebuilds_and_corrupt_files_survive() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = chat(&authority);
    crate::chats::save(dir.path(), &chat).unwrap();
    let chat = edited(chat, |metadata, _, _, _| {
        metadata.name = Some("Changed outside nocterm".into())
    });
    fs::write(
        file(dir.path(), chat.id()).unwrap(),
        crate::legacy::fixture_bytes(&chat),
    )
    .unwrap();
    let broken = dir.path().join(format!("{}.json", uuid::Uuid::new_v4()));
    fs::write(&broken, "{").unwrap();
    assert_eq!(load_catalog(dir.path())[0].name, chat.metadata().name);
    assert_eq!(fs::read_to_string(broken).unwrap(), "{");
}
#[test]
fn legacy_scan_discards_payloads_and_keeps_pinned_newest() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut oldest = String::new();
    for index in 0..MAX_RESTORED + 5 {
        let chat = edited(chat(&authority), |metadata, _, _, _| {
            metadata.updated = index as u64;
            metadata.pinned = index == 0;
        });
        if chat.metadata().pinned {
            oldest = chat.id().to_owned();
        }
        fs::write(
            file(dir.path(), chat.id()).unwrap(),
            crate::legacy::fixture_bytes(&chat),
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
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = edited(chat(&authority), |metadata, _, transcript, composer| {
        transcript.entries.clear();
        composer.draft = Some("literal draft".into());
        metadata.name = Some("Named".into());
        metadata.pinned = true;
    });
    let prompt = SavedPrompt {
        id: 7,
        text: "pending".into(),
        images: vec![image("A".repeat(600_000), "image/png")],
        attachments: vec![SavedAttachment::Connection("host".into())],
    };
    let id = chat.id().to_owned();
    crate::chats::save_shared(
        dir.path(),
        &SharedChat::new(chat, vec![std::sync::Arc::new(prompt)]).unwrap(),
    )
    .unwrap();
    assert!(load_catalog(dir.path())[0].has_content);
    let loaded = load(dir.path(), &id, &authority).unwrap();
    assert_eq!(loaded.composer().draft.as_deref(), Some("literal draft"));
    assert_eq!(loaded.composer().queue[0].id, 7);
    assert_eq!(loaded.composer().queue[0].attachments.len(), 1);
    assert!(
        matches!(&loaded.composer().queue[0].images[0],Content::Media(image) if image.encoded.len()==600_000)
    );
}
#[test]
fn malformed_collection_and_nested_amplification_are_rejected_without_erasing_files() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = chat(&authority);
    let path = file(dir.path(), chat.id()).unwrap();
    let mut oversized: serde_json::Value =
        serde_json::from_slice(&crate::legacy::fixture_bytes(&chat)).unwrap();
    oversized["entries"] =
        serde_json::json!(vec![serde_json::json!({"kind":"agent","value":""}); 2001]);
    fs::write(&path, serde_json::to_vec(&oversized).unwrap()).unwrap();
    assert!(load(dir.path(), chat.id(), &authority).is_err());
    let chat = edited(chat, |_, _, transcript, _| {
        transcript.entries = vec![tool(
            &authority,
            serde_json::json!({"tiny":vec![0;140_000]}),
        )]
    });
    fs::write(&path, crate::legacy::fixture_bytes(&chat)).unwrap();
    assert!(load(dir.path(), chat.id(), &authority).is_err());
    assert!(crate::chats::save(dir.path(), &chat).is_err());
    assert!(nocterm_ai::history::prepare(chat).is_err());
    assert!(path.exists());
}
