//! Native semantic observations enter the public codec without native frame decoding.
use super::{
    prompt,
    tools::{self, DisplayJson, JsonText},
    values,
};
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::{CoordinatorEvent, ObservedBackground, ObservedEffect as NativeEffect},
    engine::{EngineEvent, TextChannel},
    journal::{JournalError, PublicationWorkBudget},
    tool::BackgroundCommand,
};
use nocterm_hub_model::cloud::events::{
    BackgroundJob, CloudEventPayload, CloudTextChannel, ObservedEffect, ToolObservationPayload,
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
        CoordinatorEvent::BackgroundLifecycle(value) => Some(value.job().execution().run()),
        CoordinatorEvent::AuthenticationRequested { request, .. } => Some(request.binding().run()),
        CoordinatorEvent::SessionState(value) => Some(value.binding().run()),
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
            display,
            expires,
        } => {
            let prepared = DisplayJson::new(display, work)?;
            let display = prepared.view(display);
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
        CoordinatorEvent::BackgroundLifecycle(value) => emit(
            CloudEventPayload::BackgroundLifecycle(Box::new(lifecycle(value)?)),
        ),
        CoordinatorEvent::AuthenticationRequested { .. } | CoordinatorEvent::SessionState(_) => {
            // The installed SDK predates the negotiated session-start feature.
            // Never encode a real session fact as unrelated display/status data.
            Err(JournalError::UnsupportedFormat)
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
        NativeEffect::Observed => ObservedEffect::Observed,
        NativeEffect::Unknown => ObservedEffect::Unknown,
        NativeEffect::Background(command) => {
            ObservedEffect::Background(Box::new(observed_background(command)?))
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
        } => prompt_bytes(command, request)?,
        CoordinatorEvent::ToolIntent { intent, call, .. }
        | CoordinatorEvent::Approval { intent, call, .. } => intent_bytes(intent, call)?,
        CoordinatorEvent::ApprovalResolved { answer, .. } => {
            std::mem::size_of::<body::ApprovalPayload>()
                .checked_add(principal(answer.authority()))
                .ok_or(JournalError::Capacity)?
        }
        CoordinatorEvent::ToolResult { observation, .. } => {
            let background = match observation.effect() {
                NativeEffect::Background(command) => {
                    std::mem::size_of::<wire::BackgroundJob<'static>>()
                        .checked_add(principal(command.execution()))
                        .ok_or(JournalError::Capacity)?
                }
                _ => 0,
            };
            std::mem::size_of::<wire::ToolObservationPayload<'static>>()
                .checked_add(background)
                .ok_or(JournalError::Capacity)?
        }
        CoordinatorEvent::BackgroundLifecycle(value) => {
            std::mem::size_of::<wire::BackgroundLifecyclePayload<'static>>()
                .checked_add(principal(value.job().execution()))
                .ok_or(JournalError::Capacity)?
        }
        CoordinatorEvent::AuthenticationRequested { .. } | CoordinatorEvent::SessionState(_) => {
            return Err(JournalError::UnsupportedFormat);
        }
        CoordinatorEvent::Status { .. }
        | CoordinatorEvent::Progress { .. }
        | CoordinatorEvent::Deleted { .. } => 0,
    }
    .max(1))
}

pub(super) fn observed_background(
    value: &ObservedBackground,
) -> Result<BackgroundJob<'_>, JournalError> {
    Ok(BackgroundJob {
        intent: *value.intent().as_bytes(),
        execution: values::execution(value.execution())?,
        terminal: value.terminal(),
        command: value.command(),
        window: value
            .window()
            .map(|window| nocterm_hub_model::cloud::events::JobWindow {
                created_at_ms: window.created_at_ms(),
                deadline_ms: window.deadline_ms(),
            }),
    })
}
fn lifecycle(
    value: &nocterm_cloud_model::coordinator::BackgroundObservation,
) -> Result<nocterm_hub_model::cloud::events::BackgroundLifecyclePayload<'_>, JournalError> {
    use nocterm_cloud_model::journal::BackgroundRecordOutcome;
    use nocterm_hub_model::cloud::events::{BackgroundLifecyclePayload, BackgroundOutcome};
    Ok(BackgroundLifecyclePayload {
        job: observed_background(value.job())?,
        step: *value.step().as_bytes(),
        arguments_digest: *value.arguments().as_bytes(),
        mutates: value.mutates(),
        dispatch_writer_epoch: value.dispatch().writer().get(),
        dispatch_revision: value.dispatch().revision().get(),
        outcome: match value.outcome() {
            BackgroundRecordOutcome::Retained => BackgroundOutcome::Retained,
            BackgroundRecordOutcome::ObservedExit(exit) => BackgroundOutcome::ObservedExit(exit),
            BackgroundRecordOutcome::Unknown => BackgroundOutcome::Unknown,
        },
    })
}

fn prompt_bytes(
    command: &nocterm_cloud_model::journal::CommandAcceptance,
    request: &nocterm_cloud_model::coordinator::ObservedPrompt,
) -> Result<usize, JournalError> {
    use nocterm_hub_model::cloud::{events as wire, payload as body};
    let target = request
        .destination()
        .map_or(0, |value| value.target().principal().as_str().len());
    let session = request
        .destination()
        .and_then(|value| value.session())
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
    })
}
