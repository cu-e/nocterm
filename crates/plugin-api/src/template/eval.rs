//! Evaluation of a checked template against a snapshot.

use alloc::{format, string::String, string::ToString};

use nocterm_plugin_model::SettingValue;

use super::{Filter, Placeholder, Segment, Source, Template};
use crate::{
    Snapshot, Tone,
    capability::{Value, notifications::OwnNotice},
};

/// The shown text of a value the host could not supply.
pub(super) const UNAVAILABLE: &str = "n/a";

/// What a template reads when it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub snapshot: &'a Snapshot,
    /// The item of the enclosing `each` list that is being drawn.
    pub item: Option<&'a OwnNotice>,
}

/// What an expression gave.
enum Evaluated {
    Value(Value),
    Text(String),
    Tone(Tone),
}

impl Template {
    /// The text with every placeholder replaced; an unavailable value shows as `n/a`.
    ///
    /// The result is not sanitized or cut: the caller applies `sanitize_display`.
    pub fn render(&self, cx: &Context<'_>) -> String {
        let mut text = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(literal) => text.push_str(literal),
                Segment::Hole(hole) => match hole.evaluate(cx) {
                    Some(Evaluated::Value(value)) => text.push_str(&show(&value)),
                    Some(Evaluated::Text(shown)) => text.push_str(&shown),
                    Some(Evaluated::Tone(_)) | None => text.push_str(UNAVAILABLE),
                },
            }
        }
        text
    }
}

impl Placeholder {
    /// The number a number expression gives, if it is available.
    pub fn number(&self, cx: &Context<'_>) -> Option<f64> {
        match self.evaluate(cx)? {
            Evaluated::Value(Value::Number(number)) if number.is_finite() => Some(number),
            _ => None,
        }
    }

    /// The switch value a boolean expression gives, if it is available.
    pub fn switch(&self, cx: &Context<'_>) -> Option<bool> {
        match self.evaluate(cx)? {
            Evaluated::Value(Value::Bool(on)) => Some(on),
            _ => None,
        }
    }

    /// The tone a threshold expression gives, if it is available.
    pub fn tone(&self, cx: &Context<'_>) -> Option<Tone> {
        match self.evaluate(cx)? {
            Evaluated::Tone(tone) => Some(tone),
            _ => None,
        }
    }

    fn evaluate(&self, cx: &Context<'_>) -> Option<Evaluated> {
        let mut current = Evaluated::Value(self.source.read(cx)?);
        for filter in &self.filters {
            current = filter.apply(current)?;
        }
        Some(current)
    }
}

impl Source {
    fn read(&self, cx: &Context<'_>) -> Option<Value> {
        match self {
            Self::Capability { capability, field } => {
                let observed = cx.snapshot.observation(capability.id)?.as_ref().ok()?;
                match field {
                    Some(field) => observed.field(field),
                    None => observed.whole(),
                }
            }
            Self::Setting(key) => match cx.snapshot.settings.get(key)? {
                SettingValue::Bool(on) => Some(Value::Bool(*on)),
                SettingValue::Choice(text) | SettingValue::Text(text) => {
                    Some(Value::Text(text.clone()))
                }
                SettingValue::Number(number) => Some(Value::Number(f64::from(*number))),
                SettingValue::Choices(_) => None,
            },
            Self::Item(field) => crate::capability::Fields::field(cx.item?, field),
        }
    }
}

impl Filter {
    fn apply(self, input: Evaluated) -> Option<Evaluated> {
        let Evaluated::Value(value) = input else {
            return None;
        };
        match (self, value) {
            (Self::Round(digits), Value::Number(number)) if number.is_finite() => Some(
                Evaluated::Text(format!("{number:.digits$}", digits = usize::from(digits))),
            ),
            (Self::Percent, Value::Number(number)) if number.is_finite() => {
                Some(Evaluated::Text(format!("{number:.0}%")))
            }
            (
                Self::HhMm,
                Value::Time {
                    unix_minutes,
                    utc_offset_seconds,
                },
            ) => {
                let local = unix_minutes + i64::from(utc_offset_seconds.div_euclid(60));
                let minute = local.rem_euclid(24 * 60);
                Some(Evaluated::Text(format!(
                    "{:02}:{:02}",
                    minute / 60,
                    minute % 60
                )))
            }
            (Self::Threshold { low, high }, Value::Number(number)) if number.is_finite() => {
                Some(Evaluated::Tone(if number < low {
                    Tone::Success
                } else if number < high {
                    Tone::Default
                } else {
                    Tone::Caution
                }))
            }
            _ => None,
        }
    }
}

fn show(value: &Value) -> String {
    match value {
        Value::Number(number) if number.is_finite() => number.to_string(),
        Value::Number(_) => UNAVAILABLE.to_string(),
        Value::Text(text) => text.clone(),
        Value::Bool(true) => "on".to_string(),
        Value::Bool(false) => "off".to_string(),
        Value::Time { .. } => UNAVAILABLE.to_string(),
    }
}
