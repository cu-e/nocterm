//! Checked provider-neutral saved locations. References are inert nonsecret locators.
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{HostLocator, ProjectValidationError, RootPath};

pub const MAX_RESOURCE_BYTES: usize = 16 * 1024;
const MAX_RESOURCE_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProjectValidationError> {
        let value = value.into();
        if value.len() > 128
            || !value.contains('.')
            || value.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
        {
            return Err(ProjectValidationError::InvalidProvider);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Bounded inert JSON object. Debug deliberately never exposes locator contents.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProviderResourceRef(serde_json::Value);
impl ProviderResourceRef {
    pub fn new(value: serde_json::Value) -> Result<Self, ProjectValidationError> {
        if !value.is_object()
            || serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > MAX_RESOURCE_BYTES)
            || depth(&value) > MAX_RESOURCE_DEPTH
        {
            return Err(ProjectValidationError::InvalidResource);
        }
        Ok(Self(value))
    }
    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }
}
fn depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => 1 + map.values().map(depth).max().unwrap_or(0),
        serde_json::Value::Array(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}
impl fmt::Debug for ProviderResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderResourceRef(<opaque>)")
    }
}
impl<'de> Deserialize<'de> for ProviderResourceRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(serde_json::Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectTarget {
    HostFolder {
        host: HostLocator,
        root: RootPath,
    },
    Provider {
        provider: ProviderId,
        resource_version: u32,
        reference: ProviderResourceRef,
    },
}
impl ProjectTarget {
    pub fn validate(&self) -> Result<(), ProjectValidationError> {
        match self {
            Self::HostFolder { host, .. } => host.validate(),
            Self::Provider {
                resource_version: 0,
                ..
            } => Err(ProjectValidationError::InvalidResource),
            Self::Provider { .. } => Ok(()),
        }
    }
    pub fn host_folder(&self) -> Option<(&HostLocator, &RootPath)> {
        match self {
            Self::HostFolder { host, root } => Some((host, root)),
            _ => None,
        }
    }
}
