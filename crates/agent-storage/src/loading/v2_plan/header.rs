//! Header fields are replayed/authenticated without materializing document JSON.
use super::{Key, V2Plan};
use crate::loading::{
    held_file::{HeldFile, HeldReadError, plan::AuthenticatedPlan},
    string_reader::{JsonStringReader, StringReadError},
};
use nocterm_ai::history::{FORMAT, HistoryCodecError, MAX_FILE_BYTES, VERSION};
use nocterm_core::SharedError;
#[derive(Debug, thiserror::Error)]
pub(in crate::loading) enum HeaderError {
    #[error("Version-two header fields are missing from this scope")]
    Shape,
    #[error("Version-two header replay failed: {0}")]
    Held(#[from] HeldReadError),
    #[error("Version-two format marker failed: {0}")]
    String(#[from] StringReadError),
    #[error("Version-two version number failed: {0}")]
    Json(#[source] SharedError<serde_json::Error>),
    #[error("Version-two header rejected: {0}")]
    Codec(#[from] HistoryCodecError),
}
/// Only a format/version check. Framed document values and ignored fields still
/// need their own syntax/semantic validation before an actual route can adopt.
pub(in crate::loading) fn check(
    plan: &AuthenticatedPlan<V2Plan>,
    held: &mut HeldFile,
) -> Result<(), HeaderError> {
    let format_span = plan
        .payload()
        .field(Key::Format)
        .ok_or(HeaderError::Shape)?
        .native_span();
    let version_span = plan
        .payload()
        .field(Key::Version)
        .ok_or(HeaderError::Shape)?
        .native_span();
    let marker = plan.replay(
        held,
        |_| format_span,
        |reader| {
            let mut string = JsonStringReader::new(reader, MAX_FILE_BYTES as usize)?;
            let mut index = 0usize;
            let mut equal = true;
            while let Some(fragment) = string.next_fragment()? {
                for byte in fragment.as_bytes() {
                    equal &= FORMAT.as_bytes().get(index) == Some(byte);
                    index += 1;
                }
            }
            Ok::<_, StringReadError>(equal && index == FORMAT.len())
        },
    )??;
    if !marker {
        return Err(HistoryCodecError::FormatMismatch.into());
    }
    let version = plan
        .replay(
            held,
            |_| version_span,
            |reader| serde_json::from_reader::<_, u32>(reader),
        )?
        .map_err(|error| HeaderError::Json(SharedError::from_error(error)))?;
    if version != VERSION {
        return Err(HistoryCodecError::UnsupportedVersion(version).into());
    }
    Ok(())
}
