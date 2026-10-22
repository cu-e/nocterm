use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// Supplied stable identity, independent of project names, hosts and paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectId([u8; 16]);

/// Separate General and per-project UI/history membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ContextId {
    General,
    Project(ProjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectIdParseError {
    #[error("identity must contain exactly 32 hexadecimal characters")]
    InvalidShape,
}

fn parse_bytes(text: &str) -> Result<[u8; 16], ProjectIdParseError> {
    if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ProjectIdParseError::InvalidShape);
    }
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| ProjectIdParseError::InvalidShape)?;
    }
    Ok(bytes)
}

macro_rules! identity {
    ($type:ident) => {
        impl $type {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn to_bytes(self) -> [u8; 16] {
                self.0
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(output, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl FromStr for $type {
            type Err = ProjectIdParseError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                parse_bytes(text).map(Self)
            }
        }

        impl Serialize for $type {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

identity!(ProjectId);
