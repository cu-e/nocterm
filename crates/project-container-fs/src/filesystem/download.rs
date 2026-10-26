use super::{
    commands::{FilesystemCommands, status, timed},
    scripts,
};
use nocterm_session::{ExecOutput, FsError};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub(super) struct Download {
    output: Option<ExecOutput>,
    pending: Vec<u8>,
    size: Option<u64>,
    received: u64,
    done: bool,
    path: String,
    alive: Arc<AtomicBool>,
}
impl Download {
    pub(super) async fn open(
        commands: &FilesystemCommands,
        path: String,
        alive: Arc<AtomicBool>,
    ) -> Result<Self, FsError> {
        let output = commands
            .open(scripts::DOWNLOAD, std::slice::from_ref(&path), None)
            .await?;
        let mut download = Self {
            output: Some(output),
            pending: Vec::new(),
            size: None,
            received: 0,
            done: false,
            path,
            alive,
        };
        let first = download.read(1).await?;
        download.pending.splice(0..0, first);
        Ok(download)
    }
    pub(super) fn alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Acquire)
    }
    fn accept(&mut self, chunk: Vec<u8>) -> Result<(), FsError> {
        if chunk.len() > 256 * 1024 {
            return Err(FsError::Other(
                "container output chunk exceeds bound".into(),
            ));
        }
        self.pending.extend(chunk);
        if self.size.is_none() {
            if let Some(end) = self.pending.iter().position(|byte| *byte == 0) {
                if end > 32 {
                    return Err(FsError::Other("malformed download admission".into()));
                }
                let size = super::codec::text(&self.pending[..end])?
                    .parse::<u64>()
                    .map_err(|_| FsError::Other("malformed download size".into()))?;
                self.pending.drain(..=end);
                self.size = Some(size);
                self.received = self.pending.len() as u64;
            } else if self.pending.len() > 32 {
                return Err(FsError::Other("malformed download admission".into()));
            }
        } else {
            self.received += self.pending.len() as u64;
        }
        if self.size.is_some_and(|size| self.received > size) {
            return Err(FsError::Other("download changed size".into()));
        }
        Ok(())
    }
    pub(super) async fn read(&mut self, max: usize) -> Result<Vec<u8>, FsError> {
        if max == 0 || max > 32768 {
            return Err(FsError::Other(
                "download reads require 1..32768 bytes".into(),
            ));
        }
        if self.done {
            return Ok(Vec::new());
        }
        // Taking the output makes abandonment cancel the held process rather than stall the actor.
        let mut output = self
            .output
            .take()
            .ok_or_else(|| FsError::Other("download was cancelled".into()))?;
        let result = timed(
            async {
                loop {
                    if self.size.is_some() && !self.pending.is_empty() {
                        let count = max.min(self.pending.len());
                        return Ok(self.pending.drain(..count).collect());
                    }
                    match output.next().await {
                        Some(chunk) => {
                            self.accept(chunk)?;
                        }
                        None => return Ok(Vec::new()),
                    }
                }
            },
            Duration::from_secs(15),
        )
        .await?;
        match result {
            Ok(bytes) if bytes.is_empty() => {
                let exit = timed(output.exit(), Duration::from_secs(15)).await?;
                self.done = true;
                if !exit.success() {
                    return Err(status(exit.status, &self.path));
                }
                if self.size != Some(self.received) {
                    return Err(FsError::Other("download was truncated".into()));
                }
                Ok(bytes)
            }
            Ok(bytes) => {
                self.output = Some(output);
                Ok(bytes)
            }
            Err(error) => Err(error),
        }
    }
}
