//! Exact compact JSON byte count without allocating the encoded payload.
mod key;
mod primitive;
mod text;
use serde::{
    Serialize,
    ser::{self, SerializeMap, SerializeSeq, SerializeStruct},
};

/// Exact destination-format costs; structural bytes follow the JSON preflight metric.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatSize {
    pub encoded_bytes: usize,
    pub structural_bytes: usize,
}
#[derive(Debug, thiserror::Error)]
pub enum MeasurementError {
    #[error("retained allocation measurement failed: {0}")]
    Retention(#[from] crate::model::retention::RetentionError),
    #[error("JSON measurement failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("JSON measurement formatting failed: {0}")]
    Formatting(#[from] std::fmt::Error),
    #[error("JSON measurement arithmetic overflow")]
    ArithmeticOverflow,
}
impl ser::Error for MeasurementError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self::Json(<serde_json::Error as ser::Error>::custom(message))
    }
}
/// Counts a borrowed serializer directly; no encoded document or Value tree is built.
pub fn measure(value: &impl Serialize) -> Result<FormatSize, MeasurementError> {
    let mut counter = Counter(0, 0, Policy::Format);
    value.serialize(&mut counter)?;
    Ok(FormatSize {
        encoded_bytes: counter.0,
        structural_bytes: counter.1,
    })
}
pub(super) fn encoded_len(value: &impl Serialize) -> Result<usize, MeasurementError> {
    measure(value).map(|size| size.encoded_bytes)
}
pub(super) fn structural_size(value: &impl Serialize) -> Result<usize, MeasurementError> {
    measure(value).map(|size| size.structural_bytes)
}
/// Examine sixteen bytes at once; ordinary UTF-8 and base64 need no escaping.
fn string_len(text: &str) -> Result<usize, MeasurementError> {
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
    let mut length = text
        .len()
        .checked_add(2)
        .ok_or(MeasurementError::ArithmeticOverflow)?;
    let (chunks, remainder) = text.as_bytes().as_chunks::<16>();
    for chunk in chunks {
        let word = u128::from_ne_bytes(*chunk);
        let control = !(word & LOW).wrapping_add(SIXTY) & !word & HIGH != 0;
        if control
            || zero(word ^ u128::from_ne_bytes([b'"'; 16]))
            || zero(word ^ u128::from_ne_bytes([b'\\'; 16]))
        {
            length = length
                .checked_add(chunk.iter().copied().map(extra).sum::<usize>())
                .ok_or(MeasurementError::ArithmeticOverflow)?;
        }
    }
    length
        .checked_add(remainder.iter().copied().map(extra).sum::<usize>())
        .ok_or(MeasurementError::ArithmeticOverflow)
}
/// Conservative retained-allocation budget, independent of destination envelope/key spelling.
/// Scalars cover vector growth, compound enums cover the largest inline variant,
/// and strings include their source/decode lifetime. It is not serialized structural cost.
pub fn retained(value: &impl Serialize) -> Result<usize, MeasurementError> {
    let mut counter = Counter(0, 0, Policy::Retained);
    value.serialize(&mut counter)?;
    Ok(counter.1)
}
#[derive(Clone, Copy)]
enum Policy {
    Format,
    Retained,
}
impl Policy {
    fn scalar(self) -> usize {
        match self {
            Self::Format => 512,
            Self::Retained => 64,
        }
    }
    fn container(self) -> usize {
        match self {
            Self::Format => 512,
            Self::Retained => 128,
        }
    }
    fn map(self, len: Option<usize>) -> usize {
        match self {
            Self::Format => 512,
            // Empty maps allocate no B-tree node. Non-empty JSON maps need an
            // initial node reserve distinct from the inline struct allowance;
            // amortized per-key costs account for subsequent nodes.
            Self::Retained if len == Some(0) => 128,
            Self::Retained => 1024,
        }
    }
    fn key(self) -> usize {
        match self {
            Self::Format => 512,
            Self::Retained => 128,
        }
    }
}
struct Counter(usize, usize, Policy);

