//! `monitor.usage`.

use serde::{Deserialize, Serialize};

use super::{Capability, Fields, Value, ValueType, sealed};
use crate::LimitError;

/// Processor and memory use of the host that runs Nocterm, in percent: no processes, no names.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Usage {
    pub cpu_percent: f32,
    pub memory_percent: f32,
}

impl Usage {
    /// Both values are from 0 to 100.
    pub fn check(&self) -> Result<(), LimitError> {
        let percent = |value: f32| (0.0..=100.0).contains(&value);
        if percent(self.cpu_percent) && percent(self.memory_percent) {
            Ok(())
        } else {
            Err(LimitError::Range {
                field: "usage",
                max: 100,
            })
        }
    }
}

impl Fields for Usage {
    const FIELDS: &'static [(&'static str, ValueType)] = &[
        ("cpu_percent", ValueType::Number),
        ("memory_percent", ValueType::Number),
    ];

    fn field(&self, name: &str) -> Option<Value> {
        match name {
            "cpu_percent" => Some(Value::Number(f64::from(self.cpu_percent))),
            "memory_percent" => Some(Value::Number(f64::from(self.memory_percent))),
            _ => None,
        }
    }
}

/// Marker of `monitor.usage`.
pub struct UsageNow;

impl sealed::Sealed for UsageNow {}

impl Capability for UsageNow {
    const ID: &'static str = "monitor.usage";
    type Value = Usage;
}
