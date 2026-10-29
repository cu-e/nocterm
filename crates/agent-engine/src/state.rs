//! SDK run ownership and host slot correlation, without any effect executor.
use crate::{attempt::FinishedTurn, failure::provider_error, limits::Accounting, mapping};
use nocterm_cloud_model::{
    engine::{
        EngineAction, EngineCompletion, EngineError, EngineToolBatch, EngineToolResults, ToolSlot,
    },
    identity::StepId,
};
use rig_agent::run::{AgentRun, AgentRunStep, ExecCall, ModelTurnOutcome, PendingToolCall};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActiveRun {
    sdk: AgentRun,
    pending: Option<[u8; 16]>,
    steps: Vec<[u8; 16]>,
    accounting: Accounting,
}
impl ActiveRun {
    pub(crate) fn new(sdk: AgentRun) -> Self {
        Self {
            sdk,
            pending: None,
            steps: Vec::new(),
            accounting: Accounting::default(),
        }
    }
    pub(crate) fn sdk(&mut self) -> &mut AgentRun {
        &mut self.sdk
    }
    pub(crate) fn accounting(&mut self) -> &mut Accounting {
        &mut self.accounting
    }
    pub(crate) fn require_budget(
        &self,
        budget: nocterm_cloud_model::engine::ModelBudget,
    ) -> Result<(), EngineError> {
        self.accounting.validate(budget)?;
        if self.steps.len() > budget.turns() as usize + 1 {
            return Err(EngineError::UnsupportedFormat);
        }
        Ok(())
    }
    pub(crate) fn begin(&mut self, step: StepId) -> Result<(), EngineError> {
        if self.steps.contains(step.as_bytes()) {
            return Err(EngineError::InvalidStream);
        }
        self.steps.push(*step.as_bytes());
        Ok(())
    }
    pub(crate) fn batch(
        &mut self,
        step: StepId,
        calls: Vec<PendingToolCall>,
    ) -> Result<EngineToolBatch, EngineError> {
        let executable = executable(calls)?;
        let calls = executable
            .iter()
            .enumerate()
            .map(|(index, call)| mapping::tool(ToolSlot::new(step, index as u32)?, call))
            .collect::<Result<_, _>>()?;
        let batch = EngineToolBatch::new(step, calls)?;
        self.pending = Some(*step.as_bytes());
        Ok(batch)
    }
    pub(crate) fn finish(
        &mut self,
        step: StepId,
        finished: FinishedTurn,
    ) -> Result<EngineAction, EngineError> {
        let outcome = self
            .sdk
            .model_response(finished.turn)
            .map_err(|_| EngineError::InvalidStream)?;
        if !matches!(outcome, ModelTurnOutcome::Continue { .. }) {
            return Err(EngineError::InvalidStream);
        }
        match self.sdk.next_step().map_err(provider_error)? {
            AgentRunStep::CallTools { calls } => Ok(EngineAction::Tools(self.batch(step, calls)?)),
            AgentRunStep::Done(done) => Ok(EngineAction::Complete(EngineCompletion {
                stop: finished.stop,
                usage: mapping::usage(done.usage),
            })),
            _ => Err(EngineError::InvalidStream),
        }
    }
    pub(crate) fn answer(
        &mut self,
        results: EngineToolResults,
        max_bytes: usize,
    ) -> Result<(), EngineError> {
        let bytes = results.results().iter().fold(0usize, |sum, result| {
            sum.saturating_add(result.output().encoded_bytes())
                .saturating_add(256)
        });
        if bytes > max_bytes {
            return Err(EngineError::BudgetExceeded);
        }
        if self.pending.as_ref() != Some(results.step().as_bytes()) {
            return Err(EngineError::InvalidStream);
        }
        // Validate every slot before mutating the SDK state. Neither duplicate
        // provider IDs nor repeated tool names choose an answer's destination.
        let mut candidate = self.sdk.clone();
        let AgentRunStep::CallTools { calls } = candidate
            .next_step()
            .map_err(|_| EngineError::InvalidStream)?
        else {
            return Err(EngineError::InvalidStream);
        };
        let calls = executable(calls)?;
        if results.results().len() != calls.len() {
            return Err(EngineError::InvalidStream);
        }
        let mut ordered = vec![None; calls.len()];
        for result in results.results() {
            let index = result.slot().index() as usize;
            let slot = ordered.get_mut(index).ok_or(EngineError::InvalidStream)?;
            if slot.replace(result).is_some() {
                return Err(EngineError::InvalidStream);
            }
        }
        for (call, result) in calls.into_iter().zip(ordered) {
            let result = result.ok_or(EngineError::InvalidStream)?;
            let answer = mapping::result(result.output(), result.failed());
            candidate
                .answer(call.answer(answer))
                .map_err(|_| EngineError::InvalidStream)?;
        }
        self.sdk = candidate;
        self.pending = None;
        Ok(())
    }
}
fn executable(calls: Vec<PendingToolCall>) -> Result<Vec<ExecCall>, EngineError> {
    if calls.is_empty() || calls.len() > nocterm_cloud_model::engine::MAX_ENGINE_TOOL_CALLS {
        return Err(EngineError::InvalidStream);
    }
    calls
        .into_iter()
        .map(|call| match call {
            PendingToolCall::Execute(call) => Ok(call),
            _ => Err(EngineError::InvalidStream),
        })
        .collect()
}
