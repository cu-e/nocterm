//! Carries a byte stream over a relay's bounded frame channels.
//!
//! The transport hands the SSH handshake an ordinary async stream. Behind it, two
//! pumps move the bytes between a [`tokio::io::duplex`] pipe and the relay's
//! channels, in a frame size the relay accepts.
//!
//! Nothing here grows without bound: a pipe holds [`PIPE_BYTES`], each channel
//! queues [`nocterm_session::RELAY_FRAMES`] frames, and a pump holds one frame, so a
//! relay that stops reading stalls the sender's writes instead of buffering them.
//!
//! The two directions end independently. Shutting the stream's write side ends the
//! upload after what is queued, and the relay closing its sender ends the
//! download with end of file. Dropping the stream, or the relay dropping a channel,
//! ends both: the pumps stop and release every channel end they hold.

use std::{
    io,
    pin::{Pin, pin},
    task::{Context, Poll},
};

use nocterm_session::{RELAY_FRAME_BYTES, RelayReceiver, RelaySender, RelayStream};
use tokio::{
    io::{
        AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream, ReadBuf,
        ReadHalf, WriteHalf, duplex, split,
    },
    sync::watch,
};

/// Bytes the pipe between the handshake and the pumps holds: two frames.
const PIPE_BYTES: usize = 2 * RELAY_FRAME_BYTES;

/// The stream the SSH handshake runs over.
///
/// Dropping it tells the pumps to stop, even while they wait on the relay.
pub(super) struct RelayIo {
    pipe: DuplexStream,
    _open: watch::Sender<()>,
}

/// How a pump's direction ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The direction reached its end; the other may go on.
    Finished,
    /// The other end of this direction is gone, so the whole stream is.
    Broken,
}

/// Starts the pumps for `stream` and returns the handshake's end of the pipe.
///
/// Must run inside a Tokio runtime, which then runs the pumps.
pub(super) fn bridge(stream: RelayStream) -> RelayIo {
    let (pipe, pumped) = duplex(PIPE_BYTES);
    let (open, watching) = watch::channel(());
    let (to_relay, from_relay) = stream.into_parts();
    tokio::spawn(pump(pumped, to_relay, from_relay, watching));
    RelayIo { pipe, _open: open }
}

async fn pump(
    pumped: DuplexStream,
    to_relay: RelaySender,
    from_relay: RelayReceiver,
    mut watching: watch::Receiver<()>,
) {
    let (reader, writer) = split(pumped);
    let mut upload = pin!(upload(reader, to_relay));
    let mut download = pin!(download(from_relay, writer));
    let (mut uploading, mut downloading) = (true, true);
    while uploading || downloading {
        tokio::select! {
            // Only a dropped stream wakes this: nothing is ever sent on the channel.
            _ = watching.changed() => return,
            ended = &mut upload, if uploading => {
                uploading = false;
                if ended == Ended::Broken {
                    return;
                }
            }
            ended = &mut download, if downloading => {
                downloading = false;
                if ended == Ended::Broken {
                    return;
                }
            }
        }
    }
}

/// Moves what the handshake writes to the relay, one frame at a time.
async fn upload(mut reader: ReadHalf<DuplexStream>, to_relay: RelaySender) -> Ended {
    let mut buffer = vec![0; RELAY_FRAME_BYTES];
    loop {
        match reader.read(&mut buffer).await {
            // The handshake shut its write side (or the stream is gone): end of file for the relay.
            Ok(0) | Err(_) => return Ended::Finished,
            Ok(count) => {
                if to_relay.send(buffer[..count].to_vec()).await.is_err() {
                    return Ended::Broken;
                }
            }
        }
    }
}

/// Moves what the relay sends to the handshake.
async fn download(from_relay: RelayReceiver, mut writer: WriteHalf<DuplexStream>) -> Ended {
    while let Ok(frame) = from_relay.recv().await {
        if writer.write_all(&frame).await.is_err() {
            return Ended::Broken;
        }
    }
    // The relay closed its sender: the handshake sees end of file after the queued bytes.
    let _ = writer.shutdown().await;
    Ended::Finished
}

impl AsyncRead for RelayIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_read(cx, buf)
    }
}

impl AsyncWrite for RelayIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.pipe).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
