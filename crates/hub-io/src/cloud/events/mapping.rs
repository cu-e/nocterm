//! Native semantic observations enter the public codec without native frame decoding.
use super::{
    prompt,
    tools::{self, DisplayJson, JsonText},
    values,
};
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::CoordinatorEvent,
    engine::{EngineEvent, TextChannel},
    journal::{JournalError, PublicationWorkBudget},
    tool::{BackgroundCommand, ToolEffectStatus},
};
use nocterm_hub_model::cloud::events::{
    BackgroundJob, CloudEventPayload, CloudTextChannel, DisplayOutcome, ObservedEffect,
    ToolObservationPayload, ToolPresentation,
};

pub(super) fn original_run(event: &CoordinatorEvent) -> Option<&RunBinding> {
    match event {
        CoordinatorEvent::CommandAccepted { run, .. } => run.as_ref(),
        CoordinatorEvent::PromptAccepted { run, .. }
        | CoordinatorEvent::Status { run, .. }
        | CoordinatorEvent::Progress { run, .. }
        | CoordinatorEvent::ToolResult { run, .. } => Some(run),
        CoordinatorEvent::ToolIntent { intent, .. } | CoordinatorEvent::Approval { intent, .. } => {
            Some(intent.authority().run())
        }
        CoordinatorEvent::ApprovalResolved { answer, .. } => Some(answer.authority().run()),
        CoordinatorEvent::Deleted { .. } => None,
    }
}

/// Every borrowed field remains alive through the synchronous encoder callback.
/// Prepared JSON and image buffers retain their own actual work reservations.
pub(super) fn with_payload<T>(
    event: &CoordinatorEvent,
    work: &dyn PublicationWorkBudget,
    emit: impl for<'a> FnOnce(CloudEventPayload<'a>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    match event {
        CoordinatorEvent::CommandAccepted { command, run } => {
            emit(CloudEventPayload::CommandAccepted {
                command: values::accepted(command)?,
                run: run.as_ref().map(values::run),
            })
        }
        CoordinatorEvent::PromptAccepted {
            command,
            run,
            request,
        } => prompt::with_prompt(request, work, |prompt| {
            emit(CloudEventPayload::PromptAccepted {
                command: values::accepted(command)?,
                run: values::run(run),
                prompt: Box::new(prompt),
            })
        }),
        CoordinatorEvent::Status { run, status } => emit(CloudEventPayload::Status {
            run: values::run(run),
            status: values::status(*status),
        }),
        CoordinatorEvent::Progress { run, event } => progress(run, event, emit),
        CoordinatorEvent::ToolIntent {
            intent,
            call,
            display,
        } => {
            let prepared = DisplayJson::new(display, work)?;
            let intent = tools::intent(intent, call, prepared.view(display))?;
            emit(CloudEventPayload::ToolIntent(Box::new(intent)))
        }
        CoordinatorEvent::Approval {
            intent,
            call,
            expires,
        } => {
            let arguments = tools::call_json(call, work)?;
            let display = ToolPresentation {
                tool: call.name(),
                arguments_json: arguments.text(),
                destination: None,
                redacted: false,
                outcome: Some(DisplayOutcome::Pending),
            };
            let intent = tools::intent(intent, call, display)?;
            emit(CloudEventPayload::Approval {
                intent: Box::new(intent),
                expires_at_ms: expires.as_millis(),
            })
        }
        CoordinatorEvent::ApprovalResolved { command, answer } => {
            emit(CloudEventPayload::ApprovalResolved {
                command: *command.as_bytes(),
                answer: Box::new(values::approval(answer)?),
            })
        }
        CoordinatorEvent::ToolResult { run, observation } => {
            with_observation(run, observation, work, emit)
        }
        CoordinatorEvent::Deleted { command } => emit(CloudEventPayload::Deleted {
            command: *command.as_bytes(),
        }),
    }
}

fn with_observation<T>(
    run: &RunBinding,
    observation: &nocterm_cloud_model::coordinator::ToolObservation,
    work: &dyn PublicationWorkBudget,
    emit: impl for<'a> FnOnce(CloudEventPayload<'a>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    let output = JsonText::new(observation.output().value(), work)?;
    let display = DisplayJson::new(observation.display(), work)?;
    let effect = match observation.effect() {
        ToolEffectStatus::Observed => ObservedEffect::Observed,
        ToolEffectStatus::Unknown => ObservedEffect::Unknown,
        ToolEffectStatus::Background(command) => {
            ObservedEffect::Background(Box::new(background(command)?))
        }
    };
    emit(CloudEventPayload::ToolResult {
        run: values::run(run),
        observation: Box::new(ToolObservationPayload {
            intent: *observation.intent().as_bytes(),
            output_json: output.text(),
            display: display.view(observation.display()),
            effect,
            failed: observation.is_failed(),
        }),
    })
}

