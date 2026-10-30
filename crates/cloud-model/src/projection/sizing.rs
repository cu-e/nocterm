//! Allocation-free structural admission reuses the existing AI history counter.
use crate::{coordinator::CoordinatorEvent, engine::EngineEvent};
use nocterm_ai::{TerminalCall, history::retained_value_bytes};

use super::ProjectionError;

pub(super) fn event_bytes(event: &CoordinatorEvent) -> Result<usize, ProjectionError> {
    // Covers coordinates, namespace strings, enum/Vec storage and small policy
    // values. Variable content is measured separately through the shared codec.
    let variable = match event {
        CoordinatorEvent::PromptAccepted { request, .. } => request
            .prompt()
            .content()
            .iter()
            .try_fold(0usize, |total, part| {
                let bytes = retained_value_bytes(part).map_err(|_| ProjectionError::Capacity)?;
                Ok(total.saturating_add(bytes))
            })?,
        CoordinatorEvent::Progress {
            event: EngineEvent::Text(text),
            ..
        } => text.text().len().saturating_mul(2),
        CoordinatorEvent::ToolIntent { call, display, .. } => call_bytes(call)?
            .saturating_add(retained_value_bytes(display).map_err(|_| ProjectionError::Capacity)?),
        CoordinatorEvent::Approval { call, display, .. } => {
            approval_bytes(call, display)?.saturating_sub(8192)
        }
        CoordinatorEvent::BackgroundLifecycle(value) => value
            .job()
            .terminal()
            .len()
            .saturating_add(value.job().command().len()),
        CoordinatorEvent::ToolResult { observation, .. } => {
            retained_value_bytes(observation.output().value())
                .map_err(|_| ProjectionError::Capacity)?
                .saturating_add(
                    retained_value_bytes(observation.display())
                        .map_err(|_| ProjectionError::Capacity)?,
                )
        }
        _ => 0,
    };
    Ok(8192usize.saturating_add(variable))
}

/// Additional retained owners created by reduction, beyond the transcript and
/// small per-event bookkeeping covered by `event_bytes`. Deletions are ignored:
/// the reservation must cover the candidate before any replaced value drops.
pub(super) fn projection_growth(event: &CoordinatorEvent) -> Result<usize, ProjectionError> {
    let owners = match event {
        CoordinatorEvent::ToolIntent { display, .. } => super::tools::TOOL_ROW_BYTES
            // ToolRecord keeps a separate generated title and name alongside
            // the display embedded in its metadata.
            .saturating_add(display.tool.len().saturating_mul(2))
            .saturating_add(
                display
                    .destination
                    .as_ref()
                    .map_or(0, |value| value.len().saturating_mul(2)),
            ),
        CoordinatorEvent::Approval { call, display, .. } => approval_bytes(call, display)?,
        // ProjectionRuns retains a separate bounded run/phase record.
        CoordinatorEvent::PromptAccepted { .. } => 1024,
        _ => 0,
    };
    Ok(event_bytes(event)?.saturating_add(owners))
}

fn call_bytes(call: &TerminalCall) -> Result<usize, ProjectionError> {
    let bytes = match call {
        TerminalCall::ListTerminals => return Ok(0),
        TerminalCall::ReadTerminal(value) => retained_value_bytes(value),
        TerminalCall::SendInput(value) => retained_value_bytes(value),
        TerminalCall::RunCommand(value) => retained_value_bytes(value),
        TerminalCall::OpenTerminal(value) => retained_value_bytes(value),
        TerminalCall::ExecCommand(value) => retained_value_bytes(value),
        TerminalCall::ReadCommand(value) => retained_value_bytes(value),
        TerminalCall::CancelCommand(value) => retained_value_bytes(value),
    };
    bytes.map_err(|_| ProjectionError::Capacity)
}

pub(super) fn approval_bytes(
    call: &TerminalCall,
    display: &nocterm_ai::tool_display::ToolDisplay,
) -> Result<usize, ProjectionError> {
    Ok(8192usize
        .saturating_add(call_bytes(call)?)
        .saturating_add(retained_value_bytes(display).map_err(|_| ProjectionError::Capacity)?))
}
