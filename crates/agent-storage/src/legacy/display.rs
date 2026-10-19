//! Moving historical presentation metadata, never an execution capability.
#[cfg(any(test, feature = "test-support"))]
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{
    controls::Presence,
    tools::{ClientDisplay, DisplayOutcome},
};
#[cfg(any(test, feature = "test-support"))]
use nocterm_ai::model::{diagnostic::ModelError, json::OwnedJson};
use serde::{Serialize, ser::SerializeMap};
#[cfg(any(test, feature = "test-support"))]
use serde_json::{Map, Value};

#[cfg(any(test, feature = "test-support"))]
const KEY: &str = "nocterm/tool-display";

/// Validate by borrowing first. Malformed, unknown-version or extended records
/// stay verbatim in inert metadata; no large arguments/outcome tree is cloned.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn take(
    authority: &RetentionAuthority,
    meta: &mut Map<String, Value>,
) -> Result<Option<ClientDisplay>, ModelError> {
    let Some(object) = meta.get(KEY).and_then(Value::as_object) else {
        return Ok(None);
    };
    if !valid(object) {
        return Ok(None);
    }
    let Value::Object(mut object) = meta
        .shift_remove(KEY)
        .ok_or(ModelError::Unsupported("validated display"))?
    else {
        unreachable!("borrowed validation established an object")
    };
    let version = object
        .remove("version")
        .and_then(|value| value.as_u64())
        .ok_or(ModelError::Unsupported("validated display version"))? as u8;
    let Value::String(tool) = object
        .remove("tool")
        .ok_or(ModelError::Unsupported("validated display tool"))?
    else {
        unreachable!("borrowed validation established a string")
    };
    let arguments = OwnedJson::from_untracked(
        authority,
        object
            .remove("arguments")
            .ok_or(ModelError::Unsupported("validated display arguments"))?,
    )?;
    let destination = presence(object.remove("destination"), |value| match value {
        Value::String(value) => value,
        _ => unreachable!("validated destination"),
    });
    let redacted = presence(object.remove("redacted"), |value| match value {
        Value::Bool(value) => value,
        _ => unreachable!("validated redaction flag"),
    });
    let outcome = match object.remove("outcome") {
        None => Presence::Absent,
        Some(Value::Null) => Presence::Null,
        Some(value) => Presence::Value(outcome(authority, value)?),
    };
    Ok(Some(ClientDisplay {
        version,
        tool,
        arguments,
        destination,
        redacted,
        outcome,
    }))
}

#[cfg(any(test, feature = "test-support"))]
fn valid(value: &Map<String, Value>) -> bool {
    if value.keys().any(|key| {
        !matches!(
            key.as_str(),
            "version" | "tool" | "arguments" | "destination" | "redacted" | "outcome"
        )
    }) {
        return false;
    }
    matches!(value.get("version").and_then(Value::as_u64), Some(1 | 2))
        && value.get("tool").is_some_and(Value::is_string)
        && value.contains_key("arguments")
        && value
            .get("destination")
            .is_none_or(|value| value.is_null() || value.is_string())
        && value.get("redacted").is_none_or(Value::is_boolean)
        && value.get("outcome").is_none_or(valid_outcome)
}

