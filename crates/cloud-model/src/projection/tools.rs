//! Tool rows retain their original run, intent and one terminal result.
use super::{
    MAX_PROJECTION_ENTRIES, ProjectionError,
    runs::{RunKey, key},
};
use crate::{
    authority::RunBinding, coordinator::ToolObservation, identity::IntentId, run::ToolIntent,
};
use nocterm_ai::{
    TerminalCall,
    model::{
        session::SessionUpdate,
        tools::{ToolRecord, ToolStatus},
    },
    thread::{Entry, ThreadState},
    tool_display::ToolDisplay,
};
use std::collections::BTreeMap;

#[derive(Clone)]
struct ToolRow {
    index: usize,
    finished: bool,
}
#[derive(Clone, Default)]
pub(super) struct ProjectionTools(BTreeMap<(RunKey, IntentId), ToolRow>);
impl ProjectionTools {
    pub(super) fn bytes(&self) -> usize {
        self.0.len().saturating_mul(128)
    }
    pub(super) fn contains(&self, intent: &ToolIntent) -> bool {
        self.0
            .contains_key(&(key(intent.authority().run()), intent.id()))
    }
    pub(super) fn push(
        &mut self,
        intent: &ToolIntent,
        call: &TerminalCall,
        display: &ToolDisplay,
        transcript: &mut ThreadState,
        now: u64,
    ) -> Result<(), ProjectionError> {
        let key = (key(intent.authority().run()), intent.id());
        if self.0.contains_key(&key) || call.name() != display.tool || display.version != 2 {
            return Err(ProjectionError::InvalidRecord);
        }
        call.validate()
            .map_err(|_| ProjectionError::InvalidRecord)?;
        if self.0.len() >= MAX_PROJECTION_ENTRIES {
            return Err(ProjectionError::Capacity);
        }
        let mut record = ToolRecord::new(
            row_id(intent.authority().run(), intent.id()),
            display.title(),
        )
        .name(display.tool.clone())
        .status(ToolStatus::Running);
        display.attach(&mut record);
        self.0.insert(
            key,
            ToolRow {
                index: transcript.entries.len(),
                finished: false,
            },
        );
        transcript.apply(SessionUpdate::ToolCall(Box::new(record)), now);
        Ok(())
    }
    pub(super) fn finish(
        &mut self,
        run: &RunBinding,
        observation: &ToolObservation,
        transcript: &mut ThreadState,
    ) -> Result<(), ProjectionError> {
        let row = self
            .0
            .get_mut(&(key(run), observation.intent()))
            .ok_or(ProjectionError::InvalidRecord)?;
        if row.finished {
            return Err(ProjectionError::InvalidRecord);
        }
        let Some(Entry::Tool(record)) = transcript.entries.get_mut(row.index) else {
            return Err(ProjectionError::InvalidRecord);
        };
        if record.name.as_deref() != Some(observation.display().tool.as_str())
            || observation.display().version != 2
        {
            return Err(ProjectionError::InvalidRecord);
        }
        record.status = if observation.is_failed() {
            ToolStatus::Failed
        } else {
            ToolStatus::Succeeded
        };
        record.raw_output = Some(observation.output().value().clone());
        observation.display().attach(record);
        row.finished = true;
        Ok(())
    }
}
fn row_id(run: &RunBinding, id: IntentId) -> String {
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    format!(
        "{}-{}-{}",
        hex(run.run().as_bytes()),
        run.epoch().get(),
        hex(id.as_bytes())
    )
}
