//! Checked version-two CPU format; storage privately owns import and legacy publication.
use super::{
    FORMAT, HistoryCodecError, HistoryEncoding, HistoryOrigin, HistoryView, LegacySizingError,
    PreparedBudget, PreparedHistory, SavedChat, SharedChat, VERSION, check_memory, check_size,
    preflight, size, wire,
};
use crate::model::{
    decode::{Seed, field, missing, seeded_struct, unknown},
    retention::RetentionAuthority,
};
use serde::de::DeserializeSeed as _;

struct Envelope {
    format: String,
    version: u32,
    document: SavedChat,
}
seeded_struct!(Envelope{format:String => plain required,version:u32 => plain required,document:SavedChat => owned required};deny_unknown);
pub fn structural_bytes(bytes: &[u8]) -> Result<usize, HistoryCodecError> {
    preflight::check(bytes)
}
pub fn decode(
    authority: &RetentionAuthority,
    bytes: &[u8],
    id: &str,
) -> Result<SavedChat, HistoryCodecError> {
    validate_id(id)?;
    check_size(bytes.len())?;
    let structural = preflight::check(bytes)?;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let envelope = Seed::<Envelope>::new(authority).deserialize(&mut decoder)?;
    decoder.end()?;
    if envelope.format != FORMAT {
        return Err(HistoryCodecError::FormatMismatch);
    }
    if envelope.version != VERSION {
        return Err(HistoryCodecError::UnsupportedVersion(envelope.version));
    }
    if envelope.document.id() != id {
        return Err(HistoryCodecError::IdentityMismatch);
    }
    prepare_origin(envelope.document, HistoryOrigin::Version2, None, structural)
}
pub fn prepare(chat: SavedChat) -> Result<SavedChat, HistoryCodecError> {
    let origin = chat.origin();
    let grammar = chat.prepared().and_then(PreparedHistory::legacy).cloned();
    prepare_origin(chat, origin, grammar, 0)
}
/// Private storage supplies only a pure historical sizing capability and source bound.
pub fn prepare_imported(
    chat: SavedChat,
    grammar: super::retention::LegacyFormat,
    source_resident: usize,
) -> Result<SavedChat, HistoryCodecError> {
    prepare_origin(
        chat,
        HistoryOrigin::LegacyV1,
        Some(grammar),
        source_resident,
    )
}
pub(super) fn prepare_origin(
    chat: SavedChat,
    origin: HistoryOrigin,
    grammar: Option<super::retention::LegacyFormat>,
    source_resident: usize,
) -> Result<SavedChat, HistoryCodecError> {
    let scalars = PreparedBudget::new(&chat, grammar.as_ref())?;
    finish_prepared(chat, origin, grammar, source_resident, scalars)
}
/// Native operation admission supplies the same real construction ledger used
/// by its parser, final DTO owners and retained caches. This is not a substitute
/// for the document scope's common concurrent phase bank.
pub(super) fn prepare_in_ledger(
    chat: SavedChat,
    origin: HistoryOrigin,
    grammar: Option<super::retention::LegacyFormat>,
    source_resident: usize,
    ledger: &crate::model::json::deferred::DecodeLedger,
) -> Result<SavedChat, HistoryCodecError> {
    if !chat.retention().same(ledger.authority()) {
        return Err(HistoryCodecError::Retention(
            nocterm_core::SharedError::from_error(
                crate::model::retention::RetentionError::ForeignAuthority,
            ),
        ));
    }
    chat.retained_owner_bytes().map_err(|error| {
        HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
    })?;
    if let Some(grammar) = &grammar {
        let source = grammar
            .retention()
            .ok_or(crate::model::retention::RetentionError::MissingAuthority)
            .map_err(|error| {
                HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
            })?;
        crate::model::retention::graph::measure(chat.retention(), source).map_err(|error| {
            HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
        })?;
    }
    let scalars = PreparedBudget::new_in_ledger(&chat, grammar.as_ref(), ledger)?;
    finish_prepared(chat, origin, grammar, source_resident, scalars)
}
fn finish_prepared(
    mut chat: SavedChat,
    origin: HistoryOrigin,
    grammar: Option<super::retention::LegacyFormat>,
    source_resident: usize,
    mut scalars: PreparedBudget,
) -> Result<SavedChat, HistoryCodecError> {
    scalars.resident_bytes = scalars.resident_bytes.max(source_resident);
    if let Some(grammar) = &grammar {
        scalars.resident_bytes = scalars
            .resident_bytes
            .checked_add(grammar.retained_heap_bytes().map_err(|error| match error {
                LegacySizingError::Json(error) => error.into(),
                LegacySizingError::Formatting(error) => HistoryCodecError::Formatting(error),
                LegacySizingError::UnsupportedRecord(reason) => {
                    HistoryCodecError::Compatibility(reason)
                }
                LegacySizingError::ArithmeticOverflow => HistoryCodecError::ArithmeticOverflow,
                LegacySizingError::Retention(error) => {
                    HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
                }
            })?)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    }
    check_memory(scalars.resident_bytes)?;
    chat.attach_prepared(PreparedHistory::new(scalars, origin, grammar)?);
    let actual = chat.retained_owner_bytes().map_err(|error| {
        HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
    })?;
    check_memory(actual)?;
    chat.retain_at_least(actual);
    // Measurement/representation errors propagate; format eligibility alone may fall back.
    let decision = decide(chat.view()?, chat.prepared(), chat.resident_bytes())?;
    chat.set_decision(decision)?;
    Ok(chat)
}
pub(super) fn legacy_size(
    result: Result<super::FormatSize, LegacySizingError>,
) -> Result<super::FormatSize, HistoryCodecError> {
    match result {
        Ok(size) => Ok(size),
        Err(LegacySizingError::UnsupportedRecord(reason)) => {
            Err(HistoryCodecError::Compatibility(reason))
        }
        Err(LegacySizingError::Json(error)) => Err(error.into()),
        Err(LegacySizingError::Formatting(error)) => Err(HistoryCodecError::Formatting(error)),
        Err(LegacySizingError::ArithmeticOverflow) => Err(HistoryCodecError::ArithmeticOverflow),
        Err(LegacySizingError::Retention(error)) => Err(HistoryCodecError::Retention(
            nocterm_core::SharedError::from_error(error),
        )),
    }
}
pub(super) fn decide(
    view: HistoryView<'_>,
    context: Option<&PreparedHistory>,
    resident: usize,
) -> Result<super::EncodingDecision, HistoryCodecError> {
    let origin = context.map_or(HistoryOrigin::New, PreparedHistory::origin);
    super::select_encoding(
        origin,
        resident,
        size::measure(&wire::Wire::new(view)),
        || {
            let grammar = context
                .and_then(PreparedHistory::legacy)
                .ok_or(HistoryCodecError::CandidateMismatch)?;
            legacy_size(grammar.measure(view))
        },
    )
}
pub fn decision_shared(chat: &SharedChat) -> Result<super::EncodingDecision, HistoryCodecError> {
    let mut resident =
        size::retained(chat.snapshot())?.max(super::allocations::chat(chat.snapshot())?);
    for prompt in chat.prompts() {
        resident = resident
            .checked_add(
                size::retained(prompt.as_ref())?.max(super::allocations::prompt(prompt.as_ref())?),
            )
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    }
    resident = resident.max(chat.snapshot().resident_bytes());
    decide(chat.view()?, chat.snapshot().prepared(), resident)
}
/// A borrowed CPU-only version-two projection for a selected immutable candidate.
/// Native adapters choose their own serializer sink; domain owns no writer or I/O.
pub fn projection_shared(
    chat: &SharedChat,
    decision: super::EncodingDecision,
) -> Result<impl serde::Serialize + '_, HistoryCodecError> {
    if decision.encoding != HistoryEncoding::Version2 {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    verify_selected(chat, decision)?;
    Ok(wire::Wire::new(chat.view()?))
}
/// This explicit serializer emits only v2; native storage performs selected legacy encoding.
pub fn encode(chat: &SavedChat) -> Result<Vec<u8>, HistoryCodecError> {
    encode_view(chat.view()?)
}
pub fn encode_shared(chat: &SharedChat) -> Result<Vec<u8>, HistoryCodecError> {
    encode_view(chat.view()?)
}
fn encode_view(view: HistoryView<'_>) -> Result<Vec<u8>, HistoryCodecError> {
    let expected = size::measure(&wire::Wire::new(view))?;
    check_size(expected.encoded_bytes)?;
    check_memory(expected.structural_bytes)?;
    let bytes = serde_json::to_vec(&wire::Wire::new(view))?;
    if bytes.len() != expected.encoded_bytes
        || preflight::check(&bytes)? != expected.structural_bytes
    {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    Ok(bytes)
}
pub fn verify_selected(
    chat: &SharedChat,
    decision: super::EncodingDecision,
) -> Result<(), HistoryCodecError> {
    if decision_shared(chat)? != decision {
        return Err(HistoryCodecError::CandidateMismatch);
    }
    if decision.encoding == HistoryEncoding::Version2 {
        let actual = size::measure(&wire::Wire::new(chat.view()?))?;
        if actual != decision.size {
            return Err(HistoryCodecError::CandidateMismatch);
        }
    }
    Ok(())
}
fn validate_id(id: &str) -> Result<(), HistoryCodecError> {
    if uuid::Uuid::parse_str(id).is_err() {
        Err(HistoryCodecError::InvalidId(id.into()))
    } else {
        Ok(())
    }
}