impl Counter {
    fn add(&mut self, encoded: usize, structural: usize) -> Result<(), MeasurementError> {
        self.0 = self
            .0
            .checked_add(encoded)
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        self.1 = self
            .1
            .checked_add(structural)
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        Ok(())
    }
    fn wrapper(&mut self, variant: &str, structural_prefix: usize) -> Result<(), MeasurementError> {
        let encoded = string_len(variant)?
            .checked_add(3)
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        let text = match self.2 {
            Policy::Format => variant
                .len()
                .checked_mul(2)
                .and_then(|n| n.checked_add(structural_prefix))
                .ok_or(MeasurementError::ArithmeticOverflow)?,
            Policy::Retained => 2048,
        };
        self.add(encoded, text)
    }
}
struct Compound<'a> {
    counter: &'a mut Counter,
    first: bool,
    close: usize,
}
impl Compound<'_> {
    fn comma(&mut self) -> Result<(), MeasurementError> {
        if self.first {
            self.first = false;
        } else {
            self.counter.add(1, 0)?;
        }
        Ok(())
    }
    fn finish(self) -> Result<(), MeasurementError> {
        self.counter.add(self.close, 0)
    }
}
macro_rules! number {
    ($name:ident, $ty:ty) => {
        fn $name(self, value: $ty) -> Result<(), Self::Error> {
            self.add(primitive::encoded(&value)?, self.2.scalar())
        }
    };
}
impl<'a> ser::Serializer for &'a mut Counter {
    type Ok = ();
    type Error = MeasurementError;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;
    fn serialize_bool(self, value: bool) -> Result<(), Self::Error> {
        self.add(if value { 4 } else { 5 }, self.2.scalar())
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
        let structural = value
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(self.2.scalar()))
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        self.add(string_len(value)?, structural)
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
        self.add(4, self.2.scalar())
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
        self.wrapper(variant, 1024)?;
        value.serialize(&mut *self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.add(1, self.2.container())?;
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
        self.wrapper(variant, 1536)?;
        Ok(Compound {
            counter: self,
            first: true,
            close: 2,
        })
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.add(1, self.2.map(len))?;
        Ok(Compound {
            counter: self,
            first: true,
            close: 1,
        })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Compound<'a>, Self::Error> {
        self.add(1, self.2.container())?;
        Ok(Compound {
            counter: self,
            first: true,
            close: 1,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.wrapper(variant, 1536)?;
        Ok(Compound {
            counter: self,
            first: true,
            close: 2,
        })
    }
    fn collect_str<T: ?Sized + std::fmt::Display>(self, value: &T) -> Result<(), Self::Error> {
        let size = text::measure(value)?;
        let structural = size
            .decoded_utf8
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(self.2.scalar()))
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        self.add(size.encoded, structural)
    }
}
macro_rules! sequence {
    ($trait:ident, $method:ident) => {
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = MeasurementError;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
                self.comma()?;
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
    type Error = MeasurementError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.comma()?;
        let key = key::measure(key)?;
        let encoded = key.encoded_with_colon;
        let structural = key
            .decoded_utf8
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(self.counter.2.key()))
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        self.counter.add(encoded, structural)?;
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
    type Error = MeasurementError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.comma()?;
        let structural = match self.counter.2 {
            Policy::Format => key
                .len()
                .checked_mul(2)
                .and_then(|n| n.checked_add(512))
                .ok_or(MeasurementError::ArithmeticOverflow)?,
            Policy::Retained => 0,
        };
        let encoded = string_len(key)?
            .checked_add(1)
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        self.counter.add(encoded, structural)?;
        value.serialize(&mut *self.counter)
    }
    fn end(self) -> Result<(), Self::Error> {
        self.finish()
    }
}
impl ser::SerializeStructVariant for Compound<'_> {
    type Ok = ();
    type Error = MeasurementError;
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
    fn retained_dynamic_maps_reserve_nodes_without_counting_static_struct_keys() {
        #[derive(Serialize)]
        struct Inline {
            present: bool,
        }
        let dynamic = serde_json::json!({"present": true});
        let inline = Inline { present: true };
        assert_eq!(measure(&dynamic).unwrap(), measure(&inline).unwrap());
        assert_eq!(retained(&inline).unwrap(), 128 + 64);
        assert_eq!(
            retained(&dynamic).unwrap(),
            1024 + 128 + 2 * "present".len() + 64
        );
        assert_eq!(retained(&serde_json::json!({})).unwrap(), 128);
        // This covers metric policy. Complete allocation/capacity domination
        // and imported legacy boundary behavior remain separate requirements.
    }

    #[test]
    fn unicode_control_bytes_and_every_chunk_boundary_match_json() {
        for length in 0..80 {
            for byte in 0..=127u8 {
                let text = format!("{}{}界👨‍👩‍👧‍👦", "A".repeat(length), char::from(byte));
                assert_eq!(
                    string_len(&text).unwrap(),
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
                structural_size(&sample).unwrap(),
                super::super::structural_bytes(&serde_json::to_vec(&sample).unwrap()).unwrap()
            );
            assert_eq!(
                encoded_len(&sample).unwrap(),
                serde_json::to_vec(&sample).unwrap().len()
            );
        }
        let value = serde_json::json!({"raw": [null, true, 1, -3.4, {"\nkey": "\u{0001}value"}]});
        assert_eq!(
            structural_size(&value).unwrap(),
            super::super::structural_bytes(&serde_json::to_vec(&value).unwrap()).unwrap()
        );
        assert_eq!(
            encoded_len(&value).unwrap(),
            serde_json::to_vec(&value).unwrap().len()
        );
    }
    #[test]
    fn external_newtype_and_escaped_map_keys_count_actual_decoded_nodes() {
        #[derive(Serialize)]
        enum Row {
            Record(std::collections::BTreeMap<String, String>),
        }
        let key = "\n\"\\界\u{0000}";
        let value = Row::Record([(key.to_owned(), "\tvalue".to_owned())].into());
        let bytes = serde_json::to_vec(&value).unwrap();
        let measured = measure(&value).unwrap();
        assert_eq!(measured.encoded_bytes, bytes.len());
        assert_eq!(
            measured.structural_bytes,
            super::super::structural_bytes(&bytes).unwrap()
        );
        // Root object + variant key + nested map + its key + value string.
        assert_eq!(
            measured.structural_bytes,
            5 * 512 + 2 * ("Record".len() + key.len() + "\tvalue".len())
        );
    }
    #[test]
    fn overflow_is_typed_and_never_a_format_limit_fallback() {
        let mut encoded = Counter(usize::MAX, 0, Policy::Format);
        assert!(matches!(
            encoded.add(1, 0),
            Err(MeasurementError::ArithmeticOverflow)
        ));
        let mut structural = Counter(0, usize::MAX, Policy::Format);
        assert!(matches!(
            structural.add(0, 1),
            Err(MeasurementError::ArithmeticOverflow)
        ));
        assert!(matches!(
            super::super::HistoryCodecError::from(MeasurementError::ArithmeticOverflow),
            super::super::HistoryCodecError::ArithmeticOverflow
        ));
    }
    #[test]
    fn real_json_key_rules_preserve_scalar_variants_and_reject_composites_before_payload() {
        #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
        enum Key {
            Unit,
        }
        let map = std::collections::BTreeMap::from([(Key::Unit, "kept")]);
        let bytes = serde_json::to_vec(&map).unwrap();
        assert_eq!(measure(&map).unwrap().encoded_bytes, bytes.len());
        assert_eq!(
            measure(&map).unwrap().structural_bytes,
            super::super::structural_bytes(&bytes).unwrap()
        );
        struct Composite;
        impl Serialize for Composite {
            fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let _sequence = serializer.serialize_seq(Some(usize::MAX))?;
                panic!("invalid map key payload must never be traversed");
            }
        }
        struct InvalidMap;
        impl Serialize for InvalidMap {
            fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(&Composite, &())?;
                map.end()
            }
        }
        assert!(serde_json::to_vec(&InvalidMap).is_err());
        assert!(matches!(
            measure(&InvalidMap),
            Err(MeasurementError::Json(_))
        ));
    }
    #[test]
    fn streamed_dynamic_key_and_display_metrics_match_actual_json() {
        #[derive(PartialEq, Eq, PartialOrd, Ord)]
        struct Formatted<'a>(&'a str);
        impl std::fmt::Display for Formatted<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                use std::fmt::Write;
                f.write_str("prefix\"\n")?;
                f.write_char('😀')?;
                f.write_str(self.0)?;
                f.write_str("\0suffix")
            }
        }
        impl Serialize for Formatted<'_> {
            fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        let key = "quoted\"/slash\\/NUL\0/é😀\n".repeat(32768);
        let map = std::collections::BTreeMap::from([(Formatted(&key), Formatted("value\n"))]);
        let bytes = serde_json::to_vec(&map).unwrap();
        let measured = measure(&map).unwrap();
        assert_eq!(measured.encoded_bytes, bytes.len());
        assert_eq!(
            measured.structural_bytes,
            super::super::structural_bytes(&bytes).unwrap()
        );
        let value = Formatted(&key);
        let actual = value.to_string();
        assert_eq!(retained(&value).unwrap(), retained(&actual).unwrap());
    }
    #[test]
    fn primitive_measurements_preserve_full_integer_and_float_formatting() {
        #[derive(Serialize)]
        struct Values {
            minimum: i128,
            maximum: u128,
            numbers: [f64; 5],
            smallest: f32,
        }
        let values = Values {
            minimum: i128::MIN,
            maximum: u128::MAX,
            numbers: [
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                -0.0,
                f64::MIN_POSITIVE,
            ],
            smallest: f32::from_bits(1),
        };
        let bytes = serde_json::to_vec(&values).unwrap();
        let measured = measure(&values).unwrap();
        assert_eq!(measured.encoded_bytes, bytes.len());
        assert_eq!(
            measured.structural_bytes,
            super::super::structural_bytes(&bytes).unwrap()
        );
    }
    #[test]
    fn invalid_display_returns_its_typed_formatting_cause() {
        struct Broken;
        impl std::fmt::Display for Broken {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                Err(std::fmt::Error)
            }
        }
        impl Serialize for Broken {
            fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        assert!(matches!(
            measure(&Broken),
            Err(MeasurementError::Formatting(_))
        ));
    }
}
