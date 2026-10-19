//! Borrow actual whole-document JSON components without a census or admission decision.
//! Work is O(P + C) in borrowed source elements and emitted components, with constant
//! internal storage and no dynamic JSON recursion. Sink storage/work is its own contract.
use super::{SavedChat, SavedPrompt};
use crate::{
    model::{
        content::*,
        extensions::Extensions,
        json::{OwnedJson, RetainedAllocation},
        tools::*,
    },
    thread::Entry,
};

/// Retained JSON owned by an auxiliary source, such as authentic legacy grammar.
/// Implementations emit their actual borrowed components, without inventing costs.
pub trait HistoryJsonRetention: Send + Sync {
    fn visit_json_allocations<'a>(
        &'a self,
        visit: &mut dyn FnMut(RetainedAllocation<'a>) -> Result<(), RetentionError>,
    ) -> Result<(), RetentionError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    #[error("retained-allocation arithmetic overflow")]
    ArithmeticOverflow,
    #[error("retained census requests {requested} bytes with {available} available")]
    Limit { requested: usize, available: usize },
    #[error("retained census allocation failed: {0}")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error("retained allocation belongs to another admission ledger")]
    ForeignLedger,
}

/// Visits ALL retained rows and queued prompts, then an explicitly supplied source.
/// This does not use the newest-2000 serialization projection. Repeated aliases are
/// emitted repeatedly: interpreting identity and making admission decisions belong
/// to the sink. No temporary receipt, payload clone, reservation or table is created.
pub fn visit_chat_json_allocations<'a>(
    chat: &'a SavedChat,
    grammar: Option<&'a dyn HistoryJsonRetention>,
    visit: &mut dyn FnMut(RetainedAllocation<'a>) -> Result<(), RetentionError>,
) -> Result<(), RetentionError> {
    let mut walker = Visitor { visit };
    for row in &chat.transcript().entries {
        walker.row(row)?;
    }
    for prompt in &chat.composer().queue {
        walker.prompt(prompt)?;
    }
    if let Some(grammar) = grammar {
        grammar.visit_json_allocations(walker.visit)?;
    }
    Ok(())
}

struct Visitor<'a, 'v> {
    visit: &'v mut dyn FnMut(RetainedAllocation<'a>) -> Result<(), RetentionError>,
}
impl<'a> Visitor<'a, '_> {
    fn components(
        &mut self,
        components: [Option<RetainedAllocation<'a>>; 2],
    ) -> Result<(), RetentionError> {
        for component in components.into_iter().flatten() {
            (self.visit)(component)?;
        }
        Ok(())
    }
    fn json(&mut self, value: &'a OwnedJson) -> Result<(), RetentionError> {
        self.components(value.retained_allocations())
    }
    fn extensions(&mut self, value: Option<&'a Extensions>) -> Result<(), RetentionError> {
        if let Some(value) = value {
            self.components(value.retained_allocations())?;
        }
        Ok(())
    }
    fn annotations(&mut self, value: Option<&'a Annotations>) -> Result<(), RetentionError> {
        self.extensions(value.and_then(|value| value.extensions.as_ref()))
    }
    fn content(&mut self, value: &'a Content) -> Result<(), RetentionError> {
        let (annotations, extensions) = match value {
            Content::Text(value) => (value.annotations.as_ref(), value.extensions.as_ref()),
            Content::Media(value) => (value.annotations.as_ref(), value.extensions.as_ref()),
            Content::ResourceReference(value) => {
                (value.annotations.as_ref(), value.extensions.as_ref())
            }
            Content::EmbeddedResource(value) => {
                let body = match &value.body {
                    ResourceBody::Text(body) => body.extensions.as_ref(),
                    ResourceBody::Encoded(body) => body.extensions.as_ref(),
                };
                self.extensions(body)?;
                (value.annotations.as_ref(), value.extensions.as_ref())
            }
        };
        self.annotations(annotations)?;
        self.extensions(extensions)
    }
    fn tool(&mut self, value: &'a ToolRecord) -> Result<(), RetentionError> {
        if let Some(value) = &value.arguments {
            self.json(value)?;
        }
        if let Some(value) = &value.result {
            self.json(value)?;
        }
        self.extensions(value.extensions.as_ref())?;
        for output in &value.output {
            match output {
                ToolOutput::Content {
                    content,
                    extensions,
                } => {
                    self.content(content)?;
                    self.extensions(extensions.as_ref())?;
                }
                ToolOutput::FileDiff(value) => self.extensions(value.extensions.as_ref())?,
                ToolOutput::TerminalReference { extensions, .. } => {
                    self.extensions(extensions.as_ref())?
                }
            }
        }
        for location in &value.locations {
            self.extensions(location.extensions.as_ref())?;
        }
        if let Some(display) = &value.client_display {
            self.json(&display.arguments)?;
            if let Some(DisplayOutcome::Succeeded(value)) = display.outcome.value() {
                self.json(value)?;
            }
        }
        Ok(())
    }
    fn row(&mut self, value: &'a Entry) -> Result<(), RetentionError> {
        match value {
            Entry::Agent(_) | Entry::Thought(_) => Ok(()),
            Entry::User(values) => {
                for value in values {
                    self.content(value)?;
                }
                Ok(())
            }
            Entry::Content(value) => self.content(value),
            Entry::Tool(value) => self.tool(value),
        }
    }
    fn prompt(&mut self, value: &'a SavedPrompt) -> Result<(), RetentionError> {
        for image in &value.images {
            self.content(image)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
