//! Named fields of observed data, as templates read them.

use alloc::string::String;

/// The type of a value a template reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Number,
    Text,
    Bool,
    /// A wall-clock instant: only the `hh-mm` filter reads it.
    Time,
}

impl ValueType {
    /// The frozen name used in the API surface.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Text => "text",
            Self::Bool => "bool",
            Self::Time => "time",
        }
    }
}

/// A value read from observed data or a setting.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    Time {
        unix_minutes: i64,
        utc_offset_seconds: i32,
    },
}

impl Value {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::Number(_) => ValueType::Number,
            Self::Text(_) => ValueType::Text,
            Self::Bool(_) => ValueType::Bool,
            Self::Time { .. } => ValueType::Time,
        }
    }
}

/// A data transfer object whose fields a template may name.
pub trait Fields {
    /// Every field name and its type.
    const FIELDS: &'static [(&'static str, ValueType)];

    /// The value of a field, or `None` for a name outside [`Self::FIELDS`].
    fn field(&self, name: &str) -> Option<Value>;
}
