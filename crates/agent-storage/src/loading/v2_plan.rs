//! Fixed-field v2 scopes; spans require authenticated parent before replay.
//! This is a shape plan, never semantic validation of framed/ignored values.
use super::{
    held_file::plan::NativeSpan,
    json_fields::Key,
    json_frames::{Element, FrameError, Member, RootFrames, Span},
};
use std::io::Read;
pub(super) mod header;
#[cfg(test)]
mod tests;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Envelope,
    Document,
    Metadata,
    Resume,
    Transcript,
    Composer,
    Entry,
    Tool,
    JsonPresence,
    Display,
    DisplayOutcome,
}
impl Scope {
    fn keys(self) -> &'static [Key] {
        use Key::*;
        match self {
            Self::Envelope => &[Format, Version, Document],
            Self::Document => &[Id, Metadata, Resume, Transcript, Composer],
            Self::Metadata => &[AgentId, Title, Name, Pinned, Updated, Model],
            Self::Resume => &[SessionId, Workdir, Config, Mode, PendingHistory],
            Self::Transcript => &[Entries, Times],
            Self::Composer => &[Draft, Queue, Attachments],
            Self::Entry => &[Kind, Value],
            Self::Tool => &[
                Id,
                Title,
                Name,
                Kind,
                Status,
                Output,
                Locations,
                Arguments,
                Result,
                Extensions,
                ClientDisplay,
            ],
            Self::JsonPresence => &[Presence, Value],
            Self::Display => &[Version, Tool, Arguments, Destination, Redacted, Outcome],
            Self::DisplayOutcome => &[OutcomePending, OutcomeOk, OutcomeErr],
        }
    }
    fn required_map(self) -> &'static [Key] {
        use Key::*;
        match self {
            Self::Envelope => &[Format, Version, Document],
            Self::Document => &[Id, Metadata, Resume, Transcript, Composer],
            Self::Metadata => &[AgentId, Pinned, Updated],
            Self::Resume => &[Config, PendingHistory],
            Self::Transcript => &[Entries],
            Self::Composer => &[],
            Self::Entry => &[Kind, Value],
            Self::Tool => &[
                Id,
                Title,
                Kind,
                Status,
                Output,
                Locations,
                Arguments,
                Result,
                ClientDisplay,
            ],
            Self::JsonPresence => &[Presence],
            Self::Display => &[Version, Tool, Arguments, Destination, Redacted, Outcome],
            Self::DisplayOutcome => &[],
        }
    }
    // Original Entry's Tagged.value is a stock Value before typed Tool decode.
    // These nested objects therefore keep last members; typed outer structs
    // and the adjacent Entry tag/content itself still reject duplicates.
    fn collapses_object_members(self) -> bool {
        matches!(
            self,
            Self::Tool | Self::JsonPresence | Self::Display | Self::DisplayOutcome
        )
    }
    fn required_sequence(self) -> usize {
        match self {
            Self::Envelope => 3,
            Self::Document => 5,
            Self::Metadata => 6,
            Self::Resume => 5,
            Self::Transcript | Self::Composer => 1,
            Self::Entry => 2,
            Self::Tool => 11,
            Self::JsonPresence => 2,
            Self::Display => 6,
            Self::DisplayOutcome => 0,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub(super) enum PlanError {
    #[error("Version-two field framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("Version-two scope {scope:?} repeats field {key:?}")]
    Duplicate { scope: Scope, key: Key },
    #[error("Version-two scope {scope:?} misses field {key:?}")]
    Missing { scope: Scope, key: Key },
    #[error("Version-two envelope contains an unknown field")]
    UnknownEnvelopeField,
    #[error("Version-two display outcome must have exactly one known variant")]
    Outcome,
    #[error("Version-two scope {0:?} has an invalid positional field count")]
    Sequence(Scope),
}
pub(super) enum Field {
    Named(Member),
    Positional(Element),
}
impl Field {
    pub(super) fn value(&self) -> Span {
        match self {
            Self::Named(member) => member.value(),
            Self::Positional(element) => element.value(),
        }
    }
    pub(super) fn native_span(&self) -> NativeSpan {
        match self {
            Self::Named(member) => member.native_span(),
            Self::Positional(element) => element.native_span(),
        }
    }
}
/// Fixed eleven slots cover the widest schema scope, without a Vec of row plans.
/// Fixed stack/native receipt costs still require the enclosing loader ledger.
pub(super) struct V2Plan {
    scope: Scope,
    fields: [Option<Field>; 11],
}
impl V2Plan {
    pub(super) fn scan<R: Read>(source: R, start: usize, scope: Scope) -> Result<Self, PlanError> {
        let mut frames = RootFrames::new_literal(source, start)?;
        let mut plan = Self {
            scope,
            fields: std::array::from_fn(|_| None),
        };
        match &mut frames {
            RootFrames::Object(frame) => {
                while let Some(member) = frame.next_member()? {
                    let key = member.kind();
                    let Some(index) = scope.keys().iter().position(|candidate| *candidate == key)
                    else {
                        if scope == Scope::Envelope {
                            return Err(PlanError::UnknownEnvelopeField);
                        }
                        if scope == Scope::DisplayOutcome {
                            return Err(PlanError::Outcome);
                        }
                        continue;
                    };
                    if plan.fields[index].is_some() && !scope.collapses_object_members() {
                        return Err(PlanError::Duplicate { scope, key });
                    }
                    plan.fields[index] = Some(Field::Named(member));
                }
                for key in scope.required_map() {
                    if plan.field(*key).is_none() {
                        return Err(PlanError::Missing { scope, key: *key });
                    }
                }
            }
            RootFrames::Array(frame) => {
                // Native routing has always required an object envelope. Nested
                // serde structs separately preserve their positional grammar.
                if matches!(scope, Scope::Envelope | Scope::DisplayOutcome) {
                    return Err(PlanError::Sequence(scope));
                }
                let mut count = 0;
                while let Some(element) = frame.next_element()? {
                    let index = element.index();
                    if index >= scope.keys().len() {
                        return Err(PlanError::Sequence(scope));
                    }
                    plan.fields[index] = Some(Field::Positional(element));
                    count += 1;
                }
                if count < scope.required_sequence() {
                    return Err(PlanError::Sequence(scope));
                }
            }
        }
        frames.finish()?;
        if scope == Scope::DisplayOutcome && plan.fields.iter().flatten().count() != 1 {
            return Err(PlanError::Outcome);
        }
        Ok(plan)
    }
    pub(super) fn field(&self, key: Key) -> Option<&Field> {
        let index = self
            .scope
            .keys()
            .iter()
            .position(|candidate| *candidate == key)?;
        self.fields[index].as_ref()
    }
}
