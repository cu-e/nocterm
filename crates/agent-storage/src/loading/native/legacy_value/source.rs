//! Genuine held receipts and active guarded decoded sources have distinct coordinates.
use super::{
    ValueError,
    frames::Frames,
    pump::{BytePump, DecodeFrame},
};
use crate::loading::{
    held_file::{
        HeldFile,
        plan::{AuthenticatedPlan, NativeSpan},
    },
    json_frames::{Element, Member},
    literal_reader::LiteralReadError,
    string_reader::JsonStringReader,
};
use nocterm_ai::model::json::deferred::{
    DecodeLedger, JsonDecodeError,
    literal::{ExactLiteralText, LiteralText},
};
use std::io::Read;
#[derive(Clone, Copy)]
struct RawId {
    slot: usize,
    generation: usize,
}
#[derive(Clone)]
enum Location {
    Held(NativeSpan),
    Raw {
        source: RawId,
        start: usize,
        end: usize,
    },
}
#[derive(Clone, Copy)]
pub(super) struct Context {
    depth: usize,
    map: bool,
}
#[derive(Clone)]
pub(super) struct SemanticSpan {
    location: Location,
    context: Context,
}
impl SemanticSpan {
    pub(super) fn held(span: NativeSpan, map: bool) -> Self {
        Self {
            location: Location::Held(span),
            context: Context { depth: 0, map },
        }
    }
    pub(super) fn start(&self) -> usize {
        match &self.location {
            Location::Held(v) => v.start(),
            Location::Raw { start, .. } => *start,
        }
    }
    pub(super) fn context(&self) -> Context {
        self.context
    }
    pub(super) fn member(&self, member: &Member) -> Result<Self, ValueError> {
        match &self.location {
            Location::Held(_) => Ok(Self {
                location: Location::Held(member.native_span()),
                context: self.child_context()?,
            }),
            Location::Raw { .. } => {
                self.logical_child(member.value().start(), member.value().len())
            }
        }
    }
    pub(super) fn element(&self, element: &Element) -> Result<Self, ValueError> {
        match &self.location {
            Location::Held(_) => Ok(Self {
                location: Location::Held(element.native_span()),
                context: self.child_context()?,
            }),
            Location::Raw { .. } => {
                self.logical_child(element.value().start(), element.value().len())
            }
        }
    }
    fn child_context(&self) -> Result<Context, ValueError> {
        Ok(Context {
            depth: self
                .context
                .depth
                .checked_add(1)
                .ok_or(JsonDecodeError::Arithmetic)?,
            map: false,
        })
    }
    fn logical_child(&self, start: usize, len: usize) -> Result<Self, ValueError> {
        let Location::Raw {
            source,
            start: parent_start,
            end: parent_end,
        } = &self.location
        else {
            return Err(ValueError::ForeignSource);
        };
        let end = start.checked_add(len).ok_or(JsonDecodeError::Arithmetic)?;
        if start < *parent_start || end > *parent_end {
            return Err(ValueError::ForeignSource);
        }
        Ok(Self {
            location: Location::Raw {
                source: *source,
                start,
                end,
            },
            context: self.child_context()?,
        })
    }
}
impl Context {
    pub(super) fn depth(self) -> usize {
        self.depth
    }
    pub(super) fn map(self) -> bool {
        self.map
    }
}
struct RawSource {
    quoted_parent: SemanticSpan,
    length: usize,
}
struct Slot {
    value: Option<RawSource>,
    generation: usize,
    next_free: Option<usize>,
    references: usize,
}
/// Each active decoded root has one driver ownership reference until its Retire
/// action. Each descendant recipe independently retains its authentic parent.
/// Map/Array spans borrow that driver lifetime and never own recursive ancestors.
pub(super) struct Sources<'a, T> {
    parent: &'a AuthenticatedPlan<T>,
    held: &'a mut HeldFile,
    slots: Frames<'a, Slot>,
    free: Option<usize>,
    ledger: &'a DecodeLedger,
}
impl<'a, T> Sources<'a, T> {
    pub(super) fn new(
        parent: &'a AuthenticatedPlan<T>,
        held: &'a mut HeldFile,
        ledger: &'a DecodeLedger,
    ) -> Result<Self, ValueError> {
        Ok(Self {
            parent,
            held,
            slots: Frames::new(ledger)?,
            free: None,
            ledger,
        })
    }
    fn source(&self, id: RawId) -> Result<&RawSource, ValueError> {
        let slot = self.slots.get(id.slot).ok_or(ValueError::ForeignSource)?;
        if slot.generation != id.generation || slot.references == 0 {
            return Err(ValueError::ForeignSource);
        }
        slot.value.as_ref().ok_or(ValueError::ForeignSource)
    }
    fn replay_raw<U>(
        &mut self,
        source: RawId,
        start: usize,
        end: usize,
        inspect: impl FnOnce(&mut dyn Read) -> Result<U, ValueError>,
    ) -> Result<U, ValueError> {
        if start > end || end > self.source(source)?.length {
            return Err(ValueError::ForeignSource);
        }
        let mut path = Frames::new(self.ledger)?;
        let mut current = source;
        let physical = loop {
            let raw = self.source(current)?;
            match &raw.quoted_parent.location {
                Location::Held(physical) => {
                    path.push(DecodeFrame::quoted(0, None))?;
                    break physical.clone();
                }
                Location::Raw { source, start, end } => {
                    if start > end || *end > self.source(*source)?.length {
                        return Err(ValueError::ForeignSource);
                    }
                    path.push(DecodeFrame::quoted(*start, Some(*end)))?;
                    current = *source;
                }
            }
        };
        path.reverse();
        let ledger = self.ledger;
        self.parent
            .replay(
                self.held,
                |_| physical,
                |reader| {
                    let mut pump = BytePump::with_frames(reader, path, ledger)?;
                    pump.skip(start)?;
                    let result = inspect(&mut (&mut pump).take((end - start) as u64));
                    match pump.take_failure() {
                        Some(error) => Err(ValueError::Literal(error.into())),
                        None => result,
                    }
                },
            )
            .map_err(ValueError::Held)?
    }
    pub(super) fn replay<U>(
        &mut self,
        span: &SemanticSpan,
        inspect: impl FnOnce(&mut dyn Read) -> Result<U, ValueError>,
    ) -> Result<U, ValueError> {
        match &span.location {
            Location::Held(physical) => self
                .parent
                .replay(self.held, |_| physical.clone(), inspect)
                .map_err(ValueError::Held)?,
            Location::Raw { source, start, end } => self.replay_raw(*source, *start, *end, inspect),
        }
    }
    fn text_length(&mut self, quoted: &SemanticSpan) -> Result<usize, ValueError> {
        let ledger = self.ledger;
        self.replay(quoted, |reader| {
            let mut string = JsonStringReader::new_managed(
                reader,
                nocterm_ai::history::MAX_FILE_BYTES as usize,
                ledger,
            )
            .map_err(LiteralReadError::from)?;
            let mut count = 0usize;
            while let Some(fragment) = string.next_fragment().map_err(LiteralReadError::from)? {
                count = count
                    .checked_add(fragment.len())
                    .ok_or(JsonDecodeError::Arithmetic)?;
            }
            Ok(count)
        })
    }
    pub(super) fn text(&mut self, quoted: &SemanticSpan) -> Result<LiteralText, ValueError> {
        let length = self.text_length(quoted)?;
        let ledger = self.ledger;
        self.replay(quoted, |reader| {
            let mut text = ExactLiteralText::new(ledger, length)?;
            let mut string = JsonStringReader::new_managed(
                reader,
                nocterm_ai::history::MAX_FILE_BYTES as usize,
                ledger,
            )
            .map_err(LiteralReadError::from)?;
            while let Some(fragment) = string.next_fragment().map_err(LiteralReadError::from)? {
                text.append(fragment)?;
            }
            Ok(text.finish()?)
        })
    }
    pub(super) fn decoded(&mut self, quoted: &SemanticSpan) -> Result<SemanticSpan, ValueError> {
        let length = self.text_length(quoted)?;
        let parent = match quoted.location {
            Location::Raw { source, .. } => Some(source),
            _ => None,
        };
        let parent_references = parent
            .map(|id| {
                self.source(id)?;
                self.slots
                    .get(id.slot)
                    .ok_or(ValueError::ForeignSource)?
                    .references
                    .checked_add(1)
                    .ok_or(ValueError::Build(JsonDecodeError::Arithmetic))
            })
            .transpose()?;
        let value = RawSource {
            quoted_parent: quoted.clone(),
            length,
        };
        let slot = if let Some(index) = self.free {
            let slot = self.slots.get_mut(index).ok_or(ValueError::ForeignSource)?;
            let generation = slot
                .generation
                .checked_add(1)
                .ok_or(JsonDecodeError::Arithmetic)?;
            self.free = slot.next_free.take();
            slot.generation = generation;
            slot.references = 1;
            slot.value = Some(value);
            index
        } else {
            let index = self.slots.len();
            self.slots.push(Slot {
                value: Some(value),
                generation: 0,
                next_free: None,
                references: 1,
            })?;
            index
        };
        if let Some(parent) = parent {
            self.slots
                .get_mut(parent.slot)
                .ok_or(ValueError::ForeignSource)?
                .references = parent_references.ok_or(ValueError::ForeignSource)?;
        }
        let generation = self
            .slots
            .get(slot)
            .ok_or(ValueError::ForeignSource)?
            .generation;
        Ok(SemanticSpan {
            location: Location::Raw {
                source: RawId { slot, generation },
                start: 0,
                end: length,
            },
            context: Context {
                depth: 0,
                map: false,
            },
        })
    }
    pub(super) fn retire(&mut self, span: SemanticSpan) -> Result<(), ValueError> {
        let Location::Raw { source, .. } = span.location else {
            return Err(ValueError::ForeignSource);
        };
        let mut current = Some(source);
        while let Some(source) = current.take() {
            self.source(source)?;
            let slot = self
                .slots
                .get_mut(source.slot)
                .ok_or(ValueError::ForeignSource)?;
            slot.references -= 1;
            if slot.references != 0 {
                break;
            }
            let raw = slot.value.take().ok_or(ValueError::ForeignSource)?;
            current = match raw.quoted_parent.location {
                Location::Raw { source, .. } => Some(source),
                _ => None,
            };
            slot.next_free = self.free;
            self.free = Some(source.slot);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
