//! Bounded routing inspects only the format/version fields, never a second document tree.
use nocterm_ai::history::{HistoryCodecError, MAX_FILE_BYTES, structural_bytes};
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use std::fmt;
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Format {
    LegacyV1,
    Version2,
}
#[derive(Default)]
struct Header {
    version: Option<u32>,
    format: Option<FormatMarker>,
    seen: u8,
}
#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum Field {
    Version,
    Format,
    #[serde(other)]
    Other,
}
enum FormatMarker {
    Known,
    Other,
}
impl<'de> Deserialize<'de> for FormatMarker {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Marker;
        impl<'de> Visitor<'de> for Marker {
            type Value = FormatMarker;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a format string")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<FormatMarker, E> {
                Ok(if value == nocterm_ai::history::FORMAT {
                    FormatMarker::Known
                } else {
                    FormatMarker::Other
                })
            }
        }
        decoder.deserialize_str(Marker)
    }
}
impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Header;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a chat format header")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Header, A::Error> {
                let mut header = Header::default();
                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Version => {
                            if header.seen & 1 != 0 {
                                return Err(serde::de::Error::duplicate_field("version"));
                            }
                            header.seen |= 1;
                            header.version = Some(map.next_value()?);
                        }
                        Field::Format => {
                            if header.seen & 2 != 0 {
                                return Err(serde::de::Error::duplicate_field("format"));
                            }
                            header.seen |= 2;
                            header.format = map.next_value()?;
                        }
                        Field::Other => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                if header.version.is_none() {
                    return Err(serde::de::Error::missing_field("version"));
                }
                Ok(header)
            }
        }
        decoder.deserialize_map(Fields)
    }
}
pub(super) fn classify(bytes: &[u8]) -> Result<Format, HistoryCodecError> {
    if bytes.len() > MAX_FILE_BYTES as usize {
        return Err(HistoryCodecError::EncodedLimit);
    }
    structural_bytes(bytes)?;
    let header: Header = serde_json::from_slice(bytes)?;
    classify_header(header)
}
fn classify_header(header: Header) -> Result<Format, HistoryCodecError> {
    match (header.version, header.format.as_ref(), header.seen & 2 != 0) {
        (Some(1), None, false) => Ok(Format::LegacyV1),
        (Some(2), Some(FormatMarker::Known), true) => Ok(Format::Version2),
        (Some(version), None, false) => Err(HistoryCodecError::UnsupportedVersion(version)),
        (Some(version), Some(FormatMarker::Known), true) => {
            Err(HistoryCodecError::UnsupportedVersion(version))
        }
        _ => Err(HistoryCodecError::FormatMismatch),
    }
}
#[cfg(test)]
pub(super) struct RouteSeed;
#[cfg(test)]
impl<'de> serde::de::DeserializeSeed<'de> for RouteSeed {
    type Value = Result<Format, HistoryCodecError>;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        Header::deserialize(decoder).map(classify_header)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_new_envelopes_never_fall_back_to_legacy_import() {
        assert!(matches!(
            classify(br#"{"version":1}"#).unwrap(),
            Format::LegacyV1
        ));
        assert!(matches!(
            classify(br#"{"version":2,"format":"nocterm.chat"}"#).unwrap(),
            Format::Version2
        ));
        for bytes in [
            br#"{"version":1,"format":null}"#.as_slice(),
            br#"{"version":1,"format":"other"}"#,
            br#"{"version":2}"#,
            br#"{"version":1,"version":1}"#,
            br#"{"version":2,"format":"nocterm.chat","format":"nocterm.chat"}"#,
        ] {
            assert!(classify(bytes).is_err());
        }
    }
}
