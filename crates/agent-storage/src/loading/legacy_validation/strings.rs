//! Executes original required/optional String semantics without full string scratch.
use super::super::{
    held_file::{HeldFile, HeldReadError, plan::AuthenticatedPlan},
    string_reader::{JsonStringReader, StringReadError},
};
use super::{keys::Key, plan::CallPlan, scopes::ScopeError};
use nocterm_core::SharedError;
use std::io::{self, Read};
#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum StringValidationError {
    #[error("Legacy call scope failed: {0}")]
    Scope(#[from] ScopeError),
    #[error("Legacy string failed: {0}")]
    String(#[from] StringReadError),
    #[error("Legacy optional string failed: {0}")]
    Json(#[source] SharedError<serde_json::Error>),
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum OptionalString {
    Absent,
    Present,
    Defaulted,
}
/// Only these three Call string fields have been semantically validated. Other
/// dynamic fields remain unvalidated and cannot be materialized from this token.
pub(super) struct CallStrings {
    name: OptionalString,
}
impl CallStrings {
    pub(super) fn name(&self) -> &OptionalString {
        &self.name
    }
}
pub(super) fn validate(
    plan: &AuthenticatedPlan<CallPlan>,
    held: &mut HeldFile,
) -> Result<Result<CallStrings, StringValidationError>, HeldReadError> {
    for key in [Key::ToolCallId, Key::Title] {
        let Some(field) = plan.payload().field(key) else {
            return Ok(Err(ScopeError::Missing(key).into()));
        };
        let span = field.native_span();
        let checked = plan.replay(held, |_| span, |reader| validate_string(reader, false))?;
        if let Err(error) = checked {
            return Ok(Err(error));
        }
    }
    let name = if let Some(field) = plan.payload().field(Key::Name) {
        let span = field.native_span();
        match plan.replay(held, |_| span, |reader| validate_string(reader, true))? {
            Ok(true) => OptionalString::Present,
            Ok(false) | Err(_) => OptionalString::Defaulted,
        }
    } else {
        OptionalString::Absent
    };
    Ok(Ok(CallStrings { name }))
}
/// Returns false for explicit null at the original optional field boundary.
pub(super) fn validate_string(
    reader: &mut dyn Read,
    nullable: bool,
) -> Result<bool, StringValidationError> {
    let mut first = [0];
    loop {
        match reader.read(&mut first) {
            Ok(0) => return Err(StringReadError::Truncated.into()),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(StringReadError::Source(SharedError::from_error(error)).into());
            }
        }
    }
    let source = io::Cursor::new(first).chain(reader);
    if nullable && first[0] != b'"' {
        serde_json::from_reader::<_, ()>(source)
            .map_err(|error| StringValidationError::Json(SharedError::from_error(error)))?;
        return Ok(false);
    }
    let mut cursor = JsonStringReader::new(source, nocterm_ai::history::MAX_FILE_BYTES as usize)?;
    let mut output = [0; 64];
    loop {
        match cursor.read(&mut output) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                let error = error
                    .get_ref()
                    .and_then(|error| error.downcast_ref::<StringReadError>())
                    .cloned()
                    .unwrap_or_else(|| StringReadError::Source(SharedError::from_error(error)));
                return Err(error.into());
            }
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn actual_call_strings_stream_and_optional_wrong_shape_defaults_locally() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("call.json");
        let text = "x".repeat(1024 * 1024);
        let input = format!(
            "{{\"toolCallId\":\"t\",\"title\":\"{}\",\"name\":{{\"future\":[1,2]}}}}",
            text
        );
        fs::write(&path, input).unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| CallPlan::scan(reader, 0))
            .unwrap()
            .transpose()
            .unwrap();
        let strings = validate(&plan, &mut held).unwrap().unwrap();
        assert_eq!(strings.name(), &OptionalString::Defaulted);
    }
    #[test]
    fn required_title_wrong_shape_is_a_call_failure_instead_of_field_default() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("call.json");
        fs::write(&path, br#"{"toolCallId":"t","title":null,"name":"ok"}"#).unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| CallPlan::scan(reader, 0))
            .unwrap()
            .transpose()
            .unwrap();
        assert!(validate(&plan, &mut held).unwrap().is_err());
    }
}
