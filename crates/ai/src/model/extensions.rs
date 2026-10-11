use super::diagnostic::ModelError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
/// Inert extensibility facts, validated before entering an owned record.
/// This is never a substitute for a known request, response or update shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Extensions(Map<String, Value>);
impl Extensions {
    pub fn checked(values: Map<String, Value>) -> Result<Self, ModelError> {
        let mut nodes = 1usize;
        let mut bytes = 0usize;
        for (key, value) in &values {
            nodes += 1;
            bytes = bytes
                .checked_add(key.len())
                .ok_or(ModelError::ExtensionLimit)?;
            measure(value, 0, &mut nodes, &mut bytes)?;
        }
        if bytes > 32 * 1024 * 1024 {
            return Err(ModelError::ExtensionLimit);
        }
        Ok(Self(values))
    }
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }
}
impl<'de> Deserialize<'de> for Extensions {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        Self::checked(Map::deserialize(decoder)?).map_err(serde::de::Error::custom)
    }
}
fn measure(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<(), ModelError> {
    *nodes = nodes.checked_add(1).ok_or(ModelError::ExtensionLimit)?;
    if depth > 128 || *nodes > (64 * 1024 * 1024) / 512 {
        return Err(ModelError::ExtensionLimit);
    }
    match value {
        Value::String(text) => {
            *bytes = bytes
                .checked_add(text.len())
                .ok_or(ModelError::ExtensionLimit)?
        }
        Value::Array(items) => {
            for item in items {
                measure(item, depth + 1, nodes, bytes)?;
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                *nodes += 1;
                *bytes = bytes
                    .checked_add(key.len())
                    .ok_or(ModelError::ExtensionLimit)?;
                measure(item, depth + 1, nodes, bytes)?;
            }
        }
        _ => {}
    }
    if *bytes > 32 * 1024 * 1024
        || nodes
            .saturating_mul(512)
            .saturating_add(bytes.saturating_mul(2))
            > 64 * 1024 * 1024
    {
        return Err(ModelError::ExtensionLimit);
    }
    Ok(())
}
