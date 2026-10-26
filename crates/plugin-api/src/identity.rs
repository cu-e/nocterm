//! Identifiers a package and its host agree on: the local name of a contribution and the hash
//! of a package.

use alloc::string::{String, ToString};
use core::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::LimitError;

const MAX_LOCAL: usize = 32;

/// The part of a contribution id after the plugin's namespace: `[a-z0-9-]{1,32}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalId(String);

impl LocalId {
    pub fn parse(text: &str) -> Result<Self, LimitError> {
        if text.is_empty() {
            return Err(LimitError::Empty { field: "local" });
        }
        if text.len() > MAX_LOCAL {
            return Err(LimitError::TooLong {
                field: "local",
                max: MAX_LOCAL,
            });
        }
        let plain = text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if plain {
            Ok(Self(text.to_string()))
        } else {
            Err(LimitError::Syntax { field: "local" })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for LocalId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LocalId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(de::Error::custom)
    }
}

/// A text that is not a SHA-256 hash in lowercase hexadecimal.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("not a 64-digit lowercase hexadecimal hash")]
pub struct HashError;

/// The SHA-256 of the exact bytes of a package. Grants and records are bound to it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(text: &str) -> Result<Self, HashError> {
        let digits = text.as_bytes();
        if digits.len() != 64 {
            return Err(HashError);
        }
        let mut bytes = [0u8; 32];
        let (pairs, _) = digits.as_chunks::<2>();
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (nibble(*high)? << 4) | nibble(*low)?;
        }
        Ok(Self(bytes))
    }

    /// The first 16 hexadecimal digits, which the consent dialog shows as the fingerprint.
    pub fn fingerprint(&self) -> String {
        let mut text = self.to_string();
        text.truncate(16);
        text
    }
}

fn nibble(digit: u8) -> Result<u8, HashError> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        _ => Err(HashError),
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(de::Error::custom)
    }
}
