//! Digest every consumed native byte, including trailing whitespace, on each pass.
use super::{HeldReadError, Operation, managed::ReadFaultContext};
use nocterm_ai::history::MAX_FILE_BYTES;
use nocterm_core::SharedError;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};
#[derive(Debug, thiserror::Error)]
#[error("Encoded document read limit exceeded")]
struct EncodedLimit;
pub(super) struct PassReader<'a> {
    file: &'a mut File,
    digest: Sha256,
    bytes: u64,
    failure: Option<SharedError<io::Error>>,
    context: Option<&'a ReadFaultContext>,
    limit_failed: bool,
}
impl<'a> PassReader<'a> {
    pub(super) fn with_context(file: &'a mut File, context: Option<&'a ReadFaultContext>) -> Self {
        Self {
            file,
            context,
            digest: Sha256::new(),
            bytes: 0,
            failure: None,
            limit_failed: false,
        }
    }
    pub(super) fn check_fault(&self, path: &Path) -> Result<(), HeldReadError> {
        if let Some(source) = self.context.and_then(ReadFaultContext::failure) {
            return Err(HeldReadError::ManagedNative(source));
        }
        if let Some(source) = &self.failure {
            return Err(HeldReadError::Native {
                operation: Operation::Read,
                path: path.to_owned(),
                source: source.clone(),
            });
        }
        if self.limit_failed {
            return Err(HeldReadError::EncodedLimit);
        }
        Ok(())
    }
    pub(super) fn finish(self) -> ([u8; 32], u64) {
        (self.digest.finalize().into(), self.bytes)
    }
    fn native_read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.context.and_then(ReadFaultContext::failure).is_some() {
            return Err(io::ErrorKind::Other.into());
        }
        loop {
            match self.file.read(output) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    if let Some(context) = self.context {
                        context.capture(error);
                        return Err(io::ErrorKind::Other.into());
                    }
                    let source = SharedError::from_error(error);
                    self.failure = Some(source.clone());
                    return Err(io::Error::other(source));
                }
                Ok(count) => return Ok(count),
            }
        }
    }
}
impl Read for PassReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.bytes == MAX_FILE_BYTES {
            let mut probe = [0];
            if self.native_read(&mut probe)? == 0 {
                return Ok(0);
            }
            self.limit_failed = true;
            return Err(if self.context.is_some() {
                io::ErrorKind::InvalidData.into()
            } else {
                io::Error::other(EncodedLimit)
            });
        }
        let count = output.len().min((MAX_FILE_BYTES - self.bytes) as usize);
        let count = self.native_read(&mut output[..count])?;
        self.bytes += count as u64;
        self.digest.update(&output[..count]);
        Ok(count)
    }
}
