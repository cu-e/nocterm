use super::{
    JournalError, JournalFuture, JournalPosition, PublicationWorkBudget, ResidentPublication,
    WriterLease,
};
use crate::{
    authority::TargetBinding, identity::AccountScope, run::ToolIntent, tool::BackgroundCommand,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundRecordOutcome {
    Retained,
    ObservedExit(u32),
    Unknown,
}

/// In-memory contract event encryption for internally observed job lifecycle.
/// The encoder supplies a fresh opaque event id and seals the exact new position
/// plus the original typed command and observed outcome. Native private journal
/// framing is never a source for public wire fields. It performs no network,
/// provider or target I/O.
pub trait JournalEventEncoder: Send + Sync {
    fn encode(
        &self,
        lease: &WriterLease,
        next: JournalPosition,
        original: &BackgroundCommand,
        outcome: BackgroundRecordOutcome,
        work: &dyn PublicationWorkBudget,
    ) -> Result<ResidentPublication, JournalError>;
}

#[derive(Clone, Debug)]
pub struct UnresolvedTargetEffect {
    intent: ToolIntent,
    background: Option<(String, String)>,
    unknown: bool,
}
impl UnresolvedTargetEffect {
    pub fn new(
        intent: ToolIntent,
        background: Option<(String, String)>,
        unknown: bool,
    ) -> Result<Self, JournalError> {
        if !intent.mutates()
            || background.as_ref().is_some_and(|(terminal, command)| {
                [terminal, command]
                    .into_iter()
                    .any(|id| id.is_empty() || id.len() > 128 || id.contains('\0'))
            })
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            intent,
            background,
            unknown,
        })
    }
    pub fn intent(&self) -> &ToolIntent {
        &self.intent
    }
    pub fn background(&self) -> Option<(&str, &str)> {
        self.background
            .as_ref()
            .map(|(terminal, command)| (terminal.as_str(), command.as_str()))
    }
    pub fn unknown(&self) -> bool {
        self.unknown
    }
}

pub trait TargetEffectReader: Send + Sync {
    /// Cross-run fence lookup. Compare actual host key, principal and canonical
    /// workspace within the original account scope; route/profile aliases cannot
    /// bypass it. Capacity must refuse, never silently truncate to an empty fence.
    fn unresolved_target(
        &self,
        scope: AccountScope,
        target: TargetBinding,
        limit: usize,
    ) -> JournalFuture<Vec<UnresolvedTargetEffect>>;
}
