//! Bounded serialization and conservative per-run model accounting.
use nocterm_cloud_model::engine::{EngineError, ModelBudget};
use rig_core::completion::Usage;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

struct JsonBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for JsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("bounded JSON capacity"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, EngineError> {
    let mut buffer = JsonBuffer {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut buffer, value);
    if buffer.exceeded {
        return Err(EngineError::BudgetExceeded);
    }
    result.map_err(|_| EngineError::UnsupportedFormat)?;
    Ok(buffer.bytes)
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Accounting {
    calls: u32,
    turns: u32,
    input_bytes: usize,
    output_bytes: usize,
    output_tokens: u64,
}
impl Accounting {
    pub(crate) fn output_bytes(&self) -> usize {
        self.output_bytes
    }
    /// Reserve before opening HTTP. Unknown output usage retains the entire
    /// reservation, so another turn cannot silently assume a free request.
    pub(crate) fn reserve(
        &mut self,
        budget: ModelBudget,
        bytes: usize,
    ) -> Result<u64, EngineError> {
        let tokens = budget.output_tokens().saturating_sub(self.output_tokens);
        if self.calls >= budget.calls()
            || self.turns >= budget.turns()
            || tokens == 0
            || bytes > budget.input_bytes().saturating_sub(self.input_bytes)
        {
            return Err(EngineError::BudgetExceeded);
        }
        self.calls += 1;
        self.turns += 1;
        self.input_bytes += bytes;
        self.output_tokens = budget.output_tokens();
        Ok(tokens)
    }
    pub(crate) fn reconcile(&mut self, reserved: u64, usage: Usage) -> Result<(), EngineError> {
        if let Some(used) = usage.output_tokens {
            if used > reserved {
                return Err(EngineError::BudgetExceeded);
            }
            self.output_tokens -= reserved - used;
        }
        Ok(())
    }
    pub(crate) fn observe(&mut self, budget: ModelBudget, bytes: usize) -> Result<(), EngineError> {
        if bytes > budget.output_bytes().saturating_sub(self.output_bytes) {
            return Err(EngineError::BudgetExceeded);
        }
        self.output_bytes += bytes;
        Ok(())
    }
    pub(crate) fn validate(&self, budget: ModelBudget) -> Result<(), EngineError> {
        if self.calls > budget.calls()
            || self.turns > budget.turns()
            || self.input_bytes > budget.input_bytes()
            || self.output_bytes > budget.output_bytes()
            || self.output_tokens > budget.output_tokens()
        {
            return Err(EngineError::UnsupportedFormat);
        }
        Ok(())
    }
}
