//! One explicitly configured provider model and its bounded streaming policy.
use crate::{
    call::{AdvanceContext, ModelCalls},
    failure::{UnsupportedContinuation, provider_error},
    limits, mapping,
    progress::{ProgressBuffer, StreamFragment},
    state::ActiveRun,
};
use futures::StreamExt;
use nocterm_ai::model::session::StopReason;
use nocterm_cloud_model::engine::{EngineError, EngineEvent, ModelBudget, ModelCallObservation};
use nocterm_core::SharedCause;
use rig_agent::run::{ModelTurn, TurnPolicy};
use rig_core::{
    DynModel,
    completion::{CompletionRequest, FinishReason, ToolDefinition},
    message::Message,
    operation::Completion,
};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) struct ModelDriver {
    model: DynModel<Completion>,
    budget: ModelBudget,
    tools: Vec<Arc<ToolDefinition>>,
    calls: ModelCalls,
}
pub(crate) struct FinishedTurn {
    pub(crate) turn: ModelTurn,
    pub(crate) stop: StopReason,
}
impl ModelDriver {
    pub(crate) fn new(
        model: DynModel<Completion>,
        budget: ModelBudget,
        calls: ModelCalls,
        allowed: Option<&BTreeSet<String>>,
    ) -> Result<Self, EngineError> {
        Ok(Self {
            model,
            budget,
            tools: mapping::tools(allowed)?,
            calls,
        })
    }
    fn policy(&self) -> Result<TurnPolicy, EngineError> {
        TurnPolicy::new(
            self.tools
                .iter()
                .map(|spec| spec.name.to_string())
                .collect(),
            None,
            None,
        )
        .map_err(|_| EngineError::InvalidStream)
    }
    pub(crate) async fn complete(
        &self,
        state: &mut ActiveRun,
        context: &AdvanceContext,
        prompt: Arc<Message>,
        history: Vec<Arc<Message>>,
        turn: usize,
    ) -> Result<FinishedTurn, EngineError> {
        tokio::runtime::Handle::try_current().map_err(|_| EngineError::Unavailable)?;
        state.sdk().advertise_tools(turn, self.tools.clone());
        let request = CompletionRequest::new(prompt)
            .reasoning(rig_core::completion::Reasoning::Off)
            .messages(history)
            .tools(self.tools.clone());
        let bytes = limits::encode(&request, self.budget.input_bytes())?.len();
        let before = state.accounting().output_bytes();
        let reserved = state.accounting().reserve(self.budget, bytes)?;
        let call = self.calls.begin(context, reserved)?;
        let result = async {
            let mut stream = self
                .model
                .stream(request.max_tokens(reserved))
                .map_err(provider_error)?;
            let mut progress = ProgressBuffer::new(context.step(), context.progress().clone());
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    item = stream.next() => {
                        let Some(item) = item else { break; };
                        let fragment = StreamFragment::from(item.map_err(provider_error)?);
                        state.accounting().observe(self.budget, fragment.bytes())?;
                        progress.push(fragment).await?;
                    }
                    _ = tick.tick() => progress.flush().await?,
                }
            }
            progress.flush().await?;
            // EOF alone, a completed tool fragment, or a partial SDK snapshot never
            // authorizes a tool. Consume the actual Rig provider stream's finish.
            let response = stream.finish().await.map_err(provider_error)?;
            if response.choice.iter().any(|part| {
                matches!(
                    part,
                    rig_core::message::AssistantContent::Reasoning(_)
                        | rig_core::message::AssistantContent::Opaque(_)
                )
            }) {
                return Err(EngineError::UnsupportedCapability(SharedCause::new(
                    UnsupportedContinuation,
                )));
            }
            let stop = terminal_reason(
                response.finish_reason(),
                response.tool_calls().next().is_some(),
            )?;
            state.accounting().reconcile(reserved, response.usage)?;
            let output_bytes = state
                .accounting()
                .output_bytes()
                .checked_sub(before)
                .ok_or(EngineError::InvalidStream)?;
            call.reconcile(ModelCallObservation::new(
                mapping::usage(response.usage),
                output_bytes,
            )?)
            .await?;
            context
                .progress()
                .publish(
                    context.step(),
                    EngineEvent::Usage(mapping::usage(response.usage)),
                )
                .await?;
            call.check_current()?;
            let policy = self.policy()?;
            Ok(FinishedTurn {
                turn: ModelTurn::from_policy(&response, policy),
                stop,
            })
        }
        .await;
        match call.failure() {
            Some(error) => Err(error),
            None => result,
        }
    }
}
fn terminal_reason(reason: Option<FinishReason>, tools: bool) -> Result<StopReason, EngineError> {
    match (reason, tools) {
        (Some(FinishReason::Stop), false) | (Some(FinishReason::ToolCalls), true) => {
            Ok(StopReason::EndTurn)
        }
        (Some(FinishReason::Length), false) => Ok(StopReason::MaxTokens),
        (Some(FinishReason::ContentFilter), false) => Ok(StopReason::Refusal),
        _ => Err(EngineError::InvalidStream),
    }
}
