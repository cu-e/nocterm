//! Host-borrowed values are copied into the original Foreground ledger before
//! any Live transition is attempted, so a refusal leaves graph and revision as-is.
use super::{AppliedUpdate, LiveAdmission, LiveError, LiveOwner, LiveRouted};
use crate::{
    history::{dispatch::DispatchFraming, physical::PhysicalError},
    model::{
        controls::ConfigOption,
        ids::{ModeId, SessionId},
        json::deferred::DecodeLedger,
        native::{NativeCopies, NativeOwned},
        tools::ClientDisplay,
    },
};

/// Short-lived pairing of one Live owner and its prepaid operation ledger.
pub struct LiveCopies<'a> {
    owner: &'a mut LiveOwner,
    ledger: DecodeLedger,
}
impl<'a> LiveCopies<'a> {
    pub fn new(owner: &'a mut LiveOwner) -> Result<Self, LiveError> {
        owner.foreground.ensure_live_phase()?;
        let ledger =
            DecodeLedger::for_operation(&owner.foreground, crate::history::MAX_RESIDENT_BYTES)
                .map_err(PhysicalError::from)?;
        Ok(Self { owner, ledger })
    }
    /// Request framing for the queued prompt, copied while the original queue is intact.
    pub fn framing(
        self,
        session: &SessionId,
        context: Option<&str>,
        history: Option<&str>,
        command: bool,
    ) -> Result<DispatchFraming, LiveError> {
        let copies = NativeCopies::new(&self.ledger);
        let session = crate::model::native::controls::session_id(
            copies.text(session.as_str()).map_err(physical)?,
        )
        .map_err(physical)?;
        Ok(DispatchFraming::new(
            session,
            copies.optional_text(context).map_err(physical)?,
            copies.optional_text(history).map_err(physical)?,
            command,
        ))
    }
    pub fn patch_display(
        self,
        admission: LiveAdmission,
        row: usize,
        display: &ClientDisplay,
    ) -> Result<AppliedUpdate, LiveError> {
        let display = NativeCopies::new(&self.ledger)
            .client_display(display)
            .map_err(physical)?;
        self.owner.patch_display(admission, row, display)
    }
    pub fn configuration(
        self,
        admission: LiveAdmission,
        options: NativeOwned<Vec<ConfigOption>>,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        self.applied(
            admission,
            crate::model::native::notice::configuration(options),
            now,
        )
    }
    pub fn mode(
        self,
        admission: LiveAdmission,
        mode: &ModeId,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        let text = NativeCopies::new(&self.ledger)
            .text(mode.as_str())
            .map_err(physical)?;
        let mode = crate::model::native::controls::mode_id(text).map_err(physical)?;
        self.applied(admission, crate::model::native::notice::mode(mode), now)
    }
    /// The resume descriptor's working directory, copied from a borrowed path.
    pub fn workdir(
        self,
        admission: LiveAdmission,
        value: Option<&std::path::Path>,
    ) -> Result<AppliedUpdate, LiveError> {
        let value = NativeCopies::new(&self.ledger)
            .optional_path(value)
            .map_err(physical)?;
        self.owner.replace_resume_workdir(admission, value)
    }
    /// The chat's recorded model; an unchanged value publishes nothing.
    pub fn model(
        self,
        admission: LiveAdmission,
        value: Option<&str>,
    ) -> Result<AppliedUpdate, LiveError> {
        drop(self.ledger);
        self.owner
            .edit_text(admission, value, super::creation::TextTarget::Model, None)
    }
    /// One chosen configuration value of the resume descriptor.
    pub fn choice(
        self,
        admission: LiveAdmission,
        key: &str,
        value: &crate::session_config::ConfigValue,
    ) -> Result<AppliedUpdate, LiveError> {
        let copies = NativeCopies::new(&self.ledger);
        let key = copies.text(key).map_err(physical)?;
        let value = copies.config_value(value).map_err(physical)?;
        self.owner.replace_resume_choice(admission, key, value)
    }
    fn applied(
        self,
        admission: LiveAdmission,
        update: NativeOwned<crate::model::notice::SessionUpdate>,
        now: u64,
    ) -> Result<AppliedUpdate, LiveError> {
        match self.owner.apply_owned(admission, update, now)? {
            LiveRouted::Applied(applied) => Ok(applied),
            LiveRouted::Forward(update) => {
                drop(update);
                Err(LiveError::Role)
            }
        }
    }
}
fn physical(error: crate::model::json::deferred::JsonDecodeError) -> PhysicalError {
    PhysicalError::from(error)
}
