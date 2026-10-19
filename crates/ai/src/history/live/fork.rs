//! A fork aliases the rows of one document into another document's roles,
//! then gives the copy its own identity, name, resume facts and empty composer.
use super::{ForegroundMutation, LiveError, LiveOwner};
use crate::{
    history::{
        ComposerData,
        physical::{ComposerInputEdit, PhysicalError, SnapshotBuilder},
    },
    model::{
        json::deferred::{DecodeLedger, JsonDecodeError},
        native::{NativeCopies, NativeList, NativeOwned},
    },
};

/// Caller-supplied identity facts of the new document.
pub struct ForkFacts<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// Rows kept from the source; later rows stay out of the fork.
    pub len: usize,
    pub pending_history: bool,
    pub now: u64,
}
pub struct LiveFork<'a> {
    target: &'a mut LiveOwner,
    source: &'a LiveOwner,
}
impl<'a> LiveFork<'a> {
    pub fn new(target: &'a mut LiveOwner, source: &'a LiveOwner) -> Self {
        Self { target, source }
    }
    pub fn adopt(self, facts: ForkFacts<'_>) -> Result<(), LiveError> {
        let Self { target, source } = self;
        target.foreground.ensure_live_phase()?;
        let foreground = ForegroundMutation(&mut target.foreground);
        let staged = (|| {
            let mut builder = SnapshotBuilder::from_fork_role(foreground.0, &source.live)?;
            builder.truncate_rows(facts.len)?;
            let staged = builder.finish(None)?;
            crate::history::decision_managed(staged.snapshot())?;
            target.live.replace(staged)?;
            Ok::<_, LiveError>(())
        })();
        drop(foreground);
        staged?;
        target.revision += 1;
        let ledger =
            DecodeLedger::for_operation(&target.foreground, crate::history::MAX_RESIDENT_BYTES)
                .map_err(PhysicalError::from)?;
        let identity = Identity::copy(&ledger, &facts).map_err(PhysicalError::from)?;
        target.transition(|builder| {
            builder.replace_id(identity.id)?;
            builder.rename(identity.name)?;
            builder.replace_session(identity.session)?;
            builder.replace_workdir(identity.workdir)?;
            builder.set_pending_history(facts.pending_history)?;
            builder.stamp_metadata(facts.now, false)?;
            builder.replace_composer(identity.composer)?;
            builder.edit_inputs(ComposerInputEdit::Reset)?;
            Ok(((), true))
        })?;
        Ok(())
    }
}
struct Identity {
    id: NativeOwned<String>,
    name: NativeOwned<Option<String>>,
    session: NativeOwned<Option<String>>,
    workdir: NativeOwned<Option<std::path::PathBuf>>,
    composer: NativeOwned<ComposerData>,
}
impl Identity {
    fn copy(ledger: &DecodeLedger, facts: &ForkFacts<'_>) -> Result<Self, JsonDecodeError> {
        let copies = NativeCopies::new(ledger);
        Ok(Self {
            id: copies.text(facts.id)?,
            name: copies.optional_text(Some(facts.name))?,
            session: NativeOwned::absent(ledger)?,
            workdir: NativeOwned::absent(ledger)?,
            composer: crate::history::native::document::composer(
                NativeOwned::absent(ledger)?,
                NativeList::new(ledger, 0)?.finish(),
                NativeList::new(ledger, 0)?.finish(),
            )?,
        })
    }
}
