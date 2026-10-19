//! Winner-only construction never exposes discarded descendants to LiteralBuilder.
use super::{
    ValueError,
    array_plan::ArrayPlan,
    frames::Frames,
    map_plan::MapPlan,
    source::{SemanticSpan, Sources},
    validate::ValidatedValue,
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key,
    json_frames::ObjectFrames,
    literal_reader::{
        input::Input,
        scalar::{self, Scalar},
    },
};
use nocterm_ai::model::json::deferred::{
    DecodeLedger, literal::LiteralBuilder, native::NativeNode,
};
use std::io::Read;
enum Probe {
    Scalar(Scalar),
    String,
    Array,
    Object,
    Raw(SemanticSpan),
}
fn probe(
    reader: &mut dyn Read,
    span: &SemanticSpan,
    ledger: &DecodeLedger,
) -> Result<Probe, ValueError> {
    let mut input = Input::new_managed_marker(reader);
    let first = input.nonwhite()?.ok_or(ValueError::Punctuation)?;
    input.unread(first)?;
    Ok(match first {
        b'"' => Probe::String,
        b'[' => Probe::Array,
        b'{' => {
            if span.context().map() {
                Probe::Object
            } else {
                let mut object = ObjectFrames::new_literal_managed(&mut input, span.start())?;
                match object.next_member_with_ledger(Some(ledger))? {
                    Some(member) if member.kind() == Key::RawValue => {
                        Probe::Raw(span.member(&member)?)
                    }
                    _ => Probe::Object,
                }
            }
        }
        _ => Probe::Scalar(scalar::read(&mut input, ledger)?),
    })
}
enum Action {
    Value(SemanticSpan),
    Map(MapPlan),
    Array(ArrayPlan),
    Retire(SemanticSpan),
}
struct Materializer<'a, T> {
    sources: Sources<'a, T>,
    builder: LiteralBuilder,
    actions: Frames<'a, Action>,
    ledger: &'a DecodeLedger,
}
impl<'a, T> Materializer<'a, T> {
    pub(super) fn build(
        parent: &'a AuthenticatedPlan<T>,
        held: &'a mut HeldFile,
        validated: ValidatedValue,
        ledger: &'a DecodeLedger,
    ) -> Result<NativeNode, ValueError> {
        let mut owner = Self {
            sources: Sources::new(parent, held, ledger)?,
            builder: LiteralBuilder::new(ledger)?,
            actions: Frames::new(ledger)?,
            ledger,
        };
        owner.actions.push(Action::Value(validated.into_span()))?;
        while let Some(action) = owner.actions.pop() {
            owner.step(action)?;
        }
        Ok(owner.builder.finish_native()?)
    }
    fn step(&mut self, action: Action) -> Result<(), ValueError> {
        match action {
            Action::Value(span) => self.value(span),
            Action::Retire(span) => self.sources.retire(span),
            Action::Map(mut plan) => {
                if let Some(member) = plan.next() {
                    let (key, value) = member.into_parts();
                    self.builder.key(key)?;
                    self.actions.push(Action::Map(plan))?;
                    self.actions.push(Action::Value(value))
                } else {
                    drop(plan);
                    Ok(self.builder.end_container()?)
                }
            }
            Action::Array(mut plan) => {
                if let Some(value) = plan.next() {
                    self.actions.push(Action::Array(plan))?;
                    self.actions.push(Action::Value(value))
                } else {
                    drop(plan);
                    Ok(self.builder.end_container()?)
                }
            }
        }
    }
    fn value(&mut self, span: SemanticSpan) -> Result<(), ValueError> {
        let ledger = self.ledger;
        match self
            .sources
            .replay(&span, |reader| probe(reader, &span, ledger))?
        {
            Probe::Scalar(value) => {
                match value {
                    Scalar::Null => self.builder.null()?,
                    Scalar::Boolean(v) => self.builder.boolean(v)?,
                    Scalar::Number(v) => self.builder.number(v)?,
                }
                Ok(())
            }
            Probe::String => {
                let text = self.sources.text(&span)?;
                Ok(self.builder.string(text)?)
            }
            Probe::Raw(quoted) => {
                let raw = self.sources.decoded(&quoted)?;
                self.actions.push(Action::Retire(raw.clone()))?;
                self.actions.push(Action::Value(raw))
            }
            Probe::Object => {
                let ledger = self.ledger;
                let plan = self
                    .sources
                    .replay(&span, |reader| MapPlan::read(reader, &span, ledger))?;
                self.builder.start_object()?;
                self.actions.push(Action::Map(plan))
            }
            Probe::Array => self.array(span),
        }
    }
    fn array(&mut self, span: SemanticSpan) -> Result<(), ValueError> {
        let count = self
            .sources
            .replay(&span, |reader| ArrayPlan::count(reader, &span))?;
        let ledger = self.ledger;
        let plan = self.sources.replay(&span, |reader| {
            ArrayPlan::read(reader, &span, count, ledger)
        })?;
        self.builder.start_array_exact(count)?;
        self.actions.push(Action::Array(plan))
    }
}
pub(super) fn build<T>(
    parent: &AuthenticatedPlan<T>,
    held: &mut HeldFile,
    value: ValidatedValue,
    ledger: &DecodeLedger,
) -> Result<NativeNode, ValueError> {
    Materializer::build(parent, held, value, ledger)
}
