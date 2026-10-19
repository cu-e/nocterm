use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::ops::{Deref, DerefMut};

/// Inert provider metadata (`_meta`). It is never a substitute for a known field.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Extensions(Map<String, Value>);
impl Extensions {
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }
    pub fn into_map(self) -> Map<String, Value> {
        self.0
    }
}
impl From<Map<String, Value>> for Extensions {
    fn from(value: Map<String, Value>) -> Self {
        Self(value)
    }
}
impl Deref for Extensions {
    type Target = Map<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Extensions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
