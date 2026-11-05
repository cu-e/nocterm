//! Pure live-transcript admission. A stream is bounded before it mutates accepted rows.
use super::{Entry, ThreadState};
use crate::{
    history::{HistoryCodecError, MAX_RESIDENT_BYTES, retained_value_bytes},
    model::{content::Content, controls::Change, session::SessionUpdate},
};

const MAX_LIVE_ENTRIES: usize = 20_000;
pub fn resident_bytes(state: &ThreadState) -> Result<usize, HistoryCodecError> {
    Ok(retained_value_bytes(&state.entries)?
        .saturating_add(plan_bytes(state.plan.as_ref()))
        .saturating_add(retained_value_bytes(&state.title)?)
        .saturating_add(usage_bytes(state.usage.as_ref())?)
        .saturating_add(state.times.len().saturating_mul(8)))
}
fn plan_bytes(value: Option<&crate::model::plan::Plan>) -> usize {
    value.map_or(0, |plan| {
        plan.entries.iter().fold(1024usize, |bytes, entry| {
            bytes
                .saturating_add(entry.content.len().saturating_mul(2))
                .saturating_add(1024)
        })
    })
}
fn usage_bytes(
    value: Option<&crate::model::usage::ContextUsage>,
) -> Result<usize, HistoryCodecError> {
    value.map_or(Ok(0), |usage| {
        Ok(retained_value_bytes(&usage.meta)?
            .saturating_add(
                usage
                    .cost
                    .as_ref()
                    .map_or(0, |cost| cost.currency.len().saturating_mul(2)),
            )
            .saturating_add(1024))
    })
}
pub fn admit_user(
    state: &ThreadState,
    current: usize,
    content: &[Content],
) -> Result<usize, HistoryCodecError> {
    if state.entries.len() >= MAX_LIVE_ENTRIES {
        return Err(HistoryCodecError::CollectionLimit);
    }
    let next = current
        .saturating_add(retained_value_bytes(&content)?)
        .saturating_add(4096);
    if next > MAX_RESIDENT_BYTES {
        return Err(HistoryCodecError::StructuralLimit);
    }
    Ok(next)
}
/// Computes only the changed content; a text append never rescans the existing message.
pub fn admit_update(
    state: &ThreadState,
    current: usize,
    update: &SessionUpdate,
) -> Result<usize, HistoryCodecError> {
    let Some((removed, added, row)) = update_change(state, update)? else {
        return Ok(current);
    };
    if state.entries.len().saturating_add(usize::from(row)) > MAX_LIVE_ENTRIES {
        return Err(HistoryCodecError::CollectionLimit);
    }
    let next = current
        .saturating_sub(removed)
        .saturating_add(added)
        .saturating_add(usize::from(row) * 8);
    if next > MAX_RESIDENT_BYTES {
        return Err(HistoryCodecError::StructuralLimit);
    }
    Ok(next)
}

