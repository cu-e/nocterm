//! The host-driven model stepper. There is deliberately no tool executor field.
use crate::{
    attempt::ModelDriver,
    call::{AdvanceContext, ModelCalls},
    checkpoint::{self, EngineIdentity},
    mapping,
    state::ActiveRun,
};
use futures::{
    FutureExt,
    future::{AbortRegistration, Abortable, BoxFuture},
};
use nocterm_ai::model::session::StopReason;
use nocterm_cloud_model::{
    engine::{
        AgentEngine, EngineAction, EngineCheckpoint, EngineCompletion, EngineConfig, EngineError,
        EngineEventSink, EngineInput, EngineRequest, ModelCallAdmission,
    },
    identity::StreamRevision,
};
use rig_agent::run::{AgentRun, AgentRunStep, PromptError};
use rig_core::{DynModel, operation::Completion};
use std::sync::Arc;

pub(crate) struct RigEngine {
    config: EngineConfig,
    identity: EngineIdentity,
    model: ModelDriver,
    lease: crate::ProviderLease,
    state: Option<ActiveRun>,
    poisoned: bool,
}
impl RigEngine {
    pub(crate) fn new(
        config: EngineConfig,
        identity: EngineIdentity,
        model: DynModel<Completion>,
        lease: crate::ProviderLease,
        restore: Option<EngineCheckpoint>,
        calls: ModelCalls,
    ) -> Result<Self, EngineError> {
        let state = restore
            .map(|saved| checkpoint::restore(&config, &identity, saved))
            .transpose()?
            .flatten();
        let model = ModelDriver::new(model, config.budget, calls)?;
        Ok(Self {
            config,
            identity,
            model,
            lease,
            state,
            poisoned: false,
        })
    }
    async fn drive(
        &mut self,
        input: EngineInput,
        context: AdvanceContext,
    ) -> Result<EngineAction, EngineError> {
        match input.request {
            EngineRequest::Prompt(prompt) if self.state.is_none() => {
                let sdk = AgentRun::new(mapping::prompt(&prompt)?)
                    .max_turns(self.config.budget.turns() as usize)
                    .with_history(mapping::history(&self.config.history)?);
                self.state = Some(ActiveRun::new(sdk));
            }
            EngineRequest::ContinueHistory if self.state.is_none() => {
                if !matches!(self.config.history.messages().last(), Some(nocterm_cloud_model::engine::EngineHistoryMessage::ToolResult(_))) {
                    return Err(EngineError::InvalidStream);
                }
                let mut history = mapping::history(&self.config.history)?;
                let prompt = history.pop().ok_or(EngineError::InvalidStream)?;
                let sdk = AgentRun::new(prompt)
                    .max_turns(self.config.budget.turns() as usize)
                    .with_history(history);
                self.state = Some(ActiveRun::new(sdk));
            }
            EngineRequest::ToolResults(results) => {
                self.state
                    .as_mut()
                    .ok_or(EngineError::InvalidStream)?
                    .answer(results, self.config.budget.input_bytes())?;
            }
            _ => return Err(EngineError::InvalidStream),
        }
        let state = self.state.as_mut().ok_or(EngineError::InvalidStream)?;
        state.begin(input.next_step)?;
        let step = match state.sdk().next_step() {
            Ok(step) => step,
            Err(PromptError::MaxTurns { .. }) => {
                return Ok(EngineAction::Complete(EngineCompletion {
                    stop: StopReason::MaxTurnRequests,
                    usage: mapping::usage(state.sdk().usage()),
                }));
            }
            Err(_) => return Err(EngineError::InvalidStream),
        };
        let AgentRunStep::CallModel {
            prompt,
            history,
            turn,
        } = step
        else {
            return Err(EngineError::InvalidStream);
        };
        let finished = self
            .model
            .complete(state, &context, prompt, history, turn)
            .await?;
        let action = state.finish(input.next_step, finished)?;
        checkpoint::validate_size(&self.identity, state)?;
        Ok(action)
    }
}
impl AgentEngine for RigEngine {
    fn advance<'a>(
        &'a mut self,
        input: EngineInput,
        progress: Arc<dyn EngineEventSink>,
        admission: Arc<dyn ModelCallAdmission>,
        cancellation: AbortRegistration,
    ) -> BoxFuture<'a, Result<EngineAction, EngineError>> {
        async move {
            if self.poisoned {
                return Err(EngineError::Interrupted);
            }
            let (operation, lease_cancellation) = self.lease.admit()?;
            // Remains poisoned if the caller drops this future while it is
            // pending. Only a fully returned action restores checkpointability.
            self.poisoned = true;
            let context = AdvanceContext::new(
                self.config.run.clone(),
                input.next_step,
                progress,
                admission,
                cancellation.handle(),
            );
            let work = Abortable::new(self.drive(input, context), cancellation);
            let result = match Abortable::new(work, lease_cancellation).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(EngineError::Interrupted),
                Err(_) => Err(EngineError::NeedsProviderAccess),
            };
            drop(operation);
            // Expiry may be detected by the HTTP credential gate in the same
            // poll that finishes before Abortable observes its wakeup. Classify
            // that race as lost provider access, never an unrelated provider error.
            let result = if self.lease.is_available() {
                result
            } else {
                Err(EngineError::NeedsProviderAccess)
            };
            // Retrying requires the host's last durable checkpoint, so partial
            // progress is never appended twice by silently advancing this owner.
            self.poisoned = result.is_err();
            result
        }
        .boxed()
    }
    fn checkpoint(&self, revision: StreamRevision) -> Result<EngineCheckpoint, EngineError> {
        if self.poisoned {
            return Err(EngineError::Interrupted);
        }
        checkpoint::save(&self.config, &self.identity, self.state.as_ref(), revision)
    }
}
