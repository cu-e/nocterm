//! Small authenticated delivery outcomes, independent from hub transport ACKs.
use super::{CommandAcknowledgement, ReceivedCommand};
use crate::authority::StreamBinding;
use crate::journal::CommandReceipt;
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;

/// Retains the real native generation through durable admission. A source must
/// reserve its actual bytes before decoding or materializing this command.
pub type ResidentCommand = Arc<Allocated<ReceivedCommand>>;
#[derive(Debug)]
pub enum CommandInput {
    Ready(ResidentCommand),
    /// Cryptographically checked small coordinates whose body could not receive
    /// an ordinary allocation. No payload allocation or final inbox ACK occurred.
    Deferred {
        command: Box<CommandAcknowledgement>,
        reason: RetryReason,
        allocation: AllocationLease,
    },
    /// A chat key for this runner was durably admitted (or already installed).
    /// The runner opens the chat's stream so its writer, head and catalog row
    /// exist. This is not a command: it has no acceptance, acknowledgement or
    /// control position, and repeating it changes nothing.
    ChatOpened {
        stream: StreamBinding,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryReason {
    Capacity,
    NotCurrent,
    Unavailable,
    NeedsGrant,
    NeedsUnlock,
    NeedsProviderAccess,
    Stopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRefusal {
    InvalidCommand,
    Conflict,
    Expired,
    Unsupported,
}
/// No error chains, provider response bodies or command payloads enter the ACK
/// pump. Accepted contains the actual immutable native journal receipt.
#[derive(Clone, Debug)]
pub enum CommandReply {
    Accepted(Box<CommandReceipt>),
    /// An informational semantic refusal, never proof of durable admission or
    /// durable rejection. The producer needs a separately durable rejection or
    /// an explicit authenticated safe-discard policy before final hub ACK.
    Refused(CommandRefusal),
    /// The original inbox command remains unacknowledged and the same ID must
    /// be revisited. Scan cursor advancement is not durable ACK advancement.
    RetryLater(RetryReason),
}
