//! Bounded neutral prior conversation supplied by the authoritative journal.
//! Historical calls are descriptive records and convey no execution authority.
use super::{EngineError, EnginePrompt, MAX_ENGINE_TOOL_CALLS};
use crate::{
    identity::IntentId,
    payload::{BoundedJson, MAX_LOGICAL_PAYLOAD_BYTES, require_size},
};
use nocterm_ai::{TerminalCall, history::text_size};
use std::{collections::BTreeSet, fmt, sync::Arc};

const MAX_HISTORY_MESSAGES: usize = 4096;

#[derive(Clone)]
pub struct HistoricalToolCall {
    intent: IntentId,
    call: TerminalCall,
    arguments: BoundedJson,
}
impl HistoricalToolCall {
    pub fn new(intent: IntentId, call: TerminalCall) -> Result<Self, EngineError> {
        call.validate()?;
        let arguments = BoundedJson::new(call.arguments()?)?;
        Ok(Self {
            intent,
            call,
            arguments,
        })
    }
    pub fn intent(&self) -> IntentId {
        self.intent
    }
    pub fn call(&self) -> &TerminalCall {
        &self.call
    }
    pub fn arguments(&self) -> &BoundedJson {
        &self.arguments
    }
}
impl fmt::Debug for HistoricalToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoricalToolCall")
            .field("intent", &self.intent)
            .field("tool", &self.call.name())
            .finish()
    }
}
#[derive(Clone)]
pub struct HistoricalAssistant {
    text: String,
    calls: Vec<HistoricalToolCall>,
    bytes: usize,
}
impl HistoricalAssistant {
    pub fn new(text: String, calls: Vec<HistoricalToolCall>) -> Result<Self, EngineError> {
        if (text.trim().is_empty() && calls.is_empty()) || calls.len() > MAX_ENGINE_TOOL_CALLS {
            return Err(EngineError::InvalidStream);
        }
        require_size(text.len(), MAX_LOGICAL_PAYLOAD_BYTES)?;
        let mut bytes = text_size(&text)
            .map_err(crate::payload::PayloadError::from)?
            .saturating_add(128);
        for call in &calls {
            bytes = bytes
                .saturating_add(call.arguments.encoded_bytes())
                .saturating_add(256);
        }
        require_size(bytes, MAX_LOGICAL_PAYLOAD_BYTES)?;
        Ok(Self { text, calls, bytes })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn calls(&self) -> &[HistoricalToolCall] {
        &self.calls
    }
}
impl fmt::Debug for HistoricalAssistant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoricalAssistant")
            .field("text_bytes", &self.text.len())
            .field("calls", &self.calls.len())
            .finish()
    }
}
#[derive(Clone, Debug)]
pub struct HistoricalToolResult {
    intent: IntentId,
    output: BoundedJson,
    failed: bool,
}
impl HistoricalToolResult {
    pub fn new(intent: IntentId, output: BoundedJson, failed: bool) -> Self {
        Self {
            intent,
            output,
            failed,
        }
    }
    pub fn intent(&self) -> IntentId {
        self.intent
    }
    pub fn output(&self) -> &BoundedJson {
        &self.output
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
}
#[derive(Clone, Debug)]
pub enum EngineHistoryMessage {
    User(EnginePrompt),
    Assistant(HistoricalAssistant),
    ToolResult(HistoricalToolResult),
}
#[derive(Clone, Default)]
pub struct EngineHistory {
    messages: Arc<[EngineHistoryMessage]>,
    bytes: usize,
}
impl EngineHistory {
    /// Reject unmatched/reused intents and unfinished tool batches. An old
    /// Unknown effect needs an explicit host reconciliation result before it
    /// can enter this model context; it is never silently dispatched again.
    pub fn new(messages: Vec<EngineHistoryMessage>) -> Result<Self, EngineError> {
        if messages.len() > MAX_HISTORY_MESSAGES {
            return Err(EngineError::BudgetExceeded);
        }
        let mut pending = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut bytes = 2usize;
        for message in &messages {
            let size = match message {
                EngineHistoryMessage::User(prompt) if pending.is_empty() => {
                    prompt.encoded_bytes().saturating_add(128)
                }
                EngineHistoryMessage::Assistant(assistant) if pending.is_empty() => {
                    for call in assistant.calls() {
                        if !seen.insert(call.intent()) {
                            return Err(EngineError::InvalidStream);
                        }
                        pending.insert(call.intent());
                    }
                    assistant.bytes
                }
                EngineHistoryMessage::ToolResult(result) if pending.remove(&result.intent()) => {
                    result.output.encoded_bytes().saturating_add(256)
                }
                _ => return Err(EngineError::InvalidStream),
            };
            bytes = bytes.saturating_add(size);
            require_size(bytes, MAX_LOGICAL_PAYLOAD_BYTES)?;
        }
        if !pending.is_empty() {
            return Err(EngineError::InvalidStream);
        }
        Ok(Self {
            messages: messages.into(),
            bytes,
        })
    }
    pub fn messages(&self) -> &[EngineHistoryMessage] {
        &self.messages
    }
    /// Conservative JSON context size including role/intent framing, before
    /// the adapter applies its exact provider request and cumulative budget.
    pub fn encoded_bytes(&self) -> usize {
        self.bytes
    }
}
impl fmt::Debug for EngineHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineHistory")
            .field("messages", &self.messages.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests;
