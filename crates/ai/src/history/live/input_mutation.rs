//! A scoped input transaction owns exclusive access to the original Live publication.
use super::{AppliedUpdate, LiveAdmission, LiveError, LiveOwner};
use crate::{history::physical::ComposerInputEdit, model::native::NativeOwned};

pub(super) struct InputMutation<'a> {
    owner: &'a mut LiveOwner,
}
impl<'a> InputMutation<'a> {
    pub(super) fn attachments(
        self,
        edit: super::AttachmentEdit<'_>,
        locals: &[super::LocalAttachmentFact<'_>],
        updated: u64,
    ) -> Result<(AppliedUpdate, bool), LiveError> {
        let snapshot = self
            .owner
            .live
            .snapshot()
            .ok_or(LiveError::MissingPayload)?;
        let preparation = super::creation::AttachmentPreparation::new(
            snapshot.live_inputs(),
            snapshot.composer().attachments(),
            edit,
            locals,
        );
        if !preparation.changed() {
            return Ok((self.owner.applied(false), false));
        }
        let pinned = snapshot.metadata().pinned();
        self.owner.foreground.ensure_live_phase()?;
        let (live, saved, removed) = preparation.prepare(&self.owner.foreground)?.into_parts();
        let (saved_changed, _) = self.owner.transition(|builder| {
            let changed = builder.replace_input_scope(live, saved)?;
            if changed {
                builder.stamp_metadata(updated, pinned)?;
            }
            Ok((changed, true))
        })?;
        Ok((self.owner.applied(saved_changed), removed))
    }
    pub(super) fn new(
        owner: &'a mut LiveOwner,
        admission: LiveAdmission,
    ) -> Result<Self, LiveError> {
        owner.check(&admission)?;
        Ok(Self { owner })
    }
    pub(super) fn edit(self, edit: ComposerInputEdit) -> Result<AppliedUpdate, LiveError> {
        self.owner.foreground.ensure_live_phase()?;
        self.owner
            .transition(|builder| builder.edit_inputs(edit).map(|changed| ((), changed)))?;
        Ok(self.owner.applied(false))
    }
    pub(super) fn replace_attachments(
        self,
        value: NativeOwned<Vec<super::inputs::LiveAttachment>>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.owner.transition(|builder| {
            builder
                .replace_live_attachments(value)
                .map(|changed| ((), changed))
        })?;
        Ok(self.owner.applied(false))
    }
    pub(super) fn replace_images(
        self,
        value: NativeOwned<Vec<crate::images::managed::PreparedPromptImage>>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.owner
            .transition(|builder| builder.replace_images(value).map(|()| ((), true)))?;
        Ok(self.owner.applied(false))
    }
    pub(super) fn add_image(
        self,
        value: NativeOwned<crate::images::managed::PreparedPromptImage>,
    ) -> Result<AppliedUpdate, LiveError> {
        self.owner.foreground.ensure_live_phase()?;
        self.owner
            .transition(|builder| builder.add_prepared_image(value).map(|()| ((), true)))?;
        Ok(self.owner.applied(false))
    }
}