fn progress<T>(
    run: &RunBinding,
    event: &EngineEvent,
    emit: impl for<'a> FnOnce(CloudEventPayload<'a>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    let run = values::run(run);
    match event {
        EngineEvent::Text(text) => emit(CloudEventPayload::Text {
            run,
            step: None, // Native Progress has no StepId; never invent one.
            channel: match text.channel() {
                TextChannel::Assistant => CloudTextChannel::Assistant,
                TextChannel::Thought => CloudTextChannel::Thought,
            },
            text: text.text(),
        }),
        EngineEvent::Usage(usage) => emit(CloudEventPayload::Usage {
            run,
            usage: values::usage(*usage),
        }),
    }
}

pub(super) fn background(command: &BackgroundCommand) -> Result<BackgroundJob<'_>, JournalError> {
    Ok(BackgroundJob {
        intent: *command.dispatch().intent().id().as_bytes(),
        execution: values::execution(command.dispatch().intent().authority())?,
        terminal: command.terminal(),
        command: command.command(),
        window: None,
    })
}

/// Count model-owned strings, borrowed-argument vectors and boxed DTOs before
/// building them. JSON/base64 output is separately counted by its actual owner.
pub(super) fn mapping_bytes(event: &CoordinatorEvent) -> Result<usize, JournalError> {
    use nocterm_hub_model::cloud::{events as wire, payload as body};
    let principal = |authority: &nocterm_cloud_model::authority::ExecutionAuthority| {
        authority.session().target().principal().as_str().len()
    };
    let intent_bytes = |intent: &nocterm_cloud_model::run::ToolIntent,
                        call: &nocterm_ai::TerminalCall|
     -> Result<usize, JournalError> {
        let arguments = match call {
            nocterm_ai::TerminalCall::ExecCommand(value) => value
                .args
                .len()
                .checked_mul(std::mem::size_of::<&str>())
                .ok_or(JournalError::Capacity)?,
            _ => 0,
        };
        std::mem::size_of::<wire::ToolIntentPayload<'static>>()
            .checked_add(principal(intent.authority()))
            .and_then(|n| n.checked_add(arguments))
            .ok_or(JournalError::Capacity)
    };
    Ok(match event {
        CoordinatorEvent::CommandAccepted { command, .. } => {
            command.issuer().device().as_str().len()
        }
        CoordinatorEvent::PromptAccepted {
            command, request, ..
        } => {
            let target = request.destination().target().principal().as_str().len();
            let session = request
                .destination()
                .session()
                .map_or(0, |value| value.target().principal().as_str().len());
            [
                command.issuer().device().as_str().len(),
                target,
                session,
                request.selection().provider().as_str().len(),
                request.selection().model().as_str().len(),
                request.selection().pricing().as_str().len(),
                request
                    .prompt()
                    .content()
                    .len()
                    .checked_mul(std::mem::size_of::<body::PromptPart<'static>>())
                    .ok_or(JournalError::Capacity)?,
                std::mem::size_of::<wire::AcceptedPrompt<'static>>(),
            ]
            .into_iter()
            .try_fold(0usize, |sum, value| {
                sum.checked_add(value).ok_or(JournalError::Capacity)
            })?
        }
        CoordinatorEvent::ToolIntent { intent, call, .. }
        | CoordinatorEvent::Approval { intent, call, .. } => intent_bytes(intent, call)?,
        CoordinatorEvent::ApprovalResolved { answer, .. } => {
            std::mem::size_of::<body::ApprovalPayload>()
                .checked_add(principal(answer.authority()))
                .ok_or(JournalError::Capacity)?
        }
        CoordinatorEvent::ToolResult { observation, .. } => {
            let background = match observation.effect() {
                ToolEffectStatus::Background(command) => {
                    std::mem::size_of::<wire::BackgroundJob<'static>>()
                        .checked_add(principal(command.dispatch().intent().authority()))
                        .ok_or(JournalError::Capacity)?
                }
                _ => 0,
            };
            std::mem::size_of::<wire::ToolObservationPayload<'static>>()
                .checked_add(background)
                .ok_or(JournalError::Capacity)?
        }
        CoordinatorEvent::Status { .. }
        | CoordinatorEvent::Progress { .. }
        | CoordinatorEvent::Deleted { .. } => 0,
    }
    .max(1))
}
