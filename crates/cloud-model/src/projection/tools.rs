//! Tool rows retain their original run, intent and one terminal result.
use super::{
    MAX_PROJECTION_ENTRIES, ProjectionError, RecordKind,
    runs::{RunKey, key},
};
use crate::{
    authority::RunBinding,
    coordinator::{BackgroundObservation, ObservedBackground, ObservedEffect, ToolObservation},
    identity::IntentId,
    run::ToolIntent,
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

pub(super) const TOOL_ROW_BYTES: usize = 16 * 1024;

#[derive(Clone)]
struct ToolRow {
    index: usize,
    finished: bool,
    intent: ToolIntent,
    background: Option<ObservedBackground>,
    lifecycle: Option<BackgroundObservation>,
    terminal: Option<String>,
}
#[derive(Clone, Default)]
pub(super) struct ProjectionTools(BTreeMap<(RunKey, IntentId), ToolRow>);
impl ProjectionTools {
    pub(super) fn bytes(&self) -> usize {
        self.0.len().saturating_mul(TOOL_ROW_BYTES)
    }
    pub(super) fn contains(&self, intent: &ToolIntent) -> bool {
        self.0
            .get(&(key(intent.authority().run()), intent.id()))
            .is_some_and(|row| row.intent == *intent)
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
            return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
        }
        call.validate()
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Tool))?;
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
                intent: intent.clone(),
                background: None,
                lifecycle: None,
                terminal: match call {
                    TerminalCall::ExecCommand(value) => Some(value.terminal_id.clone()),
                    _ => None,
                },
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
            .ok_or(ProjectionError::InvalidRecord(RecordKind::Tool))?;
        if row.finished {
            return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
        }
        let Some(Entry::Tool(record)) = transcript.entries.get_mut(row.index) else {
            return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
        };
        if record.name.as_deref() != Some(observation.display().tool.as_str())
            || observation.display().version != 2
        {
            return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
        }
        if let ObservedEffect::Background(background) = observation.effect() {
            if background.intent() != row.intent.id()
                || background.execution() != row.intent.authority()
                || row.terminal.as_deref() != Some(background.terminal())
            {
                return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
            }
            row.background = Some((**background).clone());
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
    pub(super) fn background(
        &mut self,
        value: &BackgroundObservation,
    ) -> Result<(), ProjectionError> {
        let row = self
            .0
            .get_mut(&(key(value.job().execution().run()), value.job().intent()))
            .ok_or(ProjectionError::InvalidRecord(RecordKind::Tool))?;
        if row.terminal.as_deref() != Some(value.job().terminal())
            || value.job().execution() != row.intent.authority()
            || value.step() != row.intent.step()
            || value.arguments() != row.intent.arguments()
            || value.mutates() != row.intent.mutates()
        {
            return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
        }
        if row
            .background
            .as_ref()
            .is_some_and(|original| original != value.job())
            || row.lifecycle.as_ref().is_some_and(|previous| {
                previous.dispatch() != value.dispatch() || previous.job() != value.job()
            })
        {
            return Err(ProjectionError::InvalidRecord(RecordKind::Tool));
        }
        row.lifecycle = Some(value.clone());
        Ok(())
    }
    pub(super) fn observations(&self) -> impl Iterator<Item = &BackgroundObservation> {
        self.0.values().filter_map(|row| row.lifecycle.as_ref())
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
