//! Historical tool grammar borrows complete output and reconstitutes client metadata.
use super::order::RowOrderExt;
use super::{content_borrowed as content, display};
use nocterm_ai::{history::CompatibilityReason, model::tools::*};
use serde::{
    Serialize,
    ser::{SerializeMap, SerializeSeq},
};
const DISPLAY_KEY: &str = "nocterm/tool-display";

pub(super) fn check(value: &ToolRecord) -> Result<(), CompatibilityReason> {
    // The historical SDK optional-field decoder equates root JSON null with
    // absence. A new neutral supplied-null fact cannot round-trip through v1;
    // report representation refusal rather than silently erasing that fact.
    if value
        .arguments
        .as_ref()
        .is_some_and(|value| value.is_null())
        || value.result.as_ref().is_some_and(|value| value.is_null())
    {
        return Err(CompatibilityReason::ToolField);
    }

    if value.client_display.is_some()
        && value
            .extensions
            .as_ref()
            .is_some_and(|value| value.contains_key(DISPLAY_KEY))
    {
        return Err(CompatibilityReason::ExtensionField);
    }
    if value.client_display.as_ref().is_some_and(|value| {
        !matches!(value.version, 1 | 2)
            || matches!(value.redacted, nocterm_ai::model::controls::Presence::Null)
    }) {
        return Err(CompatibilityReason::ToolField);
    }
    for output in &value.output {
        if let ToolOutput::Content { content: value, .. } = output {
            content::check(value)?;
        }
    }
    Ok(())
}
pub(super) struct Borrowed<'a>(
    pub(super) &'a ToolRecord,
    pub(super) Option<&'a super::order::RowOrder>,
);
impl Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("toolCallId", &value.id)?;
        map.serialize_entry("title", &value.title)?;
        content::optional(&mut map, "name", &value.name)?;
        if value.kind != ToolKind::Other {
            map.serialize_entry("kind", kind(value.kind))?;
        }
        if value.status != ToolStatus::Pending {
            map.serialize_entry("status", status(value.status))?;
        }
        if !value.output.is_empty() {
            map.serialize_entry("content", &Outputs(&value.output))?;
        }
        if !value.locations.is_empty() {
            map.serialize_entry("locations", &Locations(&value.locations))?;
        }
        content::optional(&mut map, "rawInput", &value.arguments)?;
        content::optional(&mut map, "rawOutput", &value.result)?;
        if value.extensions.is_some() || value.client_display.is_some() {
            map.serialize_entry("_meta", &Metadata(value, self.1))?;
        }
        map.end()
    }
}
fn kind(value: ToolKind) -> &'static str {
    match value {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::ChangeMode => "switch_mode",
        ToolKind::Other => "other",
    }
}
fn status(value: ToolStatus) -> &'static str {
    match value {
        ToolStatus::Pending => "pending",
        ToolStatus::Running => "in_progress",
        ToolStatus::Succeeded => "completed",
        ToolStatus::Failed => "failed",
    }
}
struct Outputs<'a>(&'a [ToolOutput]);
impl Serialize for Outputs<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Output(value))?;
        }
        seq.end()
    }
}
struct Output<'a>(&'a ToolOutput);
impl Serialize for Output<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            ToolOutput::Content {
                content: value,
                extensions,
            } => {
                map.serialize_entry("type", "content")?;
                map.serialize_entry("content", &content::Borrowed(value))?;
                content::metadata(&mut map, extensions)?;
            }
            ToolOutput::FileDiff(value) => {
                map.serialize_entry("type", "diff")?;
                map.serialize_entry("path", &value.path)?;
                content::optional(&mut map, "oldText", &value.previous)?;
                map.serialize_entry("newText", &value.replacement)?;
                content::metadata(&mut map, &value.extensions)?;
            }
            ToolOutput::TerminalReference { id, extensions } => {
                map.serialize_entry("type", "terminal")?;
                map.serialize_entry("terminalId", id)?;
                content::metadata(&mut map, extensions)?;
            }
        }
        map.end()
    }
}
struct Locations<'a>(&'a [Location]);
impl Serialize for Locations<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Located(value))?;
        }
        seq.end()
    }
}
struct Located<'a>(&'a Location);
impl Serialize for Located<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("path", &self.0.path)?;
        content::optional(&mut map, "line", &self.0.line)?;
        content::metadata(&mut map, &self.0.extensions)?;
        map.end()
    }
}
struct Metadata<'a>(&'a ToolRecord, Option<&'a super::order::RowOrder>);
impl Serialize for Metadata<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        let mut display = self.0.client_display.as_deref();
        let fields = self.1.map_or(super::order::FieldOrder::NATIVE, |order| {
            order.fields(self.0)
        });
        let slot = self.1.and_then(|order| order.slot(self.0));
        if let Some(extensions) = &self.0.extensions {
            for (index, (key, value)) in extensions.iter().enumerate() {
                if slot == Some(index)
                    && let Some(value) = display.take()
                {
                    map.serialize_entry(DISPLAY_KEY, &display::Borrowed(value, fields))?;
                }
                map.serialize_entry(key, value)?;
            }
        }
        if let Some(value) = display {
            map.serialize_entry(DISPLAY_KEY, &display::Borrowed(value, fields))?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests;
