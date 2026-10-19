//! Original component-local Plain policy over real guarded physical values.
//! Original neutral element sizes are preserved POLICY, not physical layouts.
//! Real logical capacities and each shared semantic occurrence are projected;
//! unique A separately measures physical vectors, slabs and common headers.
use super::{
    composer::{AttachmentSlot, PromptSlot},
    content::{ContentSlot, ResourceSlot},
    rows::{RowBody, RowSlot},
    slots::{PayloadGuard, TextGuard},
    tools::{ClaimSlot, OutputSlot, ToolSlot},
};
use crate::{
    history::{MeasurementError, SavedAttachment, SavedPrompt},
    model::{
        content::{Audience, Content},
        extensions::Extensions,
        shared_display::SharedDisplay,
        tools::{DisplayOutcome, Location, ToolOutput},
    },
    thread::Entry,
};
use std::mem::size_of;

struct Heap {
    bytes: usize,
}
impl Heap {
    fn add(&mut self, bytes: usize) -> Result<(), MeasurementError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        Ok(())
    }
    fn array<T>(&mut self, capacity: usize) -> Result<(), MeasurementError> {
        self.add(
            capacity
                .checked_mul(size_of::<T>())
                .ok_or(MeasurementError::ArithmeticOverflow)?,
        )
    }
    fn optional(&mut self, value: Option<TextGuard<'_>>) -> Result<(), MeasurementError> {
        if let Some(value) = value {
            self.add(value.capacity())?;
        }
        Ok(())
    }
    fn extensions(
        &mut self,
        value: Option<PayloadGuard<'_, Extensions>>,
    ) -> Result<(), MeasurementError> {
        if let Some(value) = value {
            self.add(
                value
                    .retained_heap_bytes()
                    .ok_or(MeasurementError::ArithmeticOverflow)?,
            )?;
        }
        Ok(())
    }
    fn content(&mut self, value: &ContentSlot) -> Result<(), MeasurementError> {
        match value {
            ContentSlot::Text(value) => self.add(value.text().capacity())?,
            ContentSlot::Media(value) => {
                self.add(value.encoded().capacity())?;
                self.add(value.mime().capacity())?;
                self.optional(value.uri())?;
            }
            ContentSlot::Reference(value) => {
                self.add(value.uri().capacity())?;
                self.add(value.name().capacity())?;
                self.optional(value.title())?;
                self.optional(value.description())?;
                self.optional(value.mime())?;
            }
            ContentSlot::Embedded(value) => match value.body() {
                ResourceSlot::Text(value) => {
                    self.add(value.uri().capacity())?;
                    self.add(value.text().capacity())?;
                    self.optional(value.mime())?;
                    self.extensions(value.extensions())?;
                }
                ResourceSlot::Encoded(value) => {
                    self.add(value.uri().capacity())?;
                    self.add(value.encoded().capacity())?;
                    self.optional(value.mime())?;
                    self.extensions(value.extensions())?;
                }
            },
        }
        if let Some(value) = value.annotations() {
            if let Some(capacity) = value.audience_capacity() {
                self.array::<Audience>(capacity)?;
            }
            self.optional(value.modified())?;
            self.extensions(value.extensions())?;
        }
        self.extensions(value.extensions())
    }
    fn claim(&mut self, value: &ClaimSlot) -> Result<(), MeasurementError> {
        if let Some((registration, tool)) = value.known() {
            self.add(registration.capacity())?;
            self.add(tool.capacity())?;
        }
        Ok(())
    }
    fn tool(&mut self, value: &ToolSlot) -> Result<(), MeasurementError> {
        self.add(
            value
                .id()
                .retained_heap_bytes()
                .ok_or(MeasurementError::ArithmeticOverflow)?,
        )?;
        self.add(value.title().capacity())?;
        self.optional(value.name())?;
        self.array::<ToolOutput>(value.output_capacity())?;
        for output in value.output() {
            match output {
                OutputSlot::Content {
                    content,
                    extensions,
                } => {
                    self.add(size_of::<Content>())?;
                    self.content(content)?;
                    self.extensions(extensions.as_ref().map(|value| value.read()))?;
                }
                OutputSlot::Diff(value) => {
                    self.add(value.path().capacity())?;
                    self.optional(value.previous())?;
                    self.add(value.replacement().capacity())?;
                    self.extensions(value.extensions())?;
                }
                OutputSlot::Terminal { id, extensions } => {
                    self.add(
                        id.id()
                            .retained_heap_bytes()
                            .ok_or(MeasurementError::ArithmeticOverflow)?,
                    )?;
                    self.extensions(extensions.as_ref().map(|value| value.read()))?;
                }
            }
        }
        self.array::<Location>(value.location_capacity())?;
        for value in value.locations() {
            self.add(value.path().capacity())?;
            self.extensions(value.extensions())?;
        }
        for value in [value.arguments(), value.result()].into_iter().flatten() {
            self.add(value.retained_heap_bytes())?;
        }
        self.extensions(value.extensions())?;
        let claims = value.claims();
        for claim in [claims.name(), claims.title(), claims.input()] {
            self.claim(claim)?;
        }
        self.optional(claims.legacy_display_hint())?;
        if let Some(value) = value.client_display() {
            self.add(SharedDisplay::inline_bytes())?;
            self.add(value.tool.capacity())?;
            self.add(value.arguments.retained_heap_bytes())?;
            if let Some(value) = value.destination.value() {
                self.add(value.capacity())?;
            }
            if let Some(value) = value.outcome.value() {
                match value {
                    DisplayOutcome::Succeeded(value) => self.add(value.retained_heap_bytes())?,
                    DisplayOutcome::Failed(value) => self.add(value.capacity())?,
                    DisplayOutcome::Pending => {}
                }
            }
        }
        Ok(())
    }
    fn attachments(
        &mut self,
        values: &[AttachmentSlot],
        capacity: usize,
    ) -> Result<(), MeasurementError> {
        self.array::<SavedAttachment>(capacity)?;
        for value in values {
            self.add(value.id().capacity())?;
        }
        Ok(())
    }
}
pub(crate) fn row(value: &RowSlot) -> Result<usize, MeasurementError> {
    let mut heap = Heap {
        bytes: size_of::<Entry>(),
    };
    match value.view() {
        RowBody::Agent(value) | RowBody::Thought(value) => heap.add(value.read().capacity())?,
        RowBody::Content(value) => heap.content(value)?,
        RowBody::Tool(value) => heap.tool(value)?,
        RowBody::User(value) => {
            heap.array::<Content>(value.capacity())?;
            for value in value.contents() {
                heap.content(value)?;
            }
        }
    }
    Ok(heap.bytes)
}
pub(crate) fn prompt(value: &PromptSlot) -> Result<usize, MeasurementError> {
    let mut heap = Heap {
        bytes: size_of::<SavedPrompt>(),
    };
    heap.add(value.text().capacity())?;
    heap.array::<Content>(value.image_capacity())?;
    for value in value.images() {
        heap.content(value)?;
    }
    heap.attachments(value.attachments(), value.attachment_capacity())?;
    Ok(heap.bytes)
}

