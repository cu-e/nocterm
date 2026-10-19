//! Exact compact JSON byte count without allocating the encoded payload.
use serde::{
    Serialize,
    ser::{self, SerializeMap, SerializeSeq, SerializeStruct},
};
use std::io;

pub(super) fn encoded_len(value: &impl Serialize) -> Result<usize, serde_json::Error> {
    let mut counter = Counter(0, 0);
    value.serialize(&mut counter)?;
    Ok(counter.0)
}
pub(super) fn structural_size(value: &impl Serialize) -> Result<usize, serde_json::Error> {
    let mut counter = Counter(0, 0);
    value.serialize(&mut counter)?;
    Ok(counter.1)
}
/// Examine sixteen bytes at once; ordinary UTF-8 and base64 need no escaping.
fn string_len(text: &str) -> usize {
    const HIGH: u128 = u128::from_ne_bytes([0x80; 16]);
    const LOW: u128 = u128::from_ne_bytes([0x7f; 16]);
    const SIXTY: u128 = u128::from_ne_bytes([0x60; 16]);
    const ONES: u128 = u128::from_ne_bytes([1; 16]);
    let zero = |word: u128| word.wrapping_sub(ONES) & !word & HIGH != 0;
    let extra = |byte: u8| match byte {
        b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 1,
        0..=31 => 5,
        _ => 0,
    };
    let mut length = text.len() + 2;
    let (chunks, remainder) = text.as_bytes().as_chunks::<16>();
    for chunk in chunks {
        let word = u128::from_ne_bytes(*chunk);
        let control = !(word & LOW).wrapping_add(SIXTY) & !word & HIGH != 0;
        if control
            || zero(word ^ u128::from_ne_bytes([b'"'; 16]))
            || zero(word ^ u128::from_ne_bytes([b'\\'; 16]))
        {
            length += chunk.iter().copied().map(extra).sum::<usize>();
        }
    }
    length + remainder.iter().copied().map(extra).sum::<usize>()
}
struct Counter(usize, usize);
struct Compound<'a> {
    counter: &'a mut Counter,
    first: bool,
    close: usize,
}
impl Compound<'_> {
    fn comma(&mut self) {
        if self.first {
            self.first = false;
        } else {
            self.counter.0 += 1;
        }
    }
    fn finish(self) -> Result<(), serde_json::Error> {
        self.counter.0 += self.close;
        Ok(())
    }
}
macro_rules! number {
    ($name:ident, $ty:ty) => {
        fn $name(self, value: $ty) -> Result<(), Self::Error> {
            self.1 += 512;
            self.0 += serde_json::to_string(&value)?.len();
            Ok(())
        }
    };
}
impl<'a> ser::Serializer for &'a mut Counter {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;
    fn serialize_bool(self, value: bool) -> Result<(), Self::Error> {
        self.1 += 512;
        self.0 += if value { 4 } else { 5 };
        Ok(())
    }
    number!(serialize_i8, i8);
    number!(serialize_i16, i16);
    number!(serialize_i32, i32);
    number!(serialize_i64, i64);
    number!(serialize_i128, i128);
    number!(serialize_u8, u8);
    number!(serialize_u16, u16);
    number!(serialize_u32, u32);
    number!(serialize_u64, u64);
    number!(serialize_u128, u128);
    number!(serialize_f32, f32);
    number!(serialize_f64, f64);
    fn serialize_char(self, value: char) -> Result<(), Self::Error> {
        self.serialize_str(value.encode_utf8(&mut [0; 4]))
    }
    fn serialize_str(self, value: &str) -> Result<(), Self::Error> {
        self.1 += 512 + value.len() * 2;
        self.0 += string_len(value);
        Ok(())
    }
    fn serialize_bytes(self, value: &[u8]) -> Result<(), Self::Error> {
        let mut seq = self.serialize_seq(Some(value.len()))?;
        for byte in value {
            seq.serialize_element(byte)?;
        }
        SerializeSeq::end(seq)
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        self.serialize_unit()
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.1 += 512;
        self.0 += 4;
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Self::Error> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.serialize_str(variant)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.1 += 1536 + variant.len() * 2;
        self.0 += string_len(variant) + 3;
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.1 += 512;
        self.0 += 1;
        Ok(Compound {
            counter: self,
            first: true,
            close: 1,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.1 += 1536 + variant.len() * 2;
        self.0 += string_len(variant) + 3;
        Ok(Compound {
            counter: self,
            first: true,
            close: 2,
        })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.1 += 512;
        self.0 += 1;
        Ok(Compound {
            counter: self,
            first: true,
            close: 1,
        })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Compound<'a>, Self::Error> {
        self.serialize_map(None)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.1 += 1536 + variant.len() * 2;
        self.0 += string_len(variant) + 3;
        Ok(Compound {
            counter: self,
            first: true,
            close: 2,
        })
    }
    fn collect_str<T: ?Sized + std::fmt::Display>(self, value: &T) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
}
macro_rules! sequence {
    ($trait:ident, $method:ident) => {
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = serde_json::Error;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
                self.comma();
                value.serialize(&mut *self.counter)
            }
            fn end(self) -> Result<(), Self::Error> {
                self.finish()
            }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
sequence!(SerializeTupleVariant, serialize_field);
impl SerializeMap for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.comma();
        let old = self.counter.0;
        self.counter.0 += match serde_json::to_value(key)? {
            serde_json::Value::String(text) => string_len(&text),
            serde_json::Value::Number(number) => number.to_string().len() + 2,
            serde_json::Value::Bool(value) => {
                if value {
                    6
                } else {
                    7
                }
            }
            _ => {
                return Err(serde_json::Error::io(io::Error::other(
                    "invalid JSON map key",
                )));
            }
        } + 1;
        self.counter.1 += 512 + self.counter.0.saturating_sub(old + 3) * 2;
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut *self.counter)
    }
    fn end(self) -> Result<(), Self::Error> {
        self.finish()
    }
}
impl SerializeStruct for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.comma();
        self.counter.1 += 512 + key.len() * 2;
        self.counter.0 += string_len(key) + 1;
        value.serialize(&mut *self.counter)
    }
    fn end(self) -> Result<(), Self::Error> {
        self.finish()
    }
}
impl ser::SerializeStructVariant for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        SerializeStruct::serialize_field(self, key, value)
    }
    fn end(self) -> Result<(), Self::Error> {
        self.finish()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_control_bytes_and_every_chunk_boundary_match_json() {
        for length in 0..80 {
            for byte in 0..=127u8 {
                let text = format!("{}{}界👨‍👩‍👧‍👦", "A".repeat(length), char::from(byte));
                assert_eq!(
                    string_len(&text),
                    serde_json::to_string(&text).unwrap().len()
                );
            }
        }
    }
    #[test]
    fn compound_variants_and_raw_json_match_encoded_size() {
        #[derive(Serialize)]
        enum Example {
            Unit,
            New(String),
            Tuple(bool, f64),
            Struct {
                number: i128,
                optional: Option<String>,
            },
        }
        let samples = [
            Example::Unit,
            Example::New("quotes\"\\\0\n".into()),
            Example::Tuple(true, -3.141e30),
            Example::Struct {
                number: i128::MIN,
                optional: Some("界".into()),
            },
        ];
        for sample in samples {
            assert_eq!(
                encoded_len(&sample).unwrap(),
                serde_json::to_vec(&sample).unwrap().len()
            );
        }
        let value = serde_json::json!({"raw": [null, true, 1, -3.4, {"\nkey": "\u{0001}value"}]});
        assert_eq!(
            encoded_len(&value).unwrap(),
            serde_json::to_vec(&value).unwrap().len()
        );
    }
}
