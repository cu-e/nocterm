//! `clock.now`.

use serde::{Deserialize, Serialize};

use super::{Capability, Fields, Value, ValueType, sealed};

/// The wall clock to the minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ClockNow {
    pub unix_minutes: i64,
    pub utc_offset_seconds: i32,
}

impl Fields for ClockNow {
    const FIELDS: &'static [(&'static str, ValueType)] = &[
        ("unix_minutes", ValueType::Number),
        ("utc_offset_seconds", ValueType::Number),
    ];

    fn field(&self, name: &str) -> Option<Value> {
        match name {
            "unix_minutes" => Some(Value::Number(self.unix_minutes as f64)),
            "utc_offset_seconds" => Some(Value::Number(f64::from(self.utc_offset_seconds))),
            _ => None,
        }
    }
}

/// Marker of `clock.now`.
pub struct Now;

impl sealed::Sealed for Now {}

impl Capability for Now {
    const ID: &'static str = "clock.now";
    type Value = ClockNow;
}
