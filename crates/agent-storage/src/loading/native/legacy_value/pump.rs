//! Flat streaming quoted-source lenses; a parent read never recurses through Read.
mod unit;
use super::{ValueError, frames::Frames};
use crate::loading::string_reader::StringReadError;
use nocterm_ai::model::json::deferred::{DecodeLedger, NativeScratch};
use std::{
    io::{self, Read},
    mem::size_of,
};
use unit::Decoder;

pub(super) struct DecodeFrame {
    decoder: Decoder,
    skip: usize,
    end: Option<usize>,
    input: usize,
    output: usize,
}
impl DecodeFrame {
    pub(super) fn quoted(start: usize, end: Option<usize>) -> Self {
        Self {
            decoder: Decoder::quoted(),
            skip: start,
            end,
            input: 0,
            output: 0,
        }
    }
    fn entered() -> Self {
        Self {
            decoder: Decoder::entered(),
            skip: 0,
            end: None,
            input: 0,
            output: 0,
        }
    }
    fn ready(&mut self) -> Option<Option<u8>> {
        let byte = self.decoder.ready()?;
        if byte.is_some() {
            self.output += 1;
        }
        Some(byte)
    }
    fn accept(&mut self, byte: Option<u8>, ledger: &DecodeLedger) -> Result<(), StringReadError> {
        let byte = byte.ok_or(StringReadError::Truncated)?;
        self.input += 1;
        if self.input > nocterm_ai::history::MAX_FILE_BYTES as usize {
            return Err(StringReadError::EncodedLimit);
        }
        if self.skip != 0 {
            self.skip -= 1;
            return Ok(());
        }
        self.decoder.accept(byte, ledger)?;
        if self.decoder.finished() && self.end.is_some_and(|end| self.input != end) {
            return Err(StringReadError::Truncated);
        }
        Ok(())
    }
}
/// One actual base buffer, plus one prepaid state per quoted layer. Recipes and
/// live validation both use this byte pump; there are no mandatory decoded caches.
pub(super) struct BytePump<'a, R> {
    source: R,
    frames: Frames<'a, DecodeFrame>,
    buffer: [u8; 4096],
    buffered_start: usize,
    buffered_end: usize,
    base_position: usize,
    failure: Option<StringReadError>,
    _fixed: NativeScratch,
    ledger: &'a DecodeLedger,
}
impl<'a, R: Read> BytePump<'a, R> {
    pub(super) fn new(source: R, ledger: &'a DecodeLedger) -> Result<Self, ValueError> {
        Self::with_frames(source, Frames::new(ledger)?, ledger)
    }
    pub(super) fn with_frames(
        source: R,
        frames: Frames<'a, DecodeFrame>,
        ledger: &'a DecodeLedger,
    ) -> Result<Self, ValueError> {
        Ok(Self {
            source,
            frames,
            buffer: [0; 4096],
            buffered_start: 0,
            buffered_end: 0,
            base_position: 0,
            failure: None,
            _fixed: NativeScratch::reserve(ledger, size_of::<Self>())?,
            ledger,
        })
    }
    pub(super) fn enter(&mut self) -> Result<(), ValueError> {
        self.frames.push(DecodeFrame::entered())
    }
    pub(super) fn leave(&mut self) -> Result<usize, ValueError> {
        let frame = self.frames.pop().ok_or(ValueError::ForeignSource)?;
        if !frame.decoder.finished() {
            return Err(ValueError::Punctuation);
        }
        Ok(self.position())
    }
    pub(super) fn position(&self) -> usize {
        self.frames
            .get(self.frames.len().saturating_sub(1))
            .map_or(self.base_position, |frame| frame.output)
    }
    pub(super) fn take_failure(&mut self) -> Option<StringReadError> {
        self.failure.take()
    }
    fn base_byte(&mut self) -> io::Result<Option<u8>> {
        if self.buffered_start == self.buffered_end {
            self.buffered_end = self.source.read(&mut self.buffer)?;
            self.buffered_start = 0;
            if self.buffered_end == 0 {
                return Ok(None);
            }
        }
        let byte = self.buffer[self.buffered_start];
        self.buffered_start += 1;
        self.base_position += 1;
        Ok(Some(byte))
    }
    fn byte(&mut self) -> io::Result<Option<u8>> {
        let target = self.frames.len();
        let mut level = target;
        let mut delivered = None;
        loop {
            let next = if level == 0 {
                Some(self.base_byte()?)
            } else {
                let frame = self
                    .frames
                    .get_mut(level - 1)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
                if let Some(byte) = delivered.take()
                    && let Err(error) = frame.accept(byte, self.ledger)
                {
                    self.failure = Some(error);
                    return Err(io::Error::from(io::ErrorKind::InvalidData));
                }
                frame.ready()
            };
            match next {
                Some(byte) if level == target => return Ok(byte),
                Some(byte) => {
                    level += 1;
                    delivered = Some(byte);
                }
                None => {
                    level -= 1;
                }
            }
        }
    }
    pub(super) fn skip(&mut self, count: usize) -> Result<(), ValueError> {
        for _ in 0..count {
            match self.byte() {
                Ok(Some(_)) => {}
                Ok(None) => return Err(ValueError::ForeignSource),
                Err(error) => {
                    return Err(self.take_failure().map_or_else(
                        || {
                            drop(error);
                            ValueError::Literal(
                                crate::loading::literal_reader::LiteralReadError::ManagedSource,
                            )
                        },
                        |failure| ValueError::Literal(failure.into()),
                    ));
                }
            }
        }
        Ok(())
    }
}
impl<R: Read> Read for BytePump<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        match self.byte()? {
            Some(byte) => {
                output[0] = byte;
                Ok(1)
            }
            None => Ok(0),
        }
    }
}

#[cfg(test)]
mod tests;
