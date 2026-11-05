//! Transport retention accounting, independent from a view's history reservation.
use nocterm_cloud_conversation_model::{ConversationUpdate, Row, ToolOutcome};
use std::mem::size_of;

pub(super) fn retained(update: &ConversationUpdate) -> usize {
    let ConversationUpdate::Transcript(delta) = update else {
        return size_of::<ConversationUpdate>() + 1024;
    };
    let rows = delta.rows.iter().fold(
        delta.rows.capacity().saturating_mul(size_of::<Row>()),
        |bytes, row| bytes.saturating_add(row_bytes(row)),
    );
    let runs = delta.runs.as_ref().map_or(0, |runs| {
        runs.iter().fold(
            runs.capacity().saturating_mul(size_of_val_element(runs)),
            |bytes, run| {
                bytes
                    .saturating_add(run.model.provider.capacity())
                    .saturating_add(run.model.model.capacity())
            },
        )
    });
    let servers = delta.servers.as_ref().map_or(0, |binding| {
        binding.servers.iter().fold(
            binding
                .servers
                .capacity()
                .saturating_mul(size_of_val_element(&binding.servers)),
            |bytes, server| {
                bytes
                    .saturating_add(server.host.capacity())
                    .saturating_add(server.user.capacity())
            },
        )
    });
    let pending = delta.pending.as_ref().map_or(0, |pending| {
        pending
            .iter()
            .fold(vector_bytes(Some(pending)), |bytes, command| {
                bytes.saturating_add(command.text.capacity())
            })
    });
    size_of::<ConversationUpdate>()
        .saturating_add(rows)
        .saturating_add(runs)
        .saturating_add(servers)
        .saturating_add(vector_bytes(delta.approvals.as_ref()))
        .saturating_add(pending)
}

fn size_of_val_element<T>(_: &[T]) -> usize {
    size_of::<T>()
}

fn vector_bytes<T>(items: Option<&Vec<T>>) -> usize {
    items.map_or(0, |items| items.capacity().saturating_mul(size_of::<T>()))
}

fn row_bytes(row: &Row) -> usize {
    let tool = row.tool.as_ref().map_or(0, |tool| {
        tool.title
            .capacity()
            .saturating_add(tool.tool.capacity())
            .saturating_add(tool.arguments_json.capacity())
            .saturating_add(tool.destination.as_ref().map_or(0, String::capacity))
            .saturating_add(tool.output_json.as_ref().map_or(0, String::capacity))
            .saturating_add(match &tool.outcome {
                Some(ToolOutcome::Ok(text) | ToolOutcome::Err(text)) => text.capacity(),
                _ => 0,
            })
    });
    row.text.capacity().saturating_add(tool).saturating_add(
        row.session
            .as_ref()
            .map_or(0, |session| session.principal.capacity()),
    )
}