#[cfg(any(test, feature = "test-support"))]
fn valid_outcome(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value == "Pending",
        Value::Object(value) if value.len() == 1 => {
            value.contains_key("Ok") || value.get("Err").is_some_and(Value::is_string)
        }
        _ => false,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn presence<T>(value: Option<Value>, convert: impl FnOnce(Value) -> T) -> Presence<T> {
    match value {
        None => Presence::Absent,
        Some(Value::Null) => Presence::Null,
        Some(value) => Presence::Value(convert(value)),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn outcome(authority: &RetentionAuthority, value: Value) -> Result<DisplayOutcome, ModelError> {
    Ok(match value {
        Value::String(_) => DisplayOutcome::Pending,
        Value::Object(mut value) => {
            if let Some(value) = value.remove("Ok") {
                DisplayOutcome::Succeeded(OwnedJson::from_untracked(authority, value)?)
            } else if let Some(Value::String(value)) = value.remove("Err") {
                DisplayOutcome::Failed(value)
            } else {
                unreachable!("validated outcome")
            }
        }
        _ => unreachable!("validated outcome"),
    })
}

/// The original metadata grammar, borrowing the moved neutral payload. Presence
/// is explicit so absent/null/false never turn into one another during fallback.
pub(super) struct Borrowed<'a>(
    pub(super) &'a ClientDisplay,
    pub(super) super::order::FieldOrder,
);
impl Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = self.0;
        let mut map = serializer.serialize_map(None)?;
        for field_name in self.1.iter() {
            use super::order::DisplayField::*;
            match field_name {
                Version => map.serialize_entry("version", &value.version)?,
                Tool => map.serialize_entry("tool", &value.tool)?,
                Arguments => map.serialize_entry("arguments", &value.arguments)?,
                Destination => field(&mut map, "destination", &value.destination)?,
                Redacted => field(&mut map, "redacted", &value.redacted)?,
                Outcome => match &value.outcome {
                    Presence::Absent => {}
                    Presence::Null => map.serialize_entry("outcome", &Option::<u8>::None)?,
                    Presence::Value(outcome) => {
                        map.serialize_entry("outcome", &BorrowedOutcome(outcome))?
                    }
                },
            }
        }
        map.end()
    }
}
fn field<M: SerializeMap, T: Serialize>(
    map: &mut M,
    key: &'static str,
    value: &Presence<T>,
) -> Result<(), M::Error> {
    match value {
        Presence::Absent => Ok(()),
        Presence::Null => map.serialize_entry(key, &Option::<u8>::None),
        Presence::Value(value) => map.serialize_entry(key, value),
    }
}
struct BorrowedOutcome<'a>(&'a DisplayOutcome);
impl Serialize for BorrowedOutcome<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            DisplayOutcome::Pending => serializer.serialize_str("Pending"),
            DisplayOutcome::Succeeded(value) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("Ok", value)?;
                map.end()
            }
            DisplayOutcome::Failed(value) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("Err", value)?;
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_presentation_preserves_missing_null_false_and_literal_outcomes() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        for optional in [
            json!({}),
            json!({"destination":null,"outcome":null}),
            json!({"destination":"old profile","redacted":false,"outcome":"Pending"}),
            json!({"redacted":true,"outcome":{"Ok":null}}),
            json!({"outcome":{"Err":"historical refusal"}}),
        ] {
            let mut original = json!({"version":2,"tool":"unknown/future","arguments":null});
            original
                .as_object_mut()
                .unwrap()
                .extend(optional.as_object().unwrap().clone());
            let mut meta = Map::from_iter([(KEY.into(), original.clone())]);
            let converted = take(&authority, &mut meta).unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(Borrowed(
                    &converted,
                    super::super::order::FieldOrder::NATIVE
                ))
                .unwrap(),
                original
            );
            assert!(meta.is_empty());
        }
    }

    #[test]
    fn malformed_or_extended_presentation_stays_inert_and_unchanged() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        for value in [
            json!({"version":2}),
            json!({"version":3,"tool":"x","arguments":{}}),
            json!({"version":2,"tool":"x","arguments":{},"redacted":null}),
            json!({"version":2,"tool":"x","arguments":{},"outcome":{"Err":4}}),
            json!({"version":2,"tool":"x","arguments":{},"future":1}),
        ] {
            let mut meta = Map::from_iter([(KEY.into(), value.clone())]);
            assert!(take(&authority, &mut meta).unwrap().is_none());
            assert_eq!(meta.get(KEY), Some(&value));
        }
    }

    #[test]
    fn large_arguments_are_moved_without_a_second_owned_string() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        let arguments = "a".repeat(600 * 1024);
        let pointer = arguments.as_ptr();
        let display = Value::Object(Map::from_iter([
            ("version".into(), Value::from(1)),
            ("tool".into(), Value::String("historical".into())),
            ("arguments".into(), Value::String(arguments)),
        ]));
        let mut meta = Map::from_iter([(KEY.into(), display)]);
        let converted = take(&authority, &mut meta).unwrap().unwrap();
        assert_eq!(converted.arguments.as_str().unwrap().as_ptr(), pointer);
        // A version-one row is retained as presentation evidence even when its
        // tool is unknown. Execution still requires fresh live authority.
        assert_eq!(converted.version, 1);
    }
}
