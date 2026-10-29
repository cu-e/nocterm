//! Bounded immutable payload values with content-free diagnostics.
use nocterm_ai::history::{HistoryCodecError, json_size};
use serde_json::Value;
use std::{fmt, sync::Arc};

pub const MAX_LOGICAL_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = MAX_LOGICAL_PAYLOAD_BYTES + 78;
pub const MAX_PROGRESS_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_RESULT_BYTES: usize = nocterm_ai::MAX_MCP_LINE;

#[derive(Clone, PartialEq, Eq)]
pub struct OpaqueBytes(Arc<[u8]>);
impl OpaqueBytes {
    /// The transport/decoder must check advertised lengths before allocating bytes.
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Result<Self, PayloadError> {
        let bytes = bytes.into();
        require_size(bytes.len(), MAX_ENVELOPE_BYTES)?;
        Ok(Self(bytes))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl fmt::Debug for OpaqueBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OpaqueBytes").field(&self.len()).finish()
    }
}

#[derive(Clone)]
pub struct BoundedJson {
    value: Arc<Value>,
    encoded_bytes: usize,
}
impl BoundedJson {
    /// Reuses AI's exact compact serializer sizing, including escaped characters.
    pub fn new(value: Value) -> Result<Self, PayloadError> {
        let encoded_bytes = json_size(&value)?;
        require_size(encoded_bytes, MAX_TOOL_RESULT_BYTES)?;
        Ok(Self {
            value: Arc::new(value),
            encoded_bytes,
        })
    }
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}
impl fmt::Debug for BoundedJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedJson")
            .field("encoded_bytes", &self.encoded_bytes)
            .finish()
    }
}

pub(crate) fn require_size(actual: usize, limit: usize) -> Result<(), PayloadError> {
    if actual > limit {
        Err(PayloadError::Capacity { actual, limit })
    } else {
        Ok(())
    }
}

#[derive(Clone, thiserror::Error)]
pub enum PayloadError {
    #[error("payload needs {actual} bytes; the limit is {limit}")]
    Capacity { actual: usize, limit: usize },
    #[error("the payload is empty or malformed")]
    Invalid,
    #[error("the payload could not be sized")]
    Json(#[from] HistoryCodecError),
}
impl fmt::Debug for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity { actual, limit } => f
                .debug_struct("Capacity")
                .field("actual", actual)
                .field("limit", limit)
                .finish(),
            Self::Invalid => f.write_str("Invalid"),
            Self::Json(_) => f.write_str("Json"),
        }
    }
}
