//! Read-only bounded observation of the unencrypted initial KEX. Stops at NEWKEYS;
//! never reads shared secrets or decrypts application traffic.
use nocterm_ssh_keys::login::{InitialHostProof, LoginProofError};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
const MAX_INITIAL: usize = 65_536;
#[derive(Default)]
pub(super) struct Observation {
    pending: Vec<u8>,
    identified: bool,
    stopped: bool,
    proof: Option<InitialHostProof>,
    total: usize,
}
impl Observation {
    pub(super) fn proof(&self) -> Result<InitialHostProof, LoginProofError> {
        self.proof.clone().ok_or(LoginProofError::Malformed)
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<(), LoginProofError> {
        if self.stopped {
            return Ok(());
        }
        self.total = self
            .total
            .checked_add(bytes.len())
            .ok_or(LoginProofError::Malformed)?;
        if self.total > MAX_INITIAL {
            return Err(LoginProofError::Malformed);
        }
        self.pending.extend_from_slice(bytes);
        loop {
            if !self.identified {
                let Some(end) = self.pending.iter().position(|b| *b == b'\n') else {
                    return Ok(());
                };
                if end > 255 {
                    return Err(LoginProofError::Malformed);
                }
                self.identified = self.pending[..=end].starts_with(b"SSH-2.0-");
                self.pending.drain(..=end);
                continue;
            }
            if self.pending.len() < 5 {
                return Ok(());
            }
            let size = u32::from_be_bytes(
                self.pending[..4]
                    .try_into()
                    .map_err(|_| LoginProofError::Malformed)?,
            ) as usize;
            if !(6..=MAX_INITIAL - 4).contains(&size) {
                return Err(LoginProofError::Malformed);
            }
            if self.pending.len() < 4 + size {
                return Ok(());
            }
            let padding = self.pending[4] as usize;
            if padding < 4 || padding >= size - 1 {
                return Err(LoginProofError::Malformed);
            }
            let payload = &self.pending[5..4 + size - padding];
            match payload[0] {
                31 => {
                    if self.proof.is_some() {
                        return Err(LoginProofError::Malformed);
                    }
                    let mut fields = &payload[1..];
                    let host = string(&mut fields)?;
                    let ephemeral = string(&mut fields)?;
                    let signature = string(&mut fields)?;
                    if ephemeral.len() != 32 || !fields.is_empty() {
                        return Err(LoginProofError::Unsupported);
                    }
                    self.proof = Some(InitialHostProof::new(host.to_vec(), signature.to_vec())?);
                }
                21 => {
                    if self.proof.is_none() {
                        return Err(LoginProofError::Malformed);
                    }
                    self.stopped = true;
                    self.pending.clear();
                    self.pending.shrink_to_fit();
                    return Ok(());
                }
                _ => {}
            }
            self.pending.drain(..4 + size);
        }
    }
}
fn string<'a>(data: &mut &'a [u8]) -> Result<&'a [u8], LoginProofError> {
    if data.len() < 4 {
        return Err(LoginProofError::Malformed);
    }
    let size = u32::from_be_bytes(
        data[..4]
            .try_into()
            .map_err(|_| LoginProofError::Malformed)?,
    ) as usize;
    if size > data.len() - 4 {
        return Err(LoginProofError::Malformed);
    }
    let value = &data[4..4 + size];
    *data = &data[4 + size..];
    Ok(value)
}
pub(super) struct ObservedStream<S> {
    stream: S,
    observed: Arc<Mutex<Observation>>,
}
impl<S> ObservedStream<S> {
    pub(super) fn new(stream: S, observed: Arc<Mutex<Observation>>) -> Self {
        Self { stream, observed }
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for ObservedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        match Pin::new(&mut this.stream).poll_read(cx, buffer) {
            Poll::Ready(Ok(())) => {
                let result = this
                    .observed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .feed(&buffer.filled()[before..]);
                Poll::Ready(result.map_err(std::io::Error::other))
            }
            other => other,
        }
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for ObservedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
#[cfg(test)]
mod tests;
