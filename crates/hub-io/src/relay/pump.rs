//! Carries frames between the transport's bounded channels and the hub's relay stream.
//!
//! Every link of the chain is bounded, so a side that stops reading stops the other instead
//! of making this side buffer:
//!
//! - towards the hub, the transport's channel queues [`nocterm_session::RELAY_FRAMES`] frames,
//!   the pump holds one, the hub client queues its own bounded number and then the HTTP/2
//!   window stops the rest;
//! - towards the transport, the HTTP/2 window stops the hub while the pump holds one frame
//!   and the transport's channel queues [`nocterm_session::RELAY_FRAMES`].
//!
//! The directions end independently. The transport dropping its sender is an end of file for
//! the target, which may still answer; the target closing is an end of file for the transport,
//! which may still send. A failure of the hub's stream or a dropped channel ends both: the
//! pump returns and releases every end it holds, which aborts the hub's stream and tells the
//! transport through its channels.

use std::future::Future;
use std::pin::pin;

use nocterm_hub_client::{RelayError, RelayReader, RelayWriter};
use nocterm_session::{RELAY_FRAME_BYTES, RelayPeer, RelayReceiver, RelaySender};

/// The way towards the target: what the transport sends.
pub(super) trait Uplink: Send {
    /// Waits while the hub still holds as many frames as it may, then queues `data`.
    fn send(&mut self, data: &[u8]) -> impl Future<Output = Result<(), RelayError>> + Send;
    /// Tells the target that nothing more follows; it may still answer.
    fn finish(self) -> impl Future<Output = ()> + Send;
}

/// The way back: what the target sends.
pub(super) trait Downlink: Send {
    /// The next bytes of the target; `Ok(None)` when it closed cleanly.
    fn next(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, RelayError>> + Send;
}

impl Uplink for RelayWriter {
    async fn send(&mut self, data: &[u8]) -> Result<(), RelayError> {
        RelayWriter::send(self, data).await
    }

    async fn finish(self) {
        // The stream is going away either way; there is nobody to tell.
        let _ = RelayWriter::finish(self).await;
    }
}

impl Downlink for RelayReader {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, RelayError> {
        self.recv().await
    }
}

/// How a direction ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The direction reached its end; the other may go on.
    Finished,
    /// The stream or the other end of this direction is gone, so the whole relay is.
    Broken,
}

/// Moves frames both ways until both directions end or one breaks.
pub(super) async fn carry<U: Uplink, D: Downlink>(peer: RelayPeer, up: U, down: D) {
    let (from_transport, to_transport) = peer.into_parts();
    let mut upload = pin!(upload(from_transport, up));
    let mut download = pin!(download(down, to_transport));
    let (mut uploading, mut downloading) = (true, true);
    while uploading || downloading {
        let ended = tokio::select! {
            ended = &mut upload, if uploading => {
                uploading = false;
                ended
            }
            ended = &mut download, if downloading => {
                downloading = false;
                ended
            }
        };
        if ended == Ended::Broken {
            return;
        }
    }
}

/// Moves what the transport sends to the hub, one frame at a time.
async fn upload<U: Uplink>(from_transport: RelayReceiver, mut up: U) -> Ended {
    while let Ok(frame) = from_transport.recv().await {
        if let Err(error) = up.send(&frame).await {
            tracing::debug!(%error, "the relay stream refused data");
            return Ended::Broken;
        }
    }
    // The transport dropped its sender: end of file for the target, after what was queued.
    up.finish().await;
    Ended::Finished
}

/// Moves what the target sends to the transport, in frames the transport accepts.
///
/// Ends as soon as the transport has dropped its receiver, even while the target is silent.
async fn download<D: Downlink>(mut down: D, to_transport: RelaySender) -> Ended {
    loop {
        let next = tokio::select! {
            next = down.next() => next,
            () = to_transport.closed() => return Ended::Broken,
        };
        match next {
            Ok(Some(data)) => {
                for frame in data.chunks(RELAY_FRAME_BYTES) {
                    if to_transport.send(frame.to_vec()).await.is_err() {
                        return Ended::Broken;
                    }
                }
            }
            // Dropping the sender is the end of file for the transport.
            Ok(None) => return Ended::Finished,
            Err(error) => {
                tracing::debug!(%error, "the relay stream ended");
                return Ended::Broken;
            }
        }
    }
}

#[cfg(test)]
mod tests;
