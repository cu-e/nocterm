//! Original legacy error ownership, selected before decoding dynamic descendants.
//! Syntax/native/digest failures remain outside these semantic fallback policies.
use super::{framing::Span, keys::Key};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Policy {
    /// `toolCallId` and `title` are required String fields in the original Call.
    RequiredString,
    /// Existing DefaultOnError<Option<String>>; absence and semantic error => None.
    OptionalName,
    /// Existing typed enum; semantic failure defaults at this field only.
    KindDefault,
    StatusDefault,
    /// Existing DefaultOnError<VecSkipError<Output>>: invalid element is skipped,
    /// wrong outer shape defaults the whole collection to empty.
    Outputs,
    Locations,
    /// Existing DefaultOnError<Option<Value>> / Option<Meta> boundaries.
    JsonDefault,
    MetaDefault,
    /// Serde ignored fields validate lexical JSON without applying Value semantics.
    Ignored,
}
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(super) enum ScopeError {
    #[error("Duplicate legacy tool field {0:?}")]
    Duplicate(Key),
    #[error("Missing legacy tool field {0:?}")]
    Missing(Key),
}
/// Tracks presence, including explicit null, independently of a defaulted result.
/// This mirrors derive duplicate/missing behavior; it does not decode any payload.
pub(super) struct CallScope {
    seen: u16,
}
pub(super) struct Field {
    span: Span,
    policy: Policy,
}
impl Field {
    pub(super) fn span(&self) -> Span {
        self.span
    }
    pub(super) fn policy(&self) -> Policy {
        self.policy
    }
}
impl CallScope {
    pub(super) fn new() -> Self {
        Self { seen: 0 }
    }
    pub(super) fn field(&mut self, key: Key, span: Span) -> Result<Field, ScopeError> {
        let (bit, policy) = match key {
            Key::ToolCallId => (1, Policy::RequiredString),
            Key::Title => (2, Policy::RequiredString),
            Key::Name => (4, Policy::OptionalName),
            Key::Kind => (8, Policy::KindDefault),
            Key::Status => (16, Policy::StatusDefault),
            Key::Content => (32, Policy::Outputs),
            Key::Locations => (64, Policy::Locations),
            Key::RawInput => (128, Policy::JsonDefault),
            Key::RawOutput => (256, Policy::JsonDefault),
            Key::Meta => (512, Policy::MetaDefault),
            _ => (0, Policy::Ignored),
        };
        if self.seen & bit != 0 {
            return Err(ScopeError::Duplicate(key));
        }
        self.seen |= bit;
        Ok(Field { span, policy })
    }
    pub(super) fn finish(self) -> Result<(), ScopeError> {
        if self.seen & 1 == 0 {
            return Err(ScopeError::Missing(Key::ToolCallId));
        }
        if self.seen & 2 == 0 {
            return Err(ScopeError::Missing(Key::Title));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::super::{framing::ObjectFrames, keys::read_key};
    use super::*;
    use std::io::Cursor;
    fn fields(input: &[u8]) -> Result<Vec<Policy>, ScopeError> {
        let mut frames = ObjectFrames::new(Cursor::new(input), 0).unwrap();
        let mut scope = CallScope::new();
        let mut policies = Vec::new();
        while let Some(member) = frames.next_member().unwrap() {
            let key = member.key();
            let (key, _) = read_key(Cursor::new(&input[key.start()..]), key.len()).unwrap();
            let field = scope.field(key, member.value())?;
            let span = field.span();
            assert!(span.len() > 0);
            policies.push(field.policy());
        }
        scope.finish()?;
        Ok(policies)
    }
    #[test]
    fn call_defaults_belong_to_each_original_field_not_document() {
        let input = br#"{"toolCallId":"t","title":"title","rawInput":{"$serde_json::private::RawValue":"not json"},"content":[{"type":"diff","_meta":{}}],"future":{"$serde_json::private::RawValue":"not json"}}"#;
        assert_eq!(
            fields(input).unwrap(),
            vec![
                Policy::RequiredString,
                Policy::RequiredString,
                Policy::JsonDefault,
                Policy::Outputs,
                Policy::Ignored
            ]
        );
    }
    #[test]
    fn explicit_null_does_not_hide_derive_duplicate_or_missing_fields() {
        assert!(matches!(
            fields(br#"{"toolCallId":"t","title":"x","rawInput":null,"rawInput":{}}"#),
            Err(ScopeError::Duplicate(Key::RawInput))
        ));
        assert!(matches!(
            fields(br#"{"toolCallId":"t","rawInput":null}"#),
            Err(ScopeError::Missing(Key::Title))
        ));
    }
}
