//! Capacity-aware heap accounting supplements the serde-shaped conservative metric.
//! JSON B-tree reserve remains a conservative policy requiring allocation proof.
use super::{MeasurementError, SavedAttachment, SavedPrompt};
use crate::model::json::OwnedJson;
use crate::model::retention::{
    CensusPermit, RetentionError,
    graph::{Count, OwnerGraph},
};
use crate::{
    model::{content::*, extensions::Extensions, tools::*},
    thread::Entry,
};
use std::mem::size_of;
enum Mode<'c, 'p, 'a> {
    Plain,
    Count(&'c mut Count<'p, 'a>),
    Cleanup(&'c CensusPermit<'a>),
    Adopt(&'c mut crate::model::retention::document::Publication<'a>),
}
struct Heap<'c, 'p, 'a> {
    bytes: usize,
    mode: Mode<'c, 'p, 'a>,
}
impl Heap<'_, '_, '_> {
    fn plain(bytes: usize) -> Self {
        Self {
            bytes,
            mode: Mode::Plain,
        }
    }
    fn cleaning(&self) -> bool {
        matches!(self.mode, Mode::Cleanup(_) | Mode::Adopt(_))
    }
    fn shared<S: OwnerGraph>(
        &mut self,
        source: &S,
        bytes: Option<usize>,
    ) -> Result<(), MeasurementError> {
        match &mut self.mode {
            Mode::Count(count) => source.count(count).map_err(MeasurementError::Retention),
            Mode::Cleanup(permit) => {
                source.cleanup(permit);
                Ok(())
            }
            Mode::Adopt(publication) => {
                source.adopt(publication);
                Ok(())
            }
            Mode::Plain => self.add(bytes.ok_or(MeasurementError::ArithmeticOverflow)?),
        }
    }

    fn add(&mut self, bytes: usize) -> Result<(), MeasurementError> {
        match &mut self.mode {
            Mode::Count(count) => count
                .add_exclusive(bytes)
                .map_err(MeasurementError::Retention),
            Mode::Cleanup(_) | Mode::Adopt(_) => Ok(()),
            Mode::Plain => {
                self.bytes = self
                    .bytes
                    .checked_add(bytes)
                    .ok_or(MeasurementError::ArithmeticOverflow)?;
                Ok(())
            }
        }
    }
    fn slots<T>(&mut self, values: &Vec<T>) -> Result<(), MeasurementError> {
        if self.cleaning() {
            return Ok(());
        }
        self.add(
            values
                .capacity()
                .checked_mul(size_of::<T>())
                .ok_or(MeasurementError::ArithmeticOverflow)?,
        )
    }
    fn string(&mut self, value: &String) -> Result<(), MeasurementError> {
        self.add(value.capacity())
    }
    fn optional_string(&mut self, value: Option<&String>) -> Result<(), MeasurementError> {
        if let Some(value) = value {
            self.string(value)?;
        }
        Ok(())
    }
    fn shared_id(&mut self, retained: Option<usize>) -> Result<(), MeasurementError> {
        self.add(retained.ok_or(MeasurementError::ArithmeticOverflow)?)
    }
    fn json(&mut self, value: &OwnedJson) -> Result<(), MeasurementError> {
        self.shared(value.receipt(), Some(value.retained_heap_bytes()))
    }
    fn extensions(&mut self, value: Option<&Extensions>) -> Result<(), MeasurementError> {
        if let Some(value) = value {
            self.shared(value, value.retained_heap_bytes())?;
        }
        Ok(())
    }
    fn annotations(&mut self, value: Option<&Annotations>) -> Result<(), MeasurementError> {
        if let Some(value) = value {
            if let Some(values) = &value.audience {
                self.slots(values)?;
            }
            self.optional_string(value.modified.as_ref())?;
            self.extensions(value.extensions.as_ref())?;
        }
        Ok(())
    }
    fn content(&mut self, value: &Content) -> Result<(), MeasurementError> {
        match value {
            Content::Text(value) => {
                self.string(&value.text)?;
                self.annotations(value.annotations.as_ref())?;
                self.extensions(value.extensions.as_ref())?;
            }
            Content::Media(value) => {
                self.string(&value.encoded)?;
                self.string(&value.mime)?;
                self.optional_string(value.uri.as_ref())?;
                self.annotations(value.annotations.as_ref())?;
                self.extensions(value.extensions.as_ref())?;
            }
            Content::ResourceReference(value) => {
                self.string(&value.uri)?;
                self.string(&value.name)?;
                self.optional_string(value.title.as_ref())?;
                self.optional_string(value.description.as_ref())?;
                self.optional_string(value.mime.as_ref())?;
                self.annotations(value.annotations.as_ref())?;
                self.extensions(value.extensions.as_ref())?;
            }
            Content::EmbeddedResource(value) => {
                match &value.body {
                    ResourceBody::Text(body) => {
                        self.string(&body.uri)?;
                        self.string(&body.text)?;
                        self.optional_string(body.mime.as_ref())?;
                        self.extensions(body.extensions.as_ref())?;
                    }
                    ResourceBody::Encoded(body) => {
                        self.string(&body.uri)?;
                        self.string(&body.encoded)?;
                        self.optional_string(body.mime.as_ref())?;
                        self.extensions(body.extensions.as_ref())?;
                    }
                }
                self.annotations(value.annotations.as_ref())?;
                self.extensions(value.extensions.as_ref())?;
            }
        }
        Ok(())
    }
    fn claim(&mut self, value: &Claim) -> Result<(), MeasurementError> {
        if let Claim::Known { registration, tool } = value {
            self.string(registration)?;
            self.string(tool)?;
        }
        Ok(())
    }
    fn tool(&mut self, value: &ToolRecord) -> Result<(), MeasurementError> {
        self.shared(&value.id, value.id.retained_heap_bytes())?;
        self.string(&value.title)?;
        self.optional_string(value.name.as_ref())?;
        self.outputs(&value.output)?;
        self.locations(&value.locations)?;
        if let Some(value) = &value.arguments {
            self.json(value)?;
        }
        if let Some(value) = &value.result {
            self.json(value)?;
        }
        self.extensions(value.extensions.as_ref())?;
        self.claim(&value.claims.name)?;
        self.claim(&value.claims.title)?;
        self.claim(&value.claims.input)?;
        self.optional_string(value.claims.legacy_display_hint.as_ref())?;
        if let Some(value) = &value.client_display {
            if !matches!(self.mode, Mode::Plain) {
                self.shared(value, None)?;
            } else {
                self.shared_id(Some(
                    crate::model::shared_display::SharedDisplay::inline_bytes(),
                ))?;
                self.string(&value.tool)?;
                self.json(&value.arguments)?;
                self.optional_string(value.destination.value())?;
                if let Some(outcome) = value.outcome.value() {
                    match outcome {
                        DisplayOutcome::Succeeded(value) => self.json(value)?,
                        DisplayOutcome::Failed(value) => self.string(value)?,
                        DisplayOutcome::Pending => {}
                    }
                }
            }
        }
        Ok(())
    }
    fn outputs(&mut self, values: &Vec<ToolOutput>) -> Result<(), MeasurementError> {
        self.slots(values)?;
        for output in values {
            match output {
                ToolOutput::Content {
                    content,
                    extensions,
                } => {
                    self.add(size_of::<Content>())?;
                    self.content(content)?;
                    self.extensions(extensions.as_ref())?;
                }
                ToolOutput::FileDiff(value) => {
                    self.add(value.path.capacity())?;
                    self.optional_string(value.previous.as_ref())?;
                    self.string(&value.replacement)?;
                    self.extensions(value.extensions.as_ref())?;
                }
                ToolOutput::TerminalReference { id, extensions } => {
                    self.shared(id, id.retained_heap_bytes())?;
                    self.extensions(extensions.as_ref())?;
                }
            }
        }
        Ok(())
    }
    fn locations(&mut self, values: &Vec<Location>) -> Result<(), MeasurementError> {
        self.slots(values)?;
        for location in values {
            self.add(location.path.capacity())?;
            self.extensions(location.extensions.as_ref())?;
        }
        Ok(())
    }
    fn row(&mut self, value: &Entry) -> Result<(), MeasurementError> {
        match value {
            Entry::Agent(value) | Entry::Thought(value) => self.string(value)?,
            Entry::User(values) => {
                self.slots(values)?;
                for value in values {
                    self.content(value)?;
                }
            }
            Entry::Content(value) => self.content(value)?,
            Entry::Tool(value) => self.tool(value)?,
        }
        Ok(())
    }
    fn chat_value(&mut self, value: &super::SavedChat) -> Result<(), MeasurementError> {
        self.document_value(
            value.id_owned(),
            value.metadata(),
            value.resume(),
            value.transcript(),
            value.composer(),
        )
    }
    fn document_value(
        &mut self,
        id: &String,
        meta: &super::ChatMetadata,
        resume: &super::ResumeDescriptor,
        transcript: &super::TranscriptData,
        composer: &super::ComposerData,
    ) -> Result<(), MeasurementError> {
        self.add(id.capacity())?;
        self.string(&meta.agent_id)?;
        self.optional_string(meta.title.as_ref())?;
        self.optional_string(meta.name.as_ref())?;
        self.optional_string(meta.model.as_ref())?;
        self.optional_string(resume.session_id.as_ref())?;
        self.optional_string(resume.mode.as_ref())?;
        if let Some(path) = &resume.workdir {
            self.add(path.capacity())?;
        }
        if !self.cleaning() {
            self.add(
                crate::session_config::choices_retained_backing_bound(resume.config.len())
                    .ok_or(MeasurementError::ArithmeticOverflow)?,
            )?;
        }
        for (key, value) in &resume.config {
            self.string(key)?;
            if let crate::session_config::ConfigValue::Value(value) = value {
                self.string(value)?;
            }
        }
        self.slots(&transcript.entries)?;
        for value in &transcript.entries {
            self.row(value)?;
        }
        self.slots(&transcript.times)?;
        self.optional_string(composer.draft.as_ref())?;
        self.attachments(&composer.attachments)?;
        self.slots(&composer.queue)?;
        for value in &composer.queue {
            self.prompt_value(value)?;
        }
        Ok(())
    }
    fn prompt_value(&mut self, value: &SavedPrompt) -> Result<(), MeasurementError> {
        self.string(&value.text)?;
        self.slots(&value.images)?;
        for value in &value.images {
            self.content(value)?;
        }
        self.attachments(&value.attachments)?;
        Ok(())
    }
    fn attachments(&mut self, values: &Vec<SavedAttachment>) -> Result<(), MeasurementError> {
        self.slots(values)?;
        for value in values {
            match value {
                SavedAttachment::Connection(value)
                | SavedAttachment::Group(value)
                | SavedAttachment::LocalTerminal(value) => self.string(value)?,
            }
        }
        Ok(())
    }
}
pub(super) fn row(value: &Entry) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(size_of::<Entry>());
    heap.row(value)?;
    Ok(heap.bytes)
}
pub(super) fn prompt(value: &SavedPrompt) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(size_of::<SavedPrompt>());
    heap.prompt_value(value)?;
    Ok(heap.bytes)
}
pub(super) fn chat(value: &super::SavedChat) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(size_of::<super::SavedChat>());
    heap.chat_value(value)?;
    Ok(heap.bytes)
}