fn update_change(
    state: &ThreadState,
    update: &SessionUpdate,
) -> Result<Option<(usize, usize, bool)>, HistoryCodecError> {
    let change = match update {
        SessionUpdate::AgentMessageChunk(content) | SessionUpdate::AgentThoughtChunk(content) => {
            let thought = matches!(update, SessionUpdate::AgentThoughtChunk(_));
            match (content, state.entries.last(), thought) {
                (Content::Text(text), Some(Entry::Agent(_)), false)
                | (Content::Text(text), Some(Entry::Thought(_)), true) => {
                    (0, text.text.len().saturating_mul(2), false)
                }
                _ => (0, retained_value_bytes(content)?.saturating_add(4096), true),
            }
        }
        SessionUpdate::UserMessageChunk(content) => (
            0,
            retained_value_bytes(content)?.saturating_add(4096),
            !matches!(state.entries.last(), Some(Entry::User(_))),
        ),
        SessionUpdate::ToolCall(call) => {
            (0, retained_value_bytes(call)?.saturating_add(4096), true)
        }
        SessionUpdate::ToolCallUpdate(patch) => return tool_change(state, patch),
        _ => return control_change(state, update),
    };
    Ok(Some(change))
}
fn tool_change(
    state: &ThreadState,
    patch: &crate::model::tools::ToolPatch,
) -> Result<Option<(usize, usize, bool)>, HistoryCodecError> {
    let Some(original) = state.entries.iter().find_map(|entry| match entry {
        Entry::Tool(call) if call.id == patch.id => Some(call),
        _ => None,
    }) else {
        return Ok(None);
    };
    let mut candidate = original.clone();
    candidate.apply(patch.changes.clone());
    Ok(Some((
        retained_value_bytes(original)?,
        retained_value_bytes(&candidate)?,
        false,
    )))
}
fn control_change(
    state: &ThreadState,
    update: &SessionUpdate,
) -> Result<Option<(usize, usize, bool)>, HistoryCodecError> {
    let change = match update {
        SessionUpdate::Plan(plan) => (
            plan_bytes(state.plan.as_ref()),
            plan_bytes(Some(plan)),
            false,
        ),
        SessionUpdate::SessionInfoUpdate(info) => match &info.title {
            Change::Set(title) => (
                retained_value_bytes(&state.title)?,
                retained_value_bytes(&Some(title))?,
                false,
            ),
            Change::Clear | Change::Keep => return Ok(None),
        },
        SessionUpdate::UsageUpdate(usage) => (
            usage_bytes(state.usage.as_ref())?,
            usage_bytes(Some(usage))?,
            false,
        ),
        _ => return Ok(None),
    };
    Ok(Some(change))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_small_chunks_refuse_before_changing_the_accepted_transcript() {
        let mut state = ThreadState::default();
        let mut bytes = resident_bytes(&state).unwrap();
        let update = SessionUpdate::AgentMessageChunk(Content::text("x".repeat(1024 * 1024)));
        let mut accepted = 0;
        while let Ok(next) = admit_update(&state, bytes, &update) {
            state.apply(update.clone(), 1);
            bytes = next;
            assert!(resident_bytes(&state).unwrap() <= bytes);
            accepted += 1;
        }
        assert!(accepted > 1 && accepted < 64);
        assert!(matches!(
            admit_update(&state, bytes, &update),
            Err(HistoryCodecError::StructuralLimit)
        ));
        assert!(
            matches!(&state.entries[0], Entry::Agent(text) if text.len() == accepted * 1024 * 1024)
        );
    }
    #[test]
    fn replacing_a_tool_payload_releases_its_previous_charge() {
        use crate::model::tools::{ToolChanges, ToolPatch, ToolRecord};
        let mut state = ThreadState::default();
        state.apply(
            SessionUpdate::ToolCall(Box::new(ToolRecord::new("tool", "x".repeat(4096)))),
            1,
        );
        let bytes = resident_bytes(&state).unwrap();
        let update = SessionUpdate::ToolCallUpdate(Box::new(ToolPatch::new(
            "tool",
            ToolChanges::default().title("done"),
        )));
        assert!(admit_update(&state, bytes, &update).unwrap() < bytes);
    }
    #[test]
    fn mixed_updates_keep_the_complete_live_state_inside_the_admitted_budget() {
        use crate::model::{
            plan::{Plan, PlanEntry, PlanPriority, PlanStatus},
            session::SessionInfo,
            tools::{ToolChanges, ToolPatch, ToolRecord},
            usage::ContextUsage,
        };
        let mut state = ThreadState::default();
        let mut bytes = resident_bytes(&state).unwrap();
        let updates = vec![
            SessionUpdate::AgentMessageChunk(Content::text("answer")),
            SessionUpdate::AgentThoughtChunk(Content::text("thinking")),
            SessionUpdate::ToolCall(Box::new(ToolRecord::new("tool", "command"))),
            SessionUpdate::ToolCallUpdate(Box::new(ToolPatch::new(
                "tool",
                ToolChanges::default().raw_output(serde_json::json!({"output": "x".repeat(4096)})),
            ))),
            SessionUpdate::Plan(Plan {
                entries: vec![PlanEntry {
                    content: "plan".repeat(1024),
                    priority: PlanPriority::High,
                    status: PlanStatus::Pending,
                }],
            }),
            SessionUpdate::UsageUpdate(ContextUsage::new(10, 100).meta(
                crate::model::extensions::Extensions::from(serde_json::Map::from_iter([(
                    "detail".into(),
                    serde_json::json!("x".repeat(2048)),
                )])),
            )),
            SessionUpdate::SessionInfoUpdate(SessionInfo {
                title: Change::Set("title".repeat(1024)),
                ..Default::default()
            }),
        ];
        for update in updates {
            bytes = admit_update(&state, bytes, &update).unwrap();
            state.apply(update, 1);
            assert!(resident_bytes(&state).unwrap() <= bytes);
        }
        let content = vec![Content::text("user prompt")];
        bytes = admit_user(&state, bytes, &content).unwrap();
        state.push_user(content, 1);
        assert!(resident_bytes(&state).unwrap() <= bytes);
    }
}
