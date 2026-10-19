//! One bounded current input cursor; lookahead never hashes/charges a byte twice.
use super::LiteralReadError;
use nocterm_ai::history::MAX_FILE_BYTES;
use nocterm_core::SharedError;
use std::io::{self, Read};
pub(in crate::loading) struct Input<R> {
    source: R,
    read: u64,
    pending: Option<u8>,
    limit: bool,
    failure: Option<SharedError<io::Error>>,
    managed_marker: bool,
    marker_failed: bool,
}
impl<R: Read> Input<R> {
    pub(in crate::loading) fn new(source: R) -> Self {
        Self {
            source,
            read: 0,
            pending: None,
            limit: false,
            failure: None,
            managed_marker: false,
            marker_failed: false,
        }
    }
    /// Only original held pass/span/BytePump readers enter this route: their
    /// outer first-fault owner retains the concrete original cause until checked.
    pub(in crate::loading) fn new_managed_marker(source: R) -> Self {
        let mut input = Self::new(source);
        input.managed_marker = true;
        input
    }
    pub(super) fn uses_managed_marker(&self) -> bool {
        self.managed_marker
    }
    /// Native Raw lenses switch only after consuming all lookahead. The caller
    /// supplies the actual byte-pump position of the resumed source layer.
    pub(in crate::loading) fn reset_position(
        &mut self,
        consumed: usize,
    ) -> Result<(), LiteralReadError> {
        if self.pending.is_some() {
            return Err(LiteralReadError::Lookahead);
        }
        self.check()?;
        self.read = consumed as u64;
        Ok(())
    }
    pub(in crate::loading) fn source_mut(&mut self) -> &mut R {
        &mut self.source
    }
    /// Logical consumed bytes, excluding the one byte of unconsumed lookahead.
    pub(in crate::loading) fn offset(&self) -> usize {
        self.read as usize - usize::from(self.pending.is_some())
    }
    pub(in crate::loading) fn unread(&mut self, byte: u8) -> Result<(), LiteralReadError> {
        if self.pending.is_some() {
            return Err(LiteralReadError::Lookahead);
        }
        self.pending = Some(byte);
        Ok(())
    }
    pub(in crate::loading) fn byte(&mut self) -> Result<Option<u8>, LiteralReadError> {
        if self.marker_failed {
            return Err(LiteralReadError::ManagedSource);
        }
        if let Some(byte) = self.pending.take() {
            return Ok(Some(byte));
        }
        let mut byte = [0];
        let count = loop {
            match self.source.read(&mut byte) {
                Ok(count) => break count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if self.managed_marker => {
                    drop(error);
                    self.marker_failed = true;
                    return Err(LiteralReadError::ManagedSource);
                }
                Err(error) => {
                    let source = SharedError::from_error(error);
                    self.failure = Some(source.clone());
                    return Err(LiteralReadError::Source(source));
                }
            }
        };
        if count == 0 {
            return Ok(None);
        }
        if self.read == MAX_FILE_BYTES {
            self.limit = true;
            return Err(LiteralReadError::EncodedLimit);
        }
        self.read += 1;
        Ok(Some(byte[0]))
    }
    pub(in crate::loading) fn nonwhite(&mut self) -> Result<Option<u8>, LiteralReadError> {
        loop {
            match self.byte()? {
                Some(b' ' | b'\r' | b'\n' | b'\t') => {}
                value => return Ok(value),
            }
        }
    }
    pub(in crate::loading) fn check(&self) -> Result<(), LiteralReadError> {
        if self.marker_failed {
            return Err(LiteralReadError::ManagedSource);
        }
        if let Some(error) = &self.failure {
            return Err(LiteralReadError::Source(error.clone()));
        }
        if self.limit {
            return Err(LiteralReadError::EncodedLimit);
        }
        Ok(())
    }
}
impl<R: Read> Read for Input<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let next = self.byte().map_err(|error| {
            if self.managed_marker {
                io::Error::from(io::ErrorKind::InvalidData)
            } else {
                io::Error::other(error)
            }
        })?;
        match next {
            Some(byte) => {
                output[0] = byte;
                Ok(1)
            }
            None => Ok(0),
        }
    }
}
