//! Validate all occurrences, then construct only surviving semantic values.
mod array_plan;
mod frames;
mod map_plan;
mod materialize;
mod pump;
mod source;
mod validate;
use crate::loading::{
    held_file::{
        HeldFile, HeldReadError,
        plan::{AuthenticatedPlan, NativeSpan},
    },
    literal_reader::LiteralReadError,
};
use nocterm_ai::model::json::deferred::{DecodeLedger, JsonDecodeError, native::NativeNode};
#[derive(Debug, thiserror::Error)]
pub(in crate::loading) enum ValueError {
    #[error("Legacy JSON literal failed: {0}")]
    Literal(#[from] LiteralReadError),
    #[error("Legacy JSON allocation failed: {0}")]
    Build(#[from] JsonDecodeError),
    #[error("Legacy JSON framing failed: {0}")]
    Frame(#[from] crate::loading::json_frames::FrameError),
    #[error("Legacy JSON source replay failed: {0}")]
    Held(#[source] HeldReadError),
    #[error("Legacy JSON span does not belong to its active source")]
    ForeignSource,
    #[error("Legacy JSON nesting exceeds the original parser budget")]
    Depth,
    #[error("Legacy RawValue has the wrong representation")]
    RawShape,
    #[error("Legacy JSON has invalid punctuation")]
    Punctuation,
}
impl ValueError {
    pub(super) fn defaultable(&self) -> bool {
        match self {
            Self::Depth | Self::RawShape | Self::Punctuation => true,
            Self::Literal(
                LiteralReadError::ManagedJson(_)
                | LiteralReadError::Token
                | LiteralReadError::SourceDepth,
            ) => true,
            Self::Literal(LiteralReadError::String(error)) => !matches!(
                error,
                crate::loading::string_reader::StringReadError::ManagedSource
                    | crate::loading::string_reader::StringReadError::Admission(_)
                    | crate::loading::string_reader::StringReadError::Source(_)
                    | crate::loading::string_reader::StringReadError::EncodedLimit
                    | crate::loading::string_reader::StringReadError::PartialFragment
            ),
            _ => false,
        }
    }
}

/// Original per-invocation physical recursion budget of the pinned parser.
const ORIGINAL_DEPTH: usize = 127;
pub(super) fn authenticated<T>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
    map: bool,
) -> std::result::Result<std::result::Result<NativeNode, ValueError>, HeldReadError> {
    read(parent, held, select, ledger, map, Some(ORIGINAL_DEPTH))
}
/// Ordinary v2 values may nest legacy RawValue expansions beyond one stock
/// parser budget; the iterative reader keeps no physical recursion cap there.
pub(super) fn version2<T>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
) -> std::result::Result<std::result::Result<NativeNode, ValueError>, HeldReadError> {
    read(parent, held, select, ledger, false, None)
}
fn read<T>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    select: impl Fn(&T) -> NativeSpan,
    ledger: &DecodeLedger,
    map: bool,
    limit: Option<usize>,
) -> std::result::Result<std::result::Result<NativeNode, ValueError>, HeldReadError> {
    let physical = select(parent.payload());
    let root = source::SemanticSpan::held(physical.clone(), map);
    // The outer replay authenticates even when semantic validation fails.
    let validated = parent.replay(
        held,
        |_| physical,
        |reader| validate::read(reader, ledger, root, limit),
    )?;
    let result = validated.and_then(|value| materialize::build(parent, held, value, ledger));
    match result {
        Err(ValueError::Held(source)) => Err(source),
        result => Ok(result),
    }
}
