//! Retained immutable writer snapshots include their separate shared queue payloads.
use nocterm_ai::history::{SavedChat, SavedPrompt, SharedChat};
use std::sync::Arc;

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
