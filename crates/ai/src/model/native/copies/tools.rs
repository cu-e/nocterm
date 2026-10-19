//! Tool payload copies preserve literal evidence and all inert opaque fields.
use super::{NativeCopies, NativeList, NativeOwned};
use crate::model::{
    controls::Presence,
    json::{OwnedJson, deferred::JsonDecodeError},
    shared_display::SharedDisplay,
    tools::*,
};
pub(super) struct ToolCopies<'a, 'l> {
    copies: &'a NativeCopies<'l>,
}
impl<'a, 'l> ToolCopies<'a, 'l> {
    pub(super) fn new(copies: &'a NativeCopies<'l>) -> Self {
        Self { copies }
    }
    pub(super) fn record(
        &self,
        value: &ToolRecord,
    ) -> Result<NativeOwned<ToolRecord>, JsonDecodeError> {
        let c = self.copies;
        let header = crate::model::native::tools::header(
            crate::model::native::tools::tool_id(c.text(value.id.as_str())?)?,
            c.text(&value.title)?,
            c.optional_text(value.name.as_deref())?,
            value.kind,
            value.status,
        )?;
        let record = crate::model::native::tools::record(
            header,
            self.outputs(&value.output)?,
            self.locations(&value.locations)?,
            self.optional_json(value.arguments.as_ref())?,
            self.optional_json(value.result.as_ref())?,
            c.extensions(value.extensions.as_ref())?,
            self.optional_display(value.client_display.as_ref())?,
        )?;
        Ok(crate::model::native::tools::with_literal(
            crate::model::native::claims::record_with_claims(record, self.claims(&value.claims)?)?,
            value.literal,
        ))
    }
    pub(super) fn patch(
        &self,
        value: &ToolPatch,
    ) -> Result<NativeOwned<ToolPatch>, JsonDecodeError> {
        let c = self.copies;
        let changes = &value.changes;
        let mut patch = crate::model::native::patch::tool_patch(
            crate::model::native::tools::tool_id(c.text(value.id.as_str())?)?,
            changes.kind,
            changes.status,
        );
        if let Some(value) = &changes.title {
            patch = crate::model::native::patch::with_title(patch, c.text(value)?)?;
        }
        if let Some(value) = &changes.name {
            patch = crate::model::native::patch::with_name(patch, c.text(value)?)?;
        }
        if let Some(value) = &changes.output {
            patch = crate::model::native::patch::with_output(patch, self.outputs(value)?)?;
        }
        if let Some(value) = &changes.locations {
            patch = crate::model::native::patch::with_locations(patch, self.locations(value)?)?;
        }
        if let Some(value) = &changes.arguments {
            patch = crate::model::native::patch::with_arguments(patch, c.json(value)?)?;
        }
        if let Some(value) = &changes.result {
            patch = crate::model::native::patch::with_result(patch, c.json(value)?)?;
        }
        Ok(crate::model::native::patch::with_literal(
            crate::model::native::claims::patch_with_claims(
                crate::model::native::claims::with_extensions(
                    patch,
                    c.extensions(value.extensions.as_ref())?,
                )?,
                self.claims(&value.claims)?,
            )?,
            value.literal,
        ))
    }
    fn outputs(
        &self,
        values: &[ToolOutput],
    ) -> Result<NativeOwned<Vec<ToolOutput>>, JsonDecodeError> {
        let c = self.copies;
        let mut output = NativeList::new(c.ledger, values.len())?;
        for value in values {
            output.push(match value {
                ToolOutput::Content {
                    content,
                    extensions,
                } => crate::model::native::tools::content_output(
                    c.content(content)?,
                    c.extensions(extensions.as_ref())?,
                )?,
                ToolOutput::FileDiff(value) => {
                    crate::model::native::tools::diff_output(crate::model::native::tools::diff(
                        NativeOwned::copy_path(c.ledger, &value.path)?,
                        c.optional_text(value.previous.as_deref())?,
                        c.text(&value.replacement)?,
                        c.extensions(value.extensions.as_ref())?,
                    )?)
                }
                ToolOutput::TerminalReference { id, extensions } => {
                    crate::model::native::tools::terminal_output(
                        crate::model::native::tools::terminal_id(c.text(id.as_str())?)?,
                        c.extensions(extensions.as_ref())?,
                    )?
                }
            })?;
        }
        Ok(output.finish())
    }
    fn locations(
        &self,
        values: &[Location],
    ) -> Result<NativeOwned<Vec<Location>>, JsonDecodeError> {
        let c = self.copies;
        let mut output = NativeList::new(c.ledger, values.len())?;
        for value in values {
            output.push(crate::model::native::tools::location(
                NativeOwned::copy_path(c.ledger, &value.path)?,
                value.line,
                c.extensions(value.extensions.as_ref())?,
            )?)?;
        }
        Ok(output.finish())
    }
    fn optional_json(
        &self,
        value: Option<&OwnedJson>,
    ) -> Result<NativeOwned<Option<OwnedJson>>, JsonDecodeError> {
        match value {
            Some(value) => Ok(NativeOwned::supplied(self.copies.json(value)?)),
            None => NativeOwned::absent(self.copies.ledger),
        }
    }
    fn claim(&self, value: &Claim) -> Result<NativeOwned<Claim>, JsonDecodeError> {
        match value {
            Claim::Absent => crate::model::native::claims::absent_claim(self.copies.ledger),
            Claim::Invalid => crate::model::native::claims::invalid_claim(self.copies.ledger),
            Claim::Known { registration, tool } => crate::model::native::claims::known(
                self.copies.text(registration)?,
                self.copies.text(tool)?,
            ),
        }
    }
    fn claims(&self, value: &BridgeClaims) -> Result<NativeOwned<BridgeClaims>, JsonDecodeError> {
        crate::model::native::claims::claims(
            self.claim(&value.name)?,
            self.claim(&value.title)?,
            self.claim(&value.input)?,
            self.copies
                .optional_text(value.legacy_display_hint.as_deref())?,
        )
    }
    fn optional_display(
        &self,
        value: Option<&SharedDisplay>,
    ) -> Result<NativeOwned<Option<SharedDisplay>>, JsonDecodeError> {
        let Some(value) = value else {
            return NativeOwned::absent(self.copies.ledger);
        };
        Ok(NativeOwned::supplied(crate::model::native::tools::shared(
            self.display(value)?,
        )?))
    }
    pub(super) fn display(
        &self,
        value: &ClientDisplay,
    ) -> Result<NativeOwned<ClientDisplay>, JsonDecodeError> {
        let c = self.copies;
        let redacted = match &value.redacted {
            Presence::Absent => Presence::Absent,
            Presence::Null => Presence::Null,
            Presence::Value(value) => Presence::Value(*value),
        };
        crate::model::native::tools::display(
            value.version,
            c.text(&value.tool)?,
            c.json(&value.arguments)?,
            self.presence(&value.destination)?,
            redacted,
            self.outcome(&value.outcome)?,
        )
    }
    fn presence(
        &self,
        value: &Presence<String>,
    ) -> Result<NativeOwned<Presence<String>>, JsonDecodeError> {
        match value {
            Presence::Absent => NativeOwned::<String>::not_supplied(self.copies.ledger),
            Presence::Null => NativeOwned::<String>::null(self.copies.ledger),
            Presence::Value(value) => Ok(self.copies.text(value)?.presence()),
        }
    }
    fn outcome(
        &self,
        value: &Presence<DisplayOutcome>,
    ) -> Result<NativeOwned<Presence<DisplayOutcome>>, JsonDecodeError> {
        let c = self.copies;
        match value {
            Presence::Absent => NativeOwned::<DisplayOutcome>::not_supplied(c.ledger),
            Presence::Null => NativeOwned::<DisplayOutcome>::null(c.ledger),
            Presence::Value(value) => Ok(match value {
                DisplayOutcome::Pending => crate::model::native::tools::pending(c.ledger)?,
                DisplayOutcome::Succeeded(value) => {
                    crate::model::native::tools::succeeded(c.json(value)?)
                }
                DisplayOutcome::Failed(value) => {
                    crate::model::native::tools::failed(c.text(value)?)
                }
            }
            .presence()),
        }
    }
}
