//! Private historical grammar; no SDK type is exported by storage or domain ports.
mod borrowed;
mod controlled;
mod managed;
#[cfg(feature = "test-support")]
pub mod test_support;
pub(crate) use controlled::{
    ControlledFieldOrder, ControlledGrammar, ControlledRowOrder, controlled_field_order,
    controlled_literal, controlled_order,
};
mod catalog;
#[cfg(any(test, feature = "test-support"))]
mod content;
mod content_borrowed;
#[cfg(any(test, feature = "test-support"))]
mod content_decode;
mod display;
mod entries;
mod identity;
mod order;
mod prompts;
#[cfg(any(test, feature = "test-support"))]
mod resource;
#[cfg(any(test, feature = "test-support"))]
mod tools;
mod tools_borrowed;
#[cfg(any(test, feature = "test-support"))]
mod tools_decode;
#[cfg(any(test, feature = "test-support"))]
mod wire;
pub(super) use catalog::decode_summary;

#[cfg(any(test, feature = "test-support"))]
use nocterm_ai::history::SavedChat;
use nocterm_ai::history::{
    EncodingDecision, HistoryCodecError, HistoryEncoding, HistoryView, LegacySizing,
    LegacySizingError,
};

/// Pure import prepares immutable legacy origin. It admits no write or revision.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn decode(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    bytes: &[u8],
    id: &str,
) -> Result<SavedChat, HistoryCodecError> {
    if bytes.len() > nocterm_ai::history::MAX_FILE_BYTES as usize {
        return Err(HistoryCodecError::EncodedLimit);
    }
    let resident = nocterm_ai::history::structural_bytes(bytes)?;
    let (chat, grammar) = wire::decode(bytes, id)?.into_owned_ordered(authority)?;
    nocterm_ai::history::prepare_imported(chat, grammar.retained(authority), resident)
}
fn map_error(error: LegacySizingError) -> HistoryCodecError {
    match error {
        LegacySizingError::UnsupportedRecord(reason) => HistoryCodecError::Compatibility(reason),
        LegacySizingError::Json(error) => error.into(),
        LegacySizingError::Formatting(error) => HistoryCodecError::Formatting(error),
        LegacySizingError::ArithmeticOverflow => HistoryCodecError::ArithmeticOverflow,
        LegacySizingError::Retention(error) => {
            HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
        }
    }
}
/// Borrow the exact private grammar after checking the selected fallback context.
pub(super) fn projection<'a>(
    view: HistoryView<'a>,
    decision: EncodingDecision,
    provider: &'a dyn LegacySizing,
) -> Result<impl crate::writing::managed_writer::ManagedWire + 'a, HistoryCodecError> {
    if decision.encoding != HistoryEncoding::LegacyV1 {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    let grammar = nocterm_ai::history::retention::legacy_value::<borrowed::Grammar>(provider)
        .ok_or(HistoryCodecError::CandidateMismatch)?;
    let measured = grammar.measure(view).map_err(map_error)?;
    if measured != decision.size {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    if measured.encoded_bytes > nocterm_ai::history::MAX_FILE_BYTES as usize {
        return Err(HistoryCodecError::EncodedLimit);
    }
    if measured.structural_bytes > nocterm_ai::history::MAX_RESIDENT_BYTES {
        return Err(HistoryCodecError::StructuralLimit);
    }
    Ok(borrowed::Borrowed(view, grammar))
}

/// Version-two restart also derives inert display facts from literal fields;
/// no transient authority evidence is restored from a persisted document.
#[cfg(test)]
pub(super) fn describe_row(
    call: &nocterm_ai::model::tools::ToolRecord,
) -> nocterm_ai::model::literal::LiteralEvidence {
    identity::literal(
        call.name.as_deref(),
        Some(&call.title),
        call.arguments.as_ref().map(|value| value.as_value()),
    )
}

#[cfg(test)]
pub(crate) fn fixture_bytes(chat: &SavedChat) -> Vec<u8> {
    let grammar = borrowed::Grammar::new(chat.retention());
    serde_json::to_vec(&borrowed::Borrowed(chat.view().unwrap(), &grammar)).unwrap()
}

/// The selected historical projection borrows the registered physical owners.
pub(super) fn projection_managed<'a>(
    snapshot: &'a nocterm_ai::history::physical::SnapshotOwner,
    decision: EncodingDecision,
    provider: &'a dyn LegacySizing,
) -> Result<impl crate::writing::managed_writer::ManagedWire + 'a, HistoryCodecError> {
    if decision.encoding != HistoryEncoding::LegacyV1 {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    let grammar = nocterm_ai::history::retention::legacy_value::<borrowed::Grammar>(provider)
        .ok_or(HistoryCodecError::CandidateMismatch)?;
    if grammar.managed_measure(snapshot).map_err(map_error)? != decision.size {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    Ok(managed::Document(snapshot, grammar))
}
