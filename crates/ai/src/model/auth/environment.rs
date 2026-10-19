//! Sorted environment pairs retain measurable vector and string allocations.
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentPair {
    key: String,
    value: String,
}
impl EnvironmentPair {
    pub(crate) fn new(key: String, value: String) -> Self {
        Self { key, value }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalEnvironment(Vec<EnvironmentPair>);
impl TerminalEnvironment {
    pub(crate) fn from_unique(
        mut pairs: Vec<EnvironmentPair>,
    ) -> Result<Self, super::super::json::deferred::JsonDecodeError> {
        pairs.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        if pairs.windows(2).any(|pair| pair[0].key == pair[1].key) {
            return Err(super::super::json::deferred::JsonDecodeError::ConstructionOrder);
        }
        Ok(Self(pairs))
    }
    pub fn get(&self, key: &str) -> Option<&String> {
        self.0
            .binary_search_by(|pair| pair.key.as_str().cmp(key))
            .ok()
            .map(|index| &self.0[index].value)
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&String, &String)> {
        self.0.iter().map(|pair| (&pair.key, &pair.value))
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl From<BTreeMap<String, String>> for TerminalEnvironment {
    fn from(value: BTreeMap<String, String>) -> Self {
        Self(
            value
                .into_iter()
                .map(|(key, value)| EnvironmentPair::new(key, value))
                .collect(),
        )
    }
}
impl FromIterator<(String, String)> for TerminalEnvironment {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(values: T) -> Self {
        // This is the ordinary DTO path. Native intake uses its prepaid unique
        // source index and the consuming named constructor below the model.
        BTreeMap::from_iter(values).into()
    }
}
impl Serialize for TerminalEnvironment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for pair in &self.0 {
            map.serialize_entry(&pair.key, &pair.value)?;
        }
        map.end()
    }
}
impl<'de> Deserialize<'de> for TerminalEnvironment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BTreeMap::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_environment_retains_sorted_map_bytes_and_last_duplicate() {
        let env: TerminalEnvironment = [
            ("é", "first"),
            ("z", "middle"),
            ("é", "last"),
            ("😀", "unicode"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        assert_eq!(env.get("é").map(String::as_str), Some("last"));
        let original = BTreeMap::from([("é", "last"), ("z", "middle"), ("😀", "unicode")]);
        assert_eq!(
            serde_json::to_vec(&env).unwrap(),
            serde_json::to_vec(&original).unwrap()
        );
        assert_eq!(
            serde_json::from_slice::<TerminalEnvironment>(&serde_json::to_vec(&env).unwrap())
                .unwrap(),
            env
        );
        assert_eq!(
            env.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
            ["z", "é", "😀"]
        );
    }
    #[test]
    fn controlled_environment_refuses_unindexed_duplicate_keys() {
        assert!(
            TerminalEnvironment::from_unique(vec![
                EnvironmentPair::new("same".into(), "first".into()),
                EnvironmentPair::new("same".into(), "last".into())
            ])
            .is_err()
        );
    }
}
