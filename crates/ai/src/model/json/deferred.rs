//! Deferred reserved raw JSON. Native files and legacy field rules stay in adapters.
//!
//! This seam controls its own allocations. Stock parser scratch is NOT yet included
//! in the ledger, so this module does not establish a complete decoder peak bound.
//!
//! DeferredJson keeps legacy SDK first-key RawValue expansion. In contrast,
//! literal::LiteralBuilder constructs ordinary literal JSON after version-two
//! format selection: even that reserved key is data. The builder validates event
//! order and ownership; adapters remain responsible for authenticated stock lexical
//! validation and current-stream allocation guards before supplying events.
mod chunks;
pub(crate) mod ledger;
pub mod literal;
pub mod native;
mod nodes;
mod pending;
#[cfg(test)]
mod tests;
mod validation;
mod visitor;
pub use ledger::{DecodeLedger, NativeAllocationError};
use std::collections::TryReserveError;
#[derive(Debug, thiserror::Error)]
pub enum JsonDecodeError {
    #[error("JSON decode arithmetic overflow")]
    Arithmetic,
    #[error("JSON decode requests {requested} bytes with {available} available")]
    Limit { requested: usize, available: usize },
    #[error("JSON decode allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("JSON map allocation failed: {0}")]
    MapAllocation(#[from] indexmap::TryReserveError),
    #[error("JSON document admission failed: {0}")]
    Retention(#[source] crate::model::retention::RetentionError),
    #[error("JSON allocation differs from the pinned capacity profile")]
    CapacityProfile,
    #[error("JSON reservations belong to different construction ledgers")]
    ForeignLedger,
    #[error(
        "JSON text length differs from its controlled allocation: expected {expected}, actual {actual}"
    )]
    TextLength { expected: usize, actual: usize },
    #[error("JSON text plan does not match its controlled lexical facts")]
    TextPlan,
    #[error("JSON cleanup frontier exceeds its pre-admitted workspace")]
    CleanupFrontier,
    #[error("JSON literal construction order is invalid")]
    ConstructionOrder,
    #[error("Remote diagnostic text exceeds {limit} Unicode scalars")]
    RemoteTextLimit { limit: usize },
    #[error("JSON syntax or semantic conversion failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// A transient pure JSON value. Drop the enclosing stock decoder before materialize.
#[derive(Debug)]
pub struct DeferredJson {
    root: nodes::Node,
    pending: pending::Pending,
}
/// Immutable result retains its construction reservation while the operation lives.
#[derive(Debug)]
pub struct DecodedJson {
    value: super::OwnedJson,
}
impl DecodedJson {
    pub(crate) fn into_owned(
        self,
        ledger: &DecodeLedger,
    ) -> Result<super::OwnedJson, JsonDecodeError> {
        if !self.value.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        Ok(self.value)
    }
    pub fn as_owned(&self) -> &super::OwnedJson {
        &self.value
    }
}
impl DeferredJson {
    pub fn from_deserializer<'de, D>(
        decoder: D,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError>
    where
        D: serde::Deserializer<'de, Error = serde_json::Error>,
    {
        let mut failure = None;
        let mut pending = pending::Pending::new(ledger)?;
        let parsed = serde::de::DeserializeSeed::deserialize(
            visitor::Seed {
                ledger,
                failure: &mut failure,
                pending: &mut pending,
            },
            decoder,
        );
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(Self {
            root: parsed?,
            pending,
        })
    }
    /// Convenience for an already owned raw JSON text, never a native file read.
    /// Parser scratch remains outside the current ledger until its reader guard lands.
    pub fn from_owned_text(input: String, ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        let reader = chunks::OwnedChunks::new(input, ledger)?;
        let mut decoder = serde_json::Deserializer::from_reader(reader);
        let parsed = Self::from_deserializer(&mut decoder, ledger)?;
        decoder.end()?;
        drop(decoder);
        Ok(parsed)
    }
    pub fn materialize(self, ledger: &DecodeLedger) -> Result<DecodedJson, JsonDecodeError> {
        if !self.root.allocation.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        // Required raw errors survive duplicate replacement. Legacy optional-field
        // defaulting/row skipping must wrap this entire operation in the adapter.
        self.pending.validate(ledger)?;
        let built = self.root.build(ledger)?;
        // Legacy node construction remains recursive/uncontrolled for cleanup;
        // its real allocation credit nevertheless follows the final shared owner.
        let guarded = super::lifecycle::GuardedValue::new(built.value, 1, None, built.allocation);
        let value = super::OwnedJson::from_admitted(guarded, ledger)?;
        Ok(DecodedJson { value })
    }
}

mod scratch;
pub use scratch::NativeScratch;
