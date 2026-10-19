//! A bounded encoded-string cursor; stock serde owns Unicode escape semantics.
//! Managed Unicode-unit scratch and concrete errors use the actual decode ledger.
#[cfg(test)]
pub(super) mod spans;
#[cfg(test)]
mod tests;
pub(in crate::loading) mod unit;
use nocterm_ai::model::{
    failure::ManagedNativeFailure,
    json::deferred::{DecodeLedger, NativeAllocationError},
};
use nocterm_core::SharedError;
use std::{
    io::{self, Read},
    str::Utf8Error,
};

#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum StringReadError {
    #[error("Expected the opening quote of a JSON string")]
    OpeningQuote,
    #[error("A partial byte read cannot be resumed as a whole UTF-8 fragment")]
    PartialFragment,
    #[error("Encoded JSON string exceeded its source span bound")]
    EncodedLimit,
    #[error("Encoded JSON string ended before its closing quote")]
    Truncated,
    #[error("Encoded JSON string source failed: {0}")]
    Source(#[source] SharedError<io::Error>),
    #[error("Encoded JSON string source has an original guarded failure")]
    ManagedSource,
    #[error("JSON string escape failed: {0}")]
    Json(#[source] SharedError<serde_json::Error>),
    #[error("JSON string escape failed: {0}")]
    ManagedJson(#[source] ManagedNativeFailure),
    #[error("JSON string parser admission failed: {0}")]
    Admission(#[source] NativeAllocationError),
    #[error("JSON string contains invalid UTF-8: {0}")]
    Utf8(#[source] Utf8Error),
}
impl StringReadError {
    fn source(error: io::Error) -> Self {
        Self::Source(SharedError::from_error(error))
    }
    fn json(error: serde_json::Error) -> Self {
        Self::Json(SharedError::from_error(error))
    }
    fn into_io(self) -> io::Error {
        io::Error::other(self)
    }
}
/// Input starts at an opening quote. The cursor stops immediately after its close.
/// No full source String, decoded String or parser scratch is retained by this owner.
pub(super) struct JsonStringReader<'a, R> {
    source: R,
    ledger: Option<&'a DecodeLedger>,
    encoded_limit: usize,
    encoded_read: usize,
    output: [u8; 8],
    output_start: usize,
    output_end: usize,
    finished: bool,
    source_failure: Option<SharedError<io::Error>>,
}
impl<'a, R: Read> JsonStringReader<'a, R> {
    pub(super) fn new(source: R, encoded_limit: usize) -> Result<Self, StringReadError> {
        Self::with_ledger(source, encoded_limit, None)
    }
    pub(super) fn new_managed(
        source: R,
        encoded_limit: usize,
        ledger: &'a DecodeLedger,
    ) -> Result<Self, StringReadError> {
        Self::with_ledger(source, encoded_limit, Some(ledger))
    }
    fn with_ledger(
        source: R,
        encoded_limit: usize,
        ledger: Option<&'a DecodeLedger>,
    ) -> Result<Self, StringReadError> {
        let mut cursor = Self {
            source,
            ledger,
            encoded_limit,
            encoded_read: 0,
            output: [0; 8],
            output_start: 0,
            output_end: 0,
            finished: false,
            source_failure: None,
        };
        if cursor.next_encoded()? != b'"' {
            return Err(StringReadError::OpeningQuote);
        }
        Ok(cursor)
    }
    /// One validated Unicode unit, with no cross-unit output allocation. Callers
    /// can reserve/copy this bounded borrowed fragment directly into managed text.
    pub(super) fn next_fragment(&mut self) -> Result<Option<&str>, StringReadError> {
        if self.output_start != self.output_end {
            return Err(StringReadError::PartialFragment);
        }
        if self.finished {
            return Ok(None);
        }
        self.next_unit()?;
        self.output_start = self.output_end;
        if self.finished {
            return Ok(None);
        }
        std::str::from_utf8(&self.output[..self.output_end])
            .map(Some)
            .map_err(StringReadError::Utf8)
    }
    pub(super) fn into_inner(self) -> R {
        self.source
    }
    #[cfg(test)]
    pub(super) fn take_source_failure(&mut self) -> Option<SharedError<io::Error>> {
        self.source_failure.take()
    }
    fn next_encoded(&mut self) -> Result<u8, StringReadError> {
        if self.encoded_read == self.encoded_limit {
            return Err(StringReadError::EncodedLimit);
        }
        let mut byte = [0];
        loop {
            match self.source.read(&mut byte) {
                Ok(0) => return Err(StringReadError::Truncated),
                Ok(_) => {
                    self.encoded_read += 1;
                    return Ok(byte[0]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if self.ledger.is_some() => {
                    drop(error);
                    return Err(StringReadError::ManagedSource);
                }
                Err(error) => {
                    // A nested cursor's semantic failure is not a new native read
                    // failure. Preserve the actual original cause and its scope.
                    let error = error
                        .get_ref()
                        .and_then(|error| error.downcast_ref::<StringReadError>())
                        .cloned()
                        .unwrap_or_else(|| StringReadError::source(error));
                    if let StringReadError::Source(source) = &error {
                        self.source_failure = Some(source.clone());
                    }
                    return Err(error);
                }
            }
        }
    }
    fn next_unit(&mut self) -> Result<(), StringReadError> {
        self.output_start = 0;
        self.output_end = 0;
        match self.next_encoded()? {
            b'"' => self.finished = true,
            b'\\' => self.escape()?,
            first @ 0..=31 => {
                // Ask the same stock String parser to reject control characters.
                let fragment = [b'"', first, b'"'];
                unit::parse(&fragment, self.ledger)?;
            }
            first @ 32..=127 => {
                self.output[0] = first;
                self.output_end = 1;
            }
            first => self.utf8(first)?,
        }
        Ok(())
    }
    fn utf8(&mut self, first: u8) -> Result<(), StringReadError> {
        let mut bytes = [0; 4];
        bytes[0] = first;
        let mut length = 1;
        loop {
            match std::str::from_utf8(&bytes[..length]) {
                Ok(_) => {
                    self.output[..length].copy_from_slice(&bytes[..length]);
                    self.output_end = length;
                    return Ok(());
                }
                Err(error) if error.error_len().is_none() && length < bytes.len() => {
                    bytes[length] = self.next_encoded()?;
                    length += 1;
                }
                Err(error) => return Err(StringReadError::Utf8(error)),
            }
        }
    }
    fn escape(&mut self) -> Result<(), StringReadError> {
        // This frames 2- or 6-byte lexical units, without interpreting hex digits,
        // codepoint values or surrogate ranges. The stock parser decides validity.
        let mut quoted = [0; 14];
        quoted[0] = b'"';
        quoted[1] = b'\\';
        quoted[2] = self.next_encoded()?;
        let mut length = if quoted[2] == b'u' { 7 } else { 3 };
        for byte in &mut quoted[3..length] {
            *byte = self.next_encoded()?;
        }
        quoted[length] = b'"';
        let first = unit::parse(&quoted[..length + 1], self.ledger);
        let decoded = match first {
            Ok(value) => value,
            Err(first_error)
                if quoted[2] == b'u' && !matches!(&first_error, StringReadError::Admission(_)) =>
            {
                // A standalone unit may require its adjacent unit. Retry through
                // stock serde; never implement or guess surrogate acceptance here.
                quoted[7] = self.next_encoded()?;
                quoted[8] = self.next_encoded()?;
                if quoted[7..9] != *b"\\u" {
                    return Err(first_error);
                }
                for byte in &mut quoted[9..13] {
                    *byte = self.next_encoded()?;
                }
                length = 13;
                quoted[length] = b'"';
                unit::parse(&quoted[..length + 1], self.ledger)?
            }
            Err(error) => return Err(error),
        };
        self.output[..decoded.as_bytes().len()].copy_from_slice(decoded.as_bytes());
        self.output_end = decoded.as_bytes().len();
        Ok(())
    }
}
impl<R: Read> Read for JsonStringReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut copied = 0;
        while copied < output.len() {
            if self.output_start == self.output_end {
                if self.finished {
                    break;
                }
                self.next_unit().map_err(StringReadError::into_io)?;
                if self.finished {
                    break;
                }
            }
            let count = (output.len() - copied).min(self.output_end - self.output_start);
            output[copied..copied + count]
                .copy_from_slice(&self.output[self.output_start..self.output_start + count]);
            self.output_start += count;
            copied += count;
        }
        Ok(copied)
    }
}
