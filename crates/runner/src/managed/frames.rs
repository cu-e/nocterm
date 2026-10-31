//! The framed standard input of `managed --control stdin` (ADR 0123): one boot
//! v2 frame, then contract `ChatAccessFrame`s until the host closes the pipe.
use super::{BootV2, ChatTokens, MAX_BOOT_BYTES};
use crate::RunnerError;
use nocterm_cloud_model::identity::CloudChatId;
use nocterm_core::SharedCause;
use nocterm_hub_io::runner_control::{ChatAccessDecoder, ChatAccessFrame};
use std::{io::Read, sync::Arc};
use tokio::sync::oneshot;
use zeroize::Zeroizing;

/// Bytes read from the pipe at once; a frame is at most 256, so one read never
/// holds more than a couple of frames.
const READ_BYTES: usize = 512;

/// Why the control reader stopped.
#[derive(Debug)]
pub(crate) enum ControlEnd {
    /// The host closed its end: the lease is over and the runner should stop.
    Eof,
    /// The stream failed or lost its framing. Tokens are revoked and no new one
    /// is accepted, but the runner keeps serving what it has.
    Refused(RunnerError),
}

/// Reads frame 1: a u32 big-endian length, then that many bytes of boot v2 JSON.
pub(crate) fn read_boot_v2(input: &mut impl Read) -> Result<BootV2, RunnerError> {
    let boot = |error| RunnerError::Boot(SharedCause::new(error));
    let mut prefix = [0u8; 4];
    input.read_exact(&mut prefix).map_err(boot)?;
    let length = usize::try_from(u32::from_be_bytes(prefix)).unwrap_or(usize::MAX);
    if length == 0 || length > MAX_BOOT_BYTES {
        return Err(RunnerError::Value("boot document size"));
    }
    let mut body = Zeroizing::new(vec![0u8; length]);
    input.read_exact(&mut body).map_err(boot)?;
    BootV2::parse(&body)
}

fn apply(tokens: &ChatTokens, frame: ChatAccessFrame) {
    let chat = CloudChatId::from_bytes(frame.chat());
    match frame {
        ChatAccessFrame::Issue { token, .. } => {
            // A token the engine refuses leaves the chat without access.
            if tokens.issue(chat, &token).is_err() {
                tokens.revoke(chat);
            }
        }
        ChatAccessFrame::Revoke { .. } => tokens.revoke(chat),
    }
}

/// Applies frames until the input ends or fails, then revokes every token.
fn pump(mut input: impl Read, tokens: &ChatTokens) -> ControlEnd {
    let mut decoder = ChatAccessDecoder::new();
    let mut buffer = Zeroizing::new([0u8; READ_BYTES]);
    let end = 'read: loop {
        let count = match input.read(&mut buffer[..]) {
            Ok(0) => break ControlEnd::Eof,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => break ControlEnd::Refused(RunnerError::Control(error)),
        };
        let mut rest = &buffer[..count];
        while !rest.is_empty() {
            match decoder.feed(rest) {
                Ok((frame, tail)) => {
                    rest = tail;
                    if let Some(frame) = frame {
                        apply(tokens, frame);
                    }
                }
                Err(error) => break 'read ControlEnd::Refused(RunnerError::ControlChannel(error)),
            }
        }
    };
    tokens.close();
    end
}

/// Reads the control channel on its own thread. The receiver completes once,
/// after every token was revoked.
pub(crate) fn spawn_reader(
    input: impl Read + Send + 'static,
    tokens: Arc<ChatTokens>,
) -> Result<oneshot::Receiver<ControlEnd>, RunnerError> {
    let (ended, closed) = oneshot::channel();
    std::thread::Builder::new()
        .name("runner-control".into())
        .spawn(move || {
            let end = pump(input, &tokens);
            let _ = ended.send(end);
        })
        .map_err(RunnerError::Control)?;
    Ok(closed)
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
