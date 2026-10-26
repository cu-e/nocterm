//! A canonical bounded prefix can be acquired without reading transcript payloads.
use super::{CatalogHeader, HistoryCodecError};
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub(super) const START: &[u8] = b"{\"version\":2,\"catalog\":";
pub(super) const PAYLOAD: &[u8] = b",\"chat\":";

/// Validate only the authoritative v2 prefix. Full hydration additionally checks
/// that the catalog projection agrees with the decoded body.
pub fn decode_catalog_header(bytes: &[u8], id: &str) -> Result<CatalogHeader, HistoryCodecError> {
    let prefix = bytes
        .get(..bytes.len().min(MAX_HEADER_BYTES))
        .unwrap_or_default();
    let input = prefix
        .strip_prefix(START)
        .ok_or(HistoryCodecError::InvalidHeader)?;
    let mut stream = serde_json::Deserializer::from_slice(input).into_iter::<CatalogHeader>();
    let header = stream
        .next()
        .ok_or(HistoryCodecError::InvalidHeader)?
        .map_err(|error| {
            if error.is_eof() {
                HistoryCodecError::HeaderLimit
            } else {
                error.into()
            }
        })?;
    let end = stream.byte_offset();
    if !input
        .get(end..)
        .is_some_and(|tail| tail.starts_with(PAYLOAD))
        || serde_json::to_vec(&header)? != input[..end]
    {
        return Err(HistoryCodecError::InvalidHeader);
    }
    header.validate(id)?;
    Ok(header)
}
