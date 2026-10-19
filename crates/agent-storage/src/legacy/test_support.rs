//! Adapter-owned fixture entry points into the actual historical grammar.
use nocterm_ai::{
    history::{HistoryCodecError, SavedChat},
    model::{retention::RetentionAuthority, tools::ToolRecord},
};
use serde::Deserialize;

/// Decodes historical tool bytes without restoring execution claims.
/// This presentation helper does not prove history peak or sidecar accounting.
pub fn decode_legacy_tool_fixture(
    authority: &RetentionAuthority,
    bytes: &[u8],
) -> Result<ToolRecord, HistoryCodecError> {
    let call = serde_json::from_slice::<super::tools_decode::Call>(bytes)?;
    Ok(super::tools::moving_ordered(authority, call)?.0)
}

#[derive(Deserialize)]
struct Header {
    id: String,
}

/// Uses the document's embedded identity and the actual historical decoder.
pub fn decode_legacy_history_fixture(
    authority: &RetentionAuthority,
    bytes: &[u8],
) -> Result<SavedChat, HistoryCodecError> {
    let header: Header = serde_json::from_slice(bytes)?;
    super::decode(authority, bytes, &header.id)
}
