//! Retained immutable writer snapshots include their separate shared queue payloads.
use nocterm_ai::history::{SavedChat, SavedPrompt, SharedChat};
use std::sync::Arc;

#[test]
fn a_large_chat_reopens_with_every_saved_row_and_timestamp() {
    use nocterm_ai::{history, thread::Entry};
    let mut chat = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-0000-0000-000000000002".into(),
        1,
    );
    chat.entries = (0..20_000)
        .map(|index| Entry::Agent(format!("answer {index}")))
        .collect();
    chat.times = (1..=20_000).collect();
    chat.entries[19_000] = Entry::Agent("x".repeat(33 * 1024 * 1024));
    let bytes = history::encode(&chat).unwrap();
    let restored = history::decode(&bytes, &chat.header.id).unwrap();
    assert_eq!(restored.entries.len(), 20_000);
    assert!(
        restored
            .entries
            .iter()
            .zip(&chat.entries)
            .all(|(actual, expected)| {
                matches!((actual, expected), (Entry::Agent(a), Entry::Agent(e)) if a == e)
            })
    );
    assert_eq!(restored.times, chat.times);
}

#[test]
fn retained_snapshot_memory_counts_transcript_and_shared_queue_without_encoding() {
    let mut metadata = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-0000-0000-000000000001".into(),
        1,
    );
    metadata
        .entries
        .push(nocterm_ai::thread::Entry::Agent("a".repeat(1024 * 1024)));
    let body = SharedChat {
        metadata,
        queue: Vec::new(),
    };
    let body_bytes = body.retained_bytes().unwrap();
    assert!(body_bytes >= 1024 * 1024);
    let shared = SharedChat {
        metadata: body.metadata,
        queue: vec![Arc::new(SavedPrompt {
            id: 1,
            text: "q".repeat(1024 * 1024),
            images: Vec::new(),
            attachments: Vec::new(),
        })],
    };
    assert!(shared.retained_bytes().unwrap() >= body_bytes + 1024 * 1024);
}
