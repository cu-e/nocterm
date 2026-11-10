//! Prepared native history keeps its original bytes and decoded allocation together.
use futures::executor::block_on;
use nocterm_agent_storage::FileChatRepository;
use nocterm_ai::{
    history::{self, ContextId, SavedChat, SharedChat},
    storage::{AgentStorageError, ChatRepository, PreparedChatBytes, PreparedChatLoad},
    testing::{InMemoryAgentRepository, StorageOperation},
};
use nocterm_core::allocation::AllocationLease;
use nocterm_session::testing::InMemoryFs;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const ID: &str = "00000000-0000-4000-8000-0000000000a1";

fn chat(draft: &str) -> SavedChat {
    let mut chat = SavedChat::for_identity("agent".into(), ID.into(), 1);
    chat.draft = Some(draft.into());
    chat
}

fn prepared() -> Box<dyn PreparedChatLoad> {
    Box::new(
        PreparedChatBytes::new(
            history::encode(&chat("original")).unwrap(),
            ContextId::General.into(),
            ID.into(),
            Some(2),
        )
        .unwrap(),
    )
}

#[test]
fn prepared_decode_refuses_insufficient_bytes_and_missing_payload_slot() {
    let source = prepared();
    let bytes = source.resident_bytes();
    assert!(bytes > 0);
    let result = block_on(source.decode(AllocationLease::new(bytes - 1, 1, ())));
    assert!(matches!(
        result,
        Err(AgentStorageError::ResidentCapacity { .. })
    ));
    let source = prepared();
    let bytes = source.resident_bytes();
    let result = block_on(source.decode(AllocationLease::new(bytes, 0, ())));
    assert!(matches!(result, Err(AgentStorageError::PayloadCapacity)));
}

struct Release(Arc<AtomicUsize>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn detached_decode_and_returned_generation_retain_the_same_allocation() {
    let source = prepared();
    let released = Arc::new(AtomicUsize::new(0));
    let bytes = source.resident_bytes();
    let decode = source.decode(AllocationLease::new(bytes, 1, Release(released.clone())));
    assert_eq!(released.load(Ordering::SeqCst), 0);
    let generation = std::thread::spawn(move || Arc::new(block_on(decode).unwrap()))
        .join()
        .unwrap();
    assert_eq!(generation.value().draft.as_deref(), Some("original"));
    assert_eq!(generation.allocation().bytes(), bytes);
    assert_eq!(released.load(Ordering::SeqCst), 0);
    drop(generation);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn native_prepared_history_survives_delete_and_replacement_without_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let filesystem = Arc::new(InMemoryFs::default());
    for ancestor in directory
        .path()
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        filesystem.seed_native_directory(ancestor).unwrap();
    }
    let repository = FileChatRepository::new(directory.path().into(), filesystem);
    let original = SharedChat {
        metadata: chat("original"),
        queue: Vec::new(),
    };
    block_on(repository.save(Arc::new(original))).unwrap();
    let source =
        block_on(repository.prepare_load_in(ContextId::General.into(), ID.into())).unwrap();
    block_on(repository.delete(ContextId::General, ID.into())).unwrap();
    let replacement = SharedChat {
        metadata: chat("replacement"),
        queue: Vec::new(),
    };
    block_on(repository.save(Arc::new(replacement))).unwrap();
    let bytes = source.resident_bytes();
    let loaded = block_on(source.decode(AllocationLease::new(bytes, 1, ()))).unwrap();
    assert_eq!(loaded.value().draft.as_deref(), Some("original"));
    let current = block_on(repository.load(ContextId::General, ID.into())).unwrap();
    assert_eq!(current.draft.as_deref(), Some("replacement"));
}

#[test]
fn shared_repository_preparation_uses_existing_load_failures_and_frozen_bytes() {
    let repository = InMemoryAgentRepository::default();
    repository.seed_chat(&chat("original")).unwrap();
    repository.fail_next(
        StorageOperation::Load(ID.into()),
        AgentStorageError::Interrupted,
    );
    assert!(matches!(
        block_on(repository.prepare_load_in(ContextId::General.into(), ID.into())),
        Err(AgentStorageError::Interrupted)
    ));
    let source =
        block_on(repository.prepare_load_in(ContextId::General.into(), ID.into())).unwrap();
    repository.seed_chat(&chat("replacement")).unwrap();
    let bytes = source.resident_bytes();
    let loaded = block_on(source.decode(AllocationLease::new(bytes, 1, ()))).unwrap();
    assert_eq!(loaded.value().draft.as_deref(), Some("original"));
    assert_eq!(
        repository.operations(),
        vec![
            StorageOperation::Load(ID.into()),
            StorageOperation::Load(ID.into()),
            StorageOperation::Decoded(ID.into()),
        ]
    );
}
