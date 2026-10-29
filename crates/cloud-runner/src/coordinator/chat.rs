//! One writer and bounded foreground records for an exact account/chat.
use super::{frame::Frame, log::ChatLog, run::RunRecord};
use futures::lock::Mutex;
use nocterm_cloud_model::{authority::RunBinding, coordinator::*, journal::*};
use parking_lot::Mutex as SyncMutex;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Chat {
    log: Arc<ChatLog>,
    commands: Mutex<()>,
    runs: SyncMutex<BTreeMap<RunBinding, Arc<RunRecord>>>,
    deleted: AtomicBool,
}
impl Chat {
    pub(super) fn new(log: ChatLog) -> Self {
        Self {
            log: Arc::new(log),
            commands: Mutex::new(()),
            runs: SyncMutex::new(BTreeMap::new()),
            deleted: AtomicBool::new(false),
        }
    }
    pub(super) fn log(&self) -> &Arc<ChatLog> {
        &self.log
    }
    pub(super) fn commands(&self) -> &Mutex<()> {
        &self.commands
    }
    pub(super) fn run(&self, binding: &RunBinding) -> Result<Arc<RunRecord>, CoordinatorError> {
        self.runs
            .lock()
            .get(binding)
            .cloned()
            .ok_or(CoordinatorError::Conflict)
    }
    pub(super) fn runs(&self) -> Vec<Arc<RunRecord>> {
        self.runs.lock().values().cloned().collect()
    }
    pub(super) fn insert(&self, run: Arc<RunRecord>) -> Result<(), CoordinatorError> {
        let mut runs = self.runs.lock();
        if runs.len() >= 4096 || runs.contains_key(run.binding()) {
            return Err(CoordinatorError::Overloaded);
        }
        runs.insert(run.binding().clone(), run);
        Ok(())
    }
    pub(super) fn delete(&self) {
        self.deleted.store(true, Ordering::Release);
        for run in self.runs() {
            run.control().stop();
        }
    }
    pub(super) fn check(&self) -> Result<(), CoordinatorError> {
        if self.deleted.load(Ordering::Acquire) {
            Err(CoordinatorError::Stopped)
        } else {
            Ok(())
        }
    }
    pub(super) async fn accept(
        &self,
        command: &ReceivedCommand,
        allocated: Option<&RunBinding>,
        extra: Vec<(JournalRecord, Option<CoordinatorEvent>)>,
        work: &super::intake::PublicationWork,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let context = allocated
            .or(command.run())
            .map_or(RecordContext::Chat, |run| RecordContext::Run(run.clone()));
        let (body, allocation) = work.encode(&Frame::accepted(command, allocated))?;
        let record = JournalRecord::new(
            context,
            JournalRecordKind::CommandAccepted {
                id: command.acceptance().id(),
                kind: command.acceptance().kind(),
            },
            body,
        )?;
        let event = match (command.body(), allocated) {
            (CommandBody::Submit(request), Some(run)) => CoordinatorEvent::PromptAccepted {
                command: command.acceptance().clone(),
                run: run.clone(),
                request: nocterm_cloud_model::coordinator::ObservedPrompt::from_request(request),
            },
            _ => CoordinatorEvent::CommandAccepted {
                command: command.acceptance().clone(),
                run: command.run().cloned(),
            },
        };
        let mut records = vec![(record, Some(event))];
        records.extend(extra);
        match self
            .log
            .append_resident(command.acceptance().clone(), records, allocation)
            .await?
        {
            CommitOutcome::Duplicate(receipt) => Ok(*receipt),
            CommitOutcome::Committed(receipt) => receipt
                .command()
                .cloned()
                .ok_or_else(|| JournalError::Corrupt.into()),
        }
    }
}