pub(super) fn document(value: &super::SavedDocument) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(size_of::<super::SavedDocument>());
    heap.document_value(
        &value.id,
        &value.metadata,
        &value.resume,
        &value.transcript,
        &value.composer,
    )?;
    Ok(heap.bytes)
}

fn retained_error(error: MeasurementError) -> RetentionError {
    match error {
        MeasurementError::Retention(error) => error,
        // The capacity-only walker never invokes a serializer and cannot emit Json.
        MeasurementError::ArithmeticOverflow
        | MeasurementError::Json(_)
        | MeasurementError::Formatting(_) => RetentionError::Arithmetic,
    }
}
pub(super) fn count_chat(
    value: &super::SavedChat,
    count: &mut Count<'_, '_>,
) -> Result<(), RetentionError> {
    count.validate_context(value.retention())?;
    let mut heap = Heap {
        bytes: 0,
        mode: Mode::Count(count),
    };
    heap.add(size_of::<super::SavedChat>())
        .map_err(retained_error)?;
    heap.chat_value(value).map_err(retained_error)
}
pub(super) fn cleanup_chat(value: &super::SavedChat, permit: &CensusPermit<'_>) {
    let mut heap = Heap {
        bytes: 0,
        mode: Mode::Cleanup(permit),
    };
    // Cleanup skips all fallible arithmetic; shared sources are sealed and infallible.
    let _ = heap.chat_value(value);
}

