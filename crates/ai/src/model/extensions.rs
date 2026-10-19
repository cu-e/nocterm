use super::diagnostic::ModelError;
use super::json::{JsonReceipt, OwnedJson, RetainedAllocation};
use super::retention::RetentionAuthority;
use serde::{Deserialize, Serialize, ser::SerializeMap};
use serde_json::{Map, Value};
const DISPLAY: &str = crate::tool_display::META_KEY;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Visibility {
    Complete,
    WithoutClientDisplay,
}
/// Immutable inert facts. Filtering never clones or discards retained backing data.
#[derive(Clone, Debug)]
pub struct Extensions {
    entries: OwnedJson,
    visibility: Visibility,
}
/// Opaque ownership identity; the weak owner prevents allocation address reuse.
#[derive(Clone, Debug)]
pub struct ExtensionsIdentity {
    entries: JsonReceipt,
    visibility: Visibility,
}
impl ExtensionsIdentity {
    pub fn matches(&self, current: &Extensions) -> bool {
        self.visibility == current.visibility && self.entries.matches(&current.entries)
    }
    /// Visibility affects semantic identity, never ownership of shared backing.
    pub fn retained_allocations(&self) -> [Option<RetainedAllocation<'_>>; 2] {
        self.entries.retained_allocations()
    }
    pub fn retained_heap_bytes(&self) -> Option<usize> {
        self.entries.retained_identity_bytes()
    }
}
impl PartialEq for Extensions {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, value)| other.get(key) == Some(value))
    }
}
impl Extensions {
    /// Native controlled values have already paid their complete syntax/phase
    /// budget. No recursive compatibility normalizer or depth cap is re-applied.
    pub fn from_decoded(
        value: super::json::deferred::DecodedJson,
        ledger: &super::json::deferred::DecodeLedger,
    ) -> Result<Self, super::json::deferred::JsonDecodeError> {
        let entries = value.into_owned(ledger)?;
        if !entries.as_value().is_object() {
            return Err(super::json::deferred::JsonDecodeError::ConstructionOrder);
        }
        Ok(Self {
            entries,
            visibility: Visibility::Complete,
        })
    }
    pub fn checked(
        authority: &RetentionAuthority,
        values: Map<String, Value>,
    ) -> Result<Self, ModelError> {
        let mut nodes = 1usize;
        let mut bytes = 0usize;
        for (key, value) in &values {
            nodes = nodes.checked_add(1).ok_or(ModelError::ExtensionLimit)?;
            bytes = bytes
                .checked_add(key.len())
                .ok_or(ModelError::ExtensionLimit)?;
            measure(value, 0, &mut nodes, &mut bytes)?;
        }
        if bytes > 32 * 1024 * 1024 {
            return Err(ModelError::ExtensionLimit);
        }
        Ok(Self {
            entries: OwnedJson::from_untracked(authority, Value::Object(values))?,
            visibility: Visibility::Complete,
        })
    }
    pub(crate) fn without_client_display(mut self) -> Self {
        if self.entries.get(DISPLAY).is_some() {
            self.visibility = Visibility::WithoutClientDisplay;
        }
        self
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.entries
            .as_object()
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter(|(key, _)| self.visibility == Visibility::Complete || key.as_str() != DISPLAY)
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        if self.visibility == Visibility::WithoutClientDisplay && key == DISPLAY {
            None
        } else {
            self.entries.get(key)
        }
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub fn len(&self) -> usize {
        self.entries.as_object().map_or(0, Map::len)
            - usize::from(
                self.visibility == Visibility::WithoutClientDisplay
                    && self.entries.get(DISPLAY).is_some(),
            )
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn identity(&self) -> ExtensionsIdentity {
        ExtensionsIdentity {
            entries: self.entries.receipt().clone(),
            visibility: self.visibility,
        }
    }
    pub fn retained_allocations(&self) -> [Option<RetainedAllocation<'_>>; 2] {
        self.entries.retained_allocations()
    }
    pub fn retained_heap_bytes(&self) -> Option<usize> {
        Some(self.entries.retained_heap_bytes())
    }
}
impl Serialize for Extensions {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (key, value) in self.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
/// Explicit-context compatibility seed; the controlled native parser has its
/// own preallocation protocol and does not gain a whole-peak proof from this seed.
pub struct ExtensionsSeed<'a>(pub &'a RetentionAuthority);
impl<'de> serde::de::DeserializeSeed<'de> for ExtensionsSeed<'_> {
    type Value = Extensions;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        Extensions::checked(self.0, Map::deserialize(decoder)?).map_err(serde::de::Error::custom)
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

#[cfg(test)]
mod tests;

impl crate::model::retention::graph::sealed::Source for ExtensionsIdentity {}
impl crate::model::retention::graph::OwnerGraph for ExtensionsIdentity {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        self.entries.count(count)
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.entries.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.entries.adopt(publication);
    }
}

impl crate::model::retention::graph::sealed::Source for Extensions {}
impl crate::model::retention::graph::OwnerGraph for Extensions {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        self.entries.receipt().count(count)
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.entries.receipt().cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.entries.receipt().adopt(publication);
    }
}
