//! Closed pending semantic projections, computed before the sealed census.
//!
//! A native envelope/session ID is protocol framing, not a history row. These
//! facts measure only retained payloads. A patch measures supplied values only;
//! the final complete row acquires its separate immutable Live policy proof.
use crate::{
    history::{MeasurementError, allocations, size},
    model::{
        content::{Content, ResourceBody},
        controls::{ConfigKind, ConfigOption, SelectChoices},
        extensions::Extensions,
        notice::{SessionNotice, SessionUpdate},
        tools::{ToolOutput, ToolPatch},
    },
};
use serde::Serialize;
#[cfg(test)]
mod tests;

pub(super) struct SourcePolicy(usize);
impl SourcePolicy {
    pub(super) fn notice(value: &SessionNotice) -> Result<Self, MeasurementError> {
        let mut total = Total(0);
        match &value.update {
            SessionUpdate::UserChunk(value)
            | SessionUpdate::AgentChunk(value)
            | SessionUpdate::ThoughtChunk(value) => {
                total.component(allocations::content_payload(value)?, value)?;
            }
            SessionUpdate::ToolStarted(value) => {
                total.component(allocations::tool_payload(value)?, value)?;
            }
            SessionUpdate::ToolChanged(value) => total.patch(value)?,
            SessionUpdate::Configuration(values) => total.configuration(values)?,
            // These have no existing history retained-component wrapper. Their
            // actual allocations remain fully charged on the physical lane.
            SessionUpdate::Plan(_)
            | SessionUpdate::Commands(_)
            | SessionUpdate::Mode(_)
            | SessionUpdate::Info(_)
            | SessionUpdate::Usage(_) => {}
        }
        Ok(Self(total.0))
    }
    pub(super) fn opened(
        value: &crate::model::session::SessionOpened,
    ) -> Result<Self, MeasurementError> {
        let mut total = Total(0);
        if let Some(modes) = &value.modes {
            total.add(modes_policy(modes)?)?;
        }
        if let Some(options) = &value.configuration {
            total.configuration(options)?;
        }
        Ok(Self(total.0))
    }
    pub(super) fn permission(
        value: &crate::model::auth::ProviderPermission,
    ) -> Result<Self, MeasurementError> {
        let mut total = Total(0);
        total.opaque_patch(&value.tool)?;
        total.extensions(value.extensions.as_ref())?;
        for choice in &value.choices {
            total.extensions(choice.extensions.as_ref())?;
        }
        Ok(Self(total.0))
    }
    pub(super) fn info(_value: &crate::AgentInfo) -> Result<Self, MeasurementError> {
        Ok(Self(0))
    }
    pub(super) fn bytes(&self) -> usize {
        self.0
    }
}
struct Total(usize);
impl Total {
    fn add(&mut self, bytes: usize) -> Result<(), MeasurementError> {
        self.0 = self
            .0
            .checked_add(bytes)
            .ok_or(MeasurementError::ArithmeticOverflow)?;
        Ok(())
    }
    fn component(&mut self, plain: usize, value: &impl Serialize) -> Result<(), MeasurementError> {
        self.add(plain.max(size::retained(value)?))
    }
    fn optional<T: Serialize>(
        &mut self,
        value: Option<&T>,
        plain: impl FnOnce(&T) -> Result<usize, MeasurementError>,
    ) -> Result<(), MeasurementError> {
        if let Some(value) = value {
            self.component(plain(value)?, value)?;
        }
        Ok(())
    }
    fn extensions(&mut self, value: Option<&Extensions>) -> Result<(), MeasurementError> {
        self.optional(value, |value| {
            value
                .retained_heap_bytes()
                .ok_or(MeasurementError::ArithmeticOverflow)
        })
    }
    fn patch(&mut self, value: &ToolPatch) -> Result<(), MeasurementError> {
        let changes = &value.changes;
        self.optional(changes.kind.as_ref(), |_| Ok(0))?;
        self.optional(changes.status.as_ref(), |_| Ok(0))?;
        self.optional(changes.title.as_ref(), |value| Ok(value.capacity()))?;
        self.optional(changes.name.as_ref(), |value| Ok(value.capacity()))?;
        self.optional(changes.output.as_ref(), allocations::outputs_payload)?;
        self.optional(changes.locations.as_ref(), allocations::locations_payload)?;
        self.optional(changes.arguments.as_ref(), |value| {
            Ok(value.retained_heap_bytes())
        })?;
        self.optional(changes.result.as_ref(), |value| {
            Ok(value.retained_heap_bytes())
        })?;
        self.extensions(value.extensions.as_ref())?;
        // Claims are intentionally skipped by the historical serializer, but
        // their original Plain heap occurrences remain a retained contribution.
        self.add(allocations::claims_payload(&value.claims)?)
    }
    fn opaque_patch(&mut self, value: &ToolPatch) -> Result<(), MeasurementError> {
        let changes = &value.changes;
        self.optional(changes.arguments.as_ref(), |value| {
            Ok(value.retained_heap_bytes())
        })?;
        self.optional(changes.result.as_ref(), |value| {
            Ok(value.retained_heap_bytes())
        })?;
        self.extensions(value.extensions.as_ref())?;
        if let Some(outputs) = &changes.output {
            for output in outputs {
                match output {
                    ToolOutput::Content {
                        content,
                        extensions,
                    } => {
                        self.opaque_content(content)?;
                        self.extensions(extensions.as_ref())?;
                    }
                    ToolOutput::FileDiff(value) => self.extensions(value.extensions.as_ref())?,
                    ToolOutput::TerminalReference { extensions, .. } => {
                        self.extensions(extensions.as_ref())?
                    }
                }
            }
        }
        if let Some(locations) = &changes.locations {
            for value in locations {
                self.extensions(value.extensions.as_ref())?;
            }
        }
        Ok(())
    }
    fn opaque_content(&mut self, value: &Content) -> Result<(), MeasurementError> {
        let (annotations, extensions) = match value {
            Content::Text(value) => (value.annotations.as_ref(), value.extensions.as_ref()),
            Content::Media(value) => (value.annotations.as_ref(), value.extensions.as_ref()),
            Content::ResourceReference(value) => {
                (value.annotations.as_ref(), value.extensions.as_ref())
            }
            Content::EmbeddedResource(value) => {
                match &value.body {
                    ResourceBody::Text(value) => self.extensions(value.extensions.as_ref())?,
                    ResourceBody::Encoded(value) => self.extensions(value.extensions.as_ref())?,
                }
                (value.annotations.as_ref(), value.extensions.as_ref())
            }
        };
        if let Some(value) = annotations {
            self.extensions(value.extensions.as_ref())?;
        }
        self.extensions(extensions)
    }
    fn configuration(&mut self, values: &[ConfigOption]) -> Result<(), MeasurementError> {
        for value in values {
            self.extensions(value.extensions.as_ref())?;
            if let ConfigKind::Select { choices, .. } = &value.kind {
                match choices {
                    SelectChoices::Flat(values) => {
                        for value in values {
                            self.extensions(value.extensions.as_ref())?;
                        }
                    }
                    SelectChoices::Groups(values) => {
                        for group in values {
                            self.extensions(group.extensions.as_ref())?;
                            for value in &group.choices {
                                self.extensions(value.extensions.as_ref())?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub(in crate::history) fn modes_policy(
    value: &crate::model::controls::Modes,
) -> Result<usize, MeasurementError> {
    let mut total = Total(0);
    total.extensions(value.extensions.as_ref())?;
    for mode in &value.choices {
        total.extensions(mode.extensions.as_ref())?;
    }
    Ok(total.0)
}
