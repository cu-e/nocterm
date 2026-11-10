//! In-memory byte sinks with caller-supplied bounds; no filesystem or codec policy.
use std::io::{self, Write};

/// Counts bytes without retaining them and refuses writes beyond its limit.
pub struct CountingWriter {
    count: usize,
    limit: usize,
}
impl CountingWriter {
    pub fn new(limit: usize) -> Self {
        Self { count: 0, limit }
    }
    pub fn count(&self) -> usize {
        self.count
    }
}
impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.count = self
            .count
            .checked_add(bytes.len())
            .filter(|count| *count <= self.limit)
            .ok_or_else(capacity)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Retains at most `limit` bytes; a rejected write leaves the buffer untouched.
pub struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl BoundedBuffer {
    pub fn new(limit: usize) -> Self {
        Self::with_capacity(limit, 0)
    }
    pub fn with_capacity(limit: usize, capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity.min(limit)),
            limit,
            exceeded: false,
        }
    }
    pub fn exceeded(&self) -> bool {
        self.exceeded
    }
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}
impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(capacity());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn capacity() -> io::Error {
    io::ErrorKind::OutOfMemory.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_limit_is_accepted_and_failed_write_retains_prefix() {
        let mut output = BoundedBuffer::with_capacity(3, 100);
        output.write_all(b"abc").unwrap();
        assert!(!output.exceeded());
        assert_eq!(
            output.write_all(b"d").unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        assert!(output.exceeded());
        assert_eq!(output.into_inner(), b"abc");
    }

    #[test]
    fn counting_refuses_overflow_without_changing_accepted_count() {
        let mut output = CountingWriter::new(3);
        output.write_all(b"abc").unwrap();
        assert_eq!(
            output.write_all(b"d").unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        assert_eq!(output.count(), 3);
        output.write_all(b"").unwrap();
        let mut empty = CountingWriter::new(0);
        assert!(empty.write_all(b"x").is_err());
        assert_eq!(empty.count(), 0);
    }
}
