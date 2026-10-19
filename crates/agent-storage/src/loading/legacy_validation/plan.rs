//! A bounded current-row plan: ten actual Call slots, no whole-row/index vector.
use super::{
    framing::{Element, FrameError, Member, RootFrames, Span, SpanDigest},
    keys::Key,
    scopes::{CallScope, Policy, ScopeError},
};
use std::io::Read;
#[derive(Debug, thiserror::Error)]
pub(super) enum PlanError {
    #[error("Legacy call framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("Legacy call scope failed: {0}")]
    Scope(#[from] ScopeError),
    #[error("Too many legacy Call sequence fields")]
    SequenceLength,
}
/// Every receipt here is still provisional until its enclosing scan is authenticated.
pub(super) struct CallPlan {
    fields: [Option<CallMember>; 10],
}
impl CallPlan {
    pub(super) fn scan<R: Read>(source: R, start: usize) -> Result<Self, PlanError> {
        let mut scope = CallScope::new();
        let mut plan = Self {
            fields: std::array::from_fn(|_| None),
        };
        match RootFrames::new(source, start)? {
            RootFrames::Object(mut frames) => {
                while let Some(member) = frames.next_member()? {
                    plan.record(&mut scope, member.kind(), CallMember::Named(member))?;
                }
            }
            RootFrames::Array(mut frames) => {
                while let Some(element) = frames.next_element()? {
                    let key = sequence_key(element.index()).ok_or(PlanError::SequenceLength)?;
                    plan.record(&mut scope, key, CallMember::Positional(element))?;
                }
            }
        }
        scope.finish()?;
        Ok(plan)
    }
    fn record(
        &mut self,
        scope: &mut CallScope,
        key: Key,
        member: CallMember,
    ) -> Result<(), PlanError> {
        let field = scope.field(key, member.value())?;
        if field.policy() != Policy::Ignored
            && let Some(slot) = slot(key)
        {
            self.fields[slot] = Some(member);
        }
        Ok(())
    }
    pub(super) fn field(&self, key: Key) -> Option<&CallMember> {
        self.fields[slot(key)?].as_ref()
    }
}
/// Named and positional input remain distinct; a sequence never invents encoded
/// field names or changes the digest provenance of its physical value bytes.
pub(super) enum CallMember {
    Named(Member),
    Positional(Element),
}
impl CallMember {
    pub(super) fn value(&self) -> Span {
        match self {
            Self::Named(v) => v.value(),
            Self::Positional(v) => v.value(),
        }
    }
    pub(super) fn receipt(&self) -> &SpanDigest {
        match self {
            Self::Named(v) => v.receipt(),
            Self::Positional(v) => v.receipt(),
        }
    }
    pub(super) fn native_span(&self) -> super::super::held_file::plan::NativeSpan {
        match self {
            Self::Named(v) => v.native_span(),
            Self::Positional(v) => v.native_span(),
        }
    }
}
fn sequence_key(index: usize) -> Option<Key> {
    [
        Key::ToolCallId,
        Key::Title,
        Key::Name,
        Key::Kind,
        Key::Status,
        Key::Content,
        Key::Locations,
        Key::RawInput,
        Key::RawOutput,
        Key::Meta,
    ]
    .get(index)
    .copied()
}
fn slot(key: Key) -> Option<usize> {
    Some(match key {
        Key::ToolCallId => 0,
        Key::Title => 1,
        Key::Name => 2,
        Key::Kind => 3,
        Key::Status => 4,
        Key::Content => 5,
        Key::Locations => 6,
        Key::RawInput => 7,
        Key::RawOutput => 8,
        Key::Meta => 9,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::loading::held_file::HeldFile;
    use sha2::{Digest, Sha256};
    use std::{fs, io::Cursor};
    #[test]
    fn actual_call_plan_is_minted_from_a_whole_authenticated_native_scan() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("call.json");
        let input = br#"{"title":"title","raw\u0049nput":{"x":1},"toolCallId":"t"}"#;
        fs::write(&path, input).unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let authenticated = held
            .plan_pass(&stamp)
            .scan(|reader| CallPlan::scan(reader, 0))
            .unwrap();
        let plan = authenticated.payload().as_ref().unwrap();
        let member = plan.field(Key::RawInput).unwrap();
        let span = member.value();
        let covered = &input[span.start()..member.receipt().covered_end()];
        assert_eq!(
            member.receipt().digest(),
            <[u8; 32]>::from(Sha256::digest(covered))
        );
    }
    #[test]
    fn unknown_huge_keys_are_not_retained_in_the_ten_slot_plan() {
        let unknown = "x".repeat(1024 * 1024);
        let input = format!(
            "{{\"toolCallId\":\"t\",\"{}\":null,\"title\":\"x\"}}",
            unknown
        );
        let plan = CallPlan::scan(Cursor::new(input.as_bytes()), 0).unwrap();
        assert_eq!(plan.fields.iter().flatten().count(), 2);
    }
    #[test]
    fn field_replay_rejects_new_bytes_even_when_file_is_restored_before_return() {
        use crate::loading::held_file::HeldReadError;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("call.json");
        let original = br#"{"toolCallId":"t","title":"x","rawInput":{"x":1}}"#;
        let changed = br#"{"toolCallId":"t","title":"x","rawInput":{"x":2}}"#;
        fs::write(&path, original).unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| CallPlan::scan(reader, 0))
            .unwrap();
        fs::write(&path, changed).unwrap();
        let result = plan.replay(
            &mut held,
            |plan| {
                plan.as_ref()
                    .unwrap()
                    .field(Key::RawInput)
                    .unwrap()
                    .native_span()
            },
            |reader| {
                let decoded = serde_json::from_reader::<_, serde_json::Value>(reader).unwrap();
                assert_eq!(decoded["x"], 2);
                fs::write(&path, original).unwrap();
                decoded
            },
        );
        assert!(matches!(result, Err(HeldReadError::ChangedDocument)));
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn actual_field_replay_uses_its_authenticated_span_and_original_descriptor() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("call.json");
        fs::write(
            &path,
            br#"{"toolCallId":"t","title":"x","rawInput":42 ,"rawOutput":null}"#,
        )
        .unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| CallPlan::scan(reader, 0))
            .unwrap();
        let decoded = plan
            .replay(
                &mut held,
                |plan| {
                    plan.as_ref()
                        .unwrap()
                        .field(Key::RawInput)
                        .unwrap()
                        .native_span()
                },
                |reader| serde_json::from_reader::<_, u64>(reader),
            )
            .unwrap()
            .unwrap();
        assert_eq!(decoded, 42);
    }
    #[test]
    fn native_sequence_call_plan_preserves_short_and_explicit_null_tail() {
        use super::super::strings::{self, OptionalString};
        for input in [
            br#"["original","title"]"#.as_slice(),
            br#"["original","title",null,null,null,null,null,null,null,null]"#.as_slice(),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("call.json");
            fs::write(&path, input).unwrap();
            let mut held = HeldFile::open(&path).unwrap();
            let stamp = held.preflight().unwrap();
            let plan = held
                .plan_pass(&stamp)
                .scan(|reader| CallPlan::scan(reader, 0))
                .unwrap()
                .transpose()
                .unwrap();
            let strings = strings::validate(&plan, &mut held).unwrap().unwrap();
            assert_eq!(
                strings.name(),
                if input.len() < 30 {
                    &OptionalString::Absent
                } else {
                    &OptionalString::Defaulted
                }
            );
            assert!(matches!(
                plan.payload().field(Key::ToolCallId),
                Some(CallMember::Positional(_))
            ));
            assert_eq!(
                plan.payload()
                    .field(Key::ToolCallId)
                    .unwrap()
                    .value()
                    .start(),
                1
            );
        }
    }
    #[test]
    fn sequence_call_required_presence_and_extra_tail_remain_strict() {
        for input in [
            r#"[]"#,
            r#"["id"]"#,
            r#"["id","title",null,null,null,null,null,null,null,null,null]"#,
        ] {
            assert!(CallPlan::scan(Cursor::new(input.as_bytes()), 0).is_err());
        }
    }
    #[test]
    fn changed_same_length_field_is_capacity_guarded_before_span_digest_rejection() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        use crate::loading::{
            held_file::HeldReadError,
            literal_reader::{LiteralReadError, decode_retained},
        };
        use nocterm_ai::model::json::deferred::{DecodeLedger, JsonDecodeError};
        let original = br#"{"toolCallId":"t","title":"x","rawInput":"\u0061\u0061\u0061\u0061"}"#;
        let changed = br#"{"toolCallId":"t","title":"x","rawInput":"abcdefghijklmnopqrstuvwx"}"#;
        assert_eq!(original.len(), changed.len());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("call.json");
        fs::write(&path, original).unwrap();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight().unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| CallPlan::scan(reader, 0))
            .unwrap()
            .transpose()
            .unwrap();
        let baseline = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        plan.replay(
            &mut held,
            |plan| plan.field(Key::RawInput).unwrap().native_span(),
            |reader| decode_retained(reader, &baseline),
        )
        .unwrap()
        .unwrap();
        let allowance = baseline.peak_requested_bytes();
        fs::write(&path, changed).unwrap();
        let ledger = DecodeLedger::new(&authority, allowance).unwrap();
        let result = plan.replay(
            &mut held,
            |plan| plan.field(Key::RawInput).unwrap().native_span(),
            |reader| {
                let parsed = decode_retained(reader, &ledger);
                assert!(matches!(
                    parsed,
                    Err(LiteralReadError::Build(JsonDecodeError::Limit { .. }))
                ));
                assert!(ledger.peak_requested_bytes() <= allowance);
                fs::write(&path, original).unwrap();
            },
        );
        assert!(matches!(result, Err(HeldReadError::ChangedDocument)));
    }
}
