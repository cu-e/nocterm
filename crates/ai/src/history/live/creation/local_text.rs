//! A named local text edit owns the paid copy until atomic publication.
use crate::{
    history::physical::{PhysicalError, SnapshotBuilder, SnapshotOwner, TextGuard},
    model::{
        json::deferred::DecodeLedger, native::NativeOwned, retention::document::ConstructionRole,
    },
};
#[derive(Clone, Copy)]
pub(in crate::history::live) enum TextTarget {
    Draft,
    Title,
    Name,
    Session,
    Mode,
    Model,
}
impl TextTarget {
    pub(in crate::history::live) fn current(
        self,
        snapshot: &SnapshotOwner,
    ) -> Option<TextGuard<'_>> {
        match self {
            Self::Draft => snapshot.composer().draft(),
            Self::Title => snapshot.metadata().title(),
            Self::Name => snapshot.metadata().name(),
            Self::Session => snapshot.resume().session_id(),
            Self::Mode => snapshot.resume().mode(),
            Self::Model => snapshot.metadata().model(),
        }
    }
}
pub(in crate::history::live) struct LocalTextEdit {
    value: NativeOwned<Option<String>>,
    target: TextTarget,
    updated: Option<u64>,
    pinned: bool,
}
impl LocalTextEdit {
    pub(in crate::history::live) fn prepare(
        role: &ConstructionRole,
        text: Option<&str>,
        target: TextTarget,
        updated: Option<u64>,
        pinned: bool,
    ) -> Result<Self, PhysicalError> {
        let ledger = DecodeLedger::for_operation(role, crate::history::MAX_RESIDENT_BYTES)?;
        let allocation = ledger.reserve(text.map_or(0, str::len))?;
        let value = if let Some(text) = text {
            let mut value = String::new();
            value.try_reserve_exact(text.len())?;
            if value.capacity() != text.len() {
                return Err(PhysicalError::CapacityProfile);
            }
            value.push_str(text);
            Some(value)
        } else {
            None
        };
        Ok(Self {
            value: NativeOwned::from_parts(value, allocation),
            target,
            updated,
            pinned,
        })
    }
    pub(in crate::history::live) fn apply(
        self,
        builder: &mut SnapshotBuilder<'_>,
    ) -> Result<(), PhysicalError> {
        match self.target {
            TextTarget::Draft => builder.replace_draft(self.value)?,
            TextTarget::Title => builder.replace_title(self.value)?,
            TextTarget::Name => builder.rename(self.value)?,
            TextTarget::Session => builder.replace_session(self.value)?,
            TextTarget::Mode => builder.replace_mode(self.value)?,
            TextTarget::Model => builder.replace_model(self.value)?,
        }
        if let Some(updated) = self.updated {
            builder.stamp_metadata(updated, self.pinned)?;
        }
        Ok(())
    }
}
