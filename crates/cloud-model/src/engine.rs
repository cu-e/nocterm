//! Host-driven model stepping: the engine reports tool requests and cannot execute them.
mod admission;
pub mod history;
use crate::{
    authority::RunBinding,
    identity::{NamespaceId, StepId, StreamRevision},
    payload::{
        BoundedJson, MAX_LOGICAL_PAYLOAD_BYTES, MAX_PROGRESS_BYTES, OpaqueBytes, PayloadError,
        require_size,
    },
};
pub use admission::{
    MAX_MODEL_OUTPUT_TOKENS, ModelCallAdmission, ModelCallObservation, ModelCallRequest,
};
use futures::future::{AbortRegistration, BoxFuture};
pub use history::{
    EngineHistory, EngineHistoryMessage, HistoricalAssistant, HistoricalToolCall,
    HistoricalToolResult,
};
use nocterm_ai::{
    TerminalCall, TerminalCallError,
    history::content_size,
    model::{content::Content, session::StopReason},
};
use nocterm_core::SharedCause;
use std::{collections::BTreeSet, fmt, sync::Arc};

pub const MAX_ENGINE_TOOL_CALLS: usize = 64;
const MAX_PROMPT_PARTS: usize = 256;

#[derive(Clone)]
pub struct EnginePrompt {
    content: Arc<[Content]>,
    encoded_bytes: usize,
}
impl EnginePrompt {
    pub fn new(content: Vec<Content>) -> Result<Self, EngineError> {
        if content.is_empty() || content.len() > MAX_PROMPT_PARTS {
            return Err(EngineError::InvalidPrompt);
        }
        let mut encoded_bytes = 1usize;
        for part in &content {
            match part {
                Content::Text(_) => {}
                Content::Image(image) if image.uri.is_none() => {}
                _ => return Err(EngineError::UnsupportedContent),
            }
            encoded_bytes = encoded_bytes
                .saturating_add(content_size(part).map_err(PayloadError::from)?)
                .saturating_add(1);
            require_size(encoded_bytes, MAX_LOGICAL_PAYLOAD_BYTES)?;
        }
        Ok(Self {
            content: content.into(),
            encoded_bytes,
        })
    }
    pub fn content(&self) -> &[Content] {
        &self.content
    }
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}
impl fmt::Debug for EnginePrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnginePrompt")
            .field("parts", &self.content.len())
            .field("bytes", &self.encoded_bytes)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct EngineCheckpoint {
    run: RunBinding,
    format: NamespaceId,
    journal_revision: StreamRevision,
    bytes: OpaqueBytes,
}
impl EngineCheckpoint {
    pub fn new(
        run: RunBinding,
        format: NamespaceId,
        journal_revision: StreamRevision,
        bytes: OpaqueBytes,
    ) -> Result<Self, EngineError> {
        if bytes.is_empty() {
            return Err(EngineError::UnsupportedFormat);
        }
        require_size(bytes.len(), MAX_LOGICAL_PAYLOAD_BYTES)?;
        Ok(Self {
            run,
            format,
            journal_revision,
            bytes,
        })
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn format(&self) -> &NamespaceId {
        &self.format
    }
    pub fn journal_revision(&self) -> StreamRevision {
        self.journal_revision
    }
    pub fn bytes(&self) -> &OpaqueBytes {
        &self.bytes
    }
    pub fn require_context(&self, run: &RunBinding, at: StreamRevision) -> Result<(), EngineError> {
        if &self.run != run || self.journal_revision != at {
            return Err(EngineError::InvalidStream);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolSlot {
    step: StepId,
    index: u32,
}
impl ToolSlot {
    pub fn new(step: StepId, index: u32) -> Result<Self, EngineError> {
        if index as usize >= MAX_ENGINE_TOOL_CALLS {
            return Err(EngineError::InvalidStream);
        }
        Ok(Self { step, index })
    }
    pub fn step(self) -> StepId {
        self.step
    }
    pub fn index(self) -> u32 {
        self.index
    }
}

#[derive(Clone)]
pub struct EngineToolCall {
    slot: ToolSlot,
    provider_id: Option<NamespaceId>,
    call: TerminalCall,
}
impl EngineToolCall {
    pub fn new(
        slot: ToolSlot,
        provider_id: Option<NamespaceId>,
        call: TerminalCall,
    ) -> Result<Self, EngineError> {
        call.validate()?;
        Ok(Self {
            slot,
            provider_id,
            call,
        })
    }
    pub fn slot(&self) -> ToolSlot {
        self.slot
    }
    pub fn provider_id(&self) -> Option<&NamespaceId> {
        self.provider_id.as_ref()
    }
    pub fn call(&self) -> &TerminalCall {
        &self.call
    }
}
impl fmt::Debug for EngineToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineToolCall")
            .field("slot", &self.slot)
            .field("tool", &self.call.name())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct EngineToolBatch {
    step: StepId,
    calls: Vec<EngineToolCall>,
}
impl EngineToolBatch {
    pub fn new(step: StepId, calls: Vec<EngineToolCall>) -> Result<Self, EngineError> {
        validate_slots(step, calls.iter().map(EngineToolCall::slot))?;
        Ok(Self { step, calls })
    }
    pub fn step(&self) -> StepId {
        self.step
    }
    pub fn calls(&self) -> &[EngineToolCall] {
        &self.calls
    }
}

#[derive(Clone, Debug)]
pub struct EngineToolResult {
    slot: ToolSlot,
    output: BoundedJson,
    failed: bool,
}
impl EngineToolResult {
    pub fn new(slot: ToolSlot, output: BoundedJson, failed: bool) -> Self {
        Self {
            slot,
            output,
            failed,
        }
    }
    pub fn slot(&self) -> ToolSlot {
        self.slot
    }
    pub fn output(&self) -> &BoundedJson {
        &self.output
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
}

#[derive(Clone, Debug)]
pub struct EngineToolResults {
    step: StepId,
    results: Vec<EngineToolResult>,
}
impl EngineToolResults {
    pub fn new(step: StepId, results: Vec<EngineToolResult>) -> Result<Self, EngineError> {
        validate_slots(step, results.iter().map(EngineToolResult::slot))?;
        Ok(Self { step, results })
    }
    pub fn step(&self) -> StepId {
        self.step
    }
    pub fn results(&self) -> &[EngineToolResult] {
        &self.results
    }
}

fn validate_slots(step: StepId, slots: impl Iterator<Item = ToolSlot>) -> Result<(), EngineError> {
    let mut seen = BTreeSet::new();
    for slot in slots {
        if slot.step != step || !seen.insert(slot) {
            return Err(EngineError::InvalidStream);
        }
        if seen.len() > MAX_ENGINE_TOOL_CALLS {
            return Err(EngineError::BudgetExceeded);
        }
    }
    if seen.is_empty() {
        return Err(EngineError::InvalidStream);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageObservation {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub thought_tokens: Option<u64>,
    pub cost_micros: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextChannel {
    Assistant,
    Thought,
}
#[derive(Clone)]
pub struct EngineText {
    channel: TextChannel,
    text: String,
}
impl EngineText {
    pub fn new(channel: TextChannel, text: String) -> Result<Self, EngineError> {
        require_size(text.len(), MAX_PROGRESS_BYTES)?;
        Ok(Self { channel, text })
    }
    pub fn channel(&self) -> TextChannel {
        self.channel
    }
    pub fn text(&self) -> &str {
        &self.text
    }
}
impl fmt::Debug for EngineText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineText")
            .field("channel", &self.channel)
            .field("bytes", &self.text.len())
            .finish()
    }
}
#[derive(Clone, Debug)]
pub enum EngineEvent {
    Text(EngineText),
    /// The completed original HTTP call's provider report, after reconcile ACK.
    /// The coordinator has already atomically published the complete cumulative
    /// snapshot with Budget reconciliation; this callback validates the original
    /// report without publishing or adding a second usage fact.
    Usage(UsageObservation),
}

/// Applying backpressure here bounds model production behind durable host delivery.
pub trait EngineEventSink: Send + Sync {
    fn publish(
        &self,
        step: StepId,
        event: EngineEvent,
    ) -> BoxFuture<'static, Result<(), EngineError>>;
}

#[derive(Clone, Debug)]
pub enum EngineRequest {
    Prompt(EnginePrompt),
    ToolResults(EngineToolResults),
    /// Fresh engine only: continue complete authoritative history ending in a
    /// real tool result, without inventing a user prompt or execution authority.
    ContinueHistory,
}
#[derive(Clone, Debug)]
pub struct EngineInput {
    pub next_step: StepId,
    pub request: EngineRequest,
}
#[derive(Clone, Debug)]
pub struct EngineCompletion {
    pub stop: StopReason,
    pub usage: UsageObservation,
}
#[derive(Clone, Debug)]
pub enum EngineAction {
    Tools(EngineToolBatch),
    Complete(EngineCompletion),
}

#[derive(Clone, Copy, Debug)]
pub struct ModelBudget {
    calls: u32,
    turns: u32,
    input_bytes: usize,
    output_bytes: usize,
    output_tokens: u64,
}
impl ModelBudget {
    pub fn new(
        turns: u32,
        input_bytes: usize,
        output_bytes: usize,
        output_tokens: u64,
    ) -> Result<Self, EngineError> {
        if turns == 0
            || turns > 1024
            || input_bytes == 0
            || input_bytes > MAX_LOGICAL_PAYLOAD_BYTES
            || output_bytes == 0
            || output_bytes > MAX_LOGICAL_PAYLOAD_BYTES
            || output_tokens == 0
            || output_tokens > MAX_MODEL_OUTPUT_TOKENS
        {
            return Err(EngineError::BudgetExceeded);
        }
        Ok(Self {
            calls: turns,
            turns,
            input_bytes,
            output_bytes,
            output_tokens,
        })
    }
    /// Sets an independent finite actual-HTTP allowance. This never authorizes
    /// a repeated StepId/AttemptId or an automatic provider retry.
    pub fn with_calls(mut self, calls: u32) -> Result<Self, EngineError> {
        if calls == 0 || calls > 1024 {
            return Err(EngineError::BudgetExceeded);
        }
        self.calls = calls;
        Ok(self)
    }
    pub fn calls(&self) -> u32 {
        self.calls
    }
    pub fn turns(&self) -> u32 {
        self.turns
    }
    pub fn input_bytes(&self) -> usize {
        self.input_bytes
    }
    pub fn output_bytes(&self) -> usize {
        self.output_bytes
    }
    pub fn output_tokens(&self) -> u64 {
        self.output_tokens
    }
}
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub run: RunBinding,
    pub model: NamespaceId,
    pub history: EngineHistory,
    pub budget: ModelBudget,
}

pub trait AgentEngine: Send {
    /// Only a verified complete provider turn may return Tools/Complete. Partial
    /// progress may be published, but never authorizes tools or a successful end.
    fn advance<'a>(
        &'a mut self,
        input: EngineInput,
        progress: Arc<dyn EngineEventSink>,
        admission: Arc<dyn ModelCallAdmission>,
        cancellation: AbortRegistration,
    ) -> BoxFuture<'a, Result<EngineAction, EngineError>>;
    /// Versioned SDK-internal derived state, anchored to the authoritative journal.
    fn checkpoint(&self, journal_revision: StreamRevision)
    -> Result<EngineCheckpoint, EngineError>;
}
pub trait AgentEngineFactory: Send + Sync {
    /// Provider leases are explicitly installed in this factory, never prompt/config data.
    fn create(
        &self,
        config: EngineConfig,
        restore: Option<EngineCheckpoint>,
    ) -> Result<Box<dyn AgentEngine>, EngineError>;
}

#[derive(Clone, thiserror::Error)]
pub enum EngineError {
    #[error("the prompt is empty or exceeds its collection limit")]
    InvalidPrompt,
    #[error("this engine does not support the supplied content")]
    UnsupportedContent,
    #[error("the checkpoint format is not supported")]
    UnsupportedFormat,
    #[error("the provider stream or tool correlation is invalid or incomplete")]
    InvalidStream,
    #[error("the engine operation was interrupted")]
    Interrupted,
    #[error("the engine budget is exhausted or invalid")]
    BudgetExceeded,
    /// The complete model input exceeds a known request ceiling before HTTP.
    #[error("the model request exceeds its input limit of {limit} bytes")]
    RequestTooLarge { limit: usize },
    #[error("provider access must be installed again")]
    NeedsProviderAccess,
    #[error("the configured model needs an unsupported provider capability")]
    UnsupportedCapability(#[source] SharedCause),
    #[error("the historical tool arguments could not be encoded")]
    ToolEncoding(#[from] nocterm_ai::ToolArgumentsError),
    #[error("the engine is unavailable")]
    Unavailable,
    #[error("the tool arguments are invalid")]
    InvalidTool(#[from] TerminalCallError),
    #[error("an engine payload was refused")]
    Payload(#[from] PayloadError),
    #[error("the provider request failed")]
    Provider(#[source] SharedCause),
    #[error("the host refused model-call admission")]
    Admission(#[source] SharedCause),
    #[error("progress could not be durably delivered")]
    Progress(#[source] SharedCause),
}
impl fmt::Debug for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidPrompt => "InvalidPrompt",
            Self::UnsupportedContent => "UnsupportedContent",
            Self::UnsupportedFormat => "UnsupportedFormat",
            Self::InvalidStream => "InvalidStream",
            Self::Interrupted => "Interrupted",
            Self::BudgetExceeded => "BudgetExceeded",
            Self::RequestTooLarge { .. } => "RequestTooLarge",
            Self::Unavailable => "Unavailable",
            Self::NeedsProviderAccess => "NeedsProviderAccess",
            Self::UnsupportedCapability(_) => "UnsupportedCapability",
            Self::ToolEncoding(_) => "ToolEncoding",
            Self::InvalidTool(_) => "InvalidTool",
            Self::Payload(_) => "Payload",
            Self::Provider(_) => "Provider",
            Self::Progress(_) => "Progress",
            Self::Admission(_) => "Admission",
        })
    }
}
