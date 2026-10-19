//! The actual serde_json map-key serializer supplies grammar; a streaming sink
//! counts its quoted spelling and decoded UTF8 without materializing the key.
use super::MeasurementError;
use serde::{
    Serialize,
    ser::{self, SerializeMap},
};
use std::io::{self, Write};
struct Key<'a, T: ?Sized>(&'a T);
impl<T: Serialize + ?Sized> Serialize for Key<'_, T> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(self.0, &())?;
        map.end()
    }
}
#[derive(Clone, Copy)]
enum State {
    OpeningBrace,
    OpeningQuote,
    Text,
    Escape,
    Unicode(u16, u8),
    Suffix(u8),
    Done,
}
struct Counter {
    state: State,
    encoded: usize,
    decoded: usize,
    high_surrogate: bool,
    overflow: bool,
    invalid: bool,
}
impl Counter {
    fn decoded(&mut self, bytes: usize) -> io::Result<()> {
        match self.decoded.checked_add(bytes) {
            Some(next) => {
                self.decoded = next;
                Ok(())
            }
            None => {
                self.overflow = true;
                Err(io::ErrorKind::InvalidData.into())
            }
        }
    }
    fn reject(&mut self) -> io::Result<()> {
        self.invalid = true;
        Err(io::ErrorKind::InvalidData.into())
    }
    fn byte(&mut self, byte: u8) -> io::Result<()> {
        self.state = match self.state {
            State::OpeningBrace if byte == b'{' => State::OpeningQuote,
            State::OpeningQuote if byte == b'"' => State::Text,
            State::Text => match byte {
                b'"' if !self.high_surrogate => State::Suffix(0),
                b'\\' => State::Escape,
                _ if !self.high_surrogate => {
                    self.decoded(1)?;
                    State::Text
                }
                _ => return self.reject(),
            },
            State::Escape => match byte {
                b'u' => State::Unicode(0, 0),
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' if !self.high_surrogate => {
                    self.decoded(1)?;
                    State::Text
                }
                _ => return self.reject(),
            },
            State::Unicode(value, digits) => {
                let digit = match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => return self.reject(),
                };
                let value = (value << 4) | u16::from(digit);
                if digits < 3 {
                    State::Unicode(value, digits + 1)
                } else {
                    match value {
                        0xD800..=0xDBFF if !self.high_surrogate => self.high_surrogate = true,
                        0xDC00..=0xDFFF if self.high_surrogate => {
                            self.high_surrogate = false;
                            self.decoded(4)?;
                        }
                        0xD800..=0xDFFF => return self.reject(),
                        _ if self.high_surrogate => return self.reject(),
                        0..=0x7F => self.decoded(1)?,
                        0x80..=0x7FF => self.decoded(2)?,
                        _ => self.decoded(3)?,
                    }
                    State::Text
                }
            }
            State::Suffix(position) => {
                if b":null}".get(usize::from(position)) != Some(&byte) {
                    return self.reject();
                }
                if position == 5 {
                    State::Done
                } else {
                    State::Suffix(position + 1)
                }
            }
            _ => return self.reject(),
        };
        Ok(())
    }
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(encoded) = self.encoded.checked_add(bytes.len()) else {
            self.overflow = true;
            return Err(io::ErrorKind::InvalidData.into());
        };
        self.encoded = encoded;
        for &byte in bytes {
            self.byte(byte)?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) struct Size {
    pub(super) encoded_with_colon: usize,
    pub(super) decoded_utf8: usize,
}
pub(super) fn measure(key: &(impl Serialize + ?Sized)) -> Result<Size, MeasurementError> {
    let mut counter = Counter {
        state: State::OpeningBrace,
        encoded: 0,
        decoded: 0,
        high_surrogate: false,
        overflow: false,
        invalid: false,
    };
    let result = serde_json::to_writer(&mut counter, &Key(key));
    if counter.overflow {
        return Err(MeasurementError::ArithmeticOverflow);
    }
    result?;
    if counter.invalid || !matches!(counter.state, State::Done) {
        return Err(MeasurementError::ArithmeticOverflow);
    }
    Ok(Size {
        encoded_with_colon: counter
            .encoded
            .checked_sub(6)
            .ok_or(MeasurementError::ArithmeticOverflow)?,
        decoded_utf8: counter.decoded,
    })
}
