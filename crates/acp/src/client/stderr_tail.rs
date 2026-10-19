//! Bounded tail of an agent's standard error, kept for exit diagnostics.
use std::sync::{Arc, Mutex};

use futures::{AsyncRead, AsyncReadExt as _};

use super::{error::StartupError, remote_errors::scrub};

/// Only the most recent bytes of stderr are retained.
const LIMIT: usize = 64 * 1024;

#[derive(Clone, Default)]
pub(super) struct StderrTail(Arc<Mutex<Vec<u8>>>);
impl StderrTail {
    /// Drains `stderr` on a dedicated thread until the stream ends.
    pub(super) fn capture(
        stderr: impl AsyncRead + Unpin + Send + 'static,
    ) -> Result<Self, StartupError> {
        let tail = Self::default();
        let writer = tail.clone();
        std::thread::Builder::new()
            .name("nocterm-agent-stderr".into())
            .spawn(move || futures::executor::block_on(writer.pump(stderr)))
            .map_err(StartupError::ThreadSpawn)?;
        Ok(tail)
    }
    async fn pump(&self, mut stderr: impl AsyncRead + Unpin) {
        let mut buffer = [0u8; 4096];
        while let Ok(count) = stderr.read(&mut buffer).await {
            if count == 0 {
                break;
            }
            let mut tail = self.0.lock().unwrap_or_else(|e| e.into_inner());
            tail.extend_from_slice(&buffer[..count]);
            let excess = tail.len().saturating_sub(LIMIT);
            tail.drain(..excess);
        }
    }
    /// The retained text with configured secrets and credentials removed.
    pub(super) fn scrubbed(&self, secrets: &[String]) -> String {
        let bytes = self.0.lock().unwrap_or_else(|e| e.into_inner());
        scrub(&String::from_utf8_lossy(&bytes), secrets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pumped(input: Vec<u8>) -> StderrTail {
        let tail = StderrTail::default();
        futures::executor::block_on(tail.pump(futures::io::Cursor::new(input)));
        tail
    }

    #[test]
    fn only_the_last_sixty_four_kibibytes_are_kept() {
        let mut input = vec![b'a'; 100 * 1024];
        input.extend_from_slice(b"final line");
        let text = pumped(input).scrubbed(&[]);
        assert_eq!(text.len(), 64 * 1024);
        assert!(text.ends_with("final line"));
        assert!(text.starts_with('a'));
    }

    #[test]
    fn configured_secrets_never_appear_in_the_tail() {
        let tail = pumped(b"failed with hunter2-secret token\n".to_vec());
        let text = tail.scrubbed(&["hunter2-secret".into()]);
        assert!(!text.contains("hunter2-secret"));
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn a_silent_agent_has_an_empty_tail() {
        assert_eq!(pumped(Vec::new()).scrubbed(&[]), "");
    }
}
