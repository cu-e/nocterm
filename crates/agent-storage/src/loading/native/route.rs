//! Native format fields retain occurrence identity before any JSON map collapse.
mod stock;
use crate::{
    format::Format,
    loading::{
        held_file::{HeldFile, HeldReadError, plan::AuthenticatedPlan},
        json_fields::Key,
        json_frames::{FrameError, Member, ObjectFrames, RootFrames},
        string_reader::{JsonStringReader, StringReadError},
    },
};
use nocterm_ai::history::{FORMAT, HistoryCodecError, MAX_FILE_BYTES};
use std::io::{Cursor, Read};

#[derive(Debug, thiserror::Error)]
pub(in crate::loading) enum RouteError {
    #[error("Native saved-chat format framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("Native saved-chat format replay failed: {0}")]
    Held(#[from] HeldReadError),
    #[error("Native saved-chat format string failed: {0}")]
    String(#[from] StringReadError),
    #[error("Native saved-chat format version failed: {0}")]
    ManagedJson(#[source] nocterm_ai::model::failure::ManagedNativeFailure),
    #[error("Native saved-chat format allocation failed: {0}")]
    Admission(#[source] nocterm_ai::model::json::deferred::NativeAllocationError),
    #[error("Native saved-chat format source has an original guarded failure")]
    ManagedSource,
    #[error("Saved chat repeats format field {0:?}")]
    Duplicate(Key),
    #[error("Saved chat has no version field")]
    MissingVersion,
    #[error("Saved chat format field is neither a string nor null")]
    FormatType,
    #[error(transparent)]
    Codec(#[from] HistoryCodecError),
}
/// This scan frames fields; it does not validate ignored values or admit a document.
/// Complete syntax/schema validation remains mandatory on the authenticated source.
pub(in crate::loading) struct RoutePlan {
    version: Option<Member>,
    marker: Option<Member>,
}
impl RoutePlan {
    pub(in crate::loading) fn scan(
        source: &mut dyn Read,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> std::result::Result<Self, RouteError> {
        let mut frames = ObjectFrames::new_literal_managed(source, 0)?;
        let mut result = Self {
            version: None,
            marker: None,
        };
        while let Some(member) = frames.next_member_with_ledger(Some(ledger))? {
            let slot = match member.kind() {
                Key::Version => &mut result.version,
                Key::Format => &mut result.marker,
                _ => continue,
            };
            if slot.is_some() {
                return Err(RouteError::Duplicate(member.kind()));
            }
            *slot = Some(member);
        }
        RootFrames::Object(frames).finish()?;
        if result.version.is_none() {
            return Err(RouteError::MissingVersion);
        }
        Ok(result)
    }
    pub(in crate::loading) fn classify(
        plan: &AuthenticatedPlan<Self>,
        held: &mut HeldFile,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> std::result::Result<Format, RouteError> {
        let version = plan
            .payload()
            .version
            .as_ref()
            .ok_or(RouteError::MissingVersion)?
            .native_span();
        let encoded_bytes = version.value_bytes();
        let version = plan.replay(
            held,
            |_| version,
            |reader| stock::version(reader, encoded_bytes, ledger),
        )??;
        let marker = match plan.payload().marker.as_ref() {
            Some(field) => Some(plan.replay(
                held,
                |_| field.native_span(),
                |reader| marker(reader, ledger),
            )??),
            None => None,
        };
        match (version, marker) {
            (1, None) => Ok(Format::LegacyV1),
            (2, Some(Some(true))) => Ok(Format::Version2),
            (version, None | Some(Some(true))) => {
                Err(HistoryCodecError::UnsupportedVersion(version).into())
            }
            _ => Err(HistoryCodecError::FormatMismatch.into()),
        }
    }
}
fn marker(
    source: &mut dyn Read,
    ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
) -> std::result::Result<Option<bool>, RouteError> {
    let mut first = [0];
    source
        .read_exact(&mut first)
        .map_err(|_| RouteError::ManagedSource)?;
    let source = Cursor::new(first).chain(source);
    if first[0] == b'n' {
        stock::null(source, ledger)?;
        return Ok(None);
    }
    if first[0] != b'"' {
        return Err(RouteError::FormatType);
    }
    let mut cursor = JsonStringReader::new_managed(source, MAX_FILE_BYTES as usize, ledger)?;
    let mut index = 0usize;
    let mut same = true;
    while let Some(fragment) = cursor.next_fragment()? {
        for byte in fragment.as_bytes() {
            same &= FORMAT.as_bytes().get(index) == Some(byte);
            index = index
                .checked_add(1)
                .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        }
    }
    Ok(Some(same && index == FORMAT.len()))
}
