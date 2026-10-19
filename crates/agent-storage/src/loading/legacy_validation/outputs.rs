//! Variant-specific required fields are validated before any dynamic metadata.
//! A successful shape report is NOT validation of a Block/Meta or a retained row.
use super::super::{
    held_file::{HeldFile, HeldReadError, plan::AuthenticatedPlan},
    string_reader::{JsonStringReader, StringReadError},
};
use super::{
    framing::{FrameError, Member, ObjectFrames},
    keys::Key,
    strings::{StringValidationError, validate_string},
};
use std::io::Read;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Content,
    Diff,
    Terminal,
}
#[derive(Debug, thiserror::Error)]
pub(super) enum OutputError {
    #[error("Legacy output framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("Missing legacy output field {0:?}")]
    Missing(Key),
    #[error("Duplicate legacy output field {0:?}")]
    Duplicate(Key),
    #[error("Legacy output tag is unsupported")]
    Tag,
    #[error("Legacy output string failed: {0}")]
    String(#[from] StringValidationError),
}
pub(super) struct OutputPlan {
    fields: [Option<Member>; 7],
    duplicates: u8,
}
impl OutputPlan {
    pub(super) fn scan<R: Read>(source: R, start: usize) -> Result<Self, OutputError> {
        let mut frames = ObjectFrames::new(source, start)?;
        let mut result = Self {
            fields: std::array::from_fn(|_| None),
            duplicates: 0,
        };
        while let Some(member) = frames.next_member()? {
            if let Some(index) = slot(member.kind()) {
                if result.fields[index].is_some() {
                    result.duplicates |= 1 << index;
                } else {
                    result.fields[index] = Some(member);
                }
            }
        }
        Ok(result)
    }
    fn field(&self, key: Key) -> Result<Option<&Member>, OutputError> {
        let Some(index) = slot(key) else {
            return Ok(None);
        };
        if self.duplicates & (1 << index) != 0 {
            return Err(OutputError::Duplicate(key));
        }
        Ok(self.fields[index].as_ref())
    }
    fn required(&self, key: Key) -> Result<&Member, OutputError> {
        self.field(key)?.ok_or(OutputError::Missing(key))
    }
}
fn slot(key: Key) -> Option<usize> {
    Some(match key {
        Key::Type => 0,
        Key::Content => 1,
        Key::Path => 2,
        Key::NewText => 3,
        Key::OldText => 4,
        Key::TerminalId => 5,
        Key::Meta => 6,
        _ => return None,
    })
}
/// An Output semantic error belongs to VecSkipError at the containing Call's
/// content element. Native/digest errors remain outer errors and never default.
pub(super) fn validate_shape(
    plan: &AuthenticatedPlan<OutputPlan>,
    held: &mut HeldFile,
) -> Result<Result<Kind, OutputError>, HeldReadError> {
    let tag = match plan.payload().required(Key::Type) {
        Ok(field) => field.native_span(),
        Err(error) => return Ok(Err(error)),
    };
    let kind = match plan.replay(held, |_| tag, read_kind)? {
        Ok(kind) => kind,
        Err(error) => return Ok(Err(error)),
    };
    let required: &[Key] = match kind {
        Kind::Content => &[Key::Content],
        Kind::Diff => &[Key::Path, Key::NewText],
        Kind::Terminal => &[Key::TerminalId],
    };
    // Check presence and duplicate fields before touching optional metadata.
    for key in required {
        let field = match plan.payload().required(*key) {
            Ok(field) => field,
            Err(error) => return Ok(Err(error)),
        };
        if *key != Key::Content {
            let span = field.native_span();
            if let Err(error) =
                plan.replay(held, |_| span, |reader| validate_string(reader, false))?
            {
                return Ok(Err(OutputError::String(error)));
            }
        }
    }
    if let Err(error) = plan.payload().field(Key::Meta) {
        return Ok(Err(error));
    }
    if kind == Kind::Diff {
        let old = match plan.payload().field(Key::OldText) {
            Ok(old) => old,
            Err(error) => return Ok(Err(error)),
        };
        if let Some(old) = old {
            let span = old.native_span();
            // Original DefaultOnError<Option<String>> affects oldText only.
            let _defaultable =
                plan.replay(held, |_| span, |reader| validate_string(reader, true))?;
        }
    }
    // Content's nested Block and all metadata are deliberately still pending.
    Ok(Ok(kind))
}
fn read_kind(reader: &mut dyn Read) -> Result<Kind, OutputError> {
    let mut cursor = JsonStringReader::new(reader, nocterm_ai::history::MAX_FILE_BYTES as usize)
        .map_err(StringValidationError::from)?;
    let mut prefix = [0; 8];
    let mut used = 0;
    let mut overflow = false;
    let mut output = [0; 16];
    loop {
        let count = match cursor.read(&mut output) {
            Ok(count) => count,
            Err(error) => {
                let error = error
                    .get_ref()
                    .and_then(|error| error.downcast_ref::<StringReadError>())
                    .cloned()
                    .unwrap_or_else(|| {
                        StringReadError::Source(nocterm_core::SharedError::from_error(error))
                    });
                return Err(OutputError::String(error.into()));
            }
        };
        if count == 0 {
            break;
        }
        if !overflow && count <= prefix.len() - used {
            prefix[used..used + count].copy_from_slice(&output[..count]);
            used += count;
        } else {
            overflow = true;
        }
    }
    if overflow {
        return Err(OutputError::Tag);
    }
    match &prefix[..used] {
        b"content" => Ok(Kind::Content),
        b"diff" => Ok(Kind::Diff),
        b"terminal" => Ok(Kind::Terminal),
        _ => Err(OutputError::Tag),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn check(input: &[u8]) -> Result<Kind, OutputError> {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("output.json");
        fs::write(&path, input).unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| OutputPlan::scan(reader, 0))
            .unwrap()
            .transpose()
            .unwrap();
        validate_shape(&plan, &mut held).unwrap()
    }
    #[test]
    fn missing_diff_path_is_known_before_raw_metadata_materialization() {
        let input=br#"{"_meta":{"payload":{"$serde_json::private::RawValue":"[1,2,3]"}},"newText":"new","type":"diff"}"#;
        assert!(matches!(check(input), Err(OutputError::Missing(Key::Path))));
        assert!(
            serde_json::from_slice::<agent_client_protocol_schema::v1::ToolCallContent>(input)
                .is_err()
        );
    }
    #[test]
    fn duplicate_fields_are_interpreted_only_after_late_variant_tag() {
        let terminal = br#"{"path":"a","path":"b","terminalId":"t","type":"terminal"}"#;
        assert_eq!(check(terminal).unwrap(), Kind::Terminal);
        assert!(
            serde_json::from_slice::<agent_client_protocol_schema::v1::ToolCallContent>(terminal)
                .is_ok()
        );
        let diff = br#"{"path":"a","path":"b","newText":"new","type":"diff"}"#;
        assert!(matches!(
            check(diff),
            Err(OutputError::Duplicate(Key::Path))
        ));
        assert!(
            serde_json::from_slice::<agent_client_protocol_schema::v1::ToolCallContent>(diff)
                .is_err()
        );
    }
    #[test]
    fn wrong_optional_old_text_defaults_but_new_text_is_required() {
        let input =
            br#"{"oldText":{"future":[1,2]},"path":"/tmp/a","newText":"new","type":"diff"}"#;
        assert_eq!(check(input).unwrap(), Kind::Diff);
        assert!(
            serde_json::from_slice::<agent_client_protocol_schema::v1::ToolCallContent>(input)
                .is_ok()
        );
    }
}
