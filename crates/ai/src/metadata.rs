//! Native facts for chat identity allocation and accepted durable transitions.
//!
//! Callers supply these values to pure documents and reducers. Reading or framing
//! an imported document requires neither allocation nor a clock query.

/// Values allocated once when a genuinely new chat or fork is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatIdentity {
    pub id: String,
    pub created: u64,
}

/// The consumer's native identity and wall-clock capabilities, supplied by the host.
pub trait ChatMetadataSource: Send + Sync {
    fn new_identity(&self) -> ChatIdentity;
    fn timestamp(&self) -> u64;
}
