//! Explicitly owned raw transport components for controlled native codecs.
//!
//! Unlike the typed connection, these components never parse with serde or
//! materialize a JSON value. Every SDK allocation first obtains a credit from
//! the supplied budget. A native codec retains the source and supplies validated
//! routing facts; payload bytes remain in their original source allocation.
//!
//! These primitives are additive. The ordinary typed connection and its wire
//! grammar are unchanged.
mod budget;
mod buffer;
mod cancellation;
mod connection;
mod frame;
mod index;
mod map;
mod queue;
mod receive;
mod reply;
mod respond;
mod shared;
mod source;

pub use budget::{AllocationKind, Budget, ManagedError};
pub use buffer::{Buffer, Values};
pub use cancellation::{Cancellation, Cancelled};
pub use connection::{Connection, Driver, Outgoing};
pub use frame::{Entry, Envelope, Frame, Malformed, MessageId, MessageKind, Payload};
pub use index::Index;
pub use receive::{Dispatch, Reception};
pub use reply::{Request, Response};
pub use respond::{Attempt, Fault, Reply, Responder};
pub use shared::{Shared, Weak};
pub use source::{Source, Span};

#[cfg(test)]
mod tests;
