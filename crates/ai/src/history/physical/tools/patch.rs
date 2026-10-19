//! Live tool patches with `ToolRecord::apply` semantics over physical bundles.
//! Unchanged payloads alias the previous version; only supplied fields allocate.
use super::{LocationsSlot, OutputsSlot, ToolSlot, json_part, text_part};
use crate::{
    history::physical::{
        bundle,
        slots::{
            DisplaySlot, ExtensionSlot, NativeCredit, NativeSourcePool, PhysicalError, TextSlot,
        },
    },
    model::{
        json::lifetime::Shared,
        native::NativeOwned,
        retention::document::ConstructionRole,
        tools::{ClientDisplay, ToolChanges, ToolPatch, ToolStatus},
    },
};
use std::ops::Deref;

impl ToolSlot {
    /// Durable equality follows the original transcript saved-window classifier.
    pub(crate) fn changes_saved(&self, changes: &ToolChanges) -> bool {
        changes.kind.is_some_and(|value| value != self.kind)
            || changes.status.is_some_and(|value| value != self.status)
            || changes
                .title
                .as_ref()
                .is_some_and(|value| value.as_str() != &*self.title())
            || changes
                .name
                .as_ref()
                .is_some_and(|value| self.name().as_deref() != Some(value.as_str()))
            || changes.output.as_ref().is_some_and(|value| {
                value.len() != self.output().len()
                    || !self
                        .output()
                        .iter()
                        .zip(value)
                        .all(|(left, right)| left.matches(right))
            })
            || changes.locations.as_ref().is_some_and(|value| {
                value.len() != self.locations().len()
                    || !self
                        .locations()
                        .iter()
                        .zip(value)
                        .all(|(left, right)| left.matches(right))
            })
            || changes.arguments.as_ref().is_some_and(|value| {
                self.arguments()
                    .as_ref()
                    .is_none_or(|left| **left != *value)
            })
            || changes
                .result
                .as_ref()
                .is_some_and(|value| self.result().as_ref().is_none_or(|left| **left != *value))
    }
    pub(in crate::history::physical) fn changed_by(&self, patch: &ToolPatch) -> bool {
        let changes = &patch.changes;
        let mut literal = self.literal;
        literal.apply(
            patch.literal,
            changes.name.is_some(),
            changes.title.is_some(),
            changes.arguments.is_some(),
        );
        self.changes_saved(changes)
            || literal != self.literal
            || self.claims.changed_by(&patch.claims)
    }
    /// `None` when the patch addresses another tool, as `ToolRecord::apply`.
    /// `ToolPatch::extensions` is not applied, matching the neutral record.
    pub(in crate::history::physical) fn patched(
        &self,
        patch: ToolPatch,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
        pool: &mut NativeSourcePool,
    ) -> Result<Option<Self>, PhysicalError> {
        if *self.id() != patch.id {
            return Ok(None);
        }
        let changes = patch.changes;
        let mut next = self.alias();
        next.literal.apply(
            patch.literal,
            changes.name.is_some(),
            changes.title.is_some(),
            changes.arguments.is_some(),
        );
        if let Some(value) = changes.kind {
            next.kind = value;
        }
        if let Some(value) = changes.status {
            next.status = value;
        }
        if let Some(value) = changes.title {
            next.title = bundle::part(role, original, TextSlot::new(value, pool)?)?;
        }
        if let Some(value) = changes.name {
            next.name = text_part(Some(value), role, original, pool)?;
        }
        if let Some(value) = changes.output {
            next.output = OutputsSlot::build(value, role, original, pool)?;
        }
        if let Some(value) = changes.locations {
            next.locations = LocationsSlot::build(value, role, original, pool)?;
        }
        if let Some(value) = changes.arguments {
            next.arguments = json_part(Some(value), role, original)?;
        }
        if let Some(value) = changes.result {
            next.result = json_part(Some(value), role, original)?;
        }
        next.claims = self.claims.merged(patch.claims, role, original, pool)?;
        Ok(Some(next))
    }
    /// A Pending/Running tool version finished as Failed; every payload aliases.
    pub(in crate::history::physical) fn interrupted(&self) -> Option<Self> {
        if !matches!(self.status, ToolStatus::Pending | ToolStatus::Running) {
            return None;
        }
        let mut next = self.alias();
        next.status = ToolStatus::Failed;
        Some(next)
    }
    /// Installs a client display. Equality is borrowed; only the extensions
    /// view (hiding the display key) and the display bundle are renewed, the
    /// shared extension backing, JSON, claims and provenance alias.
    pub(in crate::history::physical) fn displayed(
        &self,
        display: NativeOwned<ClientDisplay>,
        role: &ConstructionRole,
        original: &Shared<NativeCredit>,
    ) -> Result<Option<Self>, PhysicalError> {
        if self
            .display
            .as_ref()
            .is_some_and(|current| current.body().read().deref().deref() == display.get())
        {
            return Ok(None);
        }
        let (display, allocation) = crate::model::native::tools::shared(display)?.into_parts();
        drop(allocation);
        let mut next = self.alias();
        if let Some(extensions) = &self.extensions {
            let hidden = extensions.body().read().clone().without_client_display();
            next.extensions = Some(bundle::part(role, original, ExtensionSlot::new(hidden))?);
        }
        next.display = Some(bundle::part(role, original, DisplaySlot::new(display))?);
        Ok(Some(next))
    }
}
