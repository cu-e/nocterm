//! Authoritative complete neutral history, never inferred from an SDK checkpoint.
use super::{frame::Frame, services::CoordinatorServices};
use nocterm_cloud_model::{
    authority::RunBinding, coordinator::*, engine::*, identity::*, journal::*, payload::BoundedJson,
};
use std::collections::{BTreeMap, BTreeSet};

struct Conversation {
    order: Vec<RunBinding>,
    messages: BTreeMap<RunBinding, Vec<EngineHistoryMessage>>,
    unknown: BTreeMap<RunBinding, BTreeSet<IntentId>>,
}
impl Conversation {
    fn new() -> Self {
        Self {
            order: Vec::new(),
            messages: BTreeMap::new(),
            unknown: BTreeMap::new(),
        }
    }
    fn add(
        &mut self,
        run: &RunBinding,
        message: EngineHistoryMessage,
    ) -> Result<(), CoordinatorError> {
        self.messages
            .get_mut(run)
            .ok_or(JournalError::Corrupt)?
            .push(message);
        Ok(())
    }
    fn apply(&mut self, stored: &StoredRecord) -> Result<(), CoordinatorError> {
        match stored.record().kind() {
            JournalRecordKind::CommandAccepted { id, kind } => {
                let Frame::Accepted { command, allocated } =
                    Frame::read(stored.record().payload())?
                else {
                    return Err(JournalError::Corrupt.into());
                };
                let command = command.restore()?;
                if command.stream() != stored.stream()
                    || command.acceptance().id() != *id
                    || command.acceptance().kind() != *kind
                {
                    return Err(JournalError::Corrupt.into());
                }
                if let CommandBody::Submit(prompt) = command.body() {
                    let run = allocated.ok_or(JournalError::Corrupt)?.restore()?;
                    if stored.record().context().run() != Some(&run)
                        || self.messages.contains_key(&run)
                    {
                        return Err(JournalError::Corrupt.into());
                    }
                    self.order.push(run.clone());
                    self.messages.insert(
                        run,
                        vec![EngineHistoryMessage::User(prompt.prompt().clone())],
                    );
                }
            }
            JournalRecordKind::ModelAction { .. } => {
                let Frame::Action { text, calls, .. } = Frame::read(stored.record().payload())?
                else {
                    return Err(JournalError::Corrupt.into());
                };
                if !text.is_empty() || !calls.is_empty() {
                    let calls = calls
                        .into_iter()
                        .map(|planned| {
                            Ok::<_, CoordinatorError>(HistoricalToolCall::new(
                                planned.intent(),
                                planned.call()?,
                            )?)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    self.add(
                        stored
                            .record()
                            .context()
                            .run()
                            .ok_or(JournalError::Corrupt)?,
                        EngineHistoryMessage::Assistant(HistoricalAssistant::new(text, calls)?),
                    )?;
                }
            }
            JournalRecordKind::Outcome { intent, .. } | JournalRecordKind::Declined { intent } => {
                let Frame::Result {
                    intent: original,
                    output,
                    failed,
                    unknown,
                    ..
                } = Frame::read(stored.record().payload())?
                else {
                    return Err(JournalError::Corrupt.into());
                };
                if original != *intent.as_bytes() {
                    return Err(JournalError::Corrupt.into());
                }
                if unknown {
                    let run = stored
                        .record()
                        .context()
                        .run()
                        .ok_or(JournalError::Corrupt)?;
                    self.unknown.entry(run.clone()).or_default().insert(*intent);
                }
                self.add(
                    stored
                        .record()
                        .context()
                        .run()
                        .ok_or(JournalError::Corrupt)?,
                    EngineHistoryMessage::ToolResult(HistoricalToolResult::new(
                        *intent,
                        BoundedJson::new(output)?,
                        failed,
                    )),
                )?;
            }
            JournalRecordKind::Reconciled { intent, .. } => {
                let Frame::Reconciled { original, .. } = Frame::read(stored.record().payload())?
                else {
                    return Err(JournalError::Corrupt.into());
                };
                let original = original.restore()?;
                if original.id() != *intent
                    || Some(original.authority().run()) != stored.record().context().run()
                {
                    return Err(JournalError::Corrupt.into());
                }
                if let Some(unknown) = self.unknown.get_mut(original.authority().run()) {
                    unknown.remove(intent);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn finish(mut self, current: &RunBinding) -> Result<EngineHistory, CoordinatorError> {
        let mut messages = Vec::new();
        let mut found = false;
        for run in self.order {
            if self
                .unknown
                .get(&run)
                .is_some_and(|unknown| !unknown.is_empty())
            {
                return Err(nocterm_cloud_model::tool::ToolExecutionError::OutcomeUnknown.into());
            }
            messages.extend(self.messages.remove(&run).ok_or(JournalError::Corrupt)?);
            if &run == current {
                found = true;
                break;
            }
        }
        if !found {
            return Err(JournalError::Corrupt.into());
        }
        Ok(EngineHistory::new(messages)?)
    }
}
pub(super) async fn load(
    services: &CoordinatorServices,
    current: &RunBinding,
) -> Result<EngineHistory, CoordinatorError> {
    let mut conversation = Conversation::new();
    let replay = super::replay::JournalReplay::load(services, current.stream()).await?;
    for stored in replay.records() {
        conversation.apply(stored)?;
    }
    conversation.finish(current)
}
