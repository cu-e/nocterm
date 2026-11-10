//! CPU-only versioned transcript encoding and bounded loading.
use super::{
    HistoryCodecError, PreparedBudget, SavedChat, SharedChat, check_size, preflight, wire,
};

pub fn structural_bytes(bytes: &[u8]) -> Result<usize, HistoryCodecError> {
    preflight::check(bytes)
}
pub fn decode(bytes: &[u8], id: &str) -> Result<SavedChat, HistoryCodecError> {
    validate_id(id)?;
    check_size(bytes.len())?;
    let resident = preflight::check(bytes)?;
    let mut chat = super::incoming::decode_bytes(bytes, id)?;
    if chat.header.id != id {
        return Err(HistoryCodecError::IdentityMismatch);
    }
    let mut prepared = PreparedBudget::new(&chat)?;
    prepared.resident_bytes = resident;
    chat.prepared = Some(prepared);
    Ok(chat)
}
/// Validate an already decoded construction input without trimming its contents.
/// Unlike the persistence view, this retains every entry and full image payload.
pub fn prepare(chat: SavedChat) -> Result<SavedChat, HistoryCodecError> {
    validate_id(&chat.header.id)?;
    super::header::resolve_scope(chat.version, Some(chat.header.scope))?;
    super::header::validate_environment(chat.version, &chat.header.environment)?;
    if chat.archived_draft {
        return Err(HistoryCodecError::ArchivedPatch);
    }
    chat.validate_collections()?;
    if chat.entries.len() > super::MAX_SAVED_ENTRIES || chat.times.len() > super::MAX_SAVED_ENTRIES
    {
        return Err(HistoryCodecError::CollectionLimit);
    }
    let bytes = serde_json::to_vec(&chat)?;
    decode(&bytes, &chat.header.id)
}
/// Validate and prepare the same owned decoded body. This avoids serializing
/// and decoding a second full transcript merely to create its cached budgets.
pub fn prepare_owned(mut chat: SavedChat) -> Result<SavedChat, HistoryCodecError> {
    validate_id(&chat.header.id)?;
    super::header::resolve_scope(chat.version, Some(chat.header.scope))?;
    super::header::validate_environment(chat.version, &chat.header.environment)?;
    if chat.archived_draft {
        return Err(HistoryCodecError::ArchivedPatch);
    }
    chat.validate_collections()?;
    if chat.entries.len() > super::MAX_SAVED_ENTRIES || chat.times.len() > super::MAX_SAVED_ENTRIES
    {
        return Err(HistoryCodecError::CollectionLimit);
    }
    let resident = {
        let value = wire::Wire::complete(&chat);
        // Preserve decode's public refusal ordering: encoded limit before the
        // structural limit, using the existing allocation-free counters.
        check_size(super::size::encoded_len(&value).map_err(HistoryCodecError::from)?)?;
        let resident = super::size::structural_size(&value).map_err(HistoryCodecError::from)?;
        super::check_memory(resident)?;
        resident
    };
    super::CatalogHeader::from_chat(&chat, false).validate(&chat.header.id)?;
    let mut prepared = PreparedBudget::new(&chat)?;
    prepared.resident_bytes = resident;
    chat.prepared = Some(prepared);
    Ok(chat)
}

pub fn encode(chat: &SavedChat) -> Result<Vec<u8>, HistoryCodecError> {
    if chat.archived_draft {
        return Err(HistoryCodecError::ArchivedPatch);
    }
    chat.validate_collections()?;
    encode_value(
        chat,
        &wire::Wire::new(chat, (!chat.queue.is_empty()).then_some(&chat.queue)),
    )
}
pub fn encode_shared(chat: &SharedChat) -> Result<Vec<u8>, HistoryCodecError> {
    if chat.metadata.archived_draft {
        return Err(HistoryCodecError::ArchivedPatch);
    }
    if chat.queue.len() > 1024
        || chat
            .queue
            .iter()
            .any(|p| p.images.len() > 128 || p.attachments.len() > 1024)
    {
        return Err(HistoryCodecError::CollectionLimit);
    }
    chat.metadata.validate_collections()?;
    let prompts = wire::Prompts(&chat.queue);
    encode_value(
        &chat.metadata,
        &wire::Wire::new(&chat.metadata, (!chat.queue.is_empty()).then_some(&prompts)),
    )
}
fn encode_value(
    chat: &SavedChat,
    value: &impl serde::Serialize,
) -> Result<Vec<u8>, HistoryCodecError> {
    validate_id(&chat.header.id)?;
    super::header::resolve_scope(chat.version, Some(chat.header.scope))?;
    super::header::validate_environment(chat.version, &chat.header.environment)?;
    let bytes = serde_json::to_vec(value)?;
    check_size(bytes.len())?;
    preflight::check(&bytes)?;
    Ok(bytes)
}
pub(super) fn validate_id(id: &str) -> Result<(), HistoryCodecError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(HistoryCodecError::InvalidId(id.into()));
    }
    Ok(())
}
