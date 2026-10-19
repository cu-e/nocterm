//! Primitive JSON formatting measures the real serializer without an owned buffer.
use super::MeasurementError;
use serde::Serialize;
use std::io::{self, Write};
#[derive(Default)]
struct Counter {
    bytes: usize,
    overflow: bool,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.bytes.checked_add(bytes.len()) {
            Some(next) => {
                self.bytes = next;
                Ok(bytes.len())
            }
            None => {
                self.overflow = true;
                Err(io::ErrorKind::InvalidData.into())
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encoded(value: &impl Serialize) -> Result<usize, MeasurementError> {
    let mut counter = Counter::default();
    let result = serde_json::to_writer(&mut counter, value);
    if counter.overflow {
        return Err(MeasurementError::ArithmeticOverflow);
    }
    result?;
    Ok(counter.bytes)
}
