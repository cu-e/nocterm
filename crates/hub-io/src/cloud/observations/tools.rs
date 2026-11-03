//! Exact canonical tool arguments become descriptive native calls only.
use super::{budget::DecodeBudget, values};
use nocterm_ai::{
    CancelCommand, CancelTransfer, ExecCommand, OpenTerminal, ReadCommand, ReadTerminal,
    ReadTransfer, RunCommand, SendInput, TerminalCall, TransferFiles,
    tool_display::{ToolDisplay, ToolOutcome},
};
use nocterm_cloud_model::{
    authority::StreamBinding,
    coordinator::{ObservedBackground, ObservedJobWindow},
    identity::{Fingerprint, IntentId, StepId},
    projection::{ProjectionError, RecordKind},
    run::ToolIntent,
};
use nocterm_hub_model::cloud::{
    events::{BackgroundJob, DisplayOutcome, ToolIntentPayload, ToolPresentation},
    tool_arguments::ToolArguments,
};

pub(super) fn display(
    value: &ToolPresentation<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<ToolDisplay, ProjectionError> {
    budget.admit(
        value
            .tool
            .len()
            .checked_add(value.destination.map_or(0, str::len))
            .ok_or(ProjectionError::Capacity)?,
    )?;
    let arguments = budget.json(value.arguments_json)?;
    let outcome = match value.outcome.as_ref() {
        None => None,
        Some(DisplayOutcome::Pending) => Some(ToolOutcome::Pending),
        Some(DisplayOutcome::Ok(json)) => Some(ToolOutcome::Ok(budget.json(json)?)),
        Some(DisplayOutcome::Err(text)) => {
            budget.admit(text.len())?;
            Some(ToolOutcome::Err((*text).to_owned()))
        }
    };
    Ok(ToolDisplay {
        version: 2,
        tool: value.tool.to_owned(),
        arguments,
        destination: value.destination.map(str::to_owned),
        redacted: value.redacted,
        outcome,
    })
}
pub(super) fn intent(
    stream: &StreamBinding,
    value: &ToolIntentPayload<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<(ToolIntent, TerminalCall, ToolDisplay), ProjectionError> {
    let call = arguments(&value.arguments, budget)?;
    call.validate()
        .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Tool))?;
    if call.writes() != value.mutates
        || value
            .arguments
            .digest()
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Tool))?
            != value.arguments_digest
    {
        return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
    }
    let intent = ToolIntent::from_journal(
        IntentId::from_bytes(value.id),
        StepId::from_bytes(value.step),
        values::execution(stream, &value.execution)?,
        Fingerprint::from_bytes(value.arguments_digest),
        value.mutates,
    );
    Ok((intent, call, display(&value.display, budget)?))
}
pub(super) fn background(
    stream: &StreamBinding,
    value: &BackgroundJob<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<ObservedBackground, ProjectionError> {
    budget.admit(
        value
            .terminal
            .len()
            .checked_add(value.command.len())
            .ok_or(ProjectionError::Capacity)?,
    )?;
    let window = value
        .window
        .map(|window| ObservedJobWindow::new(window.created_at_ms, window.deadline_ms))
        .transpose()
        .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Tool))?;
    ObservedBackground::new(
        IntentId::from_bytes(value.intent),
        values::execution(stream, &value.execution)?,
        value.terminal.to_owned(),
        value.command.to_owned(),
        window,
    )
    .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Tool))
}
fn string(text: &str, budget: &mut DecodeBudget<'_>) -> Result<String, ProjectionError> {
    budget.admit(text.len())?;
    Ok(text.to_owned())
}
fn optional(
    text: Option<&str>,
    budget: &mut DecodeBudget<'_>,
) -> Result<Option<String>, ProjectionError> {
    text.map(|text| string(text, budget)).transpose()
}
fn arguments(
    value: &ToolArguments<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<TerminalCall, ProjectionError> {
    Ok(match value {
        ToolArguments::ListTerminals => TerminalCall::ListTerminals,
        ToolArguments::OpenTerminal { server_id } => TerminalCall::OpenTerminal(OpenTerminal {
            server_id: string(server_id, budget)?,
        }),
        ToolArguments::ReadTerminal {
            terminal_id,
            lines,
            since,
        } => TerminalCall::ReadTerminal(ReadTerminal {
            terminal_id: string(terminal_id, budget)?,
            lines: lines
                .map(usize::try_from)
                .transpose()
                .map_err(|_| ProjectionError::Capacity)?,
            since: *since,
        }),
        ToolArguments::SendInput {
            terminal_id,
            text,
            press_enter,
        } => TerminalCall::SendInput(SendInput {
            terminal_id: string(terminal_id, budget)?,
            text: string(text, budget)?,
            press_enter: *press_enter,
        }),
        ToolArguments::RunCommand {
            terminal_id,
            command,
            timeout_ms,
            idle_ms,
        } => TerminalCall::RunCommand(RunCommand {
            terminal_id: string(terminal_id, budget)?,
            command: string(command, budget)?,
            timeout_ms: *timeout_ms,
            idle_ms: *idle_ms,
        }),
        ToolArguments::ExecCommand {
            terminal_id,
            program,
            args,
            stdin,
            timeout_ms,
            yield_ms,
        } => {
            let arguments = argument_array(args, budget)?;
            TerminalCall::ExecCommand(ExecCommand {
                terminal_id: string(terminal_id, budget)?,
                program: string(program, budget)?,
                args: arguments,
                stdin: optional(*stdin, budget)?,
                timeout_ms: *timeout_ms,
                yield_ms: *yield_ms,
            })
        }
        ToolArguments::ReadCommand {
            terminal_id,
            command_id,
            yield_ms,
        } => TerminalCall::ReadCommand(ReadCommand {
            terminal_id: string(terminal_id, budget)?,
            command_id: string(command_id, budget)?,
            yield_ms: *yield_ms,
        }),
        ToolArguments::CancelCommand {
            terminal_id,
            command_id,
        } => TerminalCall::CancelCommand(CancelCommand {
            terminal_id: string(terminal_id, budget)?,
            command_id: string(command_id, budget)?,
        }),
        ToolArguments::TransferFiles { .. }
        | ToolArguments::ReadTransfer { .. }
        | ToolArguments::CancelTransfer { .. } => transfer_arguments(value, budget)?,
    })
}

/// The transfer tools; `arguments` routes only these variants here.
fn transfer_arguments(
    value: &ToolArguments<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<TerminalCall, ProjectionError> {
    Ok(match value {
        ToolArguments::TransferFiles {
            source_terminal_id,
            sources,
            destination_terminal_id,
            destination,
            overwrite,
            yield_ms,
        } => TerminalCall::TransferFiles(TransferFiles {
            source_terminal_id: string(source_terminal_id, budget)?,
            sources: argument_array(sources, budget)?,
            destination_terminal_id: string(destination_terminal_id, budget)?,
            destination: string(destination, budget)?,
            overwrite: *overwrite,
            yield_ms: *yield_ms,
        }),
        ToolArguments::ReadTransfer {
            transfer_id,
            yield_ms,
        } => TerminalCall::ReadTransfer(ReadTransfer {
            transfer_id: string(transfer_id, budget)?,
            yield_ms: *yield_ms,
        }),
        ToolArguments::CancelTransfer { transfer_id } => {
            TerminalCall::CancelTransfer(CancelTransfer {
                transfer_id: string(transfer_id, budget)?,
            })
        }
        _ => return Err(ProjectionError::InvalidRecord(RecordKind::Tool)),
    })
}

fn argument_array(
    args: &[&str],
    budget: &mut DecodeBudget<'_>,
) -> Result<Vec<String>, ProjectionError> {
    budget.admit(
        args.len()
            .checked_mul(std::mem::size_of::<String>())
            .ok_or(ProjectionError::Capacity)?,
    )?;
    let mut arguments = Vec::with_capacity(args.len());
    for argument in args {
        arguments.push(string(argument, budget)?);
    }
    Ok(arguments)
}
