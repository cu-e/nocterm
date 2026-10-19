//! Certified original native control owners become real Live graph edges atomically.
use super::{AppliedUpdate, LiveAdmission, LiveError, LiveOwner};
use crate::history::{
    physical::PresentationUpdate,
    source::{ManagedAgentInfo, ManagedOpened},
};

pub(super) struct ControlApplication<'a> {
    owner: &'a mut LiveOwner,
}
impl<'a> ControlApplication<'a> {
    pub(super) fn new(
        owner: &'a mut LiveOwner,
        admission: LiveAdmission,
    ) -> Result<Self, LiveError> {
        owner.check(&admission)?;
        Ok(Self { owner })
    }
    pub(super) fn info(self, info: ManagedAgentInfo) -> Result<AppliedUpdate, LiveError> {
        if !info.belongs_to(self.owner.live.scope()) {
            return Err(LiveError::Role);
        }
        let (value, mut liability, _source) = info.into_parts();
        self.owner
            .transition_certified(Some(&mut liability), |builder| {
                builder
                    .update_presentation(PresentationUpdate::Info(value))
                    .map(|changed| ((), changed))
            })?;
        Ok(self.owner.applied(false))
    }
    pub(super) fn opened(self, opened: ManagedOpened) -> Result<AppliedUpdate, LiveError> {
        if !opened.belongs_to(self.owner.live.scope()) {
            return Err(LiveError::Role);
        }
        let (value, mut liability, source) = opened.into_parts();
        // Only a differing session, mode or pending-history flag changes the saved document.
        let saved_changed = {
            let snapshot = self
                .owner
                .live
                .snapshot()
                .ok_or(LiveError::MissingPayload)?;
            let resume = snapshot.resume();
            let session = resume.session_id().map(|text| text.to_string());
            let mode = resume.mode().map(|text| text.to_string());
            resume.pending_history()
                || session.as_deref() != Some(value.get().session.as_str())
                || mode.as_deref()
                    != value
                        .get()
                        .modes
                        .as_ref()
                        .map(|modes| modes.current.as_str())
        };
        let copies = crate::model::native::NativeCopies::new(source.ledger());
        let session = copies
            .optional_text(Some(value.get().session.as_str()))
            .map_err(crate::history::physical::PhysicalError::from)?;
        let mode = copies
            .optional_text(
                value
                    .get()
                    .modes
                    .as_ref()
                    .map(|modes| modes.current.as_str()),
            )
            .map_err(crate::history::physical::PhysicalError::from)?;
        self.owner
            .transition_certified(Some(&mut liability), |builder| {
                builder.replace_session(session)?;
                builder.replace_mode(mode)?;
                builder.set_pending_history(false)?;
                builder.update_presentation(PresentationUpdate::Opened(value))?;
                Ok(((), true))
            })?;
        Ok(self.owner.applied(saved_changed))
    }
}
