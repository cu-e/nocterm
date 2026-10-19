//! Pure in-memory input. Consumed chunk buffers retire while stock JSON reads.
use super::{DecodeLedger, JsonDecodeError, ledger::Reservation};
use std::{collections::VecDeque, io, mem::size_of};
const CHUNK: usize = 4096;
#[derive(Debug)]
struct Chunk {
    bytes: Vec<u8>,
    _reservation: Reservation,
}
#[derive(Debug)]
pub(super) struct OwnedChunks {
    chunks: VecDeque<Chunk>,
    position: usize,
    remaining: usize,
    _headers: Reservation,
}
impl OwnedChunks {
    pub(super) fn new(input: String, ledger: &DecodeLedger) -> Result<Self, JsonDecodeError> {
        let source = ledger.reserve(input.capacity())?;
        Self::from_reserved(input, source, ledger)
    }
    pub(super) fn from_reserved(
        input: String,
        source: Reservation,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        if !source.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        // The old raw String remains charged until ALL new chunks exist.
        // Reserve copied bytes first, then choose a power-of-two chunk schedule
        // whose REAL header count fits the remaining construction room.
        let mut payload = ledger.reserve(input.len())?;
        let available_headers = ledger.available() / size_of::<Chunk>();
        let chunk_size = if input.is_empty() {
            CHUNK
        } else {
            if available_headers == 0 {
                return Err(JsonDecodeError::Limit {
                    requested: size_of::<Chunk>(),
                    available: ledger.available(),
                });
            }
            input
                .len()
                .div_ceil(available_headers)
                .checked_next_power_of_two()
                .ok_or(JsonDecodeError::Arithmetic)?
                .max(CHUNK)
        };
        let count = input.len().div_ceil(chunk_size);
        let headers = count
            .checked_mul(size_of::<Chunk>())
            .ok_or(JsonDecodeError::Arithmetic)?;
        let headers = ledger.reserve(headers)?;
        let mut chunks = VecDeque::new();
        chunks.try_reserve_exact(count)?;
        // Pinned Global RawVec grow_exact records the requested capacity.
        if chunks.capacity() != count {
            return Err(JsonDecodeError::CapacityProfile);
        }
        for part in input.as_bytes().chunks(chunk_size) {
            let reservation = payload.split(part.len())?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(part.len())?;
            if bytes.capacity() != part.len() {
                return Err(JsonDecodeError::CapacityProfile);
            }
            bytes.extend_from_slice(part);
            chunks.push_back(Chunk {
                bytes,
                _reservation: reservation,
            });
        }
        let remaining = input.len();
        drop(input);
        drop(source);
        drop(payload);
        Ok(Self {
            chunks,
            position: 0,
            remaining,
            _headers: headers,
        })
    }
    #[cfg(test)]
    pub(super) fn remaining(&self) -> usize {
        self.remaining
    }
}
impl io::Read for OwnedChunks {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let Some(front) = self.chunks.front() else {
            return Ok(0);
        };
        let count = output.len().min(front.bytes.len() - self.position);
        output[..count].copy_from_slice(&front.bytes[self.position..self.position + count]);
        self.position += count;
        self.remaining -= count;
        if self.position == front.bytes.len() {
            self.chunks.pop_front();
            self.position = 0;
        }
        Ok(count)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    #[test]
    fn consumed_chunks_retire_with_front_slack_and_headers_charged() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let ledger = DecodeLedger::new(&authority, 1 << 20).unwrap();
        let fixed = ledger.live_requested_bytes();
        let input = "x".repeat(CHUNK * 2 + 3);
        let capacity = input.capacity();
        let mut reader = OwnedChunks::new(input, &ledger).unwrap();
        let headers = 3 * size_of::<Chunk>();
        assert_eq!(
            ledger.peak_requested_bytes(),
            fixed + capacity + headers + CHUNK * 2 + 3
        );
        assert_eq!(
            ledger.live_requested_bytes(),
            fixed + headers + CHUNK * 2 + 3
        );
        let mut output = [0; CHUNK - 1];
        reader.read_exact(&mut output).unwrap();
        assert_eq!(
            ledger.live_requested_bytes(),
            fixed + headers + CHUNK * 2 + 3
        );
        reader.read_exact(&mut [0]).unwrap();
        assert_eq!(ledger.live_requested_bytes(), fixed + headers + CHUNK + 3);
        assert_eq!(reader.remaining(), CHUNK + 3);
        drop(reader);
        assert_eq!(ledger.live_requested_bytes(), fixed);
    }
    #[test]
    fn chunk_header_schedule_adapts_before_allocating_near_double_payload_limit() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let input = "x".repeat(2 * 1024 * 1024);
        let limit = input.capacity() + input.len() + 256;
        let ledger = DecodeLedger::new(&authority, limit).unwrap();
        let fixed = ledger.live_requested_bytes();
        let reader = OwnedChunks::new(input, &ledger).unwrap();
        assert!(reader.chunks.len() < 10);
        assert!(ledger.peak_requested_bytes() <= limit);
        drop(reader);
        assert_eq!(ledger.live_requested_bytes(), fixed);
    }
}
