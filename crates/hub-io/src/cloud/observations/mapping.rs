//! Semantic mapping after the configured SDK opener verified the signed record.
use super::{budget::DecodeBudget, prompt, tools, values};
use nocterm_cloud_model::{
    authority::StreamBinding,
    coordinator::{BackgroundObservation, CoordinatorEvent, ObservedEffect, ToolObservation},
    engine::{EngineEvent, EngineText, TextChannel},
    identity::*,
    journal::BackgroundRecordOutcome,
    payload::{BoundedJson, MAX_TOOL_RESULT_BYTES},
    projection::ProjectionError,
    run::CommitStamp,
};
use nocterm_hub_model::cloud::events::{
    CloudEventPayload, CloudTextChannel, ObservedEffect as WireEffect, ToolObservationPayload,
};

pub(super) fn event(
    stream: &StreamBinding,
    payload: &CloudEventPayload<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<CoordinatorEvent, ProjectionError> {
    // Closed native coordinate/enum/Box storage and bounded namespace copies.
    // Content, strings, vectors and JSON nodes are admitted separately below.
    budget.admit(8192)?;
    Ok(match payload {
        CloudEventPayload::CommandAccepted { command, run } => CoordinatorEvent::CommandAccepted {
            command: values::accepted(command)?,
            run: run.map(|value| values::run(stream, value)).transpose()?,
        },
        CloudEventPayload::PromptAccepted {
            command,
            run,
            prompt: value,
        } => CoordinatorEvent::PromptAccepted {
            command: values::accepted(command)?,
            run: values::run(stream, *run)?,
            request: prompt::prompt(value, budget)?,
        },
        CloudEventPayload::Status { run, status } => CoordinatorEvent::Status {
            run: values::run(stream, *run)?,
            status: values::status(*status),
        },
        CloudEventPayload::Text {
            run, channel, text, ..
        } => text_event(stream, *run, *channel, text, budget)?,
        CloudEventPayload::Usage { run, usage } => CoordinatorEvent::Progress {
            run: values::run(stream, *run)?,
            event: EngineEvent::Usage(values::usage(*usage)),
        },
        CloudEventPayload::ToolIntent(value) => {
            let (intent, call, display) = tools::intent(stream, value, budget)?;
            CoordinatorEvent::ToolIntent {
                intent,
                call,
                display,
            }
        }
        CloudEventPayload::Approval {
            intent: value,
            expires_at_ms,
        } => {
            let (intent, call, display) = tools::intent(stream, value, budget)?;
            CoordinatorEvent::Approval {
                intent,
                call,
                display,
                expires: Timestamp::from_millis(*expires_at_ms),
            }
        }
        CloudEventPayload::ApprovalResolved { command, answer } => {
            CoordinatorEvent::ApprovalResolved {
                command: CommandId::from_bytes(*command),
                answer: values::approval(stream, answer)?,
            }
        }
        CloudEventPayload::ToolResult { run, observation } => CoordinatorEvent::ToolResult {
            run: values::run(stream, *run)?,
            observation: result(stream, observation, budget)?,
        },
        CloudEventPayload::Deleted { command } => CoordinatorEvent::Deleted {
            command: CommandId::from_bytes(*command),
        },
        CloudEventPayload::BackgroundLifecycle(value) => background(stream, value, budget)?,
    })
}
fn result(
    stream: &StreamBinding,
    value: &ToolObservationPayload<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<ToolObservation, ProjectionError> {
    if value.output_json.len() > MAX_TOOL_RESULT_BYTES {
        return Err(ProjectionError::Capacity);
    }
    let output = BoundedJson::new(budget.json(value.output_json)?)
        .map_err(|_| ProjectionError::InvalidRecord)?;
    let effect = match &value.effect {
        WireEffect::Observed => ObservedEffect::Observed,
        WireEffect::Unknown => ObservedEffect::Unknown,
        WireEffect::Background(job) => {
            ObservedEffect::Background(Box::new(tools::background(stream, job, budget)?))
        }
    };
    Ok(ToolObservation::new(
        IntentId::from_bytes(value.intent),
        output,
        tools::display(&value.display, budget)?,
        effect,
        value.failed,
    ))
}
fn background(
    stream: &StreamBinding,
    value: &nocterm_hub_model::cloud::events::BackgroundLifecyclePayload<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<CoordinatorEvent, ProjectionError> {
    let job = tools::background(stream, &value.job, budget)?;
    let dispatch = CommitStamp::new(
        job.execution().run().clone(),
        WriterEpoch::new(value.dispatch_writer_epoch)
            .map_err(|_| ProjectionError::InvalidRecord)?,
        Revision::from_u64(value.dispatch_revision),
    );
    let outcome = match value.outcome {
        nocterm_hub_model::cloud::events::BackgroundOutcome::Retained => {
            BackgroundRecordOutcome::Retained
        }
        nocterm_hub_model::cloud::events::BackgroundOutcome::ObservedExit(exit) => {
            BackgroundRecordOutcome::ObservedExit(exit)
        }
        nocterm_hub_model::cloud::events::BackgroundOutcome::Unknown => {
            BackgroundRecordOutcome::Unknown
        }
    };
    let observation = BackgroundObservation::new(
        job,
        StepId::from_bytes(value.step),
        Fingerprint::from_bytes(value.arguments_digest),
        value.mutates,
        dispatch,
        outcome,
    )
    .map_err(|_| ProjectionError::InvalidRecord)?;
    Ok(CoordinatorEvent::BackgroundLifecycle(Box::new(observation)))
}

fn text_event(
    stream: &StreamBinding,
    run: nocterm_hub_model::cloud::RunCoordinate,
    channel: CloudTextChannel,
    text: &str,
    budget: &mut DecodeBudget<'_>,
) -> Result<CoordinatorEvent, ProjectionError> {
    budget.admit(text.len())?;
    Ok(CoordinatorEvent::Progress {
        run: values::run(stream, run)?,
        event: EngineEvent::Text(
            EngineText::new(
                match channel {
                    CloudTextChannel::Assistant => TextChannel::Assistant,
                    CloudTextChannel::Thought => TextChannel::Thought,
                },
                text.to_owned(),
            )
            .map_err(|_| ProjectionError::InvalidRecord)?,
        ),
    })
}
