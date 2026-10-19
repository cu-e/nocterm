//! Controlled schema materialization from one authenticated held document.
mod content;
mod document;
mod failure_capture;
mod fields;
mod legacy;
pub(super) mod legacy_value;
mod lists;
mod loading;
pub(crate) use loading::materialize;
#[cfg(test)]
pub(crate) use loading::read_request;
mod route;
pub(in crate::loading) mod schema;
mod tools;
use crate::loading::{
    held_file::HeldReadError, json_fields::Key, json_frames::FrameError,
    literal_reader::LiteralReadError,
};
use nocterm_ai::model::json::deferred::JsonDecodeError;
#[derive(Debug, thiserror::Error)]
pub(in crate::loading) enum Error {
    #[error("Native semantic field was consumed or borrowed twice")]
    Consumed,
    #[error("Native history route failed: {0}")]
    Route(#[from] route::RouteError),
    #[error("Legacy value semantics failed: {0}")]
    LegacyValue(#[from] legacy_value::ValueError),
    #[error("Native history framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("Native history replay failed: {0}")]
    Held(#[from] HeldReadError),
    #[error("Native history literal failed: {0}")]
    Literal(#[from] LiteralReadError),
    #[error("Native history construction failed: {0}")]
    Build(#[from] JsonDecodeError),
    #[error("Native history repeats field {0:?}")]
    Duplicate(Key),
    #[error("Native history misses field {0:?}")]
    Missing(Key),
    #[error("Native history contains an unknown field")]
    UnknownField,
    #[error("Native history has an invalid positional field count")]
    Positional,
    #[error("Native history has the wrong scalar or container type")]
    Type,
    #[error("Native history contains an unknown variant")]
    Variant,
    #[error("Native history collection exceeds its element bound")]
    Collection,
    #[error("Native history allocation failed: {0}")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error(transparent)]
    Codec(#[from] nocterm_ai::history::HistoryCodecError),
}

impl Error {
    fn defaultable(&self) -> bool {
        match self {
            Self::Type
            | Self::Variant
            | Self::Missing(_)
            | Self::Duplicate(_)
            | Self::Positional
            | Self::UnknownField => true,
            Self::LegacyValue(error) => error.defaultable(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
