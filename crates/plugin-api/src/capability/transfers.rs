//! `transfers.counts`.

use serde::{Deserialize, Serialize};

use super::{Capability, Fields, Value, ValueType, sealed};

/// How many transfers run and how many failed: no names, paths or rates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Counts {
    pub active: u16,
    pub failed: u16,
}

impl Fields for Counts {
    const FIELDS: &'static [(&'static str, ValueType)] =
        &[("active", ValueType::Number), ("failed", ValueType::Number)];

    fn field(&self, name: &str) -> Option<Value> {
        match name {
            "active" => Some(Value::Number(f64::from(self.active))),
            "failed" => Some(Value::Number(f64::from(self.failed))),
            _ => None,
        }
    }
}

/// Marker of `transfers.counts`.
pub struct CountsNow;

impl sealed::Sealed for CountsNow {}

impl Capability for CountsNow {
    const ID: &'static str = "transfers.counts";
    type Value = Counts;
}
