//! The first route step through a relay: a byte stream the application is handed
//! instead of a socket.
//!
//! The transport asks a [`RelayDialer`] for a stream to a [`Target`] and runs its
//! own SSH handshake over it, so host-key checks, jump hosts and sign-in stay with
//! the transport. The relay sees an address and the bytes that cross it, nothing
//! more. The hub that implements the port is named by an opaque string; no hub type
//! appears here.
//!
//! A [`RelayStream`] is two bounded channels of frames, one for each direction. The
//! bound is the back-pressure: a side that stops reading makes the other side's
//! sends wait instead of buffering. A stream has no other state: dropping a
//! sender ends that direction after the queued frames, dropping a receiver makes
//! the sender's next send fail, and dropping a whole end closes both directions.

use std::{fmt, sync::Arc};

use async_channel::{Receiver, Sender, bounded};
use futures::future::BoxFuture;
use nocterm_core::SharedCause;

use crate::Target;

/// How many frames one direction of a stream queues before its sender waits.
pub const RELAY_FRAMES: usize = 16;
/// The largest frame either end sends: a longer write is split into several frames.
pub const RELAY_FRAME_BYTES: usize = 32 * 1024;

/// Frames travelling to the relay.
pub type RelaySender = Sender<Vec<u8>>;
/// Frames arriving from the relay.
pub type RelayReceiver = Receiver<Vec<u8>>;

/// Opens relayed streams. Implemented by the application over its hub connections.
pub trait RelayDialer: Send + Sync {
    /// Captures the actual requester before a transport spawns background work.
    /// An implementation must retain that requester and exact route, never
    /// resolve a later account selection from the supplied strings.
    fn capture(
        &self,
        _hub: &str,
        _node: Option<&str>,
        _target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        Err(RelayError::CaptureUnsupported)
    }

    /// Asks the hub known as `hub` (and its node `node`, if named) for a stream to `target`.
    ///
    /// Dropping the returned future abandons the request; a stream already opened
    /// closes when its ends are dropped.
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>>;
}

/// The transport's end of a relayed connection.
pub struct RelayStream {
    tx: RelaySender,
    rx: RelayReceiver,
}

/// The relay's end of a [`RelayStream`]: what the transport sends arrives on
/// `rx`, and what is sent on `tx` reaches the transport.
pub struct RelayPeer {
    rx: RelayReceiver,
    tx: RelaySender,
}

impl RelayStream {
    /// Creates the two ends of a connection with the shared bounds.
    pub fn pair() -> (RelayStream, RelayPeer) {
        let (to_relay, from_transport) = bounded(RELAY_FRAMES);
        let (to_transport, from_relay) = bounded(RELAY_FRAMES);
        (
            RelayStream {
                tx: to_relay,
                rx: from_relay,
            },
            RelayPeer {
                rx: from_transport,
                tx: to_transport,
            },
        )
    }

    /// Splits the stream into what is sent to the relay and what it sends back.
    pub fn into_parts(self) -> (RelaySender, RelayReceiver) {
        (self.tx, self.rx)
    }
}

impl RelayPeer {
    /// Splits the end into what the transport sent and what is sent back to it.
    pub fn into_parts(self) -> (RelayReceiver, RelaySender) {
        (self.rx, self.tx)
    }
}

impl fmt::Debug for RelayStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayStream").finish_non_exhaustive()
    }
}

impl fmt::Debug for RelayPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayPeer").finish_non_exhaustive()
    }
}

/// Why a relay declined to connect, as the relay states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RelayRefusal {
    /// The relay's rules do not allow this destination.
    #[error("the relay's policy does not allow this destination")]
    Policy,
    /// The destination is on a private network the relay does not reach.
    #[error("the relay does not reach private networks")]
    PrivateNetwork,
    /// The destination port is not allowed.
    #[error("the relay does not allow this port")]
    Port,
    /// The account has reached a relay limit.
    #[error("a relay limit was reached")]
    Limit,
}

/// Why a relayed stream could not be opened or ended.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RelayError {
    #[error("this relay does not support captured requesters")]
    CaptureUnsupported,
    /// The relay hub is not one the user is signed in to.
    #[error("not signed in to the hub {hub}")]
    NotSignedIn { hub: String },
    #[error("the hub refused the connection: {reason}")]
    Refused { reason: RelayRefusal },
    /// The named node of the hub is not connected.
    #[error("the hub node {node} is not available")]
    NodeUnavailable { node: String },
    /// The relay could not reach the destination, or lost the hub on the way.
    #[error("the hub could not reach the host: {0}")]
    Unreachable(#[source] SharedCause),
    /// The relay stopped before the stream was open.
    #[error("the relay closed")]
    Closed,
}

impl PartialEq for RelayError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotSignedIn { hub: a }, Self::NotSignedIn { hub: b }) => a == b,
            (Self::Refused { reason: a }, Self::Refused { reason: b }) => a == b,
            (Self::NodeUnavailable { node: a }, Self::NodeUnavailable { node: b }) => a == b,
            // A cause carries no identity of its own; its text is what the user sees.
            (Self::Unreachable(a), Self::Unreachable(b)) => a.to_string() == b.to_string(),
            (Self::Closed, Self::Closed) | (Self::CaptureUnsupported, Self::CaptureUnsupported) => {
                true
            }
            _ => false,
        }
    }
}

impl Eq for RelayError {}

#[cfg(test)]
mod tests;
