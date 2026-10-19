//! Fallible allocation approval, explicit in each physical owner.
use std::{alloc::Layout, fmt};

/// The allocation's actual responsibility, for diagnostics and native policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationKind {
    /// Original UTF-8 line or serialized output backing.
    Source,
    /// A collection's replacement backing, while the old backing is alive.
    Collection,
    /// A genuine shared allocation, including its strong and weak counters.
    Shared,
    /// A queued request, reply or task owner.
    Operation,
}

/// A native allocation authority explicitly supplied to the SDK.
///
/// `Credit` must be an owned, non-cloneable reservation whose destructor refunds
/// the approved layout. Its own storage must already be paid. Implementations
/// must not allocate an unaccounted boxed token in `reserve`. No callback is
/// invoked from a census. Budget clones must alias their original paid owner
/// without allocating; approval must not reenter the SDK routing owners.
pub trait Budget: Clone {
    /// Concrete reservation type; no type-erasing allocation is required.
    type Credit;
    /// Native refusal preserving the original typed cause.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Compare the genuine original operation/account identity. Implementations
    /// must compare their private context identity without allocating or renewing
    /// a capability; equal numeric document IDs are not ownership evidence.
    fn same_source(&self, other: &Self) -> bool;

    /// Reserve the actual requested layout before any physical allocation.
    fn reserve(&self, layout: Layout, kind: AllocationKind) -> Result<Self::Credit, Self::Error>;
}

/// A managed operation failed before publishing its replacement storage.
#[derive(Debug)]
pub enum ManagedError<E> {
    /// The native authority refused the actual requested layout.
    Refused(E),
    /// An allocation layout or collection length overflowed.
    CapacityOverflow,
    /// The system allocator could not provide approved storage.
    AllocationFailed,
    /// A native reader supplied bytes that are not UTF-8.
    InvalidUtf8,
    /// A source span is outside its original source or splits a UTF-8 character.
    InvalidSpan,
    /// A validated JSON-RPC envelope has inconsistent routing facts.
    InvalidEnvelope,
    /// The original request or connection has already closed.
    Closed,
    /// A response does not belong to this original request.
    ForeignResponse,
}
impl<E: fmt::Display> fmt::Display for ManagedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => write!(f, "managed allocation refused: {error}"),
            Self::CapacityOverflow => f.write_str("managed allocation capacity overflow"),
            Self::AllocationFailed => f.write_str("managed system allocation failed"),
            Self::InvalidUtf8 => f.write_str("managed source is not UTF-8"),
            Self::InvalidSpan => f.write_str("managed source span is invalid"),
            Self::InvalidEnvelope => f.write_str("managed routing envelope is invalid"),
            Self::Closed => f.write_str("managed request is closed"),
            Self::ForeignResponse => f.write_str("managed response belongs to another request"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ManagedError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(error) => Some(error),
            _ => None,
        }
    }
}
