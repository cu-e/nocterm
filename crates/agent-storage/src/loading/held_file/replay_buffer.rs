//! Fixed inline read-ahead for authenticated spans; no decoded source cache.
use std::io::{self, BufRead, Read};

/// Bytes remain authenticated by the underlying SpanReader at native read time.
/// Dropping unread read-ahead never skips hashing; the caller still drains and
/// compares the complete original covered span after its inspector returns.
pub(in crate::loading) struct ReplayBuffer<R, const N: usize = 4096> {
    source: R,
    bytes: [u8; N],
    start: usize,
    end: usize,
}
impl<R, const N: usize> ReplayBuffer<R, N> {
    pub(in crate::loading) fn with_inline_capacity(source: R) -> Self {
        Self {
            source,
            bytes: [0; N],
            start: 0,
            end: 0,
        }
    }
}
impl<R> ReplayBuffer<R> {
    pub(super) fn new(source: R) -> Self {
        Self::with_inline_capacity(source)
    }
}
impl<R: Read, const N: usize> Read for ReplayBuffer<R, N> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.start == self.end {
            if output.len() >= self.bytes.len() {
                return self.source.read(output);
            }
            self.end = self.source.read(&mut self.bytes)?;
            self.start = 0;
        }
        let count = output.len().min(self.end - self.start);
        output[..count].copy_from_slice(&self.bytes[self.start..self.start + count]);
        self.start += count;
        Ok(count)
    }
}

impl<R: Read, const N: usize> BufRead for ReplayBuffer<R, N> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.start == self.end {
            self.end = self.source.read(&mut self.bytes)?;
            self.start = 0;
        }
        Ok(&self.bytes[self.start..self.end])
    }
    fn consume(&mut self, amount: usize) {
        self.start += amount.min(self.end - self.start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, io::Cursor};
    struct Counted<'a, R> {
        source: R,
        calls: &'a Cell<usize>,
    }
    impl<R: Read> Read for Counted<'_, R> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            self.calls.set(self.calls.get() + 1);
            self.source.read(output)
        }
    }
    #[test]
    fn bytewise_consumers_read_each_native_block_once() {
        let bytes = vec![b'a'; 12_345];
        let calls = Cell::new(0);
        let source = Counted {
            source: Cursor::new(&bytes),
            calls: &calls,
        };
        let mut buffered = ReplayBuffer::new(source);
        let mut actual = Vec::new();
        let mut byte = [0];
        while buffered.read(&mut byte).unwrap() != 0 {
            actual.push(byte[0]);
        }
        assert_eq!(actual, bytes);
        assert_eq!(calls.get(), 5);
    }
    #[test]
    fn span_limit_prevents_read_ahead_into_another_value() {
        let calls = Cell::new(0);
        let mut source = Counted {
            source: Cursor::new(b"original adjacent"),
            calls: &calls,
        };
        {
            let mut buffered = ReplayBuffer::new((&mut source).take(8));
            let mut byte = [0];
            buffered.read_exact(&mut byte).unwrap();
            assert_eq!(byte[0], b'o');
        }
        let mut rest = String::new();
        source.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, " adjacent");
    }
    #[test]
    fn a_failed_fill_preserves_typed_source_error_and_can_retry() {
        let error = io::Error::from(io::ErrorKind::PermissionDenied);
        let mut buffered = ReplayBuffer::new(error_source(error));
        assert_eq!(
            buffered.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(buffered.read(&mut [0]).unwrap(), 0);
    }
    fn error_source(error: io::Error) -> impl Read {
        struct Once(Option<io::Error>);
        impl Read for Once {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.0.take().map_or(Ok(0), Err)
            }
        }
        Once(Some(error))
    }
}
