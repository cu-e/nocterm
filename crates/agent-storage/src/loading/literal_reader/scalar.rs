//! Stock serde number/null/boolean grammar, with streaming delimiter lookahead.
//! No numeric-token buffer, digit-length limit or handwritten floating parser.
use super::{LiteralReadError, input::Input};
mod managed;
use serde::{Deserialize, de::Visitor};
use serde_json::Number;
use std::{
    fmt,
    io::{self, Read},
};
pub(in crate::loading) enum Scalar {
    Null,
    Boolean(bool),
    Number(Number),
}
impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Leaf;
        impl<'de> Visitor<'de> for Leaf {
            type Value = Scalar;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON scalar")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Scalar, E> {
                Ok(Scalar::Null)
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Scalar, E> {
                Ok(Scalar::Boolean(value))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Scalar, E> {
                Ok(Scalar::Number(value.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Scalar, E> {
                Ok(Scalar::Number(value.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Scalar, E> {
                Number::from_f64(value)
                    .map(Scalar::Number)
                    .ok_or_else(|| E::custom("non-finite JSON scalar"))
            }
        }
        decoder.deserialize_any(Leaf)
    }
}
struct Atom<'a, R> {
    input: &'a mut Input<R>,
    ended: bool,
}
impl<R: Read> Read for Atom<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.ended {
            return Ok(0);
        }
        let Some(byte) = self.input.byte().map_err(|error| {
            if self.input.uses_managed_marker() {
                io::Error::from(io::ErrorKind::InvalidData)
            } else {
                io::Error::other(error)
            }
        })?
        else {
            self.ended = true;
            return Ok(0);
        };
        if matches!(byte, b',' | b']' | b'}' | b' ' | b'\r' | b'\n' | b'\t') {
            self.input.unread(byte).map_err(|error| {
                if self.input.uses_managed_marker() {
                    io::Error::from(io::ErrorKind::InvalidData)
                } else {
                    io::Error::other(error)
                }
            })?;
            self.ended = true;
            return Ok(0);
        }
        output[0] = byte;
        Ok(1)
    }
}
pub(in crate::loading) fn read<R: Read>(
    input: &mut Input<R>,
    ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
) -> Result<Scalar, LiteralReadError> {
    managed::read(input, ledger)
}
