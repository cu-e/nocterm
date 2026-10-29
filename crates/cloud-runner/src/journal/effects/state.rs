use crate::codec::record::{Intent, Stamp};
use nocterm_cloud_model::{
    journal::{BackgroundRecordOutcome, JournalError, UnresolvedTargetEffect},
    run::{CommitStamp, EffectOutcome, ToolIntent},
    tool::BackgroundCommand,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum State {
    Prepared,
    Dispatched,
    Unknown,
    Resolved,
    Declined,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Effect {
    intent: Intent,
    dispatch: Option<Stamp>,
    state: State,
    background: Option<(String, String)>,
    exit: Option<u32>,
    foreground_recorded: bool,
}
impl Effect {
    pub(super) fn prepared(intent: &ToolIntent) -> Self {
        Self {
            intent: intent.into(),
            dispatch: None,
            state: State::Prepared,
            background: None,
            exit: None,
            foreground_recorded: false,
        }
    }
    pub(super) fn intent(&self) -> Result<ToolIntent, JournalError> {
        self.intent.restore_ref()
    }
    pub(super) fn status(&self) -> u8 {
        match self.state {
            State::Prepared => 0,
            State::Dispatched => 1,
            State::Unknown => 2,
            State::Resolved => 3,
            State::Declined => 4,
        }
    }
    pub(super) fn dispatch(&mut self, stamp: &CommitStamp) -> Result<(), JournalError> {
        if self.state != State::Prepared || self.intent()?.authority().run() != stamp.run() {
            return Err(JournalError::Conflict);
        }
        self.dispatch = Some(stamp.into());
        self.state = State::Dispatched;
        Ok(())
    }
    pub(super) fn declined(&mut self, stamp: &CommitStamp) -> Result<(), JournalError> {
        self.require_run(stamp)?;
        if self.state != State::Prepared || self.dispatch.is_some() {
            return Err(JournalError::Conflict);
        }
        self.state = State::Declined;
        self.foreground_recorded = true;
        Ok(())
    }
    pub(super) fn outcome(
        &mut self,
        stamp: &CommitStamp,
        outcome: EffectOutcome,
    ) -> Result<(), JournalError> {
        self.require_run(stamp)?;
        if self.foreground_recorded {
            return Err(JournalError::Conflict);
        }
        let dispatch = self
            .dispatch
            .as_ref()
            .ok_or(JournalError::Conflict)?
            .restore_ref()?;
        if matches!(outcome, EffectOutcome::Known(_)) && dispatch.writer() != stamp.writer() {
            return Err(JournalError::Fenced);
        }
        if self.background.is_none() {
            if self.state != State::Dispatched {
                return Err(JournalError::Conflict);
            }
            self.state = if outcome == EffectOutcome::Unknown && self.intent()?.mutates() {
                State::Unknown
            } else {
                State::Resolved
            };
        } else if outcome == EffectOutcome::Unknown && self.state != State::Resolved {
            self.state = State::Unknown;
        }
        self.foreground_recorded = true;
        Ok(())
    }
    pub(super) fn reconciled(&mut self) -> Result<(), JournalError> {
        if self.state != State::Unknown {
            return Err(JournalError::Conflict);
        }
        self.state = State::Resolved;
        Ok(())
    }
    pub(super) fn declared_reconciled(
        &mut self,
        current_writer: nocterm_cloud_model::identity::WriterEpoch,
    ) -> Result<(), JournalError> {
        if self.state != State::Unknown
            && !(self.state == State::Dispatched && self.writer_was_replaced(current_writer)?)
        {
            return Err(JournalError::Conflict);
        }
        self.state = State::Resolved;
        Ok(())
    }
    pub(super) fn background(
        &mut self,
        command: &BackgroundCommand,
        outcome: BackgroundRecordOutcome,
    ) -> Result<bool, JournalError> {
        self.require_command(command)?;
        let ids = (command.terminal().to_owned(), command.command().to_owned());
        match outcome {
            BackgroundRecordOutcome::Retained => {
                if let Some(original) = &self.background {
                    return if original == &ids {
                        Ok(false)
                    } else {
                        Err(JournalError::Conflict)
                    };
                }
                if self.state != State::Dispatched {
                    return Err(JournalError::Conflict);
                }
                self.background = Some(ids);
            }
            BackgroundRecordOutcome::Unknown => {
                if self.background.as_ref() != Some(&ids) {
                    return Err(JournalError::Conflict);
                }
                if self.state == State::Resolved {
                    return Err(JournalError::Conflict);
                }
                if self.state == State::Unknown {
                    return Ok(false);
                }
                self.state = State::Unknown;
            }
            BackgroundRecordOutcome::ObservedExit(exit) => {
                if self.background.as_ref() != Some(&ids) {
                    return Err(JournalError::Conflict);
                }
                if let Some(original) = self.exit {
                    return if original == exit {
                        Ok(false)
                    } else {
                        Err(JournalError::Conflict)
                    };
                }
                self.exit = Some(exit);
                self.state = State::Resolved;
            }
        }
        Ok(true)
    }
    pub(super) fn unresolved(&self) -> Result<Option<UnresolvedTargetEffect>, JournalError> {
        if !matches!(self.state, State::Dispatched | State::Unknown) {
            return Ok(None);
        }
        let intent = self.intent()?;
        if !intent.mutates() {
            return Ok(None);
        }
        Ok(Some(UnresolvedTargetEffect::new(
            intent,
            self.background.clone(),
            self.state == State::Unknown,
        )?))
    }
    pub(super) fn writer_was_replaced(
        &self,
        writer: nocterm_cloud_model::identity::WriterEpoch,
    ) -> Result<bool, JournalError> {
        let original = self
            .dispatch
            .as_ref()
            .ok_or(JournalError::Corrupt)?
            .restore_ref()?;
        if original.writer() > writer {
            return Err(JournalError::Corrupt);
        }
        Ok(original.writer() < writer)
    }
    pub(super) fn validate(&self) -> Result<(), JournalError> {
        let intent = self.intent()?;
        match (&self.dispatch, self.state) {
            (None, State::Prepared)
                if self.background.is_none()
                    && self.exit.is_none()
                    && !self.foreground_recorded =>
            {
                Ok(())
            }
            (None, State::Declined)
                if self.background.is_none() && self.exit.is_none() && self.foreground_recorded =>
            {
                Ok(())
            }
            (Some(stamp), state) if !matches!(state, State::Prepared | State::Declined) => {
                let stamp = stamp.restore_ref()?;
                if stamp.run() != intent.authority().run() || stamp.revision().get() == 0 {
                    return Err(JournalError::Corrupt);
                }
                if self.exit.is_some()
                    && (self.background.is_none() || self.state != State::Resolved)
                {
                    return Err(JournalError::Corrupt);
                }
                if self.background.as_ref().is_some_and(|(terminal, command)| {
                    [terminal, command]
                        .into_iter()
                        .any(|id| id.is_empty() || id.len() > 128 || id.contains('\0'))
                }) {
                    return Err(JournalError::Corrupt);
                }
                Ok(())
            }
            _ => Err(JournalError::Corrupt),
        }
    }
    fn require_run(&self, stamp: &CommitStamp) -> Result<(), JournalError> {
        if self.intent()?.authority().run() != stamp.run() {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }
    fn require_command(&self, command: &BackgroundCommand) -> Result<(), JournalError> {
        let dispatch = self
            .dispatch
            .as_ref()
            .ok_or(JournalError::Conflict)?
            .restore_ref()?;
        if &self.intent()? != command.dispatch().intent()
            || &dispatch != command.dispatch().commit()
        {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }
}