/// Original allocations::document shell after subtracting each complete Plain
/// component. Entry/SavedPrompt inline slots therefore contribute only spare
/// capacity here; payloads and their inline part are counted by row()/prompt().
pub(crate) fn shell(value: &super::SnapshotOwner) -> Result<usize, MeasurementError> {
    let mut heap = Heap {
        bytes: size_of::<crate::history::SavedDocument>(),
    };
    heap.add(value.id().capacity())?;
    let metadata = value.metadata();
    heap.add(metadata.agent_id().capacity())?;
    heap.optional(metadata.title())?;
    heap.optional(metadata.name())?;
    heap.optional(metadata.model())?;
    let resume = value.resume();
    heap.optional(resume.session_id())?;
    heap.optional(resume.mode())?;
    if let Some(path) = resume.workdir() {
        heap.add(path.capacity())?;
    }
    heap.add(
        crate::session_config::choices_retained_backing_bound(resume.config().len())
            .ok_or(MeasurementError::ArithmeticOverflow)?,
    )?;
    for choice in resume.config() {
        heap.add(choice.key().capacity())?;
        if let super::ChoiceValueSlot::Value(text) = choice.value() {
            heap.add(text.read().capacity())?;
        }
    }
    let (rows, times) = value.transcript_policy_capacities();
    heap.array::<Entry>(
        rows.checked_sub(value.rows().len())
            .ok_or(MeasurementError::ArithmeticOverflow)?,
    )?;
    heap.array::<u64>(times)?;
    let composer = value.composer();
    heap.optional(composer.draft())?;
    heap.attachments(composer.attachments(), composer.attachment_capacity())?;
    heap.array::<SavedPrompt>(
        composer
            .queue_capacity()
            .checked_sub(composer.queue().len())
            .ok_or(MeasurementError::ArithmeticOverflow)?,
    )?;
    Ok(heap.bytes)
}

#[cfg(test)]
mod tests;
