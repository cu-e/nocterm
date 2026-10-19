//! Large legacy resources load without rewriting the stored file.
use super::*;

#[test]
fn public_repository_accepts_prior_text_and_large_embedded_resource_without_eager_write() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use futures::executor::block_on;
    use nocterm_ai::{
        history::{ArchivedDraftPatch, ChatWrite},
        model::content::{Content, ResourceBody},
    };
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let id = "a289c6ce-7f10-41dc-b5e5-564e2c7ca0df";
    let path = directory.path().join(format!("{id}.json"));
    let mut file = std::fs::File::create(&path).unwrap();
    write!(file, r#"{{"version":1,"id":"{id}","agent_id":"provider","updated":1,"entries":[{{"kind":"agent","value":""#).unwrap();
    let write_literal = |file: &mut std::fs::File, byte: u8, length: usize| {
        let chunk = [byte; 4096];
        for _ in 0..length / chunk.len() {
            file.write_all(&chunk).unwrap();
        }
        file.write_all(&chunk[..length % chunk.len()]).unwrap();
    };
    write_literal(&mut file, b'a', 14 * 1024 * 1024);
    file.write_all(
        br#""},{"kind":"content","value":{"type":"resource","resource":{"uri":"x","text":""#,
    )
    .unwrap();
    write_literal(&mut file, b'r', 17 * 1024 * 1024);
    file.write_all(br#""}}}]}"#).unwrap();
    drop(file);
    let original = std::fs::read(&path).unwrap();
    assert!(original.len() < MAX_FILE_BYTES as usize);
    assert!(
        nocterm_ai::history::structural_bytes(&original).unwrap()
            < nocterm_ai::history::MAX_RESIDENT_BYTES
    );
    // Allocation phase measurement must not include an extra full fixture copy.
    use std::hash::Hasher;
    let mut before = std::collections::hash_map::DefaultHasher::new();
    before.write(&original);
    let original_digest = before.finish();
    drop(original);
    let repository =
        FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
    let loaded = block_on(repository.load(id.into())).unwrap();
    let Entry::Agent(previous) = &loaded.transcript().entries[0] else {
        panic!("previous agent row")
    };
    assert_eq!(previous.len(), 14 * 1024 * 1024);
    assert!(previous.bytes().all(|byte| byte == b'a'));
    let Entry::Content(Content::EmbeddedResource(resource)) = &loaded.transcript().entries[1]
    else {
        panic!("embedded resource")
    };
    let ResourceBody::Text(text) = &resource.body else {
        panic!("original Text-first branch")
    };
    assert_eq!(text.text.len(), 17 * 1024 * 1024);
    assert!(text.text.bytes().all(|byte| byte == b'r'));
    block_on(
        repository.save(Arc::new(ChatWrite::ArchiveDraft(ArchivedDraftPatch {
            id: id.into(),
            agent_id: "provider".into(),
            text: None,
            updated: 1,
        }))),
    )
    .unwrap();
    use std::io::Read;
    let mut after = std::collections::hash_map::DefaultHasher::new();
    let mut file = std::fs::File::open(path).unwrap();
    let mut chunk = [0; 4096];
    loop {
        let length = file.read(&mut chunk).unwrap();
        if length == 0 {
            break;
        }
        after.write(&chunk[..length]);
    }
    assert_eq!(after.finish(), original_digest);
    // Acceptance/public-origin parity only; allocator peak proof is separate.
}