pub(super) fn adopt_chat(
    value: &super::SavedChat,
    publication: &mut crate::model::retention::document::Publication<'_>,
) {
    let mut heap = Heap {
        bytes: 0,
        mode: Mode::Adopt(publication),
    };
    let _ = heap.chat_value(value);
}

/// Original Plain payload heap only, without a fabricated enclosing Entry.
pub(super) fn content_payload(value: &Content) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(0);
    heap.content(value)?;
    Ok(heap.bytes)
}
pub(super) fn tool_payload(value: &ToolRecord) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(0);
    heap.tool(value)?;
    Ok(heap.bytes)
}
pub(super) fn outputs_payload(value: &Vec<ToolOutput>) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(0);
    heap.outputs(value)?;
    Ok(heap.bytes)
}
pub(super) fn locations_payload(value: &Vec<Location>) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(0);
    heap.locations(value)?;
    Ok(heap.bytes)
}
pub(super) fn claims_payload(value: &BridgeClaims) -> Result<usize, MeasurementError> {
    let mut heap = Heap::plain(0);
    heap.claim(&value.name)?;
    heap.claim(&value.title)?;
    heap.claim(&value.input)?;
    heap.optional_string(value.legacy_display_hint.as_ref())?;
    Ok(heap.bytes)
}
